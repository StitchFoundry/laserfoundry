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
}

impl Default for FitCard {
    fn default() -> Self {
        Self { thickness: 3.0, step: 0.05, count: 9 }
    }
}

impl FitCard {
    /// The command parameters, the card going on `layer`.
    pub fn params(&self, layer: u32) -> Value {
        json!({ "thickness": self.thickness, "step": self.step, "count": self.count, "layer": layer })
    }
}

/// Shows the window when `fit_test_open` is set.
pub(crate) fn window(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    if !app.fit_test_open {
        return;
    }
    let mut open = true;
    let mut go = false;
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
        });
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
    if !open {
        app.fit_test_open = false;
    }
}
