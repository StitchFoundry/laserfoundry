//! Timelapse frames: steps a planned job's timeline into a fixed-length image sequence, so the same simulation the
//! preview shows can be exported as a short clip. Pure and offline; encoding to GIF/MP4 is a later slice.
//! Shows only what the plan contains (cut moves at their planned times), nothing about real machines.

use crate::{Job, Motion, Move, Timeline, move_times};
use laserfoundry_geom::Point;

/// Default clip length in seconds.
pub const DEFAULT_SECONDS: f64 = 15.0;
/// Default frame rate.
pub const DEFAULT_FPS: u32 = 30;
/// Caps so hostile or silly parameters cannot exhaust memory.
pub const MAX_FRAMES: u32 = 3600;
pub const MAX_FPS: u32 = 60;
pub const MAX_SIDE: u32 = 2048;
const MARGIN: f64 = 0.06;
const BACKGROUND: [u8; 4] = [255, 255, 255, 255];
const DONE: [u8; 4] = [40, 40, 48, 255];
const HEAD: [u8; 4] = [220, 40, 40, 255];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aspect {
    /// 9:16, for Reels and Shorts.
    Vertical,
    Square,
    /// 16:9
    Landscape,
}

impl Aspect {
    /// Pixel size for a frame whose shorter side is `short` pixels (clamped to 16..=[`MAX_SIDE`] / 2).
    pub fn size(self, short: u32) -> (u32, u32) {
        let s = short.clamp(16, MAX_SIDE / 2);
        let long = s * 16 / 9;
        match self {
            Aspect::Vertical => (s, long),
            Aspect::Square => (s, s),
            Aspect::Landscape => (long, s),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum TimelapseError {
    #[error("clip length must be a positive number of seconds")]
    BadDuration,
    #[error("frame rate must be between 1 and {MAX_FPS}")]
    BadFps,
    #[error("the clip would need more than {MAX_FRAMES} frames")]
    TooManyFrames,
}

/// How many frames a clip has and which moment of the job each shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramePlan {
    pub fps: u32,
    pub count: u32,
    /// Seconds of job time at the last frame.
    pub job_seconds: f64,
}

impl FramePlan {
    pub fn new(duration: f64, fps: u32, job_seconds: f64) -> Result<FramePlan, TimelapseError> {
        if !(duration.is_finite() && duration > 0.0) {
            return Err(TimelapseError::BadDuration);
        }
        if fps == 0 || fps > MAX_FPS {
            return Err(TimelapseError::BadFps);
        }
        let frames = (duration * f64::from(fps)).round();
        if frames > f64::from(MAX_FRAMES) {
            return Err(TimelapseError::TooManyFrames);
        }
        let count = (frames as u32).max(2);
        let job_seconds = if job_seconds.is_finite() { job_seconds.max(0.0) } else { 0.0 };
        Ok(FramePlan { fps, count, job_seconds })
    }

    /// Job time shown by `frame`: the first frame is the empty bed, the last is the finished job.
    pub fn job_time(&self, frame: u32) -> f64 {
        let last = f64::from(self.count - 1);
        self.job_seconds * (f64::from(frame.min(self.count - 1)) / last)
    }

    /// Clip length in seconds.
    pub fn duration(&self) -> f64 {
        f64::from(self.count) / f64::from(self.fps)
    }
}

/// One RGBA frame, row-major, top row first.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Draws the job as far as it has run at `job_time`: finished cuts dark, the head a red dot. Returns `None` for a
/// frame size outside 1..=[`MAX_SIDE`] or a job with nothing to draw. Later passes repeat the first pass's path, so
/// only the first pass is drawn.
pub fn render_frame(job: &Job, timeline: &Timeline, rapid_speed: f64, motion: Motion, job_time: f64, width: u32, height: u32) -> Option<Frame> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    let (min, max) = bounds(job)?;
    let (w, h) = (f64::from(width), f64::from(height));
    let span_x = (max.x - min.x).max(1e-6);
    let span_y = (max.y - min.y).max(1e-6);
    let scale = (w * (1.0 - 2.0 * MARGIN) / span_x).min(h * (1.0 - 2.0 * MARGIN) / span_y);
    let (off_x, off_y) = ((w - span_x * scale) / 2.0, (h - span_y * scale) / 2.0);
    // Machine Y grows upward; image rows grow downward.
    let to_px = |p: Point| ((p.x - min.x) * scale + off_x, h - ((p.y - min.y) * scale + off_y));

    let mut frame = Frame { width, height, rgba: BACKGROUND.repeat(width as usize * height as usize) };
    let t = if job_time.is_finite() { job_time.clamp(0.0, timeline.total) } else { 0.0 };
    let mut head = None;
    let mut at = Point::ORIGIN;
    for (op, ot) in job.ops.iter().zip(&timeline.ops) {
        let start = at;
        at = op.moves.last().map_or(at, Move::target);
        let into = (t - ot.start).min(ot.pass_seconds);
        if into <= 0.0 {
            continue;
        }
        let (mut from, mut clock) = (start, 0.0);
        for (m, secs) in op.moves.iter().zip(move_times(op, start, rapid_speed, motion)) {
            let to = m.target();
            let frac = if secs > 0.0 { ((into - clock) / secs).clamp(0.0, 1.0) } else { 1.0 };
            let end = Point::new(from.x + (to.x - from.x) * frac, from.y + (to.y - from.y) * frac);
            if let Move::Cut { power, .. } = m
                && *power > 0.0
            {
                line(&mut frame, to_px(from), to_px(end), DONE);
            }
            head = Some(end);
            if frac < 1.0 {
                break;
            }
            from = to;
            clock += secs;
        }
    }
    if let Some(p) = head {
        let (x, y) = to_px(p);
        dot(&mut frame, x, y, HEAD);
    }
    Some(frame)
}

fn bounds(job: &Job) -> Option<(Point, Point)> {
    let mut b: Option<(Point, Point)> = None;
    for m in job.ops.iter().flat_map(|o| &o.moves) {
        if let Move::Cut { to, power } = m
            && *power > 0.0
            && to.x.is_finite()
            && to.y.is_finite()
        {
            let (lo, hi) = b.get_or_insert((*to, *to));
            lo.x = lo.x.min(to.x);
            lo.y = lo.y.min(to.y);
            hi.x = hi.x.max(to.x);
            hi.y = hi.y.max(to.y);
        }
    }
    b
}

fn put(f: &mut Frame, x: i64, y: i64, c: [u8; 4]) {
    if x < 0 || y < 0 || x >= i64::from(f.width) || y >= i64::from(f.height) {
        return;
    }
    let i = (y as usize * f.width as usize + x as usize) * 4;
    if let Some(px) = f.rgba.get_mut(i..i + 4) {
        px.copy_from_slice(&c);
    }
}

fn dot(f: &mut Frame, x: f64, y: f64, c: [u8; 4]) {
    if !(x.is_finite() && y.is_finite()) {
        return;
    }
    let (cx, cy) = (x.round() as i64, y.round() as i64);
    for dy in -2..=2 {
        for dx in -2..=2 {
            put(f, cx + dx, cy + dy, c);
        }
    }
}

fn line(f: &mut Frame, a: (f64, f64), b: (f64, f64), c: [u8; 4]) {
    if ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite()) {
        return;
    }
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    // One step per pixel, bounded so an absurd line cannot loop for long.
    let steps = len.min(f64::from(MAX_SIDE) * 3.0).ceil().max(1.0) as u32;
    for i in 0..=steps {
        let u = f64::from(i) / f64::from(steps);
        put(f, (a.0 + (b.0 - a.0) * u).round() as i64, (a.1 + (b.1 - a.1) * u).round() as i64, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Op, OpKind};

    #[test]
    fn plan_length_and_stepping() {
        let p = FramePlan::new(15.0, 30, 60.0).unwrap();
        assert_eq!(p.count, 450);
        assert!((p.duration() - 15.0).abs() < 1e-9);
        assert_eq!(p.job_time(0), 0.0);
        assert!((p.job_time(449) - 60.0).abs() < 1e-9);
        assert!(p.job_time(10_000) <= 60.0 + 1e-9);
        assert!(p.job_time(100) < p.job_time(101));
    }

    #[test]
    fn plan_rejects_bad_input() {
        assert_eq!(FramePlan::new(0.0, 30, 1.0), Err(TimelapseError::BadDuration));
        assert_eq!(FramePlan::new(f64::NAN, 30, 1.0), Err(TimelapseError::BadDuration));
        assert_eq!(FramePlan::new(1.0, 0, 1.0), Err(TimelapseError::BadFps));
        assert_eq!(FramePlan::new(1.0, 61, 1.0), Err(TimelapseError::BadFps));
        assert_eq!(FramePlan::new(1e9, 30, 1.0), Err(TimelapseError::TooManyFrames));
        assert_eq!(FramePlan::new(0.01, 1, f64::INFINITY).unwrap().job_seconds, 0.0);
    }

    #[test]
    fn aspect_sizes() {
        assert_eq!(Aspect::Vertical.size(540), (540, 960));
        assert_eq!(Aspect::Square.size(540), (540, 540));
        assert_eq!(Aspect::Landscape.size(540), (960, 540));
        assert_eq!(Aspect::Square.size(u32::MAX).0, MAX_SIDE / 2);
    }

    #[test]
    fn frames_grow_as_the_job_runs() {
        let job = square_job();
        let tl = Timeline::new(&job, 6000.0);
        let m = Motion::default();
        let dark = |t: f64| {
            let f = render_frame(&job, &tl, 6000.0, m, t, 100, 100).unwrap();
            f.rgba.chunks(4).filter(|p| **p == DONE).count()
        };
        assert_eq!(dark(0.0), 0);
        let half = dark(tl.total / 2.0);
        let full = dark(tl.total);
        assert!(half > 0 && full > half, "{half} {full}");
        assert_eq!(dark(f64::NAN), 0);
    }

    #[test]
    fn bad_sizes_and_empty_jobs_give_no_frame() {
        let job = square_job();
        let tl = Timeline::new(&job, 6000.0);
        let m = Motion::default();
        assert!(render_frame(&job, &tl, 6000.0, m, 0.0, 0, 10).is_none());
        assert!(render_frame(&job, &tl, 6000.0, m, 0.0, MAX_SIDE + 1, 10).is_none());
        assert!(render_frame(&Job::default(), &Timeline::default(), 6000.0, m, 0.0, 10, 10).is_none());
    }

    fn square_job() -> Job {
        let cut = |x, y| Move::Cut { to: Point::new(x, y), power: 50.0 };
        let moves = vec![Move::Travel(Point::new(0.0, 0.0)), cut(10.0, 0.0), cut(10.0, 10.0), cut(0.0, 10.0), cut(0.0, 0.0)];
        Job { ops: vec![Op { layer: 0, kind: OpKind::Line, speed: 600.0, passes: 1, air_assist: false, z: 0.0, z_per_pass: 0.0, moves }] }
    }
}
