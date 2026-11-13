//! Camera maths for placing artwork on material seen by an overhead webcam. Pure functions, no I/O, so it
//! runs in the browser build too. Camera frames never enter this crate; it only maps points.
//!
//! Lens model: a polynomial in the incidence angle (the common equidistant "fisheye" form,
//! r = f * (t + k1 t^3 + k2 t^5), t = angle from the optical axis); see the public OpenCV-style camera
//! model description at <https://docs.opencv.org/4.x/db/d58/group__calib3d__fisheye.html> (documentation only).
//! Homography: direct linear transform from four point pairs, <https://en.wikipedia.org/wiki/Homography_(computer_vision)>.

use serde::{Deserialize, Serialize};

const MAX_SIDE_MM: f64 = 100_000.0;
const MAX_PROFILE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CameraError {
    #[error("a value is not a finite number")]
    NotFinite,
    #[error("the four points do not form a usable quadrilateral (some are repeated or in a line)")]
    Degenerate,
    #[error("the profile is invalid: {0}")]
    Profile(String),
    #[error("the size is out of range")]
    Size,
}

/// Fisheye intrinsics in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Lens {
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub k1: f64,
    pub k2: f64,
}

/// A 3x3 projective map, row-major, with `h[8] == 1`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Homography(pub [f64; 9]);

/// Saved per camera (USB vendor/product id and resolution), exportable as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraProfile {
    pub vendor_id: u16,
    pub product_id: u16,
    pub width: u32,
    pub height: u32,
    pub lens: Lens,
    /// Undistorted pixels to bed millimetres, once the 4-point alignment has been done.
    pub bed: Option<Homography>,
    /// The machine the alignment belongs to.
    pub machine: String,
}

impl Lens {
    /// Distorted pixel to undistorted (pinhole) pixel with the same focal lengths. `None` when the input is
    /// not finite or the lens model cannot be inverted for it.
    pub fn undistort(&self, px: f64, py: f64) -> Option<(f64, f64)> {
        if ![px, py, self.fx, self.fy, self.cx, self.cy, self.k1, self.k2].iter().all(|v| v.is_finite()) || self.fx == 0.0 || self.fy == 0.0 {
            return None;
        }
        let (x, y) = ((px - self.cx) / self.fx, (py - self.cy) / self.fy);
        let theta_d = x.hypot(y);
        if theta_d < 1e-12 {
            return Some((self.cx, self.cy));
        }
        // Newton's method on theta + k1 theta^3 + k2 theta^5 = theta_d.
        let mut t = theta_d;
        for _ in 0..30 {
            let t2 = t * t;
            let f = t * (1.0 + self.k1 * t2 + self.k2 * t2 * t2) - theta_d;
            let d = 1.0 + 3.0 * self.k1 * t2 + 5.0 * self.k2 * t2 * t2;
            if !d.is_finite() || d.abs() < 1e-9 {
                return None;
            }
            t -= f / d;
            if !t.is_finite() {
                return None;
            }
        }
        if !(0.0..std::f64::consts::FRAC_PI_2 - 1e-3).contains(&t) {
            return None;
        }
        let scale = t.tan() / theta_d;
        Some((self.cx + x * scale * self.fx, self.cy + y * scale * self.fy))
    }

    /// Pinhole pixel to distorted pixel (the forward model; used by tests and for drawing).
    pub fn distort(&self, px: f64, py: f64) -> Option<(f64, f64)> {
        if ![px, py, self.fx, self.fy, self.cx, self.cy, self.k1, self.k2].iter().all(|v| v.is_finite()) || self.fx == 0.0 || self.fy == 0.0 {
            return None;
        }
        let (x, y) = ((px - self.cx) / self.fx, (py - self.cy) / self.fy);
        let r = x.hypot(y);
        if r < 1e-12 {
            return Some((self.cx, self.cy));
        }
        let t = r.atan();
        let t2 = t * t;
        let td = t * (1.0 + self.k1 * t2 + self.k2 * t2 * t2);
        let s = td / r;
        let out = (self.cx + x * s * self.fx, self.cy + y * s * self.fy);
        (out.0.is_finite() && out.1.is_finite()).then_some(out)
    }
}

