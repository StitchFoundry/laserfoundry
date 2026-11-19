//! The "Share it on MakerFoundry?" prompt: after a project's first successful save or export, a small window that never blocks
//! the work and is shown at most once per project.
//!
//! This slice only opens MakerFoundry's own page in the browser (the same address as the *Publish to MakerFoundry* window uses
//! for its link); it never signs in and sends nothing. Pre-filled drafts, licences and analytics come in later slices.
//! The switch to turn the offer off is in the Publish to MakerFoundry window (the app has no general settings window yet).

use crate::LaserFoundryApp;
use crate::i18n::tr;

/// Storage key of the "offer to share after export" switch; the value `off` turns the prompt off.
const OFFER_KEY: &str = "laserfoundry.maker.offer_share";
/// Storage key of the projects already asked about, one per line.
const ASKED_KEY: &str = "laserfoundry.maker.share_asked";
/// How many projects are remembered (the oldest are forgotten first), so the stored text stays small.
const MAX_ASKED: usize = 200;
/// The key of a project that has no file yet.
const UNSAVED: &str = "(unsaved)";

/// What the prompt remembers and shows.
#[derive(Default)]
pub(crate) struct ShareState {
    loaded: bool,
    offer: bool,
    asked: Vec<String>,
    open: bool,
    dont_ask: bool,
    mine: bool,
}

impl LaserFoundryApp {
    /// Call after a save or an export succeeded: opens the prompt if this project has not been asked yet and the offer is on.
    pub(crate) fn offer_share(&mut self) {
        if !self.maker_available() {
            return;
        }
        self.load_share();
        let key = self.share_key();
        let share = &mut self.maker.share;
        if !share.offer || share.open || share.asked.contains(&key) {
            return;
        }
        share.asked.push(key);
        let extra = share.asked.len().saturating_sub(MAX_ASKED);
        share.asked.drain(..extra);
        share.open = true;
        share.dont_ask = false;
        share.mine = false;
        let text = share.asked.join("\n");
        self.platform.storage_set(ASKED_KEY, &text);
    }

    /// Whether the offer is on (read from storage the first time).
    pub(crate) fn share_offer_on(&mut self) -> bool {
        self.load_share();
        self.maker.share.offer
    }

    pub(crate) fn set_share_offer(&mut self, on: bool) {
        self.load_share();
        self.maker.share.offer = on;
        self.platform.storage_set(OFFER_KEY, if on { "on" } else { "off" });
    }

    /// Draws the prompt, when it is open. Call every frame.
    pub(crate) fn share_prompt(&mut self, ctx: &egui::Context) {
        if !self.maker.share.open {
            return;
        }
        let mut choice = None;
        egui::Window::new(tr("Share it on MakerFoundry?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -36.0])
            .show(ctx, |ui| {
                ui.label(tr("Your design is saved. Want to share it? You set the details, price and licence on MakerFoundry."));
                ui.checkbox(&mut self.maker.share.mine, tr("I made this / I own the rights"));
                ui.checkbox(&mut self.maker.share.dont_ask, tr("Don't ask again"));
                ui.horizontal(|ui| {
                    let mine = self.maker.share.mine;
                    if ui.add_enabled(mine, egui::Button::new(tr("Share free"))).clicked() {
                        choice = Some(true);
                    }
                    if ui.add_enabled(mine, egui::Button::new(tr("Sell it"))).clicked() {
                        choice = Some(true);
                    }
                    if ui.button(tr("Not now")).clicked() {
                        choice = Some(false);
                    }
                });
            });
        if let Some(go) = choice {
            self.finish_share(go);
        }
    }

    /// Closes the prompt: `go` opens MakerFoundry's page in the browser (needs login only there, never here); "Don't ask again" turns the offer off.
    fn finish_share(&mut self, go: bool) {
        self.maker.share.open = false;
        if self.maker.share.dont_ask {
            self.set_share_offer(false);
        }
        if go {
            self.pending_url = Some(self.platform.services().maker().to_string());
        }
    }

    fn load_share(&mut self) {
        if std::mem::replace(&mut self.maker.share.loaded, true) {
            return;
        }
        self.maker.share.offer = self.platform.storage_get(OFFER_KEY).as_deref() != Some("off");
        let text = self.platform.storage_get(ASKED_KEY).unwrap_or_default();
        self.maker.share.asked = text.lines().filter(|l| !l.is_empty()).map(str::to_string).take(MAX_ASKED).collect();
    }

    fn share_key(&self) -> String {
        self.session.path.as_ref().map_or_else(|| UNSAVED.to_string(), |p| p.to_string_lossy().replace('\n', " "))
    }
}

#[cfg(test)]
mod tests;
