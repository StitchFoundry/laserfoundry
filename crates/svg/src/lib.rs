//! SVG import: shapes, groups and transforms are resolved by `usvg`, then converted to mm paths.
//! Text is turned into outlines with the fonts installed on the machine (when there are any), and
//! embedded raster pictures are returned as encoded bytes for the caller to decode.

use laserfoundry_geom::{Affine, BezPath};
use thiserror::Error;
use usvg::tiny_skia_path::PathSegment;

/// CSS pixels (96 per inch) to millimetres.
const MM_PER_PX: f64 = 25.4 / 96.0;
/// Largest SVG we will parse.
pub const MAX_SVG_BYTES: usize = 50 * 1024 * 1024;
/// Most bytes of text (all `<text>` elements together) we will lay out. Text layout takes time that grows faster than the
/// length (about 0.6 s at 20,000 characters, 7 s at 80,000 and minutes past a million), so a file full of text is refused.
pub const MAX_TEXT_BYTES: usize = 40_000;
/// Cap on imported paths.
pub const MAX_PATHS: usize = 100_000;

#[derive(Debug, Error)]
pub enum SvgError {
    #[error("SVG is larger than {} MB", MAX_SVG_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("SVG has more than {MAX_TEXT_BYTES} bytes of text; convert the text to paths first")]
    TooMuchText,
    #[error("could not parse SVG: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedPath {
    pub name: String,
    /// In mm, y down, relative to the SVG's top-left corner.
    pub path: BezPath,
    /// `#rrggbb` of the stroke (preferred) or fill, when the SVG specifies one.
    pub color: Option<String>,
    pub filled: bool,
}

/// An embedded raster picture (PNG, JPEG, GIF or WebP data).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedImage {
    pub name: String,
    pub bytes: Vec<u8>,
    /// The picture's size in mm and where it sits: maps its own mm space (0..width, 0..height) onto
    /// the SVG's mm space (y down, from the top-left corner).
    pub width_mm: f64,
    pub height_mm: f64,
    pub transform: Affine,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SvgImport {
    pub width_mm: f64,
    pub height_mm: f64,
    pub paths: Vec<ImportedPath>,
    pub images: Vec<ImportedImage>,
    /// `<text>` items drawn by the caller's [`TextOutliner`] (the machine has no font files, as in a browser).
    pub text_outlined: usize,
    /// Of those, the items drawn in a stand-in because the requested font was not available.
    pub text_substituted: usize,
    /// `<text>` items with words that could not be drawn at all (no font anywhere).
    pub text_dropped: usize,
}

/// What a text outliner is asked to draw: one run of text in one style.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRequest {
    pub text: String,
    /// The `font-family` list, in order of preference (generic names like `sans-serif` included).
    pub families: Vec<String>,
    pub bold: bool,
    pub italic: bool,
    /// The font size in CSS pixels.
    pub size_px: f64,
}

/// The outline of one run: in CSS pixels, y down, origin at the left end of the baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct TextOutline {
    pub path: BezPath,
    pub advance_px: f64,
    /// True when none of the requested families was available and a stand-in was used.
    pub substituted: bool,
}

/// Draws text where `usvg` cannot (no font files, as in a browser). Returns `None` when it has no font at all.
pub trait TextOutliner {
    fn outline(&self, request: &TextRequest) -> Option<TextOutline>;
}

/// Cap on imported pictures and on the bytes they may add up to.
pub const MAX_IMAGES: usize = 64;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

pub fn import_svg(data: &[u8]) -> Result<SvgImport, SvgError> {
    import_svg_with_text(data, None)
}

/// Like [`import_svg`]; text `usvg` could not draw (no fonts) is drawn by `outliner` when one is given, and counted
/// in the result either way.
pub fn import_svg_with_text(data: &[u8], outliner: Option<&dyn TextOutliner>) -> Result<SvgImport, SvgError> {
    import_inner(data, outliner, true)
}

