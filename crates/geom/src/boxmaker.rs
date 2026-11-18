//! Box maker geometry: the panels of a finger-jointed box (or tray, without a lid) as closed outlines.
//!
//! Dimensions are the *inside* size; the walls stand on the bottom and the lid sits on the walls, so
//! the outside is `inside + 2 * thickness` across. Every joint edge is split into an odd number of equal
//! segments of about the finger width: the first, third, ... segments are tabs on one panel and the
//! matching slots on the other, so the two always mate. Outlines are in panel-local millimetres with
//! the panel body at `[0, width] x [0, height]` (y up); tabs stick out past that rectangle. Kerf is
//! compensated by moving the whole outline out by half the kerf, so the finished part has the nominal
//! size once the laser has removed its half on each side.
//!
//! Layout on a canvas is the caller's business. Plain geometry: no machine motion.

use crate::{Point, Polyline};
use thiserror::Error;

/// Largest dimension (mm) accepted for any box side.
pub const MAX_DIMENSION_MM: f64 = 10_000.0;
/// Most segments one joint edge is split into.
pub const MAX_SEGMENTS: usize = 1_000;
/// Tolerance when merging points of the outline.
const EPS: f64 = 1e-9;

#[derive(Debug, Error, PartialEq)]
pub enum BoxError {
    #[error("box sizes must be finite numbers")]
    NonFinite,
    #[error("box sizes, material thickness and finger width must be above zero and kerf must not be negative")]
    NotPositive,
    #[error("a box side may not be longer than 10000 mm")]
    TooLarge,
    #[error("the fingers are too narrow for this kerf: use a wider finger width or a smaller kerf")]
    FingersTooFine,
}

/// What to make.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxSpec {
    /// Inside width (x), depth (y) and height (z), mm.
    pub width: f64,
    pub depth: f64,
    pub height: f64,
    /// Material thickness, mm.
    pub thickness: f64,
    /// Width of the beam the laser removes, mm.
    pub kerf: f64,
    /// Wanted finger width, mm; the real width is adjusted so a whole odd number fits each edge.
    pub finger: f64,
    /// Make a lid (six panels) or an open tray (five).
    pub lid: bool,
}

/// One panel: a closed cut outline, kerf-compensated.
#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub name: &'static str,
    pub outline: Polyline,
}

#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Plain,
    /// Tabs stick out past the edge over `lo..hi` along it.
    Tabs(f64, f64),
    /// Slots are cut into the panel over `lo..hi` along the edge.
    Slots(f64, f64),
}

/// The number of segments an edge of `len` is split into: odd, close to `len / finger`, at least one.
pub fn segment_count(len: f64, finger: f64) -> usize {
    let n = (len / finger).round().clamp(1.0, MAX_SEGMENTS as f64) as usize;
    if n % 2 == 1 {
        n
    } else if n >= MAX_SEGMENTS {
        n - 1
    } else {
        n + 1
    }
}

/// The panels of the box described by `spec`: bottom, front, back, left, right and, with a lid, lid.
pub fn panels(spec: &BoxSpec) -> Result<Vec<Panel>, BoxError> {
    let BoxSpec { width: w, depth: d, height: h, thickness: t, kerf: k, finger: f, lid } = *spec;
    if ![w, d, h, t, k, f].iter().all(|v| v.is_finite()) {
        return Err(BoxError::NonFinite);
    }
    if w <= 0.0 || d <= 0.0 || h <= 0.0 || t <= 0.0 || f <= 0.0 || k < 0.0 {
        return Err(BoxError::NotPositive);
    }
    if [w, d, h, t].iter().any(|v| *v > MAX_DIMENSION_MM) {
        return Err(BoxError::TooLarge);
    }
    if [w, d, h].iter().any(|len| len / segment_count(*len, f) as f64 <= 2.0 * k) {
        return Err(BoxError::FingersTooFine);
    }
    let (x, y) = (w + 2.0 * t, d + 2.0 * t);
    let top = |e: Edge| if lid { e } else { Edge::Plain };
    // Edges run counter-clockwise from the bottom edge: bottom, right, top, left.
    let flat = [Edge::Slots(t, x - t), Edge::Slots(t, y - t), Edge::Slots(t, x - t), Edge::Slots(t, y - t)];
    let wall_fb = [Edge::Tabs(t, x - t), Edge::Slots(0.0, h), top(Edge::Tabs(t, x - t)), Edge::Slots(0.0, h)];
    let wall_lr = [Edge::Tabs(0.0, d), Edge::Tabs(0.0, h), top(Edge::Tabs(0.0, d)), Edge::Tabs(0.0, h)];
    let mut out = vec![("bottom", x, y, flat), ("front", x, h, wall_fb), ("back", x, h, wall_fb), ("left", d, h, wall_lr), ("right", d, h, wall_lr)];
    if lid {
        out.push(("lid", x, y, flat));
    }
    out.into_iter()
        .map(|(name, pw, ph, edges)| {
            let points = compensate(&outline(pw, ph, &edges, t, f), k / 2.0)?;
            Ok(Panel { name, outline: Polyline::new(points, true) })
        })
        .collect()
}

