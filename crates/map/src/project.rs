//! Projection of map positions (degrees) to bed millimetres (phase 2 of the maps proposal, issue #150).
//!
//! Original design: a local equirectangular projection. East-west distances are scaled by the cosine of the
//! middle latitude of the area, so a small place keeps its shape; the result is then scaled uniformly to fit a
//! width and height in millimetres. Output has the south-west corner of the area at (0, 0), x growing east and
//! y growing north (the bed's own orientation: flip y for an image-style top-down canvas).
//!
//! Anything derived from OpenStreetMap data must still carry [`crate::OSM_CREDIT`] (ODbL).

use crate::{Geometry, LonLat, MapData};

/// A point on the bed in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mm {
    pub x: f64,
    pub y: f64,
}

/// The lon/lat box of an area, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: LonLat,
    pub max: LonLat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProjectError {
    #[error("there is no map data to fit")]
    Empty,
    #[error("the area has no extent")]
    NoExtent,
    #[error("the target size must be a positive, finite number of millimetres")]
    BadSize,
}

/// A fitted projection from lon/lat to bed millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    origin: LonLat,
    /// Millimetres per degree of latitude.
    mm_per_deg: f64,
    /// Cosine of the middle latitude, applied to longitude differences.
    lon_scale: f64,
    width_mm: f64,
    height_mm: f64,
}

impl Bounds {
    /// The box around every position in `data`; `None` when there are none.
    pub fn of(data: &MapData) -> Option<Bounds> {
        let mut bounds: Option<Bounds> = None;
        let mut add = |p: &LonLat| {
            let b = bounds.get_or_insert(Bounds { min: *p, max: *p });
            b.min = LonLat { lon: b.min.lon.min(p.lon), lat: b.min.lat.min(p.lat) };
            b.max = LonLat { lon: b.max.lon.max(p.lon), lat: b.max.lat.max(p.lat) };
        };
        for g in data.features.iter().flat_map(|f| &f.geometries) {
            match g {
                Geometry::Point(p) => add(p),
                Geometry::Line(pts) => pts.iter().for_each(&mut add),
                Geometry::Polygon(rings) => rings.iter().flatten().for_each(&mut add),
            }
        }
        bounds
    }
}

impl Projection {
    /// Fits `bounds` into `max_width_mm` x `max_height_mm`, keeping the aspect ratio (the area touches at least
    /// one of the two limits).
    pub fn fit(bounds: Bounds, max_width_mm: f64, max_height_mm: f64) -> Result<Projection, ProjectError> {
        if !(max_width_mm.is_finite() && max_height_mm.is_finite() && max_width_mm > 0.0 && max_height_mm > 0.0) {
            return Err(ProjectError::BadSize);
        }
        let lon_scale = ((bounds.min.lat + bounds.max.lat) / 2.0).to_radians().cos().abs();
        let dx = (bounds.max.lon - bounds.min.lon) * lon_scale;
        let dy = bounds.max.lat - bounds.min.lat;
        if !(dx.is_finite() && dy.is_finite()) || dx < 0.0 || dy < 0.0 {
            return Err(ProjectError::NoExtent);
        }
        // The tightest limit wins; a zero extent along one axis does not limit it.
        let mut mm_per_deg = f64::INFINITY;
        if dx > 0.0 {
            mm_per_deg = mm_per_deg.min(max_width_mm / dx);
        }
        if dy > 0.0 {
            mm_per_deg = mm_per_deg.min(max_height_mm / dy);
        }
        if !mm_per_deg.is_finite() || mm_per_deg <= 0.0 {
            return Err(ProjectError::NoExtent);
        }
        Ok(Projection { origin: bounds.min, mm_per_deg, lon_scale, width_mm: dx * mm_per_deg, height_mm: dy * mm_per_deg })
    }

    /// Fits all of `data` (see [`Projection::fit`]).
    pub fn fit_data(data: &MapData, max_width_mm: f64, max_height_mm: f64) -> Result<Projection, ProjectError> {
        let bounds = Bounds::of(data).ok_or(ProjectError::Empty)?;
        Projection::fit(bounds, max_width_mm, max_height_mm)
    }

