//! The ring jog dial: a circle with four concentric rings. Where you click around the circle picks the
//! direction (eight ways); how far from the centre picks the speed zone (a step and a feed rate each).
//! The centre goes to the work origin. Every jog is a relative move with the laser off, and the dial is
//! disabled while a job runs (the caller disables it).

use crate::LaserFoundryApp;
use crate::i18n::{tr, trf};
use crate::panels::machine_delta;
use laserfoundry_device::Link;
use laserfoundry_doc::Origin;

pub(crate) const RING_COUNT: usize = 4;
/// Fraction of the dial's radius, from the centre, that is the centre button.
const DEAD_ZONE: f32 = 0.2;
const DIAL_SIZE: f32 = 168.0;
/// Fastest a held jog goes (mm/min), whatever the pad's speed says, and the longest it can be (mm; the link's own limit).
const HOLD_FEED_MAX: f64 = 3000.0;
const HOLD_MAX_MM: f64 = 1000.0;
/// Eight directions in workspace terms (y grows downward, as on screen), clockwise from the right.
const DIRECTIONS: [(f64, f64); 8] = [(1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (-1.0, 1.0), (-1.0, 0.0), (-1.0, -1.0), (0.0, -1.0), (1.0, -1.0)];

/// One speed zone: the distance of a jog (mm) and its feed rate (mm/min).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ring {
    pub step: f64,
    pub feed: f64,
}

/// Inner (slow, fine) to outer (fast, coarse).
pub(crate) const DEFAULT_RINGS: [Ring; RING_COUNT] =
    [Ring { step: 0.1, feed: 600.0 }, Ring { step: 1.0, feed: 1500.0 }, Ring { step: 10.0, feed: 3000.0 }, Ring { step: 50.0, feed: 6000.0 }];

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Hit {
    Centre,
    Ring { ring: usize, dir: (f64, f64) },
}

/// What a point `offset` from the dial's centre (screen pixels) lands on; `None` outside the dial or for a bad number.
pub(crate) fn hit(offset: egui::Vec2, radius: f32) -> Option<Hit> {
    if !offset.x.is_finite() || !offset.y.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    let d = offset.length() / radius;
    if d > 1.0 {
        return None;
    }
    if d < DEAD_ZONE {
        return Some(Hit::Centre);
    }
    let ring = (((d - DEAD_ZONE) / (1.0 - DEAD_ZONE) * RING_COUNT as f32) as usize).min(RING_COUNT - 1);
    let sector = (offset.y.atan2(offset.x) / std::f32::consts::FRAC_PI_4).round() as i32;
    let dir = *DIRECTIONS.get(sector.rem_euclid(8) as usize)?;
    Some(Hit::Ring { ring, dir })
}

/// The machine-axis step `(dx, dy)` and feed for a jog along workspace direction `dir` in `ring`.
pub(crate) fn jog_delta(origin: Origin, ring: Ring, dir: (f64, f64)) -> (f64, f64, f64) {
    let (dx, dy) = machine_delta(origin, dir.0 * ring.step, dir.1 * ring.step);
    (dx, dy, ring.feed)
}

/// The machine-axis travel and feed of a held jog along workspace direction `dir`: as far as the bed is wide (so the head
/// runs until the cancel byte, or the bed edge, whichever comes first) at no more than [`HOLD_FEED_MAX`].
pub(crate) fn hold_delta(origin: Origin, bed: (f64, f64), dir: (f64, f64), feed: f64) -> (f64, f64, f64) {
    let reach = |v: f64| if v.is_finite() { v.clamp(1.0, HOLD_MAX_MM) } else { 1.0 };
    let (dx, dy) = machine_delta(origin, dir.0 * reach(bed.0), dir.1 * reach(bed.1));
    (dx, dy, if feed.is_finite() { feed.min(HOLD_FEED_MAX) } else { HOLD_FEED_MAX })
}

