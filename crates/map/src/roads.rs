//! Roads as outline polygons (phase 3, first piece, of the maps proposal, issue #150).
//!
//! Original design: a road class (the `highway` tag) maps to a width in millimetres at a chosen scale, and a
//! centre line in bed millimetres is widened into one closed outline: the left edge forward, then the right edge
//! back, with flat (butt) ends. Corners are mitred; a corner sharper than [`MITRE_LIMIT`] is bevelled so a spike
//! never shoots out. Overlaps at junctions are left for a later union step (`pathops`).
//!
//! Anything derived from OpenStreetMap data must still carry [`crate::OSM_CREDIT`] (ODbL).

use crate::Mm;

/// Most centre-line points one road may have (the same cap as the importer's vertices per feature).
pub const MAX_ROAD_POINTS: usize = crate::MAX_VERTICES_PER_FEATURE;

/// A mitre is used only while `1 + cos(turn)` stays above this (so the spike is under 1 / this times the half width).
pub const MITRE_LIMIT: f64 = 0.25;

/// Widest road outline drawn, mm: wider than any bed, so anything above is a misread scale.
pub const MAX_ROAD_WIDTH_MM: f64 = 10_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RoadError {
    #[error("the road width must be a positive, finite number of millimetres, at most {limit}", limit = MAX_ROAD_WIDTH_MM)]
    BadWidth,
    #[error("a road needs at least two distinct, finite points, and at most {limit}", limit = MAX_ROAD_POINTS)]
    BadLine,
}

/// The drawn width in millimetres for an OpenStreetMap `highway` value, given `mm_per_metre` (the map scale:
/// millimetres on the bed per metre on the ground). Widths are rough real-world widths in metres. `None` for a
/// class that is not a road for cutting (footpaths and the like are narrow lines, not outlines), a bad scale, or a
/// width over [`MAX_ROAD_WIDTH_MM`].
/// Classes: <https://wiki.openstreetmap.org/wiki/Key:highway>.
pub fn road_width_mm(highway: &str, mm_per_metre: f64) -> Option<f64> {
    if !(mm_per_metre.is_finite() && mm_per_metre > 0.0) {
        return None;
    }
    let metres = match highway {
        "motorway" | "trunk" => 14.0,
        "primary" => 10.0,
        "secondary" => 8.0,
        "tertiary" => 7.0,
        "residential" | "unclassified" | "living_street" | "service" => 5.0,
        _ => return None,
    };
    Some(metres * mm_per_metre).filter(|w| w.is_finite() && *w <= MAX_ROAD_WIDTH_MM)
}

