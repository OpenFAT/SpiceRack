#![cfg(feature = "backend")]
//! VACASK end-to-end: every deck here is emitted by `VacaskCodeGen` and then
//! handed to the real binary. A deck that parses and produces a raw file is the
//! only evidence that counts, so nothing is asserted against the generated text
//! that is not also asserted against the numbers VACASK returns.
//!
//! Unsupported fails, never skips: without `vacask` on PATH every run fails;
//! run the tests in `nix develop .#testing`.

use spicerack::backend::{Dialect, netlist as emit};
use spicerack::ir::*;
use spicerack::result::RawData;

mod analyses;
mod devices;
mod refusals;
mod structure;
mod waveforms;

// ── Harness ──

fn have_vacask() -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("vacask").is_file()))
        .unwrap_or(false)
}

/// Emit the deck and run it.
fn run(ir: &CircuitIR) -> RawData {
    let netlist = emit(ir, Dialect::Vacask).expect("emit vacask deck");
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

// ── IR builders ──

fn empty_sub(name: &str) -> Subcircuit {
    Subcircuit {
        name: name.into(),
        ..Default::default()
    }
}

fn empty_tb(analyses: Vec<Analysis>) -> Testbench {
    Testbench {
        analyses,
        ..Default::default()
    }
}

fn r(name: &str, n1: &str, n2: &str, v: f64) -> Component {
    Component::Resistor {
        name: name.into(),
        n1: n1.into(),
        n2: n2.into(),
        value: IrValue::Numeric { value: v },
        model: None,
        params: vec![],
    }
}

fn c(name: &str, n1: &str, n2: &str, v: f64) -> Component {
    Component::Capacitor {
        name: name.into(),
        n1: n1.into(),
        n2: n2.into(),
        value: IrValue::Numeric { value: v },
        model: None,
        params: vec![],
    }
}

fn vsrc(name: &str, np: &str, nm: &str, dc: f64) -> Component {
    Component::VoltageSource {
        name: name.into(),
        np: np.into(),
        nm: nm.into(),
        value: IrValue::Numeric { value: dc },
        ac_magnitude: None,
        ac_phase: None,
        waveform: None,
        port: None,
    }
}

/// 1k/1k divider off a 3 V rail plus a 1 uF cap on the output: a circuit with
/// a closed form for every analysis below.
fn rc_ir(analyses: Vec<Analysis>) -> CircuitIR {
    let mut top = empty_sub("rc_divider");
    top.components = vec![r("1", "vdd", "out", 1000.0), r("2", "out", "0", 1000.0)];
    let mut tb = empty_tb(analyses);
    tb.stimulus = vec![vsrc("dd", "vdd", "0", 3.0)];
    CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    }
}