impl Homography {
    /// Solve the map taking each `src[i]` to `dst[i]` (exactly four pairs).
    pub fn from_four(src: [(f64, f64); 4], dst: [(f64, f64); 4]) -> Result<Self, CameraError> {
        if src.iter().chain(dst.iter()).any(|p| !p.0.is_finite() || !p.1.is_finite()) {
            return Err(CameraError::NotFinite);
        }
        let mut a = [[0.0f64; 9]; 8];
        for i in 0..4 {
            let ((x, y), (u, v)) = (src[i], dst[i]);
            a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
            a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
        }
        // Gauss-Jordan with partial pivoting on the 8x8 system.
        for col in 0..8 {
            let piv = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs())).ok_or(CameraError::Degenerate)?;
            if a[piv][col].abs() < 1e-9 {
                return Err(CameraError::Degenerate);
            }
            a.swap(col, piv);
            let p = a[col][col];
            for v in a[col].iter_mut().skip(col) {
                *v /= p;
            }
            let pivot_row = a[col];
            for (r, row) in a.iter_mut().enumerate() {
                if r != col {
                    let f = row[col];
                    if f != 0.0 {
                        for (v, pv) in row.iter_mut().zip(pivot_row.iter()).skip(col) {
                            *v -= f * pv;
                        }
                    }
                }
            }
        }
        let h = [a[0][8], a[1][8], a[2][8], a[3][8], a[4][8], a[5][8], a[6][8], a[7][8], 1.0];
        if h.iter().any(|v| !v.is_finite()) {
            return Err(CameraError::Degenerate);
        }
        Ok(Self(h))
    }

    pub fn apply(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let h = &self.0;
        let w = h[6] * x + h[7] * y + h[8];
        if !x.is_finite() || !y.is_finite() || w.abs() < 1e-12 {
            return None;
        }
        let out = ((h[0] * x + h[1] * y + h[2]) / w, (h[3] * x + h[4] * y + h[5]) / w);
        (out.0.is_finite() && out.1.is_finite()).then_some(out)
    }
}

