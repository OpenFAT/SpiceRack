//! Waveform sources the IR carries beyond SPICE3's six: PWL `r=`/`td=`,
//! TRNOISE and TRRANDOM, and what each dialect writes for them or refuses;
//! the ngspice importer reading PWL `r=`/`td=`, RF ports and `.sp`, and the
//! Spectre importer reading PWL `delay`/`pwlperiod`.
//!
//! Oracle: real ngspice running the deck the codegen wrote, checked against
//! the waveform's closed form (ngspice 45 manual §4.1.4) or, for the random
//! sources, against their distribution with a stated statistical tolerance
//! (§4.1.7, §4.1.8). An imported ngspice deck: real ngspice on the original
//! and on the deck written from its IR, every vector equal to 1e-12. An
//! imported Spectre deck (no Spectre here): the Spectre Reference's
//! semantics as the closed form, on ngspice running the deck written from
//! its IR. A missing ngspice is a failure, not a skip.
//!
//! Contents: none (tests only).
#![cfg(feature = "backend")]

use spicerack::codegen::{Dialect, netlist};
use spicerack::ir::*;
use spicerack::result::RawData;

mod imported;
mod pwl;
mod random;

// ---- private ----

fn ms(x: f64) -> IrNum {
    IrNum::Num(x * 1e-3)
}

/// `PWL(0 0 1m 1 2m 0)`: a 1 V triangle, 2 ms long.
fn triangle(repeat: Option<IrNum>, delay: Option<IrNum>) -> IrWaveform {
    IrWaveform::Pwl {
        values: vec![
            (ms(0.0), 0.0.into()),
            (ms(1.0), 1.0.into()),
            (ms(2.0), 0.0.into()),
        ],
        repeat,
        delay,
    }
}

fn source(current: bool, wf: IrWaveform) -> Component {
    let (name, np, nm, value, ac_magnitude, ac_phase) =
        ("1".into(), "a".into(), "0".into(), wf.at_zero(), None, None);
    if current {
        Component::CurrentSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform: Some(wf),
        }
    } else {
        Component::VoltageSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform: Some(wf),
            port: None,
        }
    }
}

/// `src` into 1 Ω at `a`, under `.tran` to `stop` with steps of at most `step`.
fn bench(src: Component, step: f64, stop: f64) -> CircuitIR {
    let r = Component::Resistor {
        name: "l".into(),
        n1: "a".into(),
        n2: "0".into(),
        value: IrValue::Numeric { value: 1.0 },
        model: None,
        params: vec![],
    };
    let top = Subcircuit {
        name: "sources".into(),
        components: vec![src, r],
        ..Default::default()
    };
    let tran = Analysis::Transient {
        step,
        stop,
        start: None,
        max_step: Some(step),
        uic: false,
    };
    CircuitIR {
        top,
        testbench: Some(Testbench {
            analyses: vec![tran],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn ngspice(deck: &str) -> RawData {
    Dialect::Ngspice
        .run(deck)
        .unwrap_or_else(|e| panic!("ngspice: {e}\n{deck}"))
}

fn col<'a>(raw: &'a RawData, name: &str) -> &'a [f64] {
    let wrapped = format!("v({name})");
    let i = raw
        .variables
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case(name) || v.name.eq_ignore_ascii_case(&wrapped));
    let i = i.unwrap_or_else(|| {
        panic!(
            "no {name} in {:?}",
            raw.variables.iter().map(|v| &v.name).collect::<Vec<_>>()
        )
    });
    &raw.real_data[i]
}

/// The output at time `t`, linear between timepoints: exact for a PWL
/// source, whose corners ngspice lands on as breakpoints.
fn at(time: &[f64], v: &[f64], t: f64) -> f64 {
    let k = time.partition_point(|&x| x < t).clamp(1, time.len() - 1);
    let (t0, t1) = (time[k - 1], time[k]);
    v[k - 1] + (v[k] - v[k - 1]) * (t - t0) / (t1 - t0)
}
