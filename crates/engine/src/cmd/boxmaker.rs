use kurbo::BezPath;
use laserfoundry_doc::{LayerId, Shape};
use laserfoundry_geom::Affine;
use laserfoundry_geom::boxmaker::{BoxSpec, panels};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CommandSpec, parse, shapes::finite};
use crate::{EngineError, Session};

/// Panels are laid out in rows no wider than this (mm) before starting a new row.
const ROW_WIDTH: f64 = 500.0;

pub(super) const COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: "generate.box",
    label: "Box Maker",
    menu: "Tools",
    shortcut: None,
    params_doc: "{width, depth, height (inside size, mm), thickness? (material mm, default 3), kerf? (mm, default 0), finger? (finger width mm, default 10), lid?: true (false makes an open tray), spacing? (mm between panels, default 5), x?, y? (top-left, default 20,20), layer? (cut, default 0)}  — the panels of a finger-jointed box as closed cut outlines, laid out in rows; joints mate at the given thickness and kerf. No machine motion",
    mutates: true,
    run: make_box,
}];

#[derive(Deserialize)]
struct BoxParams {
    width: f64,
    depth: f64,
    height: f64,
    #[serde(default = "three")]
    thickness: f64,
    #[serde(default)]
    kerf: f64,
    #[serde(default = "ten")]
    finger: f64,
    #[serde(default = "yes")]
    lid: bool,
    #[serde(default = "five")]
    spacing: f64,
    #[serde(default = "twenty")]
    x: f64,
    #[serde(default = "twenty")]
    y: f64,
    #[serde(default)]
    layer: LayerId,
}

fn three() -> f64 {
    3.0
}
fn ten() -> f64 {
    10.0
}
fn five() -> f64 {
    5.0
}
fn twenty() -> f64 {
    20.0
}
fn yes() -> bool {
    true
}

fn make_box(s: &mut Session, params: &Value) -> Result<Value, EngineError> {
    let p: BoxParams = parse(params)?;
    finite(&[p.x, p.y])?;
    if !p.spacing.is_finite() || !(0.0..=1000.0).contains(&p.spacing) {
        return Err(EngineError::BadParams("spacing must be between 0 and 1000 mm".into()));
    }
    if s.doc.layer(p.layer).is_none() {
        return Err(EngineError::NotFound(format!("layer {} does not exist", p.layer)));
    }
    let spec = BoxSpec { width: p.width, depth: p.depth, height: p.height, thickness: p.thickness, kerf: p.kerf, finger: p.finger, lid: p.lid };
    let panels = panels(&spec).map_err(|e| EngineError::BadParams(e.to_string()))?;

    let (mut cx, mut cy, mut row_height) = (p.x, p.y, 0.0_f64);
    let mut ids = Vec::new();
    for panel in &panels {
        let Some(b) = panel.outline.bounds() else { continue };
        let (w, h) = (b.x1 - b.x0, b.y1 - b.y0);
        if cx > p.x && cx + w > p.x + ROW_WIDTH {
            cx = p.x;
            cy += row_height + p.spacing;
            row_height = 0.0;
        }
        let mut path = BezPath::new();
        for (i, pt) in panel.outline.points.iter().enumerate() {
            if i == 0 {
                path.move_to(*pt);
            } else {
                path.line_to(*pt);
            }
        }
        path.close_path();
        let mut shape = Shape::path(format!("Box {}", panel.name), p.layer, path);
        shape.transform = Affine::translate((cx - b.x0, cy - b.y0));
        ids.push(s.doc.add_shape(shape));
        cx += w + p.spacing;
        row_height = row_height.max(h);
    }
    s.selection = ids.clone();
    Ok(json!({ "panels": ids.len(), "ids": ids }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_100_by_60_by_40_box_makes_six_closed_panels_without_overlap_and_one_undo_step() {
        let mut s = Session::new();
        let v = s.execute("generate.box", &json!({"width": 100, "depth": 60, "height": 40})).unwrap();
        assert_eq!(v["panels"], json!(6));
        assert_eq!(s.doc.shapes.len(), 6);
        let boxes: Vec<_> = s.doc.shapes.iter().map(|sh| sh.world_bounds().unwrap()).collect();
        for (i, a) in boxes.iter().enumerate() {
            assert!(a.x0 >= 20.0 - 1e-6 && a.y0 >= 20.0 - 1e-6, "starts at the origin given");
            for b in &boxes[i + 1..] {
                let apart = a.x1 < b.x0 || b.x1 < a.x0 || a.y1 < b.y0 || b.y1 < a.y0;
                assert!(apart, "panels do not overlap");
            }
        }
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.doc.shapes.is_empty());
    }

    #[test]
    fn without_a_lid_there_are_five_panels() {
        let mut s = Session::new();
        let v = s.execute("generate.box", &json!({"width": 100, "depth": 60, "height": 40, "lid": false})).unwrap();
        assert_eq!(v["panels"], json!(5));
    }

    #[test]
    fn bad_input_is_refused_and_changes_nothing() {
        let mut s = Session::new();
        for bad in [
            json!({"width": 0, "depth": 60, "height": 40}),
            json!({"width": 100, "depth": 60}),
            json!({"width": 100, "depth": 60, "height": 40, "kerf": -1}),
            json!({"width": 100, "depth": 60, "height": 40, "kerf": 50}),
            json!({"width": 1e9, "depth": 60, "height": 40}),
            json!({"width": 100, "depth": 60, "height": 40, "spacing": -1}),
            json!({"width": 100, "depth": 60, "height": 40, "layer": 99}),
        ] {
            assert!(s.execute("generate.box", &bad).is_err(), "{bad}");
        }
        assert!(s.doc.shapes.is_empty());
    }
}