/// Like [`import_svg_with_text`] but never reads the machine's installed fonts, so the result is the same on every
/// machine (what a browser sees). For tests.
pub fn import_svg_without_system_fonts(data: &[u8], outliner: Option<&dyn TextOutliner>) -> Result<SvgImport, SvgError> {
    import_inner(data, outliner, false)
}

fn import_inner(data: &[u8], outliner: Option<&dyn TextOutliner>, system_fonts: bool) -> Result<SvgImport, SvgError> {
    if data.len() > MAX_SVG_BYTES {
        return Err(SvgError::TooLarge);
    }
    if text_bytes(data) > MAX_TEXT_BYTES {
        return Err(SvgError::TooMuchText);
    }
    let mut options = usvg::Options::default();
    // Installed fonts are only looked up when the file has text to draw (loading them takes a while).
    if system_fonts && data.windows(5).any(|w| w == b"<text") {
        load_system_fonts(&mut options);
    }
    // No fonts at all (a browser): `usvg` drops every <text>, so each one is swapped for a placeholder shape that keeps
    // its place in the transform tree, and drawn afterwards with the caller's outliner.
    let mut texts = Vec::new();
    let replaced;
    let mut data = data;
    if options.fontdb.is_empty()
        && data.windows(5).any(|w| w == b"<text")
        && let Some((svg, found)) = swap_text_for_placeholders(data)
    {
        replaced = svg;
        data = replaced.as_slice();
        texts = found;
    }
    let tree = usvg::Tree::from_data(data, &options).map_err(|e| SvgError::Parse(e.to_string()))?;
    let size = tree.size();
    let mut found = Found { outliner, texts, ..Found::default() };
    collect(tree.root(), &mut found);
    Ok(SvgImport {
        width_mm: f64::from(size.width()) * MM_PER_PX,
        height_mm: f64::from(size.height()) * MM_PER_PX,
        paths: found.paths,
        images: found.images,
        text_outlined: found.text_outlined,
        text_substituted: found.text_substituted,
        text_dropped: found.text_dropped,
    })
}

/// Bytes between each `<text` and its `</text>` (to the end of the data when there is none).
fn text_bytes(data: &[u8]) -> usize {
    let find = |from: usize, needle: &[u8]| data.get(from..).and_then(|rest| rest.windows(needle.len()).position(|w| w == needle)).map(|i| from + i);
    let mut total = 0usize;
    let mut at = 0usize;
    while let Some(start) = find(at, b"<text") {
        let end = find(start, b"</text>").map_or(data.len(), |e| e + b"</text>".len());
        total = total.saturating_add(end - start);
        at = end;
    }
    total
}

#[cfg(not(target_arch = "wasm32"))]
fn load_system_fonts(options: &mut usvg::Options<'_>) {
    std::sync::Arc::make_mut(&mut options.fontdb).load_system_fonts();
}

/// A browser has no font files to read.
#[cfg(target_arch = "wasm32")]
fn load_system_fonts(_: &mut usvg::Options<'_>) {}

#[derive(Default)]
struct Found<'a> {
    paths: Vec<ImportedPath>,
    images: Vec<ImportedImage>,
    image_bytes: usize,
    outliner: Option<&'a dyn TextOutliner>,
    texts: Vec<TextChunk>,
    text_outlined: usize,
    text_substituted: usize,
    text_dropped: usize,
}

/// Ids of the shapes that stand in for `<text>` while `usvg` reads the file.
const PLACEHOLDER: &str = "lf-text-";
/// Most text runs taken from one file (each is laid out separately).
const MAX_TEXT_CHUNKS: usize = 5_000;

/// One piece of text that starts at a point: its runs (each in one style) and how it hangs on the point.
#[derive(Debug, Clone)]
struct TextChunk {
    x: f64,
    y: f64,
    /// -1 start, 0 middle, 1 end.
    anchor: i8,
    runs: Vec<(TextRequest, Option<String>)>,
}

