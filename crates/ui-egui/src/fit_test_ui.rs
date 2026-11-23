//! The Fit test card window: material thickness, step and slot count, then Create (`generate.fit_test`).

use serde_json::{Value, json};

use crate::LaserFoundryApp;
use crate::i18n::{tr, trf};

const HELP: &str = "Cut the card, then find the slot your material just fits. The thickness minus that slot's width is the kerf to enter in the settings and the box maker.";

/// The card being set up.
#[derive(Debug, Clone, PartialEq)]
pub struct FitCard {
    pub thickness: f64,
    pub step: f64,
    pub count: u32,
    /// The width of the slot that fitted after cutting.
    pub best_slot: f64,
}

impl Default for FitCard {
    fn default() -> Self {
        Self { thickness: 3.0, step: 0.05, count: 9, best_slot: 3.0 }
    }
}

impl FitCard {
    /// The command parameters, the card going on `layer`.
    pub fn params(&self, layer: u32) -> Value {
        json!({ "thickness": self.thickness, "step": self.step, "count": self.count, "layer": layer })
    }

    /// The kerf the fitting slot implies: thickness minus slot width, never below zero.
    pub fn kerf(&self) -> f64 {
        let k = self.thickness - self.best_slot;
        if k.is_finite() { k.max(0.0) } else { 0.0 }
    }
}

/// Shows the window when `fit_test_open` is set.
pub(crate) fn window(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    if !app.fit_test_open {
        return;
    }
    let mut open = true;
    let mut go = false;
    let mut use_box = false;
    let mut use_layer = false;
    egui::Window::new(tr("Fit test card")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        let card = &mut app.fit_card;
        egui::Grid::new("fit_card").num_columns(2).show(ui, |ui| {
            ui.label(tr("Material thickness"));
            ui.add(egui::DragValue::new(&mut card.thickness).range(0.5..=50.0).speed(0.05).suffix(" mm"));
            ui.end_row();
            ui.label(tr("Step between slots"));
            ui.add(egui::DragValue::new(&mut card.step).range(0.01..=1.0).speed(0.01).suffix(" mm"));
            ui.end_row();
            ui.label(tr("Number of slots"));
            ui.add(egui::DragValue::new(&mut card.count).range(1..=21));
            ui.end_row();
            ui.label(tr("Slot that fit"));
            ui.add(egui::DragValue::new(&mut card.best_slot).range(0.1..=60.0).speed(0.01).suffix(" mm"));
            ui.end_row();
        });
        ui.label(trf!("Kerf: {kerf} mm", kerf = format!("{:.2}", card.kerf())));
        use_box = ui.button(tr("Use in the box maker")).clicked();
        use_layer = ui.button(tr("Use on the active layer")).clicked();
        go = ui.button(tr("Create on the active layer")).clicked();
        ui.weak(tr(HELP));
    });
    if go {
        let params = app.fit_card.params(app.active_layer);
        if let Some(result) = app.run("generate.fit_test", params) {
            let count = result["slots"].as_array().map_or(0, Vec::len);
            app.say(trf!("Fit test card with {count} slots created", count = count));
        }
    }
    if use_box {
        app.box_material[0] = app.fit_card.thickness;
        app.box_material[1] = app.fit_card.kerf();
    }
    if use_layer {
        let (id, offset) = (app.active_layer, app.fit_card.kerf() / 2.0);
        app.run("layer.set", json!({ "id": id, "kerf_offset": offset }));
    }
    if !open {
        app.fit_test_open = false;
    }
}
