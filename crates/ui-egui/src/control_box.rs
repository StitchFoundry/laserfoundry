//! The floating control box: the controls used most while running a job (Start, Pause, Stop, Frame, Home, Unlock and the
//! origin buttons) in one window over the canvas. Pinned, it stays where it is, on top, and its place and open state are
//! remembered per machine; unpinned, it closes when something else is clicked or with its X.

use crate::i18n::tr;
use crate::{LaserFoundryApp, panels};
use laserfoundry_device::{Link, MachineState};

/// Big enough for a finger.
const BUTTON: egui::Vec2 = egui::vec2(104.0, 40.0);
const WIDTH: f32 = 220.0;
/// Saved positions beyond this are damaged data, not a place on any screen.
const MAX_COORD: f32 = 100_000.0;
const KEY_PREFIX: &str = "control_box:";

/// What is remembered about the box for one machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Saved {
    pub pinned: bool,
    pub shown: bool,
    pub pos: Option<egui::Pos2>,
}

pub(crate) struct ControlBox {
    pub state: Saved,
    /// The machine `state` was loaded for.
    loaded_for: Option<String>,
    /// Changing it gives the window a new identity, so it takes its default place again.
    generation: u32,
    saved_text: String,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { pinned: false, shown: true, pos: None }
    }
}

impl Saved {
    pub(crate) fn to_text(self) -> String {
        let mut text = format!("pinned={} shown={}", u8::from(self.pinned), u8::from(self.shown));
        if let Some(p) = self.pos {
            text.push_str(&format!(" x={} y={}", p.x, p.y));
        }
        text
    }

    /// Reads saved text; anything unreadable, non-finite or absurd falls back to the default for that field.
    pub(crate) fn parse(text: &str) -> Saved {
        let mut saved = Saved::default();
        let (mut x, mut y) = (None, None);
        for part in text.split_whitespace().take(8) {
            let Some((key, value)) = part.split_once('=') else { continue };
            let coord = || value.parse::<f32>().ok().filter(|v| v.is_finite() && v.abs() <= MAX_COORD);
            match key {
                "pinned" => saved.pinned = value == "1",
                "shown" => saved.shown = value != "0",
                "x" => x = coord(),
                "y" => y = coord(),
                _ => {}
            }
        }
        saved.pos = x.zip(y).map(|(x, y)| egui::pos2(x, y));
        saved
    }
}

impl ControlBox {
    pub(crate) fn new() -> Self {
        ControlBox { state: Saved::default(), loaded_for: None, generation: 0, saved_text: String::new() }
    }
}

/// Draws the box when a machine is connected and it is open.
pub(crate) fn show(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    load_for_machine(app);
    if app.link.is_none() || !app.control_box.state.shown {
        return;
    }
    let mut st = app.control_box.state;
    let mut open = true;
    let id = egui::Id::unique(("control_box", app.control_box.generation));
    let mut window = egui::Window::new(tr("Control box")).id(id).open(&mut open).resizable(false);
    window = match st.pos {
        Some(p) => window.default_pos(p),
        None => window.default_pos(ctx.content_rect().left_top() + egui::vec2(12.0, 12.0)),
    };
    if st.pinned {
        window = window.order(egui::Order::Foreground);
    }
    if let Some(shown) = window.show(ctx, |ui| body(app, &mut st, ui)) {
        let rect = shown.response.rect;
        st.pos = Some(rect.left_top());
        if !st.pinned && pressed_outside(ctx, rect) {
            open = false;
        }
    }
    st.shown = open;
    app.control_box.state = st;
    // Writing while the box is being dragged would write every frame.
    if !ctx.input(|i| i.pointer.any_down()) {
        save(app);
    }
}

/// Puts the box back at its default place and opens it.
pub(crate) fn reset_position(app: &mut LaserFoundryApp) {
    let b = &mut app.control_box;
    b.state.pos = None;
    b.state.shown = true;
    b.generation = b.generation.wrapping_add(1);
    save(app);
}

/// Why Start is unavailable (`None` when it can start).
pub(crate) fn start_blocked(app: &LaserFoundryApp) -> Option<String> {
    if app.job_active() {
        return Some(tr("A job is already running"));
    }
    app.snap.as_ref().and_then(|s| s.status.as_ref()).and_then(|s| crate::not_ready(s.state))
}