/// Value of a presentation property on an element or the nearest ancestor that sets it (`style` wins over the attribute).
fn inherited(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    for n in node.ancestors().filter(|n| n.is_element()) {
        let from_style = n
            .attribute("style")
            .and_then(|st| st.split(';').filter_map(|d| d.split_once(':')).find(|(k, _)| k.trim() == name).map(|(_, v)| v.trim().to_string()));
        if let Some(v) = from_style.or_else(|| n.attribute(name).map(|v| v.trim().to_string())) {
            return Some(v);
        }
    }
    None
}

/// The first number of a coordinate list (`"10 20"`, `"5px"`); other units are not worked out.
fn first_number(v: &str) -> Option<f64> {
    let t = v.split(|c: char| c == ',' || c.is_whitespace()).find(|p| !p.is_empty())?;
    let (num, unit) = match t.find(|c: char| c.is_ascii_alphabetic() || c == '%') {
        Some(i) => t.split_at(i),
        None => (t, ""),
    };
    let n: f64 = num.parse().ok()?;
    let scale = match unit {
        "" | "px" => 1.0,
        "pt" => 4.0 / 3.0,
        "mm" => 96.0 / 25.4,
        "cm" => 96.0 / 2.54,
        "in" => 96.0,
        _ => return None,
    };
    (n * scale).is_finite().then_some(n * scale)
}

