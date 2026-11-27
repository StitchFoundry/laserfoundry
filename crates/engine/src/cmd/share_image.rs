//! A share-ready picture of the finished design: the layers drawn in their own colours on a plain
//! background, centred with a margin, as a PNG at a fixed size (square or portrait). Pure CPU, offline.

use std::io::Cursor;

use image::{ImageEncoder, codecs::png::PngEncoder};
use laserfoundry_doc::{Document, LayerMode, ShapeKind};
use laserfoundry_geom::{Point, Rect, flatten};

use crate::EngineError;

/// Output width in pixels; the height comes from the format.
pub const WIDTH: u32 = 1080;
/// Space kept free around the design, in pixels.
const MARGIN: f64 = 90.0;
const BACKGROUND: [u8; 3] = [250, 250, 248];
/// Line thickness in pixels.
const LINE_PX: f64 = 3.0;
const FLATTEN_MM: f64 = 0.05;

/// The two sizes social networks take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Square,
    Portrait,
}

impl Format {
    pub fn parse(name: &str) -> Option<Format> {
        match name {
            "square" | "1080x1080" => Some(Format::Square),
            "portrait" | "1080x1350" => Some(Format::Portrait),
            _ => None,
        }
    }

    pub fn height(self) -> u32 {
        match self {
            Format::Square => 1080,
            Format::Portrait => 1350,
        }
    }
}

/// One layer ready to draw: its colour, whether it fills, and its outlines (with closed flags).
struct Drawn {
    colour: [u8; 3],
    fill: bool,
    outlines: Vec<(Vec<Point>, bool)>,
}

/// The design as PNG bytes. Layers whose output is off are left out. Fails when there is nothing to draw.
pub fn render_png(doc: &Document, format: Format) -> Result<Vec<u8>, EngineError> {
    let (w, h) = (WIDTH, format.height());
    let mut drawn: Vec<Drawn> = Vec::new();
    let mut bounds: Option<Rect> = None;
    for layer in doc.layers.iter().filter(|l| l.output) {
        let mut outlines = Vec::new();
        for shape in doc.shapes.iter().filter(|s| s.layer == layer.id) {
            let (ShapeKind::Path { path } | ShapeKind::Text { path, .. }) = &shape.kind else { continue };
            let mut p = path.clone();
            p.apply_affine(shape.transform);
            for poly in flatten(&p, FLATTEN_MM) {
                let pts: Vec<Point> = poly.points.into_iter().filter(|q| q.x.is_finite() && q.y.is_finite()).collect();
                for q in &pts {
                    let r = Rect::new(q.x, q.y, q.x, q.y);
                    bounds = Some(bounds.map_or(r, |b| b.union(r)));
                }
                if pts.len() > 1 {
                    outlines.push((pts, poly.closed));
                }
            }
        }
        drawn.push(Drawn { colour: parse_colour(&layer.color), fill: !matches!(layer.mode, LayerMode::Line), outlines });
    }
    let b = bounds.ok_or_else(|| EngineError::BadParams("there is nothing to draw".into()))?;
    let (dw, dh) = ((b.x1 - b.x0).max(1e-6), (b.y1 - b.y0).max(1e-6));
    let scale = ((f64::from(w) - 2.0 * MARGIN) / dw).min((f64::from(h) - 2.0 * MARGIN) / dh);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(EngineError::BadParams("the design cannot be drawn at this size".into()));
    }
    let (ox, oy) = ((f64::from(w) - dw * scale) / 2.0 - b.x0 * scale, (f64::from(h) - dh * scale) / 2.0 - b.y0 * scale);
    let to_px = |p: &Point| Point::new(p.x * scale + ox, p.y * scale + oy);

    let mut canvas = Canvas { w, h, rgb: BACKGROUND.repeat(w as usize * h as usize) };
    for layer in &drawn {
        let px: Vec<(Vec<Point>, bool)> = layer.outlines.iter().map(|(l, c)| (l.iter().map(to_px).collect(), *c)).collect();
        if layer.fill {
            canvas.fill_even_odd(&px, layer.colour);
        }
        for (l, closed) in &px {
            for pair in l.windows(2) {
                canvas.line(pair[0], pair[1], layer.colour);
            }
            if let (true, Some(first), Some(last)) = (*closed, l.first(), l.last()) {
                canvas.line(*last, *first, layer.colour);
            }
        }
    }
    let mut out = Vec::new();
    PngEncoder::new(Cursor::new(&mut out))
        .write_image(&canvas.rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| EngineError::BadParams(format!("could not encode the image: {e}")))?;
    Ok(out)
}

struct Canvas {
    w: u32,
    h: u32,
    rgb: Vec<u8>,
}

impl Canvas {
    fn put(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if x < 0 || y < 0 || x >= i64::from(self.w) || y >= i64::from(self.h) {
            return;
        }
        let i = (y as usize * self.w as usize + x as usize) * 3;
        if let Some(px) = self.rgb.get_mut(i..i + 3) {
            px.copy_from_slice(&c);
        }
    }

    /// A line stamped with a small disc every half pixel.
    fn line(&mut self, a: Point, b: Point, c: [u8; 3]) {
        let len = (b - a).hypot();
        if !len.is_finite() || len > 1.0e5 {
            return;
        }
        let steps = (len * 2.0).ceil().max(1.0) as u32;
        let r = LINE_PX / 2.0;
        for i in 0..=steps {
            let t = f64::from(i) / f64::from(steps);
            let (cx, cy) = (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
            for dy in -2..=2i64 {
                for dx in -2..=2i64 {
                    let (px, py) = (cx.floor() as i64 + dx, cy.floor() as i64 + dy);
                    let (ex, ey) = (px as f64 + 0.5 - cx, py as f64 + 0.5 - cy);
                    if ex * ex + ey * ey <= r * r {
                        self.put(px, py, c);
                    }
                }
            }
        }
    }

    /// Scanline fill, even-odd, of every outline taken as closed.
    fn fill_even_odd(&mut self, outlines: &[(Vec<Point>, bool)], c: [u8; 3]) {
        for y in 0..self.h {
            let sy = f64::from(y) + 0.5;
            let mut xs: Vec<f64> = Vec::new();
            for (o, _) in outlines {
                for (i, a) in o.iter().enumerate() {
                    let Some(b) = o.get((i + 1) % o.len()) else { continue };
                    if (a.y <= sy) != (b.y <= sy) {
                        xs.push(a.x + (sy - a.y) / (b.y - a.y) * (b.x - a.x));
                    }
                }
            }
            xs.sort_by(f64::total_cmp);
            for pair in xs.as_chunks::<2>().0 {
                let (x0, x1) = (pair[0].round().max(0.0) as i64, pair[1].round().min(f64::from(self.w)) as i64);
                for x in x0..x1 {
                    self.put(x, i64::from(y), c);
                }
            }
        }
    }
}

/// `#rrggbb`, or black when it is anything else.
fn parse_colour(hex: &str) -> [u8; 3] {
    let h = hex.trim().trim_start_matches('#');
    let v = (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten().unwrap_or(0);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}