/// Hold-to-jog: the press of a direction starts one long jog; letting go (or no longer pressing anything) cancels it.
/// `pressed` is the workspace direction and feed being held, if any.
pub(crate) fn hold_jog(app: &mut LaserFoundryApp, pressed: Option<((f64, f64), f64)>) {
    match (pressed, app.jog_held) {
        (Some((dir, feed)), false) => {
            let m = &app.session.doc.machine;
            let (dx, dy, feed) = hold_delta(m.origin, (m.bed_width, m.bed_height), dir, feed);
            app.jog_held = true;
            app.link_do(|l| l.jog(dx, dy, feed));
        }
        (None, true) => {
            app.jog_held = false;
            app.link_do(Link::jog_cancel);
        }
        _ => {}
    }
}

pub(crate) fn show(app: &mut LaserFoundryApp, ui: &mut egui::Ui) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(DIAL_SIZE, DIAL_SIZE), egui::Sense::click());
    let (centre, radius) = (rect.center(), DIAL_SIZE / 2.0);
    let enabled = ui.is_enabled();
    let name = tr("Jog dial");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name.clone()));
    let hovered = resp.hover_pos().and_then(|p| hit(p - centre, radius));
    paint(app, ui, centre, radius, hovered);
    let line = match hovered {
        Some(Hit::Ring { ring, .. }) => {
            app.ring_steps.get(ring).map(|r| trf!("Ring {n}: {step} mm at {feed} mm/min", n = ring + 1, step = r.step, feed = r.feed))
        }
        Some(Hit::Centre) => Some(tr("Go to origin")),
        None => None,
    };
    ui.label(line.unwrap_or_else(|| tr("Click a ring to jog; the centre goes to the origin")));
    let pointer_hit = resp.interact_pointer_pos().and_then(|p| hit(p - centre, radius));
    if app.jog_hold {
        let pressed = match pointer_hit {
            Some(Hit::Ring { ring, dir }) if resp.is_pointer_button_down_on() => app.ring_steps.get(ring).map(|r| (dir, r.feed)),
            _ => None,
        };
        hold_jog(app, pressed);
    }
    if resp.clicked()
        && let Some(h) = pointer_hit
    {
        match h {
            Hit::Centre => app.link_do(Link::go_to_zero),
            Hit::Ring { .. } if app.jog_hold => {}
            Hit::Ring { ring, dir } => {
                if let Some(r) = app.ring_steps.get(ring).copied() {
                    let (dx, dy, feed) = jog_delta(app.session.doc.machine.origin, r, dir);
                    app.link_do(|l| l.jog(dx, dy, feed));
                }
            }
        }
    }
    ui.collapsing(tr("Ring steps"), |ui| {
        for (i, r) in app.ring_steps.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(trf!("Ring {n}", n = i + 1));
                ui.add(egui::DragValue::new(&mut r.step).range(0.01..=100.0).suffix(" mm"));
                ui.add(egui::DragValue::new(&mut r.feed).range(10.0..=30_000.0).suffix(" mm/min"));
            });
        }
    });
}

/// Rings from the outside in, each a different shade and labelled with its step, so no ring depends on colour alone.
fn paint(app: &LaserFoundryApp, ui: &egui::Ui, centre: egui::Pos2, radius: f32, hovered: Option<Hit>) {
    let painter = ui.painter();
    let visuals = ui.visuals();
    let width = radius * (1.0 - DEAD_ZONE) / RING_COUNT as f32;
    for i in (0..RING_COUNT).rev() {
        let outer = radius - width * (RING_COUNT - 1 - i) as f32;
        let shade = visuals.faint_bg_color.lerp_to_gamma(visuals.strong_text_color(), 0.1 + 0.08 * i as f32);
        painter.circle_filled(centre, outer, shade);
        let active = matches!(hovered, Some(Hit::Ring { ring, .. }) if ring == i);
        let stroke = if active { visuals.selection.stroke } else { egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color) };
        painter.circle_stroke(centre, outer, stroke);
        if let Some(r) = app.ring_steps.get(i) {
            let at = centre - egui::vec2(0.0, outer - width / 2.0);
            painter.text(at, egui::Align2::CENTER_CENTER, format!("{}", r.step), egui::FontId::proportional(10.0), visuals.text_color());
        }
    }
    let inner = radius * DEAD_ZONE;
    let fill = if matches!(hovered, Some(Hit::Centre)) { visuals.selection.bg_fill } else { visuals.widgets.inactive.weak_bg_fill };
    painter.circle_filled(centre, inner, fill);
    painter.circle_stroke(centre, inner, visuals.widgets.noninteractive.bg_stroke);
    painter.text(centre, egui::Align2::CENTER_CENTER, "0", egui::FontId::proportional(11.0), visuals.text_color());
}

