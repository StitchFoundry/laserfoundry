//! `fuzz/` is its own workspace with its own `Cargo.lock`, which CI neither builds nor audits. Every crate it shares with the
//! main lockfile must be at a version the main lockfile has, so the main dependency audit covers it (issue #94: the fuzz
//! lockfile drifted and kept advisory-flagged and yanked versions).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub fn run(root: &Path) -> Result<(), String> {
    let main = read(&root.join("Cargo.lock"))?;
    let fuzz_path = root.join("fuzz").join("Cargo.lock");
    if !fuzz_path.exists() {
        println!("fuzzlock: ok (no fuzz/Cargo.lock)");
        return Ok(());
    }
    let drift = drift(&main, &read(&fuzz_path)?);
    if drift.is_empty() {
        println!("fuzzlock: ok");
        Ok(())
    } else {
        Err(format!(
            "fuzz/Cargo.lock has versions the main Cargo.lock does not (copy Cargo.lock into fuzz/ and run `cargo update --workspace` there):\n  {}",
            drift.join("\n  ")
        ))
    }
}

/// Crate name to the versions a lockfile pins.
type Pins = BTreeMap<String, BTreeSet<String>>;

fn read(path: &Path) -> Result<Pins, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    Ok(pins(&text))
}

/// Shared crates pinned at a version the main lockfile does not have, as `name version`.
fn drift(main: &Pins, fuzz: &Pins) -> Vec<String> {
    let mut out = Vec::new();
    for (name, versions) in fuzz {
        if let Some(known) = main.get(name) {
            out.extend(versions.difference(known).map(|v| format!("{name} {v}")));
        }
    }
    out
}

/// The `name`/`version` pairs of the `[[package]]` blocks of a lockfile.
fn pins(text: &str) -> Pins {
    let mut out = Pins::new();
    let mut name: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if line == "[[package]]" {
            name = None;
        } else if let Some(v) = value(line, "name") {
            name = Some(v);
        } else if let (Some(v), Some(n)) = (value(line, "version"), name.take()) {
            out.entry(n).or_default().insert(v);
        }
    }
    out
}

fn value(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
    Some(rest.trim_matches('"').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_repository_fuzz_lockfile_is_in_step_with_the_main_one() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        run(root).unwrap();
    }

    #[test]
    fn a_shared_crate_at_another_version_is_drift_and_fuzz_only_crates_are_not() {
        let main = pins(&lock(&[("rustix", "1.1.6"), ("serde", "1.0.1")]));
        let fuzz = pins(&lock(&[("rustix", "1.1.5"), ("serde", "1.0.1"), ("libfuzzer-sys", "0.4.13")]));
        assert_eq!(drift(&main, &fuzz), vec!["rustix 1.1.5".to_string()]);
        assert!(drift(&main, &pins(&lock(&[("rustix", "1.1.6"), ("arbitrary", "1.5.0")]))).is_empty());
    }

    #[test]
    fn two_versions_of_one_crate_are_both_read() {
        let p = pins(&lock(&[("bitflags", "1.3.2"), ("bitflags", "2.9.0")]));
        assert_eq!(p.get("bitflags").map(BTreeSet::len), Some(2));
    }

    fn lock(packages: &[(&str, &str)]) -> String {
        let mut s = String::from("version = 4\n\n");
        for (n, v) in packages {
            s.push_str(&format!(
                "[[package]]\nname = \"{n}\"\nversion = \"{v}\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n"
            ));
        }
        s
    }
}
