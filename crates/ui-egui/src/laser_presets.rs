//! Starting points for the setup wizard's "Pick your laser" list: bed size (and optical power, for display) of common GRBL diode
//! lasers, read from `data/laser-presets.json`. Every entry carries the maker's own page it came from and is shown as
//! "from the spec sheet, not tested". Model names only say what the program works with. A value a maker did not state is left out.

use std::sync::OnceLock;

use serde_json::Value;

/// The file shipped with the program.
const DATA: &str = include_str!("../data/laser-presets.json");
/// Largest bed side a preset may state, in mm (the machine settings accept up to 100 000).
const MAX_BED_MM: f64 = 5_000.0;
/// Most entries read; the list is meant to stay short.
const MAX_PRESETS: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Preset {
    pub name: String,
    pub bed_width: f64,
    pub bed_height: f64,
    /// Optical output the maker states, in watts.
    pub optical_watts: Option<f64>,
    /// The maker's page the numbers came from.
    pub source: String,
}

/// The shipped presets (empty if the file were ever unreadable, which a test rules out).
pub(crate) fn presets() -> &'static [Preset] {
    static LIST: OnceLock<Vec<Preset>> = OnceLock::new();
    LIST.get_or_init(|| {
        parse(DATA).unwrap_or_else(|e| {
            log::warn!("laser presets unusable: {e}");
            Vec::new()
        })
    })
}

/// Reads and checks a presets file: every entry needs a name, a sane bed and an https source.
pub(crate) fn parse(text: &str) -> Result<Vec<Preset>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let list = root["presets"].as_array().ok_or("no presets list")?;
    if list.len() > MAX_PRESETS {
        return Err("too many presets".into());
    }
    let mut out = Vec::new();
    for (i, v) in list.iter().enumerate() {
        let name = v["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).ok_or(format!("entry {i}: no name"))?;
        let side = |key: &str| v[key].as_f64().filter(|n| n.is_finite() && *n > 0.0 && *n <= MAX_BED_MM).ok_or(format!("{name}: bad {key}"));
        let optical_watts = match v.get("optical_watts") {
            None | Some(Value::Null) => None,
            Some(w) => Some(w.as_f64().filter(|n| n.is_finite() && *n > 0.0 && *n < 1000.0).ok_or(format!("{name}: bad optical_watts"))?),
        };
        let source = v["source"].as_str().filter(|s| s.starts_with("https://") && s.len() > 12).ok_or(format!("{name}: no https source"))?;
        out.push(Preset {
            name: name.to_string(),
            bed_width: side("bed_width")?,
            bed_height: side("bed_height")?,
            optical_watts,
            source: source.to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_list_loads_and_every_entry_has_a_source_and_a_sane_bed() {
        let list = parse(DATA).expect("shipped file parses");
        assert!(list.len() >= 8);
        let mut names = std::collections::BTreeSet::new();
        for p in &list {
            assert!(p.source.starts_with("https://"), "{}", p.name);
            assert!(p.bed_width > 0.0 && p.bed_width.is_finite() && p.bed_height > 0.0 && p.bed_height.is_finite(), "{}", p.name);
            assert!(names.insert(p.name.clone()), "duplicate {}", p.name);
        }
        assert_eq!(presets().len(), list.len());
    }

    #[test]
    fn bad_entries_are_refused() {
        let one = |body: &str| parse(&format!("{{\"presets\":[{body}]}}"));
        assert!(one(r#"{"name":"A","bed_width":100,"bed_height":100,"source":"https://example.org/a"}"#).is_ok());
        assert!(one(r#"{"name":"A","bed_width":100,"bed_height":100}"#).is_err(), "no source");
        assert!(one(r#"{"name":"A","bed_width":100,"bed_height":100,"source":"http://example.org/a"}"#).is_err(), "not https");
        assert!(one(r#"{"name":"A","bed_width":0,"bed_height":100,"source":"https://example.org/a"}"#).is_err(), "zero bed");
        assert!(one(r#"{"name":"A","bed_width":-5,"bed_height":100,"source":"https://example.org/a"}"#).is_err(), "negative bed");
        assert!(one(r#"{"name":"A","bed_width":1e9,"bed_height":100,"source":"https://example.org/a"}"#).is_err(), "huge bed");
        assert!(one(r#"{"name":"","bed_width":100,"bed_height":100,"source":"https://example.org/a"}"#).is_err(), "no name");
        assert!(one(r#"{"name":"A","bed_width":100,"bed_height":100,"optical_watts":-1,"source":"https://example.org/a"}"#).is_err());
        assert!(parse("not json").is_err());
        assert!(parse("{}").is_err());
    }
}
