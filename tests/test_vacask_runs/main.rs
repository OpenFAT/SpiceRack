//! VACASK end-to-end: every deck here is ngspice text written for VACASK by
//! `syntax::render` and then handed to the real binary. A deck that parses and produces a raw file is the
//! only evidence that counts, so nothing is asserted against the generated text
//! that is not also asserted against the numbers VACASK returns.
//!
//! Unsupported fails, never skips: without `vacask` on PATH every run fails;
//! run the tests in `nix develop .#testing`.
//!
//! Cites: real VACASK; the closed forms named in each module's tests.
//!
//! Contents: none (tests only).
#![cfg(feature = "backend")]

use spicerack::backend::Dialect;
use spicerack::result::RawData;
use spicerack::syntax::diag::Code;
use spicerack::syntax::render::{Numbers, Source, render};
use spicerack::syntax::{lexicon, parse, tokenize};

mod analyses;
mod devices;
mod refusals;
mod structure;
mod waveforms;

// ---- private ----

// ── Harness ──

fn have_vacask() -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("vacask").is_file()))
        .unwrap_or(false)
}

/// The ngspice deck `deck` (line 1 its title) written for VACASK, or the
/// codes of its refusal. Its numbers are read as the doubles they spell
/// (`lexicon::NGSPICE_GIVEN`, as the Python API's decks are), which also
/// reads a `.step`.
fn emit(deck: &str) -> Result<String, Vec<Code>> {
    let lex = &lexicon::NGSPICE_GIVEN;
    let t = tokenize(deck, lex);
    let ast = parse(&t, lex);
    let src = Source {
        ast: &ast,
        tokens: &t,
        dialect: Dialect::Ngspice,
        name: deck.lines().next().unwrap_or_default(),
        kit: None,
        loads: &[],
        numbers: Numbers::Given,
    };
    render(&src, Dialect::Vacask)
        .map(|w| w.text)
        .map_err(|d| d.diags.iter().map(|d| d.code).collect())
}

/// Emit the deck and run it.
fn run(deck: &str) -> RawData {
    let netlist = emit(deck).expect("emit vacask deck");
    assert!(
        have_vacask(),
        "vacask not on PATH: run the tests in `nix develop .#testing`"
    );
    match Dialect::Vacask.run(&netlist) {
        Ok(raw) => raw,
        Err(e) => panic!(
            "vacask rejected the generated deck: {}\n--- deck ---\n{}",
            e, netlist
        ),
    }
}

/// One variable's real data, looked up by VACASK's own column name.
fn col(raw: &RawData, name: &str) -> Vec<f64> {
    let i = raw
        .variables
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| {
            panic!(
                "no column '{}' in {:?}",
                name,
                raw.variables.iter().map(|v| &v.name).collect::<Vec<_>>()
            )
        });
    raw.re(i).into_owned()
}

fn cplx(raw: &RawData, name: &str) -> Vec<num_complex::Complex64> {
    let i = raw
        .variables
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| {
            panic!(
                "no column '{}' in {:?}",
                name,
                raw.variables.iter().map(|v| &v.name).collect::<Vec<_>>()
            )
        });
    raw.complex_data[i].clone()
}

fn close(got: f64, want: f64, tol: f64) {
    assert!(
        (got - want).abs() <= tol * want.abs().max(1e-12),
        "got {:e}, want {:e} (rel tol {:e})",
        got,
        want,
        tol
    );
}

// ── Decks ──

/// 1k/1k divider off a 3 V rail: a circuit with a closed form for every
/// analysis below, the analyses' cards after it.
fn rc(analyses: &str) -> String {
    format!("rc_divider\nR1 vdd out 1k\nR2 out 0 1k\nVdd vdd 0 3\n{analyses}\n.end\n")
}