/// Widens a centre line into a closed outline of `width_mm` (see the module docs). Repeated points are dropped; an
/// outline that would not be finite is refused.
pub fn offset_polyline(line: &[Mm], width_mm: f64) -> Result<Vec<Mm>, RoadError> {
    if !(width_mm.is_finite() && width_mm > 0.0 && width_mm <= MAX_ROAD_WIDTH_MM) {
        return Err(RoadError::BadWidth);
    }
    if line.len() > MAX_ROAD_POINTS {
        return Err(RoadError::BadLine);
    }
    let half = width_mm / 2.0;
    let mut pts: Vec<Mm> = Vec::with_capacity(line.len());
    for p in line {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return Err(RoadError::BadLine);
        }
        if pts.last().is_none_or(|q| q.x != p.x || q.y != p.y) {
            pts.push(*p);
        }
    }
    // Unit direction of each segment; a segment of zero or non-finite length is refused.
    let mut dirs: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
    for pair in pts.windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else { continue };
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len = dx.hypot(dy);
        if !(len.is_finite() && len > 0.0) {
            return Err(RoadError::BadLine);
        }
        dirs.push((dx / len, dy / len));
    }
    if dirs.is_empty() {
        return Err(RoadError::BadLine);
    }
    let mut left: Vec<Mm> = Vec::with_capacity(pts.len() + 2);
    let mut right: Vec<Mm> = Vec::with_capacity(pts.len() + 2);
    for (i, p) in pts.iter().enumerate() {
        let before = if i == 0 { None } else { dirs.get(i - 1) };
        let after = dirs.get(i);
        // Left normals of the segments meeting at this point.
        let n1 = before.or(after).map(|d| (-d.1, d.0));
        let n2 = after.or(before).map(|d| (-d.1, d.0));
        let (Some(n1), Some(n2)) = (n1, n2) else { continue };
        let dot = n1.0 * n2.0 + n1.1 * n2.1;
        if 1.0 + dot > MITRE_LIMIT {
            let k = half / (1.0 + dot);
            let (ox, oy) = ((n1.0 + n2.0) * k, (n1.1 + n2.1) * k);
            left.push(Mm { x: p.x + ox, y: p.y + oy });
            right.push(Mm { x: p.x - ox, y: p.y - oy });
        } else {
            for n in [n1, n2] {
                left.push(Mm { x: p.x + n.0 * half, y: p.y + n.1 * half });
                right.push(Mm { x: p.x - n.0 * half, y: p.y - n.1 * half });
            }
        }
    }
    right.reverse();
    left.extend(right);
    // Finite points can still overflow once the half width is added (issue #192).
    if left.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
        return Err(RoadError::BadLine);
    }
    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(x: f64, y: f64) -> Mm {
        Mm { x, y }
    }

    #[test]
    fn widths_by_class() {
        assert_eq!(road_width_mm("residential", 0.5), Some(2.5));
        assert_eq!(road_width_mm("motorway", 1.0), Some(14.0));
        assert_eq!(road_width_mm("footway", 1.0), None);
        assert_eq!(road_width_mm("residential", 0.0), None);
        assert_eq!(road_width_mm("residential", f64::NAN), None);
    }

    #[test]
    fn straight_road_is_a_rectangle() {
        let out = offset_polyline(&[mm(0.0, 0.0), mm(10.0, 0.0)], 2.0).unwrap();
        assert_eq!(out, vec![mm(0.0, 1.0), mm(10.0, 1.0), mm(10.0, -1.0), mm(0.0, -1.0)]);
    }

    #[test]
    fn right_angle_is_mitred() {
        let out = offset_polyline(&[mm(0.0, 0.0), mm(10.0, 0.0), mm(10.0, 10.0)], 2.0).unwrap();
        assert_eq!(out.len(), 6);
        // Outer corner of the turn is at (11, -1), inner at (9, 1).
        assert!(out.iter().any(|p| (p.x - 11.0).abs() < 1e-9 && (p.y + 1.0).abs() < 1e-9));
        assert!(out.iter().any(|p| (p.x - 9.0).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9));
    }

    #[test]
    fn hairpin_is_bevelled_not_spiked() {
        let out = offset_polyline(&[mm(0.0, 0.0), mm(10.0, 0.0), mm(0.0, 0.1)], 2.0).unwrap();
        assert!(out.iter().all(|p| p.x.abs() < 20.0 && p.y.abs() < 20.0));
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn repeated_points_are_dropped() {
        let out = offset_polyline(&[mm(0.0, 0.0), mm(0.0, 0.0), mm(10.0, 0.0)], 2.0).unwrap();
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn refuses_bad_input() {
        let ok = [mm(0.0, 0.0), mm(1.0, 0.0)];
        for w in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(offset_polyline(&ok, w), Err(RoadError::BadWidth));
        }
        assert_eq!(offset_polyline(&[], 1.0), Err(RoadError::BadLine));
        assert_eq!(offset_polyline(&[mm(0.0, 0.0)], 1.0), Err(RoadError::BadLine));
        assert_eq!(offset_polyline(&[mm(1.0, 1.0), mm(1.0, 1.0)], 1.0), Err(RoadError::BadLine));
        assert_eq!(offset_polyline(&[mm(0.0, 0.0), mm(f64::NAN, 1.0)], 1.0), Err(RoadError::BadLine));
        assert_eq!(offset_polyline(&[mm(0.0, 0.0), mm(f64::MAX, f64::MAX)], 1.0), Err(RoadError::BadLine));
        let long = vec![mm(0.0, 0.0); MAX_ROAD_POINTS + 1];
        assert_eq!(offset_polyline(&long, 1.0), Err(RoadError::BadLine));
    }

    #[test]
    fn results_that_would_overflow_are_refused_not_returned() {
        // Issue #192: finite inputs whose result overflows to infinity.
        assert_eq!(road_width_mm("motorway", 1e308), None);
        assert_eq!(offset_polyline(&[mm(0.0, 1.5e308), mm(1.0, 1.5e308)], 1e308), Err(RoadError::BadWidth));
        assert_eq!(offset_polyline(&[mm(0.0, 0.0), mm(1.0, 0.0)], MAX_ROAD_WIDTH_MM + 1.0), Err(RoadError::BadWidth));
        let edge = offset_polyline(&[mm(0.0, f64::MAX), mm(1.0, f64::MAX), mm(1.0, 0.0)], MAX_ROAD_WIDTH_MM).unwrap();
        assert!(edge.iter().all(|p| p.x.is_finite() && p.y.is_finite()), "{edge:?}");
    }

    #[test]
    fn out_and_back_is_bevelled_without_nan() {
        let out = offset_polyline(&[mm(0.0, 0.0), mm(5.0, 0.0), mm(0.0, 0.0)], 1.0).unwrap();
        assert!(out.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    }
}