impl CameraProfile {
    /// Camera pixel (as captured) to bed millimetres: undistort, then the alignment homography.
    pub fn pixel_to_bed(&self, px: f64, py: f64) -> Option<(f64, f64)> {
        let (ux, uy) = self.lens.undistort(px, py)?;
        self.bed?.apply(ux, uy)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Import a profile; size-capped and checked for finite numbers.
    pub fn from_json(text: &str) -> Result<Self, CameraError> {
        if text.len() > MAX_PROFILE_BYTES {
            return Err(CameraError::Profile("too large".into()));
        }
        let p: Self = serde_json::from_str(text).map_err(|e| CameraError::Profile(e.to_string()))?;
        let l = p.lens;
        let finite = [l.fx, l.fy, l.cx, l.cy, l.k1, l.k2].iter().all(|v| v.is_finite()) && p.bed.is_none_or(|h| h.0.iter().all(|v| v.is_finite()));
        if !finite || l.fx == 0.0 || l.fy == 0.0 || p.width == 0 || p.height == 0 || p.width > 16_384 || p.height > 16_384 {
            return Err(CameraError::Profile("values out of range".into()));
        }
        Ok(p)
    }
}

/// Where to burn the four alignment crosses: `inset` mm in from each bed corner, clockwise from the
/// origin corner. Refuses a bed too small to hold them.
pub fn alignment_marks(bed_w: f64, bed_h: f64, inset: f64) -> Result<[(f64, f64); 4], CameraError> {
    if ![bed_w, bed_h, inset].iter().all(|v| v.is_finite()) {
        return Err(CameraError::NotFinite);
    }
    if bed_w > MAX_SIDE_MM || bed_h > MAX_SIDE_MM || inset < 0.0 || bed_w < 2.0 * inset + 1.0 || bed_h < 2.0 * inset + 1.0 {
        return Err(CameraError::Size);
    }
    Ok([(inset, inset), (bed_w - inset, inset), (bed_w - inset, bed_h - inset), (inset, bed_h - inset)])
}

/// Printable checkerboard as SVG (`cols` x `rows` squares of `square_mm`), for lens calibration.
pub fn checkerboard_svg(cols: u32, rows: u32, square_mm: f64) -> Result<String, CameraError> {
    if !square_mm.is_finite() || !(1.0..=200.0).contains(&square_mm) || !(3..=40).contains(&cols) || !(3..=40).contains(&rows) {
        return Err(CameraError::Size);
    }
    let (w, h) = (cols as f64 * square_mm, rows as f64 * square_mm);
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">\n");
    for r in 0..rows {
        for c in 0..cols {
            if (r + c) % 2 == 0 {
                s.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{square_mm}\" height=\"{square_mm}\" fill=\"#000\"/>\n",
                    c as f64 * square_mm,
                    r as f64 * square_mm
                ));
            }
        }
    }
    s.push_str("</svg>\n");
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_error_under_half_a_millimetre_across_a_400mm_bed() {
        let lens = test_lens();
        // Synthetic truth: bed mm -> pinhole px is a known projective map; the camera then adds lens distortion.
        let truth = Homography([2.1, 0.15, 150.0, -0.1, 2.0, 100.0, 1e-4, 5e-5, 1.0]);
        let to_px = |x: f64, y: f64| truth.apply(x, y).and_then(|(u, v)| lens.distort(u, v));
        let marks = alignment_marks(400.0, 400.0, 10.0).unwrap();
        let src = marks.map(|(x, y)| {
            let (px, py) = to_px(x, y).unwrap();
            lens.undistort(px, py).unwrap()
        });
        let h = Homography::from_four(src, marks).unwrap();
        let profile = CameraProfile { vendor_id: 1, product_id: 2, width: 1920, height: 1080, lens, bed: Some(h), machine: "m".into() };
        let mut worst = 0.0f64;
        for i in 0..=20 {
            for j in 0..=20 {
                let (x, y) = (i as f64 * 20.0, j as f64 * 20.0);
                let (px, py) = to_px(x, y).unwrap();
                let (bx, by) = profile.pixel_to_bed(px, py).unwrap();
                worst = worst.max((bx - x).hypot(by - y));
            }
        }
        assert!(worst <= 0.5, "worst error {worst} mm");
    }

    #[test]
    fn lens_round_trips() {
        let lens = test_lens();
        for (x, y) in [(100.0, 80.0), (960.0, 540.0), (1800.0, 1000.0), (-300.0, 50.0)] {
            let (dx, dy) = lens.distort(x, y).unwrap();
            let (ux, uy) = lens.undistort(dx, dy).unwrap();
            assert!((ux - x).hypot(uy - y) < 1e-6);
        }
    }

    #[test]
    fn hostile_input_is_refused_not_panicked() {
        let p = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        assert_eq!(Homography::from_four([(f64::NAN, 0.0), p[1], p[2], p[3]], p), Err(CameraError::NotFinite));
        assert_eq!(Homography::from_four([p[0]; 4], p), Err(CameraError::Degenerate));
        assert_eq!(Homography::from_four([(0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (3.0, 3.0)], p), Err(CameraError::Degenerate));
        assert!(Homography([0.0; 9]).apply(1.0, 1.0).is_none());
        let zero = Lens { fx: 0.0, ..test_lens() };
        assert!(zero.undistort(1.0, 1.0).is_none() && zero.distort(1.0, 1.0).is_none());
        assert!(test_lens().undistort(f64::INFINITY, 0.0).is_none());
        assert!(test_lens().undistort(1e12, 1e12).is_none());
        assert!(alignment_marks(f64::NAN, 100.0, 5.0).is_err());
        assert!(alignment_marks(10.0, 10.0, 8.0).is_err());
        assert!(checkerboard_svg(0, 5, 20.0).is_err() && checkerboard_svg(9, 6, f64::NAN).is_err() && checkerboard_svg(9, 6, 0.0).is_err());
    }

    #[test]
    fn profile_json_round_trips_and_rejects_bad_files() {
        let h = Homography::from_four([(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], [(1.0, 1.0), (21.0, 1.0), (21.0, 21.0), (1.0, 21.0)])
            .unwrap();
        let p = CameraProfile {
            vendor_id: 0x046d,
            product_id: 0x0825,
            width: 1920,
            height: 1080,
            lens: test_lens(),
            bed: Some(h),
            machine: "Test".into(),
        };
        assert_eq!(CameraProfile::from_json(&p.to_json()).unwrap(), p);
        for bad in ["", "{", "null", "[]", &"x".repeat(MAX_PROFILE_BYTES + 1), &p.to_json().replace("1920", "0")] {
            assert!(CameraProfile::from_json(bad).is_err());
        }
        assert!(CameraProfile { bed: None, ..p }.pixel_to_bed(10.0, 10.0).is_none());
    }

    #[test]
    fn checkerboard_has_half_the_squares() {
        let svg = checkerboard_svg(9, 6, 25.0).unwrap();
        assert_eq!(svg.matches("<rect").count(), 27);
        assert!(svg.contains("width=\"225mm\""));
    }

    fn test_lens() -> Lens {
        Lens { fx: 700.0, fy: 700.0, cx: 960.0, cy: 540.0, k1: -0.05, k2: 0.01 }
    }
}