/// Finds the `<text>` elements, returns the file with each replaced by placeholder shapes (one per chunk of text, same
/// `transform`, so ancestors' transforms still apply) and the chunks, indexed by placeholder number.
fn swap_text_for_placeholders(data: &[u8]) -> Option<(Vec<u8>, Vec<TextChunk>)> {
    let src = std::str::from_utf8(data).ok()?;
    let doc = roxmltree::Document::parse(src).ok()?;
    let is_text = |n: &roxmltree::Node| n.is_element() && n.tag_name().name() == "text";
    let mut chunks: Vec<TextChunk> = Vec::new();
    let mut out = Vec::with_capacity(data.len());
    let mut at = 0usize;
    for el in doc.descendants().filter(|n| is_text(n) && !n.ancestors().skip(1).any(|a| is_text(&a))) {
        let range = el.range();
        if range.start < at || chunks.len() >= MAX_TEXT_CHUNKS {
            continue;
        }
        let first = chunks.len();
        text_chunks(el, &mut chunks);
        out.extend_from_slice(src.get(at..range.start)?.as_bytes());
        let transform = el.attribute("transform").unwrap_or("");
        for i in first..chunks.len() {
            out.extend_from_slice(
                format!(r#"<rect id="{PLACEHOLDER}{i}" width="1" height="1" transform="{}"/>"#, transform.replace('"', "")).as_bytes(),
            );
        }
        at = range.end;
    }
    out.extend_from_slice(src.get(at..)?.as_bytes());
    (!chunks.is_empty()).then_some((out, chunks))
}

/// Splits one `<text>` element into chunks: a new one starts wherever an element sets `x` or `y`.
fn text_chunks(text_el: roxmltree::Node<'_, '_>, chunks: &mut Vec<TextChunk>) {
    let mut started: Vec<roxmltree::NodeId> = Vec::new();
    for node in text_el.descendants().filter(|n| n.is_text()) {
        let Some(parent) = node.parent_element() else { continue };
        let Some(piece) = node.text().map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")) else { continue };
        if piece.is_empty() {
            continue;
        }
        // Between this text and the <text> element: the nearest element with a new position starts a chunk.
        let chain: Vec<_> = parent.ancestors().take_while(|a| a.is_element()).take_while(|a| *a != text_el).chain(std::iter::once(text_el)).collect();
        let mut new_start = false;
        for a in &chain {
            if (a.has_attribute("x") || a.has_attribute("y")) && !started.contains(&a.id()) {
                started.push(a.id());
                new_start = true;
            }
        }
        if new_start || chunks.is_empty() || chunks.last().is_none_or(|c| c.runs.is_empty()) {
            let find = |name: &str| chain.iter().find_map(|a| a.attribute(name)).and_then(first_number).unwrap_or(0.0);
            let anchor = match inherited(parent, "text-anchor").as_deref() {
                Some("middle") => 0,
                Some("end") => 1,
                _ => -1,
            };
            chunks.push(TextChunk { x: find("x"), y: find("y"), anchor, runs: Vec::new() });
        }
        let families = inherited(parent, "font-family")
            .map(|f| f.split(',').map(|n| n.trim().trim_matches(|c| c == '"' || c == '\'').to_string()).filter(|n| !n.is_empty()).collect())
            .unwrap_or_default();
        let weight = inherited(parent, "font-weight").unwrap_or_default();
        let request = TextRequest {
            text: piece,
            families,
            bold: weight == "bold" || weight == "bolder" || weight.parse::<u32>().is_ok_and(|w| w >= 600),
            italic: matches!(inherited(parent, "font-style").as_deref(), Some("italic" | "oblique")),
            size_px: inherited(parent, "font-size").and_then(|v| first_number(&v)).filter(|s| *s > 0.0).unwrap_or(16.0),
        };
        let color = inherited(parent, "fill").and_then(|f| usvg_hex(&f));
        if let Some(last) = chunks.last_mut() {
            last.runs.push((request, color));
        }
    }
}

/// `#rgb` / `#rrggbb` as `#rrggbb`; anything else (names, `none`, urls) gives `None`.
fn usvg_hex(v: &str) -> Option<String> {
    let h = v.strip_prefix('#')?;
    match h.len() {
        6 if h.chars().all(|c| c.is_ascii_hexdigit()) => Some(format!("#{}", h.to_lowercase())),
        3 if h.chars().all(|c| c.is_ascii_hexdigit()) => Some(format!("#{}", h.chars().flat_map(|c| [c, c]).collect::<String>().to_lowercase())),
        _ => None,
    }
}

/// Draws the text a placeholder stands for, where the placeholder sits.
fn place_text(p: &usvg::Path, found: &mut Found) {
    let Some(chunk) = p.id().strip_prefix(PLACEHOLDER).and_then(|n| n.parse::<usize>().ok()).and_then(|n| found.texts.get(n)).cloned() else {
        return;
    };
    let ts = p.abs_transform();
    let m = Affine::new([ts.sx as f64, ts.ky as f64, ts.kx as f64, ts.sy as f64, ts.tx as f64, ts.ty as f64]);
    let mut drawn: Vec<(BezPath, f64, Option<String>)> = Vec::new();
    let (mut width, mut substituted) = (0.0, false);
    if let Some(outliner) = found.outliner {
        for (request, color) in &chunk.runs {
            let Some(out) = outliner.outline(request) else { continue };
            if !(out.advance_px.is_finite() && out.advance_px >= 0.0) {
                continue;
            }
            substituted |= out.substituted;
            drawn.push((out.path, width, color.clone()));
            width += out.advance_px;
        }
    }
    if drawn.is_empty() {
        found.text_dropped += 1;
        return;
    }
    let x0 = chunk.x + (f64::from(chunk.anchor) + 1.0) * -width / 2.0;
    for (mut path, offset, color) in drawn {
        if found.paths.len() >= MAX_PATHS {
            return;
        }
        path.apply_affine(Affine::scale(MM_PER_PX) * m * Affine::translate((x0 + offset, chunk.y)));
        let name = format!("text {}", found.paths.len() + 1);
        found.paths.push(ImportedPath { name, path, color, filled: true });
    }
    found.text_outlined += 1;
    if substituted {
        found.text_substituted += 1;
    }
}

fn collect(group: &usvg::Group, found: &mut Found) {
    for node in group.children() {
        if found.paths.len() >= MAX_PATHS {
            return;
        }
        let out = &mut found.paths;
        match node {
            usvg::Node::Group(g) => collect(g, found),
            // Text arrives already converted to outlines (an empty group without fonts).
            usvg::Node::Text(t) => collect(t.flattened(), found),
            usvg::Node::Image(img) => {
                let bytes = match img.kind() {
                    usvg::ImageKind::PNG(d) | usvg::ImageKind::JPEG(d) | usvg::ImageKind::GIF(d) | usvg::ImageKind::WEBP(d) => d.as_slice(),
                    usvg::ImageKind::SVG(_) => continue,
                };
                if found.images.len() >= MAX_IMAGES || found.image_bytes + bytes.len() > MAX_IMAGE_BYTES {
                    continue;
                }
                let size = img.size();
                let ts = img.abs_transform();
                let m = Affine::new([ts.sx as f64, ts.ky as f64, ts.kx as f64, ts.sy as f64, ts.tx as f64, ts.ty as f64]);
                let transform = Affine::scale(MM_PER_PX) * m * Affine::scale(1.0 / MM_PER_PX);
                found.image_bytes += bytes.len();
                let name = if img.id().is_empty() { format!("image {}", found.images.len() + 1) } else { img.id().to_string() };
                found.images.push(ImportedImage {
                    name,
                    bytes: bytes.to_vec(),
                    width_mm: f64::from(size.width()) * MM_PER_PX,
                    height_mm: f64::from(size.height()) * MM_PER_PX,
                    transform,
                });
            }
            usvg::Node::Path(p) if p.id().starts_with(PLACEHOLDER) => place_text(p, found),
            usvg::Node::Path(p) => {
                let ts = p.abs_transform();
                let m = Affine::new([ts.sx as f64, ts.ky as f64, ts.kx as f64, ts.sy as f64, ts.tx as f64, ts.ty as f64]);
                let mut bez = convert(p.data());
                bez.apply_affine(Affine::scale(MM_PER_PX) * m);
                let color = p.stroke().and_then(|s| paint_hex(s.paint())).or_else(|| p.fill().and_then(|f| paint_hex(f.paint())));
                let name = if p.id().is_empty() { format!("path {}", out.len() + 1) } else { p.id().to_string() };
                out.push(ImportedPath { name, path: bez, color, filled: p.fill().is_some() });
            }
        }
    }
}

fn convert(path: &usvg::tiny_skia_path::Path) -> BezPath {
    let mut b = BezPath::new();
    let pt = |p: usvg::tiny_skia_path::Point| (f64::from(p.x), f64::from(p.y));
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => b.move_to(pt(p)),
            PathSegment::LineTo(p) => b.line_to(pt(p)),
            PathSegment::QuadTo(c, p) => b.quad_to(pt(c), pt(p)),
            PathSegment::CubicTo(c1, c2, p) => b.curve_to(pt(c1), pt(c2), pt(p)),
            PathSegment::Close => b.close_path(),
        }
    }
    b
}

