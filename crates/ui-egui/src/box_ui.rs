//! The Box maker window: inside size, material thickness, kerf, finger width and lid or open tray, then Create (`generate.box`).

use serde_json::{Value, json};

use crate::LaserFoundryApp;
use crate::i18n::{tr, trf};

const HELP: &str = "Sizes are the inside of the box in mm. Run the fit test card first and enter the kerf it gives you, so the joints mate.";

/// The box being set up.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxCard {
    pub size: [f64; 3],
    pub thickness: f64,
    pub kerf: f64,
    pub finger: f64,
    pub lid: bool,
}

impl Default for BoxCard {
    fn default() -> Self {
        Self { size: [100.0, 60.0, 40.0], thickness: 3.0, kerf: 0.0, finger: 10.0, lid: true }
    }
}

impl BoxCard {
    /// The command parameters, the panels going on `layer`.
    pub fn params(&self, layer: u32) -> Value {
        json!({
            "width": self.size[0], "depth": self.size[1], "height": self.size[2], "thickness": self.thickness,
            "kerf": self.kerf, "finger": self.finger, "lid": self.lid, "layer": layer,
        })
    }
}

/// Shows the window when `box_open` is set.
pub(crate) fn window(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    if !app.box_open {
        return;
    }
    let mut open = true;
    let mut go = false;
    egui::Window::new(tr("Box maker")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        let card = &mut app.box_card;
        egui::Grid::new("box_card").num_columns(2).show(ui, |ui| {
            for (v, label) in card.size.iter_mut().zip(["Inside width", "Inside depth", "Inside height"]) {
                ui.label(tr(label));
                ui.add(egui::DragValue::new(v).range(1.0..=2000.0).suffix(" mm"));
                ui.end_row();
            }
            ui.label(tr("Material thickness"));
            ui.add(egui::DragValue::new(&mut card.thickness).range(0.5..=50.0).speed(0.05).suffix(" mm"));
            ui.end_row();
            ui.label(tr("Kerf"));
            ui.add(egui::DragValue::new(&mut card.kerf).range(0.0..=2.0).speed(0.01).suffix(" mm"));
            ui.end_row();
            ui.label(tr("Finger width"));
            ui.add(egui::DragValue::new(&mut card.finger).range(1.0..=100.0).speed(0.1).suffix(" mm"));
            ui.end_row();
        });
        ui.radio_value(&mut card.lid, true, tr("Closed box with lid"));
        ui.radio_value(&mut card.lid, false, tr("Open tray"));
        go = ui.button(tr("Create on the active layer")).clicked();
        ui.weak(tr(HELP));
    });
    if go {
        let params = app.box_card.params(app.active_layer);
        if let Some(result) = app.run("generate.box", params) {
            let count = result["ids"].as_array().map_or(0, Vec::len);
            app.say(trf!("Box with {count} panels created", count = count));
        }
    }
    if !open {
        app.box_open = false;
    }
}
