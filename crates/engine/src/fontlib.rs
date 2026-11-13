//! Library fonts the host has loaded into memory, for hosts with no font folder to point at (the
//! browser). A font is named `library:<folder>` (the folder it has in the font library); text commands
//! accept that name wherever they accept a font path. Fonts come from outside, so [`register`] parses
//! them with the same bounds as any other glyph font and the registry is capped.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use laserfoundry_text::GlyphFont;

use crate::EngineError;

/// The prefix of a library font's name.
pub const PREFIX: &str = "library:";
/// Most fonts held at once.
pub const MAX_FONTS: usize = 200;
/// Longest folder name accepted, in bytes.
const MAX_NAME_BYTES: usize = 80;

fn registry() -> &'static Mutex<BTreeMap<String, Arc<GlyphFont>>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<String, Arc<GlyphFont>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Makes the font in these two files available as `library:<folder>`, replacing any earlier one of
/// that name. `folder` is a plain name (letters, digits, `-`, `_`, `.`, space).
pub fn register(folder: &str, font_json: &str, glyphs_json: &str) -> Result<(), EngineError> {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ');
    if folder.is_empty() || folder.len() > MAX_NAME_BYTES || !folder.chars().all(plain) || folder.starts_with('.') {
        return Err(EngineError::BadParams("a library font folder name is plain letters, digits, - _ . or space".into()));
    }
    let font = GlyphFont::parse(font_json, glyphs_json).map_err(|e| EngineError::BadParams(e.to_string()))?;
    let mut fonts = registry().lock().map_err(|_| EngineError::BadParams("the font registry is unavailable".into()))?;
    let name = format!("{PREFIX}{folder}");
    if !fonts.contains_key(&name) && fonts.len() >= MAX_FONTS {
        return Err(EngineError::BadParams(format!("more than {MAX_FONTS} library fonts")));
    }
    fonts.insert(name, Arc::new(font));
    Ok(())
}

/// The names (`library:<folder>`) and display names of the fonts held, sorted by name.
pub fn list() -> Vec<(String, String)> {
    let Ok(fonts) = registry().lock() else { return Vec::new() };
    let mut out: Vec<(String, String)> =
        fonts.iter().map(|(k, f)| (k.clone(), if f.name.is_empty() { k.trim_start_matches(PREFIX).to_string() } else { f.name.clone() })).collect();
    out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Forgets every font (for tests, and a host that wants a clean slate).
pub fn clear() {
    if let Ok(mut fonts) = registry().lock() {
        fonts.clear();
    }
}

pub(crate) fn get(name: &str) -> Option<Arc<GlyphFont>> {
    registry().lock().ok()?.get(name).cloned()
}

/// Draws SVG `<text>` with the library fonts held in memory, for hosts with no font files (the browser).
/// A requested family is matched to a held font by name; the generic families and the common office
/// ones (Arial, Helvetica, Times, Courier…) go to the closest held font (`sans`, `serif`, `mono` in its
/// name), and anything else to the first font held, which is reported as a stand-in.
pub(crate) struct LibraryTextOutliner;

/// Cap height as a fraction of the font size (typical for text faces).
const CAP_HEIGHT_PER_EM: f64 = 0.72;
/// CSS pixels (96 per inch) to millimetres.
const MM_PER_PX: f64 = 25.4 / 96.0;

impl laserfoundry_svg::TextOutliner for LibraryTextOutliner {
    fn outline(&self, request: &laserfoundry_svg::TextRequest) -> Option<laserfoundry_svg::TextOutline> {
        use kurbo::{Affine, Shape};
        let held: Vec<(String, Arc<GlyphFont>)> = registry().lock().ok()?.iter().map(|(k, f)| (k.clone(), f.clone())).collect();
        let (font, substituted) = pick(&held, &request.families)?;
        let height = (request.size_px * CAP_HEIGHT_PER_EM * MM_PER_PX).clamp(laserfoundry_text::MIN_HEIGHT_MM, laserfoundry_text::MAX_HEIGHT_MM);
        let style = laserfoundry_text::TextStyle { height, ..Default::default() };
        let layout = laserfoundry_text::layout_glyph_font(&font, &request.text.replace('\n', " "), &style).ok()?;
        let mut path = layout.path;
        let bounds = path.bounding_box();
        if !(bounds.x1.is_finite() && bounds.x1 >= 0.0) {
            return None;
        }
        // The layout puts the top of the capitals at y = 0 (baseline at y = height); SVG text sits with its baseline at y = 0.
        path.apply_affine(Affine::scale(1.0 / MM_PER_PX) * Affine::translate((0.0, -height)));
        Some(laserfoundry_svg::TextOutline { path, advance_px: bounds.x1 / MM_PER_PX, substituted })
    }
}

/// The held font for the first family that has a match; `true` when none did and the first held font stands in.
fn pick(held: &[(String, Arc<GlyphFont>)], families: &[String]) -> Option<(Arc<GlyphFont>, bool)> {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let label = |(name, font): &(String, Arc<GlyphFont>)| key(&format!("{} {}", name.trim_start_matches(PREFIX), font.name));
    for family in families {
        let wanted = key(family);
        if wanted.is_empty() {
            continue;
        }
        if let Some(f) = held.iter().find(|h| label(h).contains(&wanted)) {
            return Some((f.1.clone(), false));
        }
        let hint = match wanted.as_str() {
            "monospace" | "courier" | "couriernew" | "consolas" | "menlo" | "monaco" => "mono",
            "serif" | "times" | "timesnewroman" | "georgia" | "garamond" | "palatino" | "cambria" => "serif",
            "sansserif" | "arial" | "helvetica" | "verdana" | "tahoma" | "calibri" | "segoeui" | "trebuchetms" | "system" | "systemui" => "sans",
            _ => continue,
        };
        let found = held.iter().find(|h| {
            let l = label(h);
            l.contains(hint) && (hint != "serif" || !l.contains("sans"))
        });
        if let Some(f) = found {
            return Some((f.1.clone(), false));
        }
    }
    held.first().map(|f| (f.1.clone(), true))
}