    /// Size of the fitted area in millimetres.
    pub fn size_mm(&self) -> (f64, f64) {
        (self.width_mm, self.height_mm)
    }

    pub fn project(&self, p: LonLat) -> Mm {
        Mm { x: (p.lon - self.origin.lon) * self.lon_scale * self.mm_per_deg, y: (p.lat - self.origin.lat) * self.mm_per_deg }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Feature;

    #[test]
    fn fits_wide_area_to_width_and_keeps_south_west_at_origin() {
        let b = Bounds { min: ll(10.0, 0.0), max: ll(12.0, 1.0) };
        let p = Projection::fit(b, 200.0, 200.0).unwrap();
        let (w, h) = p.size_mm();
        // cos(0.5 degrees) is just under 1, so the width limit is the tight one.
        assert!((w - 200.0).abs() < 1e-9, "{w} x {h}");
        assert!(h < w / 2.0 + 1.0 && h > w / 2.0 - 1.0, "{w} x {h}");
        assert_eq!(p.project(ll(10.0, 0.0)), Mm { x: 0.0, y: 0.0 });
        let ne = p.project(ll(12.0, 1.0));
        assert!((ne.x - w).abs() < 1e-9 && (ne.y - h).abs() < 1e-9);
    }

    #[test]
    fn east_west_shrinks_with_latitude() {
        let near_pole = Projection::fit(Bounds { min: ll(0.0, 59.5), max: ll(1.0, 60.5) }, 100.0, 100.0).unwrap();
        let (w, h) = near_pole.size_mm();
        assert!((w / h - 0.5).abs() < 0.01, "cos 60 degrees is about 0.5: {}", w / h);
    }

    #[test]
    fn line_of_points_fits_its_one_extent() {
        let p = Projection::fit_data(&data(&[ll(5.0, 50.0), ll(5.0, 50.5)]), 80.0, 40.0).unwrap();
        assert_eq!(p.size_mm().1, 40.0);
        assert_eq!(p.size_mm().0, 0.0);
    }

    #[test]
    fn refuses_bad_input() {
        let b = Bounds { min: ll(1.0, 1.0), max: ll(1.0, 1.0) };
        assert_eq!(Projection::fit(b, 10.0, 10.0), Err(ProjectError::NoExtent));
        let b = Bounds { min: ll(0.0, 0.0), max: ll(1.0, 1.0) };
        for (w, h) in [(0.0, 5.0), (-1.0, 5.0), (f64::NAN, 5.0), (5.0, f64::INFINITY)] {
            assert_eq!(Projection::fit(b, w, h), Err(ProjectError::BadSize));
        }
        assert_eq!(Projection::fit_data(&MapData::default(), 10.0, 10.0), Err(ProjectError::Empty));
        // Inverted or non-finite boxes are refused rather than producing garbage.
        let inverted = Bounds { min: ll(2.0, 0.0), max: ll(1.0, 1.0) };
        assert_eq!(Projection::fit(inverted, 10.0, 10.0), Err(ProjectError::NoExtent));
        let nan = Bounds { min: ll(f64::NAN, 0.0), max: ll(1.0, 1.0) };
        assert_eq!(Projection::fit(nan, 10.0, 10.0), Err(ProjectError::NoExtent));
    }

    #[test]
    fn extreme_latitudes_stay_finite() {
        let p = Projection::fit(Bounds { min: ll(-180.0, 89.0), max: ll(180.0, 90.0) }, 100.0, 100.0).unwrap();
        let m = p.project(ll(180.0, 90.0));
        assert!(m.x.is_finite() && m.y.is_finite());
    }

    fn ll(lon: f64, lat: f64) -> LonLat {
        LonLat { lon, lat }
    }

    fn data(points: &[LonLat]) -> MapData {
        MapData { features: vec![Feature { geometries: points.iter().map(|p| Geometry::Point(*p)).collect(), ..Feature::default() }] }
    }
}
