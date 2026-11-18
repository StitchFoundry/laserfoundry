use laserfoundry_doc::{LAYER_COLORS, LayerId, Shape, ShapeKind, TextSettings};
use laserfoundry_geom::{Affine, rect_path};
use serde::Deserialize;
use serde_json::{Value, json};

use super::text::generate;
use super::{CommandSpec, parse, shapes::finite};
use crate::{EngineError, Session};

/// Most slots a fit test card may have.
pub const MAX_SLOTS: u32 = 21;

pub(super) const COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: "generate.fit_test",
    label: "Fit Test Card",
    menu: "Tools",
    shortcut: None,
    params_doc: "{thickness? (material mm, default 3), step? (mm between slots, default 0.05), count? (slots, default 9), x?, y? (top-left, default 20,20), layer? (cut, default 0), labels?: true, label_layer?}  — a row of blocks, each with a slot drawn at a different width centred on the thickness and labelled with that width; cut it, find the slot the material just fits and enter thickness minus that width as the kerf (the result lists that kerf per slot). No machine motion",
    mutates: true,
    run: fit_test,
}];

#[derive(Deserialize)]
struct FitParams {
    #[serde(default = "three")]
    thickness: f64,
    #[serde(default = "step")]
    step: f64,
    #[serde(default = "nine")]
    count: u32,
    #[serde(default = "twenty")]
    x: f64,
    #[serde(default = "twenty")]
    y: f64,
    #[serde(default)]
    layer: LayerId,
    #[serde(default = "yes")]
    labels: bool,
    label_layer: Option<LayerId>,
}

fn three() -> f64 {
    3.0
}
fn step() -> f64 {
    0.05
}
fn nine() -> u32 {
    9
}
fn twenty() -> f64 {
    20.0
}
fn yes() -> bool {
    true
}

/// Height of the label text, mm.
const LABEL_HEIGHT: f64 = 2.5;
/// Block height, slot depth, the wall left either side of the widest slot, and the gap between blocks, mm.
const BLOCK_HEIGHT: f64 = 16.0;
const SLOT_DEPTH: f64 = 8.0;
const WALL: f64 = 6.0;
const GAP: f64 = 3.0;

/// Slot widths for a card, mm: `count` of them, `step` apart, the middle one equal to `thickness`.
/// The caller has validated the inputs.
fn slot_widths(thickness: f64, step: f64, count: u32) -> Vec<f64> {
    let mid = f64::from(count.saturating_sub(1)) / 2.0;
    (0..count).map(|i| thickness + (f64::from(i) - mid) * step).collect()
}

/// The kerf (mm the cut removes beyond the drawn line) implied by the slot the material just fits: a slot drawn `slot` wide cuts
/// `slot + kerf` wide, and the material is `thickness` thick.
fn kerf_if_fits(thickness: f64, slot: f64) -> f64 {
    thickness - slot
}

fn fit_test(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: FitParams = parse(params)?;
    finite(&[p.thickness, p.step, p.x, p.y])?;
    let smallest = p.thickness - p.step * f64::from(p.count.saturating_sub(1)) / 2.0;
    if !(0.5..=50.0).contains(&p.thickness) || !(0.01..=1.0).contains(&p.step) || !(1..=MAX_SLOTS).contains(&p.count) || smallest <= 0.1 {
        return Err(EngineError::BadParams("fit test card: thickness 0.5-50 mm, step 0.01-1 mm, 1-21 slots, smallest slot over 0.1 mm".into()));
    }
    if s.doc.layer(p.layer).is_none() {
        return Err(EngineError::NotFound(format!("layer {} does not exist", p.layer)));
    }
    let label_layer = p.label_layer.unwrap_or((p.layer + 1) % LAYER_COLORS.len() as LayerId);
    if p.labels && s.doc.layer(label_layer).is_none() {
        return Err(EngineError::NotFound(format!("layer {label_layer} does not exist")));
    }
    let widths = slot_widths(p.thickness, p.step, p.count);
    let block_width = p.thickness + p.step * f64::from(p.count) / 2.0 + 2.0 * WALL;
    let style = TextSettings { height: LABEL_HEIGHT, ..TextSettings::default() };

    let mut ids = Vec::new();
    let mut slots = Vec::new();
    for (i, width) in widths.iter().enumerate() {
        let x = p.x + i as f64 * (block_width + GAP);
        let mut block = Shape::path(format!("Fit block {width:.2}"), p.layer, rect_path(0.0, 0.0, block_width, BLOCK_HEIGHT));
        block.transform = Affine::translate((x, p.y));
        ids.push(s.doc.add_shape(block));
        let mut slot = Shape::path(format!("Fit slot {width:.2}"), p.layer, rect_path(0.0, 0.0, *width, SLOT_DEPTH));
        slot.transform = Affine::translate((x + (block_width - width) / 2.0, p.y + (BLOCK_HEIGHT - SLOT_DEPTH) / 2.0));
        ids.push(s.doc.add_shape(slot));
        let text = format!("{width:.2}");
        if p.labels {
            let (path, _) = generate(&text, &style)?;
            let mut label = Shape::path(text.clone(), label_layer, path.clone());
            label.kind = ShapeKind::Text { content: text.clone(), style: style.clone(), path };
            label.transform = Affine::translate((x + 1.0, p.y + BLOCK_HEIGHT + 1.5));
            ids.push(s.doc.add_shape(label));
        }
        slots.push(json!({ "width": text, "kerf": format!("{:.2}", kerf_if_fits(p.thickness, *width)) }));
    }
    s.selection = ids.clone();
    Ok(json!({ "slots": slots, "ids": ids }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_card_has_nine_slots_from_2_80_to_3_20_and_matching_labels() {
        let mut s = Session::new();
        let v = s.execute("generate.fit_test", &json!({})).unwrap();
        let widths: Vec<_> = v["slots"].as_array().unwrap().iter().map(|e| e["width"].as_str().unwrap().to_string()).collect();
        assert_eq!(widths, ["2.80", "2.85", "2.90", "2.95", "3.00", "3.05", "3.10", "3.15", "3.20"]);
        assert_eq!(v["slots"][4]["kerf"], json!("0.00"));
        assert_eq!(v["slots"][0]["kerf"], json!("0.20"));
        let labels: Vec<_> = s
            .doc
            .shapes
            .iter()
            .filter_map(|sh| match &sh.kind {
                ShapeKind::Text { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, widths, "each label is its slot's width");
        let slot = s.doc.shapes.iter().find(|sh| sh.name == "Fit slot 3.05").unwrap().world_bounds().unwrap();
        assert!((slot.x1 - slot.x0 - 3.05).abs() < 1e-9, "the drawn slot is as wide as its label");
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shapes.is_empty(), "one undo step");
    }

    #[test]
    fn labels_can_be_left_off_and_hostile_parameters_are_refused() {
        let mut s = Session::new();
        s.execute("generate.fit_test", &json!({"labels": false, "count": 3})).unwrap();
        assert_eq!(s.doc.shapes.len(), 6, "a block and a slot per width");
        for bad in [
            json!({"thickness": 0}),
            json!({"thickness": 1e9}),
            json!({"step": 0}),
            json!({"step": "x"}),
            json!({"count": 0}),
            json!({"count": 22}),
            json!({"thickness": 1, "step": 0.2, "count": 21}),
            json!({"layer": 99}),
        ] {
            assert!(s.execute("generate.fit_test", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.doc.shapes.len(), 6, "failures change nothing");
    }
}