/// The nominal outline of a `w` x `h` body with joints on its four edges.
fn outline(w: f64, h: f64, edges: &[Edge; 4], t: f64, finger: f64) -> Vec<Point> {
    let corners = [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)];
    let mut pts: Vec<Point> = Vec::new();
    for (i, edge) in edges.iter().enumerate() {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        let len = a.distance(b);
        let u = (b - a) / len;
        // Counter-clockwise, so the outward normal is the direction turned a quarter clockwise.
        let n = kurbo::Vec2::new(u.y, -u.x);
        let at = |s: f64, off: f64| a + u * s + n * off;
        pts.push(at(0.0, 0.0));
        if let Edge::Tabs(lo, hi) | Edge::Slots(lo, hi) = *edge {
            let depth = if matches!(edge, Edge::Tabs(..)) { t } else { -t };
            let count = segment_count(hi - lo, finger);
            let seg = (hi - lo) / count as f64;
            for j in (0..count).step_by(2) {
                let (s0, s1) = (lo + seg * j as f64, lo + seg * (j + 1) as f64);
                pts.extend([at(s0, 0.0), at(s0, depth), at(s1, depth), at(s1, 0.0)]);
            }
        }
    }
    tidy(pts)
}

/// Drops repeated points and points in the middle of a straight run.
fn tidy(pts: Vec<Point>) -> Vec<Point> {
    let mut cur = pts;
    // A slot that ends on a corner leaves a zero-width spike (out to the corner and straight back):
    // dropping the spike's tip leaves two equal points, so repeat until nothing more changes.
    loop {
        let mut dedup: Vec<Point> = Vec::new();
        for p in &cur {
            if dedup.last().is_none_or(|q| q.distance(*p) > EPS) {
                dedup.push(*p);
            }
        }
        while dedup.len() > 1 && dedup.first().zip(dedup.last()).is_some_and(|(a, b)| a.distance(*b) <= EPS) {
            dedup.pop();
        }
        let n = dedup.len();
        let next: Vec<Point> = (0..n)
            .filter_map(|i| {
                let (p, a, b) = (dedup.get(i)?, dedup.get((i + n - 1) % n)?, dedup.get((i + 1) % n)?);
                let cross = (*p - *a).cross(*b - *p);
                (cross.abs() > EPS).then_some(*p)
            })
            .collect();
        if next.len() == cur.len() {
            return next;
        }
        cur = next;
    }
}