fn body(app: &mut LaserFoundryApp, st: &mut Saved, ui: &mut egui::Ui) {
    ui.set_max_width(WIDTH);
    let active = app.job_active();
    let paused = app.snap.as_ref().is_some_and(|s| s.job == laserfoundry_device::JobState::Paused);
    let alarm = app.snap.as_ref().and_then(|s| s.status.as_ref()).is_some_and(|s| s.state == MachineState::Alarm);
    let pin_label = if st.pinned { tr("Pinned") } else { tr("Pin") };
    ui.toggle_value(&mut st.pinned, pin_label).on_hover_text(tr("Pinned: stays where it is, on top. Unpinned: closes when you click elsewhere"));
    let blocked = start_blocked(app);
    if big(ui, blocked.is_none(), tr("Start")).clicked() {
        panels::start_action(app);
    }
    if let Some(why) = blocked {
        ui.add(egui::Label::new(egui::RichText::new(why).small().color(app.tokens.error)).wrap());
    }
    ui.horizontal_wrapped(|ui| {
        if active {
            if paused {
                if big(ui, true, tr("Resume")).clicked() {
                    app.link_do(Link::resume);
                }
            } else if big(ui, true, tr("Pause")).clicked() {
                app.link_do(Link::pause);
            }
        }
        // Stop is always there: it soft-resets the controller, which turns the laser off.
        if big(ui, true, tr("Stop")).on_hover_text(tr("Soft reset: stops motion and turns the laser off")).clicked() {
            app.link_do(Link::stop);
        }
        if active {
            return;
        }
        if big(ui, true, tr("Frame")).clicked() {
            panels::frame_action(app);
        }
        if big(ui, true, tr("Home")).on_hover_text(tr("Run the homing cycle ($H)")).clicked() {
            app.link_do(Link::home);
        }
        if alarm && big(ui, true, tr("Unlock")).on_hover_text(tr("Clear an alarm ($X)")).clicked() {
            app.link_do(Link::unlock);
        }
        if big(ui, true, tr("Set origin here")).on_hover_text(tr("Make the head's position the work origin (G92)")).clicked() {
            app.link_do(Link::set_zero);
        }
        if big(ui, true, tr("Go to origin")).clicked() {
            app.link_do(Link::go_to_zero);
        }
    });
}

fn big(ui: &mut egui::Ui, enabled: bool, label: String) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(label).min_size(BUTTON))
}

/// A press this frame on the bare window background (not on a menu, dialog or another window) outside `rect`.
fn pressed_outside(ctx: &egui::Context, rect: egui::Rect) -> bool {
    let Some(p) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None }) else { return false };
    !rect.contains(p) && ctx.layer_id_at(p).is_some_and(|l| l.order == egui::Order::Background)
}

/// Loads what was saved for the connected machine when the machine changes.
fn load_for_machine(app: &mut LaserFoundryApp) {
    let name = &app.session.doc.machine.name;
    if app.control_box.loaded_for.as_deref() == Some(name.as_str()) {
        return;
    }
    let name = name.clone();
    let text = app.platform.storage_get(&format!("{KEY_PREFIX}{name}"));
    let b = &mut app.control_box;
    b.state = text.as_deref().map(Saved::parse).unwrap_or_default();
    b.saved_text = b.state.to_text();
    b.loaded_for = Some(name);
    b.generation = b.generation.wrapping_add(1);
}

fn save(app: &mut LaserFoundryApp) {
    let text = app.control_box.state.to_text();
    if text == app.control_box.saved_text {
        return;
    }
    let Some(name) = app.control_box.loaded_for.clone() else { return };
    app.platform.storage_set(&format!("{KEY_PREFIX}{name}"), &text);
    app.control_box.saved_text = text;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NoPlatform, Session};

    #[test]
    fn pin_state_survives_a_round_trip() {
        let saved = Saved { pinned: true, shown: false, pos: Some(egui::pos2(120.5, 64.0)) };
        assert_eq!(Saved::parse(&saved.to_text()), saved);
        let unplaced = Saved { pinned: false, shown: true, pos: None };
        assert_eq!(Saved::parse(&unplaced.to_text()), unplaced);
    }

    #[test]
    fn damaged_saved_text_falls_back_to_the_defaults() {
        for text in ["", "garbage", "x=NaN y=1", "x=inf y=inf", "x=1e30 y=2", "x=5", "pinned= shown=", "=== = x==y"] {
            let parsed = Saved::parse(text);
            assert_eq!(parsed.pos, None, "{text}");
            assert!(!parsed.pinned, "{text}");
        }
        assert_eq!(Saved::parse("x=1 y=2 pinned=1").pos, Some(egui::pos2(1.0, 2.0)));
    }

    #[test]
    fn start_is_not_blocked_on_an_idle_machine() {
        let app = LaserFoundryApp::new(Session::new(), Box::new(NoPlatform));
        assert_eq!(start_blocked(&app), None);
    }

    #[test]
    fn reset_position_forgets_the_place_and_reopens_the_box() {
        let mut app = LaserFoundryApp::new(Session::new(), Box::new(NoPlatform));
        load_for_machine(&mut app);
        app.control_box.state = Saved { pinned: true, shown: false, pos: Some(egui::pos2(300.0, 200.0)) };
        let generation = app.control_box.generation;
        reset_position(&mut app);
        assert_eq!(app.control_box.state, Saved { pinned: true, shown: true, pos: None });
        assert_ne!(app.control_box.generation, generation);
    }
}