fn paint_hex(paint: &usvg::Paint) -> Option<String> {
    match paint {
        usvg::Paint::Color(c) => Some(format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use laserfoundry_geom::{bounds_of_polylines, flatten};

    use super::*;

    const RECT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96">
        <rect x="10" y="20" width="30" height="40" fill="none" stroke="#ff0000"/>
        <g transform="translate(48 0)"><circle cx="20" cy="20" r="10" fill="#0000ff"/></g>
    </svg>"##;

    struct BoxOutliner(Option<f64>);
    impl TextOutliner for BoxOutliner {
        fn outline(&self, r: &TextRequest) -> Option<TextOutline> {
            let w = self.0? * r.text.chars().count() as f64;
            let mut path = BezPath::new();
            path.move_to((0.0, 0.0));
            path.line_to((w, 0.0));
            path.line_to((w, -r.size_px));
            path.line_to((0.0, -r.size_px));
            path.close_path();
            Some(TextOutline { path, advance_px: w, substituted: r.families.iter().any(|f| f == "Nope") })
        }
    }

    const TEXT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><text x="100" y="50" font-family="Arial" font-weight="bold" font-size="20" text-anchor="middle" fill="#ff0000">Hi</text></svg>"##;

    #[test]
    fn text_with_no_fonts_is_drawn_by_the_outliner_where_the_anchor_puts_it() {
        let out = import_svg_without_system_fonts(TEXT_SVG.as_bytes(), Some(&BoxOutliner(Some(10.0)))).unwrap();
        assert_eq!((out.text_outlined, out.text_substituted, out.text_dropped), (1, 0, 0));
        assert_eq!(out.paths.len(), 1);
        let b = bounds_of_polylines(&flatten(&out.paths[0].path, 0.01)).unwrap();
        // "Hi" is 20 px wide, centred on x = 100 px; the baseline is y = 50 px and the box is 20 px tall.
        let mm = 25.4 / 96.0;
        assert!((b.x0 - 90.0 * mm).abs() < 1e-6 && (b.x1 - 110.0 * mm).abs() < 1e-6, "{b:?}");
        assert!((b.y1 - 50.0 * mm).abs() < 1e-6 && (b.y0 - 30.0 * mm).abs() < 1e-6, "{b:?}");
        assert_eq!(out.paths[0].color.as_deref(), Some("#ff0000"));
    }

    #[test]
    fn text_that_cannot_be_drawn_is_counted_not_silently_lost() {
        for outliner in [None, Some(&BoxOutliner(None) as &dyn TextOutliner)] {
            let out = import_svg_without_system_fonts(TEXT_SVG.as_bytes(), outliner).unwrap();
            assert_eq!((out.text_outlined, out.text_dropped), (0, 1));
            assert!(out.paths.is_empty());
        }
        let blank = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="1" y="1">   </text></svg>"#;
        assert_eq!(
            import_svg_without_system_fonts(blank.as_bytes(), None).unwrap().text_dropped,
            0,
            "a text element with no words is nothing to report"
        );
    }

    #[test]
    fn text_follows_group_transforms_and_each_positioned_tspan_is_its_own_line() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><g transform="translate(96 0)" font-family="Arial" font-size="10">
            <text x="0" y="20" transform="translate(0 10)">One<tspan x="0" y="40" fill="#00f">Two</tspan><tspan font-size="20"> more</tspan></text></g></svg>"##;
        let out = import_svg_without_system_fonts(svg.as_bytes(), Some(&BoxOutliner(Some(2.0)))).unwrap();
        assert_eq!((out.text_outlined, out.text_dropped), (2, 0), "two chunks: the first line, and the tspan that sets its own position");
        let mm = 25.4 / 96.0;
        let first = bounds_of_polylines(&flatten(&out.paths[0].path, 0.01)).unwrap();
        // The group moves it 96 px right, the text's own transform 10 px down; baseline at y = 20 + 10 = 30 px.
        assert!((first.x0 - 96.0 * mm).abs() < 1e-6 && (first.y1 - 30.0 * mm).abs() < 1e-6, "{first:?}");
        assert_eq!(out.paths[1].color.as_deref(), Some("#0000ff"));
        let second = bounds_of_polylines(&flatten(&out.paths[1].path, 0.01)).unwrap();
        assert!((second.y1 - 50.0 * mm).abs() < 1e-6, "{second:?}");
        assert_eq!(out.paths.len(), 3, "the unpositioned tspan continues the second line as another run");
    }

    #[test]
    fn hostile_text_markup_is_bounded_and_never_panics() {
        let many = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{}</svg>"#, r#"<text x="1" y="1">a</text>"#.repeat(MAX_TEXT_CHUNKS + 50));
        let _ = import_svg_without_system_fonts(many.as_bytes(), Some(&BoxOutliner(Some(1.0))));
        for bad in [
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="NaN" y="1e999" font-size="-5" transform="rotate(">q</text></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text><text>nested</text></text></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text font-size="1e999">x</text></svg>"#,
        ] {
            let _ = import_svg_without_system_fonts(bad.as_bytes(), Some(&BoxOutliner(Some(1.0))));
        }
    }

    #[test]
    fn a_stand_in_font_is_reported() {
        let svg = TEXT_SVG.replace("Arial", "Nope");
        let out = import_svg_without_system_fonts(svg.as_bytes(), Some(&BoxOutliner(Some(5.0)))).unwrap();
        assert_eq!((out.text_outlined, out.text_substituted), (1, 1));
    }

    #[test]
    fn a_file_full_of_text_is_refused_before_it_is_laid_out() {
        // Regression: 2 million characters in one <text> ran for minutes (layout time grows faster than the length).
        let long = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><text font-size=\"10\">{}</text></svg>", "W".repeat(2_000_000));
        let started = std::time::Instant::now();
        assert!(matches!(import_svg(long.as_bytes()), Err(SvgError::TooMuchText)));
        assert!(started.elapsed().as_secs() < 5);
        // Unterminated, and spread over many elements.
        let open = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><text>{}", "W".repeat(100_000));
        assert!(matches!(import_svg(open.as_bytes()), Err(SvgError::TooMuchText)));
        let many = format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>", "<text>0123456789</text>".repeat(5_000));
        assert!(matches!(import_svg(many.as_bytes()), Err(SvgError::TooMuchText)));
        // A normal amount of text is fine.
        let short = "<svg xmlns=\"http://www.w3.org/2000/svg\"><text x=\"1\" y=\"10\">Hello</text></svg>";
        assert!(import_svg(short.as_bytes()).is_ok());
    }

    #[test]
    fn imports_shapes_in_mm_with_group_transforms() {
        let svg = import_svg(RECT_SVG.as_bytes()).unwrap();
        assert!((svg.width_mm - 25.4).abs() < 1e-6);
        assert_eq!(svg.paths.len(), 2);
        let rect = bounds_of_polylines(&flatten(&svg.paths[0].path, 0.01)).unwrap();
        assert!((rect.x0 - 10.0 * MM_PER_PX).abs() < 1e-4);
        assert!((rect.height() - 40.0 * MM_PER_PX).abs() < 1e-4);
        assert_eq!(svg.paths[0].color.as_deref(), Some("#ff0000"));
        let circle = bounds_of_polylines(&flatten(&svg.paths[1].path, 0.01)).unwrap();
        assert!((circle.x0 - (48.0 + 10.0) * MM_PER_PX).abs() < 0.01);
        assert!(svg.paths[1].filled);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(import_svg(b"not svg at all").is_err());
        assert!(import_svg(b"").is_err());
        assert!(import_svg(&[0xff, 0xfe, 0x00]).is_err());
    }

    #[test]
    fn huge_input_is_refused_up_front() {
        assert!(matches!(import_svg(&vec![b' '; MAX_SVG_BYTES + 1]), Err(SvgError::TooLarge)));
    }

    #[test]
    fn an_svg_with_no_shapes_imports_as_empty() {
        let svg = import_svg(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"/>"#).unwrap();
        assert!(svg.paths.is_empty() && svg.images.is_empty());
        assert!((svg.width_mm - 10.0 * MM_PER_PX).abs() < 1e-9);
        assert!((svg.height_mm - 20.0 * MM_PER_PX).abs() < 1e-9);
    }

    #[test]
    fn ids_name_paths_and_unnamed_ones_are_numbered() {
        let svg = import_svg(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <rect id="frame" width="5" height="5"/><rect width="2" height="2"/></svg>"#,
        )
        .unwrap();
        assert_eq!(svg.paths.len(), 2);
        assert_eq!(svg.paths[0].name, "frame");
        assert_eq!(svg.paths[1].name, "path 2");
    }

    #[test]
    fn stroke_colour_wins_over_fill_and_non_colour_paints_give_none() {
        let svg = import_svg(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <defs><linearGradient id="g"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/></linearGradient></defs>
                <rect width="4" height="4" fill="#00ff00" stroke="#0000ff"/>
                <rect width="4" height="4" fill="url(#g)"/>
                <rect width="4" height="4" fill="none" stroke="none"/></svg>"##,
        )
        .unwrap();
        assert_eq!(svg.paths[0].color.as_deref(), Some("#0000ff"));
        assert!(svg.paths[0].filled);
        assert_eq!(svg.paths[1].color, None);
        // A shape with neither fill nor stroke draws nothing, so it is dropped or carries no colour; it must not be "filled".
        assert!(svg.paths.get(2).is_none_or(|p| !p.filled && p.color.is_none()));
    }

    #[test]
    fn curves_and_arcs_come_through_as_geometry() {
        let svg = import_svg(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96">
                <path d="M 0 0 C 10 0 20 10 20 20 Q 30 30 40 20 A 10 10 0 0 1 60 20 Z" fill="none" stroke="#000"/></svg>"##,
        )
        .unwrap();
        assert_eq!(svg.paths.len(), 1);
        let b = bounds_of_polylines(&flatten(&svg.paths[0].path, 0.01)).unwrap();
        assert!(b.width() > 50.0 * MM_PER_PX && b.width() < 61.0 * MM_PER_PX);
        assert!(b.x0.is_finite() && b.y1.is_finite());
    }

    #[test]
    fn deeply_nested_groups_do_not_overflow_the_stack() {
        for depth in [100usize, 2_000, 20_000] {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\">{}<rect width=\"1\" height=\"1\"/>{}</svg>",
                "<g>".repeat(depth),
                "</g>".repeat(depth)
            );
            // Either result is fine; reaching this line means no crash.
            let _ = import_svg(svg.as_bytes());
        }
    }

    #[test]
    fn hostile_attribute_values_are_errors_or_finite_output() {
        let cases: [&str; 6] = [
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1e999" height="1e999"><rect width="1" height="1"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"><rect width="1" height="1"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="-5" height="-5"><rect width="1" height="1"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="NaN" height="inf"><rect width="1" height="1"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><path d="M 1e308 1e308 L -1e308 1e308 Z"/></svg>"#,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><use href="#a"/><g id="a"><use href="#a"/></g></svg>"##,
        ];
        for case in cases {
            if let Ok(svg) = import_svg(case.as_bytes()) {
                assert!(svg.width_mm.is_finite() && svg.height_mm.is_finite(), "non-finite size for {case}");
            }
        }
    }

    #[test]
    fn a_malformed_embedded_picture_never_panics_and_valid_ones_are_sane() {
        // A deliberately incomplete PNG (no valid IDAT/CRC), written out byte by byte.
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x60,
            0x60, 0x60, 0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0x5e, 0xf3, 0x2a, 0x3a, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
        ];
        let b64 = base64_encode(PNG);
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"96\" height=\"96\">\
             <image id=\"pic\" x=\"10\" y=\"10\" width=\"48\" height=\"48\" xlink:href=\"data:image/png;base64,{b64}\"/></svg>"
        );
        // The truncated PNG may be rejected by the decoder; the contract under test is "no panic, and sane output when accepted".
        if let Ok(import) = import_svg(svg.as_bytes()) {
            for img in &import.images {
                assert!(img.width_mm.is_finite() && img.height_mm.is_finite());
                assert!(!img.bytes.is_empty());
            }
        }
    }

    fn base64_encode(data: &[u8]) -> String {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    let idx = ((n >> (18 - 6 * i)) & 63) as usize;
                    out.push(ALPHABET.get(idx).copied().map_or('A', char::from));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
}
