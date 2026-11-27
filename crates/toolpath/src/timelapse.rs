//! Timelapse frames: draws the planned cuts up to a moment of the job, for a short clip. The same plan and
//! renderer are meant to be shared unchanged with the stitch-out clip in the sibling app.
//!
//! It shows only what the time model shows: the cuts made so far and where the head is. No files, threads or network.

use crate::{Job, Motion, Move, Timeline, move_times};
use laserfoundry_geom::Point;

/// Clip length used when the caller has no preference, in seconds.
pub const DEFAULT_CLIP_SECONDS: f64 = 15.0;
/// Frame rate used when the caller has no preference.
pub const DEFAULT_FPS: u32 = 30;
/// Caps so a bad request cannot ask for an endless clip.
pub const MAX_FPS: u32 = 60;
pub const MAX_FRAMES: usize = 3600;
/// Largest frame side accepted, in pixels.
pub const MAX_SIDE: u32 = 4096;
/// Cuts kept for drawing; a larger job is refused rather than drawn partly.
pub const MAX_SEGMENTS: usize = 2_000_000;

const BACKGROUND: [u8; 4] = [250, 250, 247, 255];
const INK: [u8; 4] = [30, 30, 36, 255];
const HEAD: [u8; 4] = [220, 40, 40, 255];

/// How many frames a clip has and which moment of the job each one shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramePlan {
    pub frames: usize,
    pub fps: u32,
    pub job_seconds: f64,
}

impl FramePlan {
    /// `clip_seconds` of video at `fps` for a job of `job_seconds`. Bad numbers fall back to the defaults, the frame
    /// rate is capped at [`MAX_FPS`] and the count at [`MAX_FRAMES`]. A job with no time still gets two frames.
    pub fn new(clip_seconds: f64, fps: u32, job_seconds: f64) -> FramePlan {
        let clip = if clip_seconds.is_finite() && clip_seconds > 0.0 { clip_seconds } else { DEFAULT_CLIP_SECONDS };
        let fps = if fps == 0 { DEFAULT_FPS } else { fps.min(MAX_FPS) };
        let frames = ((clip * f64::from(fps)).round() as usize).clamp(2, MAX_FRAMES);
        let job_seconds = if job_seconds.is_finite() { job_seconds.max(0.0) } else { 0.0 };
        FramePlan { frames, fps, job_seconds }
    }

    /// The job time frame `index` shows: the first frame is the empty bed, the last is the finished job.
    pub fn job_time(&self, index: usize) -> f64 {
        let last = self.frames.saturating_sub(1).max(1);
        self.job_seconds * (index.min(last) as f64) / last as f64
    }

    /// Length of the clip in seconds.
    pub fn clip_seconds(&self) -> f64 {
        self.frames as f64 / f64::from(self.fps.max(1))
    }
}

/// Frame shapes for the usual short-video sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aspect {
    Vertical,
    Square,
    Wide,
}

impl Aspect {
    /// Width and height in pixels: 1080 wide for vertical and square, 1920 wide for wide.
    pub fn size(self) -> (u32, u32) {
        match self {
            Aspect::Vertical => (1080, 1920),
            Aspect::Square => (1080, 1080),
            Aspect::Wide => (1920, 1080),
        }
    }
}

/// One RGBA frame, row by row from the top.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct Seg {
    op: usize,
    /// Seconds into one pass of the op.
    start: f64,
    end: f64,
    a: (f64, f64),
    b: (f64, f64),
}

/// A job prepared for drawing at any moment.
#[derive(Debug, Clone)]
pub struct Timelapse {
    timeline: Timeline,
    segs: Vec<Seg>,
    min: (f64, f64),
    max: (f64, f64),
}

impl Timelapse {
    /// None for a job with no cuts, with non-finite coordinates, or with more than [`MAX_SEGMENTS`] cuts.
    pub fn new(job: &Job, rapid_speed: f64, motion: Motion) -> Option<Timelapse> {
        let timeline = Timeline::with_motion(job, rapid_speed, motion);
        let mut segs = Vec::new();
        let mut at = Point::ORIGIN;
        let (mut min, mut max) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
        for (i, op) in job.ops.iter().enumerate() {
            let times = move_times(op, at, rapid_speed, motion);
            let mut t = 0.0;
            for (m, dt) in op.moves.iter().zip(times) {
                let to = m.target();
                if !(to.x.is_finite() && to.y.is_finite() && dt.is_finite()) {
                    return None;
                }
                if matches!(m, Move::Cut { power, .. } if *power > 0.0) {
                    if segs.len() >= MAX_SEGMENTS {
                        return None;
                    }
                    segs.push(Seg { op: i, start: t, end: t + dt, a: (at.x, at.y), b: (to.x, to.y) });
                    for p in [(at.x, at.y), (to.x, to.y)] {
                        min = (min.0.min(p.0), min.1.min(p.1));
                        max = (max.0.max(p.0), max.1.max(p.1));
                    }
                }
                t += dt;
                at = to;
            }
        }
        if segs.is_empty() {
            return None;
        }
        Some(Timelapse { timeline, segs, min, max })
    }

    /// Seconds the whole job takes.
    pub fn job_seconds(&self) -> f64 {
        self.timeline.total
    }