#[cfg(test)]
mod tests {
    use super::*;
    use laserfoundry_device::jog_line;

    #[test]
    fn centre_is_a_dead_zone_and_outside_is_nothing() {
        assert_eq!(hit(egui::vec2(0.0, 0.0), 80.0), Some(Hit::Centre));
        assert_eq!(hit(egui::vec2(10.0, 0.0), 80.0), Some(Hit::Centre));
        assert_eq!(hit(egui::vec2(81.0, 0.0), 80.0), None);
        assert_eq!(hit(egui::vec2(f32::NAN, 0.0), 80.0), None);
        assert_eq!(hit(egui::vec2(1.0, 1.0), 0.0), None);
    }

    #[test]
    fn distance_picks_the_ring_and_angle_the_direction() {
        let ring_at = |d: f32| match hit(egui::vec2(d, 0.0), 100.0) {
            Some(Hit::Ring { ring, dir }) => (ring, dir),
            other => panic!("{other:?}"),
        };
        assert_eq!(ring_at(25.0), (0, (1.0, 0.0)));
        assert_eq!(ring_at(45.0), (1, (1.0, 0.0)));
        assert_eq!(ring_at(65.0), (2, (1.0, 0.0)));
        assert_eq!(ring_at(99.0), (3, (1.0, 0.0)));
        let dir_at = |x: f32, y: f32| match hit(egui::vec2(x, y), 100.0) {
            Some(Hit::Ring { dir, .. }) => dir,
            other => panic!("{other:?}"),
        };
        assert_eq!(dir_at(0.0, -50.0), (0.0, -1.0));
        assert_eq!(dir_at(0.0, 50.0), (0.0, 1.0));
        assert_eq!(dir_at(-50.0, 0.0), (-1.0, 0.0));
        assert_eq!(dir_at(40.0, -40.0), (1.0, -1.0));
    }

    #[test]
    fn a_held_jog_runs_the_bed_width_and_is_capped_in_speed() {
        assert_eq!(hold_delta(Origin::TopLeft, (300.0, 200.0), (1.0, 0.0), 6000.0), (300.0, 0.0, HOLD_FEED_MAX));
        assert_eq!(hold_delta(Origin::BottomLeft, (300.0, 200.0), (0.0, -1.0), 600.0), (0.0, 200.0, 600.0));
        let (dx, dy, feed) = hold_delta(Origin::TopLeft, (f64::NAN, 5000.0), (-1.0, 1.0), f64::NAN);
        assert_eq!((dx, dy, feed), (-1.0, 1000.0, HOLD_FEED_MAX));
        assert!(jog_line(dx, dy, feed).is_ok(), "always a line the link accepts");
    }

    #[test]
    fn jog_commands_per_ring() {
        let up = (0.0, -1.0);
        let lines: Vec<String> = DEFAULT_RINGS
            .iter()
            .map(|r| {
                let (dx, dy, feed) = jog_delta(Origin::TopLeft, *r, up);
                jog_line(dx, dy, feed).unwrap()
            })
            .collect();
        assert_eq!(lines[0], "$J=G91 G21 Y-0.1 F600");
        assert_eq!(lines[3], "$J=G91 G21 Y-50 F6000");
        let (_, dy, _) = jog_delta(Origin::BottomLeft, DEFAULT_RINGS[1], up);
        assert_eq!(dy, 1.0);
    }
}
