//! The analysis suite every ngspice-family backend (ngspice, ESPice) must
//! pass: [`analysis_suite!`] instantiates it in one test crate for one
//! backend. Each test runs a real deck through `Dialect::run` and asserts a
//! closed-form number, not merely that the process exited 0.
//!
//! Unsupported fails, never skips: a backend not on PATH fails every test;
//! run them in `nix develop .#testing`.
//!
//! Cites: none (helpers; the oracles are in `suite.rs` and the test crates).
//!
//! Contents:
//! - `pub fn run(d: Dialect, deck: &str) -> RawData`
//! - `pub fn written(d: Dialect, deck: &str) -> String`
//! - `pub fn measures(d: Dialect, raw: &RawData) -> Vec<MeasureResult>`
//! - `pub fn index_of(raw: &RawData, name: &str) -> usize`
//! - `pub fn scalar(raw: &RawData, name: &str) -> f64`
//! - `pub fn column<'a>(raw: &'a RawData, name: &str) -> &'a [f64]`
//! - `pub fn close(actual: f64, expected: f64, rel: f64) -> bool`

use spicerack::backend::Dialect;
use spicerack::measure_parse::parse_measures;
use spicerack::result::{MeasureResult, RawData};

#[macro_use]
mod suite;

/// Runs `deck` on `d`.
pub fn run(d: Dialect, deck: &str) -> RawData {
    let program = d.name();
    let found = std::process::Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        found,
        "{program} not on PATH: run the tests in `nix develop .#testing`"
    );
    d.run(deck)
        .unwrap_or_else(|e| panic!("deck failed: {e}\n{deck}"))
}

/// ngspice `deck` as the renderer writes it for `d` (`syntax::render`);
/// panics with the refusal's messages when it refuses.
pub fn written(d: Dialect, deck: &str) -> String {
    spicerack::syntax::render::render_text(deck, Dialect::Ngspice, "deck", d)
        .unwrap_or_else(|e| panic!("{}\n{deck}", e.messages))
        .text
}

/// `Dialect::run` only captures stdout; `simulation::run` is what normally
/// turns it into measures. These tests go straight to the backend, so they
/// parse it themselves.
pub fn measures(d: Dialect, raw: &RawData) -> Vec<MeasureResult> {
    parse_measures(&raw.stdout, Some(d))
}

/// Index of a vector by bare node name — ngspice reports node voltages as
/// `v(out)` and case-folds inconsistently.
pub fn index_of(raw: &RawData, name: &str) -> usize {
    let wrapped = format!("v({name})");
    raw.variables
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case(name) || v.name.eq_ignore_ascii_case(&wrapped))
        .unwrap_or_else(|| {
            panic!(
                "no vector '{name}' in {:?}",
                raw.variables.iter().map(|v| &v.name).collect::<Vec<_>>()
            )
        })
}

pub fn scalar(raw: &RawData, name: &str) -> f64 {
    raw.real_data[index_of(raw, name)][0]
}

pub fn column<'a>(raw: &'a RawData, name: &str) -> &'a [f64] {
    &raw.real_data[index_of(raw, name)]
}

pub fn close(actual: f64, expected: f64, rel: f64) -> bool {
    (actual - expected).abs() <= rel * expected.abs().max(1e-30)
}