    /// Draws the cuts made by job time `t` on a plain background, with a dot at the head. None for a side that is
    /// zero or above [`MAX_SIDE`].
    pub fn render(&self, t: f64, width: u32, height: u32) -> Option<Frame> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return None;
        }
        let mut frame = Frame { width, height, rgba: BACKGROUND.repeat(width as usize * height as usize) };
        let moment = self.timeline.at(t)?;
        let margin = 0.08 * f64::from(width.min(height));
        let span = ((self.max.0 - self.min.0).max(1e-6), (self.max.1 - self.min.1).max(1e-6));
        let scale = ((f64::from(width) - 2.0 * margin) / span.0).min((f64::from(height) - 2.0 * margin) / span.1);
        let off = ((f64::from(width) - span.0 * scale) / 2.0 - self.min.0 * scale, (f64::from(height) - span.1 * scale) / 2.0 - self.min.1 * scale);
        // Workspace y grows upward; pixel rows grow downward.
        let px = |p: (f64, f64)| (p.0 * scale + off.0, f64::from(height) - (p.1 * scale + off.1));
        let mut head = None;
        for s in self.segs.iter().take_while(|s| s.op <= moment.op) {
            let mut b = s.b;
            if s.op == moment.op {
                if s.start >= moment.into_pass {
                    continue;
                }
                if s.end > moment.into_pass && s.end > s.start {
                    let f = (moment.into_pass - s.start) / (s.end - s.start);
                    b = (s.a.0 + (s.b.0 - s.a.0) * f, s.a.1 + (s.b.1 - s.a.1) * f);
                }
            }
            line(&mut frame, px(s.a), px(b), INK);
            head = Some(px(b));
        }
        if let Some((x, y)) = head {
            dot(&mut frame, x, y, 4, HEAD);
        }
        Some(frame)
    }
}

fn put(frame: &mut Frame, x: i64, y: i64, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= i64::from(frame.width) || y >= i64::from(frame.height) {
        return;
    }
    let i = (y as usize * frame.width as usize + x as usize) * 4;
    if let Some(px) = frame.rgba.get_mut(i..i + 4) {
        px.copy_from_slice(&color);
    }
}

fn line(frame: &mut Frame, a: (f64, f64), b: (f64, f64), color: [u8; 4]) {
    let n = (b.0 - a.0).abs().max((b.1 - a.1).abs());
    if !n.is_finite() || n > 1.0e5 {
        return;
    }
    let steps = n.ceil().max(1.0) as u32;
    for k in 0..=steps {
        let f = f64::from(k) / f64::from(steps);
        put(frame, (a.0 + (b.0 - a.0) * f).round() as i64, (a.1 + (b.1 - a.1) * f).round() as i64, color);
    }
}

fn dot(frame: &mut Frame, x: f64, y: f64, r: i64, color: [u8; 4]) {
    if !(x.is_finite() && y.is_finite()) {
        return;
    }
    let (cx, cy) = (x.round() as i64, y.round() as i64);
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                put(frame, cx + dx, cy + dy, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Op, OpKind};

    #[test]
    fn plan_length_math() {
        let p = FramePlan::new(15.0, 30, 100.0);
        assert_eq!(p.frames, 450);
        assert!((p.clip_seconds() - 15.0).abs() < 1e-9);
        assert_eq!(p.job_time(0), 0.0);
        assert!((p.job_time(449) - 100.0).abs() < 1e-9);
        assert!((p.job_time(9999) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn plan_bad_input_is_capped() {
        let p = FramePlan::new(f64::NAN, 0, f64::INFINITY);
        assert_eq!((p.frames, p.fps, p.job_seconds), (450, DEFAULT_FPS, 0.0));
        assert_eq!(p.job_time(3), 0.0);
        assert_eq!(FramePlan::new(1e9, 1000, 1.0).frames, MAX_FRAMES);
        assert_eq!(FramePlan::new(0.001, 1, 1.0).frames, 2);
    }

    #[test]
    fn aspect_sizes() {
        assert_eq!(Aspect::Vertical.size(), (1080, 1920));
        assert_eq!(Aspect::Square.size(), (1080, 1080));
        assert_eq!(Aspect::Wide.size(), (1920, 1080));
    }

    #[test]
    fn frames_grow_as_the_job_runs() {
        let tl = Timelapse::new(&square_job(), 6000.0, Motion::default()).unwrap();
        let ink = |t: f64| tl.render(t, 200, 200).unwrap().rgba.chunks(4).filter(|p| *p == INK).count();
        let (empty, half, done) = (ink(0.0), ink(tl.job_seconds() / 2.0), ink(tl.job_seconds()));
        assert_eq!(empty, 0);
        assert!(empty < half && half < done, "{empty} {half} {done}");
    }

    #[test]
    fn bad_requests_give_none() {
        let tl = Timelapse::new(&square_job(), 6000.0, Motion::default()).unwrap();
        assert!(tl.render(0.0, 0, 10).is_none());
        assert!(tl.render(0.0, MAX_SIDE + 1, 10).is_none());
        assert!(tl.render(f64::NAN, 50, 50).is_some());
        assert!(Timelapse::new(&Job::default(), 6000.0, Motion::default()).is_none());
        let mut bad = square_job();
        bad.ops[0].moves.push(Move::Cut { to: Point::new(f64::NAN, 0.0), power: 50.0 });
        assert!(Timelapse::new(&bad, 6000.0, Motion::default()).is_none());
    }

    fn square_job() -> Job {
        let c = |x, y| Move::Cut { to: Point::new(x, y), power: 50.0 };
        Job {
            ops: vec![Op {
                layer: 0,
                kind: OpKind::Line,
                speed: 600.0,
                passes: 1,
                air_assist: false,
                z: 0.0,
                z_per_pass: 0.0,
                moves: vec![Move::Travel(Point::new(0.0, 0.0)), c(50.0, 0.0), c(50.0, 50.0), c(0.0, 50.0), c(0.0, 0.0)],
            }],
        }
    }
}