/// Moves a counter-clockwise rectilinear outline out by `half` (the corner offset is exact for right angles).
fn compensate(pts: &[Point], half: f64) -> Result<Vec<Point>, BoxError> {
    let n = pts.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (Some(p), Some(a), Some(b)) = (pts.get(i), pts.get((i + n - 1) % n), pts.get((i + 1) % n)) else {
            return Err(BoxError::FingersTooFine);
        };
        let (d1, d2) = ((*p - *a), (*b - *p));
        let (l1, l2) = (d1.hypot(), d2.hypot());
        if l1 <= EPS || l2 <= EPS {
            return Err(BoxError::FingersTooFine);
        }
        let (n1, n2) = (kurbo::Vec2::new(d1.y / l1, -d1.x / l1), kurbo::Vec2::new(d2.y / l2, -d2.x / l2));
        let denom = 1.0 + n1.dot(n2);
        if denom <= EPS {
            return Err(BoxError::FingersTooFine);
        }
        out.push(*p + (n1 + n2) * (half / denom));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(lid: bool, kerf: f64) -> BoxSpec {
        BoxSpec { width: 100.0, depth: 60.0, height: 40.0, thickness: 3.0, kerf, finger: 10.0, lid }
    }

    fn panel<'a>(all: &'a [Panel], name: &str) -> &'a Polyline {
        &all.iter().find(|p| p.name == name).unwrap().outline
    }

    fn area(p: &Polyline) -> f64 {
        let n = p.points.len();
        (0..n).map(|i| (p.points[i].to_vec2().cross(p.points[(i + 1) % n].to_vec2())) / 2.0).sum()
    }

    /// Total length of the active (first, third, ...) segments of an edge `len` long.
    fn active(len: f64) -> f64 {
        let n = segment_count(len, 10.0);
        n.div_ceil(2) as f64 * len / n as f64
    }

    #[test]
    fn six_panels_with_lid_five_without() {
        let all = panels(&spec(true, 0.0)).unwrap();
        let names: Vec<_> = all.iter().map(|p| p.name).collect();
        assert_eq!(names, ["bottom", "front", "back", "left", "right", "lid"]);
        assert_eq!(panels(&spec(false, 0.0)).unwrap().len(), 5);
        assert!(all.iter().all(|p| p.outline.closed && area(&p.outline) > 0.0));
    }

    #[test]
    fn finger_counts_are_odd_and_near_the_wanted_width() {
        assert_eq!(segment_count(100.0, 10.0), 11);
        assert_eq!(segment_count(60.0, 10.0), 7);
        assert_eq!(segment_count(40.0, 10.0), 5);
        assert_eq!(segment_count(3.0, 10.0), 1);
        assert_eq!(segment_count(1e9, 1.0), MAX_SEGMENTS - 1);
    }

    #[test]
    fn panel_sizes_and_areas_follow_the_joints() {
        let all = panels(&spec(true, 0.0)).unwrap();
        let t = 3.0;
        let b = panel(&all, "bottom").bounds().unwrap();
        assert_eq!((b.width(), b.height()), (106.0, 66.0));
        assert!((area(panel(&all, "bottom")) - (106.0 * 66.0 - t * (2.0 * active(100.0) + 2.0 * active(60.0)))).abs() < 1e-6);
        assert!((area(panel(&all, "front")) - (106.0 * 40.0 + t * 2.0 * active(100.0) - t * 2.0 * active(40.0))).abs() < 1e-6);
        assert!((area(panel(&all, "left")) - (60.0 * 40.0 + t * (2.0 * active(60.0) + 2.0 * active(40.0)))).abs() < 1e-6);
        let tray = panels(&spec(false, 0.0)).unwrap();
        let f = panel(&tray, "front").bounds().unwrap();
        assert_eq!((f.width(), f.height()), (106.0, 43.0));
    }

    #[test]
    fn tabs_fit_the_slots_they_meet() {
        let all = panels(&spec(true, 0.0)).unwrap();
        let t = 3.0;
        let (front, bottom, left) = (panel(&all, "front"), panel(&all, "bottom"), panel(&all, "left"));
        // Front's bottom tabs against the bottom's front slots (front body starts at x = 0, bottom's slots span x = 3..103).
        let seg = 100.0 / 11.0;
        for i in 0..11 {
            let x = t + seg * (i as f64 + 0.5);
            assert_ne!(front.contains(Point::new(x, -t / 2.0)), bottom.contains(Point::new(x, t / 2.0)), "bottom joint, segment {i}");
        }
        // Left's end tabs against the front's side slots.
        let seg = 40.0 / 5.0;
        for i in 0..5 {
            let z = seg * (i as f64 + 0.5);
            assert_ne!(left.contains(Point::new(60.0 + t / 2.0, z)), front.contains(Point::new(106.0 - t / 2.0, z)), "side joint, segment {i}");
        }
    }

    #[test]
    fn kerf_moves_the_outline_out_by_half() {
        let plain = panels(&spec(true, 0.0)).unwrap();
        let cut = panels(&spec(true, 0.2)).unwrap();
        let (a, b) = (panel(&plain, "bottom").bounds().unwrap(), panel(&cut, "bottom").bounds().unwrap());
        assert!((b.width() - a.width() - 0.2).abs() < 1e-9 && (b.height() - a.height() - 0.2).abs() < 1e-9);
        // A tab is cut wider by the kerf and a slot narrower, so both end up nominal after the laser.
        assert!(area(panel(&cut, "left")) > area(panel(&plain, "left")));
        assert!(area(panel(&cut, "bottom")) > area(panel(&plain, "bottom")));
    }

    #[test]
    fn bad_input_is_refused() {
        let ok = spec(true, 0.0);
        assert_eq!(panels(&BoxSpec { width: f64::NAN, ..ok }), Err(BoxError::NonFinite));
        assert_eq!(panels(&BoxSpec { kerf: f64::INFINITY, ..ok }), Err(BoxError::NonFinite));
        assert_eq!(panels(&BoxSpec { height: 0.0, ..ok }), Err(BoxError::NotPositive));
        assert_eq!(panels(&BoxSpec { finger: 0.0, ..ok }), Err(BoxError::NotPositive));
        assert_eq!(panels(&BoxSpec { kerf: -0.1, ..ok }), Err(BoxError::NotPositive));
        assert_eq!(panels(&BoxSpec { depth: 1e9, ..ok }), Err(BoxError::TooLarge));
        assert_eq!(panels(&BoxSpec { kerf: 5.0, ..ok }), Err(BoxError::FingersTooFine));
    }
}
