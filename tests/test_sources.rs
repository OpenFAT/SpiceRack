#![cfg(feature = "backend")]
//! Waveform sources the IR carries beyond SPICE3's six: PWL `r=`/`td=`,
//! and what each dialect writes for them or refuses.
//!
//! Oracle: real ngspice running the deck the codegen wrote, checked against
//! the waveform's closed form (ngspice 45 manual §4.1.4). A missing ngspice
//! is a failure, not a skip.

use spicerack::codegen::{netlist, Dialect};
use spicerack::ir::*;
use spicerack::result::RawData;

fn ms(x: f64) -> IrNum {
    IrNum::Num(x * 1e-3)
}

/// `PWL(0 0 1m 1 2m 0)`: a 1 V triangle, 2 ms long.
fn triangle(repeat: Option<IrNum>, delay: Option<IrNum>) -> IrWaveform {
    IrWaveform::Pwl { values: vec![(ms(0.0), 0.0.into()), (ms(1.0), 1.0.into()), (ms(2.0), 0.0.into())], repeat, delay }
}

fn source(current: bool, wf: IrWaveform) -> Component {
    let (name, np, nm, value, ac_magnitude, ac_phase) = ("1".into(), "a".into(), "0".into(), wf.at_zero(), None, None);
    if current {
        Component::CurrentSource { name, np, nm, value, ac_magnitude, ac_phase, waveform: Some(wf) }
    } else {
        Component::VoltageSource { name, np, nm, value, ac_magnitude, ac_phase, waveform: Some(wf) }
    }
}

/// `src` into 1 Ω at `a`, under `.tran` to `stop` with steps of at most `step`.
fn bench(src: Component, step: f64, stop: f64) -> CircuitIR {
    let r = Component::Resistor { name: "l".into(), n1: "a".into(), n2: "0".into(), value: IrValue::Numeric { value: 1.0 }, model: None, params: vec![] };
    let top = Subcircuit { name: "sources".into(), components: vec![src, r], ..Default::default() };
    let tran = Analysis::Transient { step, stop, start: None, max_step: Some(step), uic: false };
    CircuitIR { top, testbench: Some(Testbench { analyses: vec![tran], ..Default::default() }), ..Default::default() }
}

fn ngspice(deck: &str) -> RawData {
    Dialect::Ngspice.run(deck).unwrap_or_else(|e| panic!("ngspice: {e}\n{deck}"))
}

fn col<'a>(raw: &'a RawData, name: &str) -> &'a [f64] {
    let wrapped = format!("v({name})");
    let i = raw.variables.iter().position(|v| v.name.eq_ignore_ascii_case(name) || v.name.eq_ignore_ascii_case(&wrapped));
    let i = i.unwrap_or_else(|| panic!("no {name} in {:?}", raw.variables.iter().map(|v| &v.name).collect::<Vec<_>>()));
    &raw.real_data[i]
}

/// The output at time `t`, linear between timepoints: exact for a PWL
/// source, whose corners ngspice lands on as breakpoints.
fn at(time: &[f64], v: &[f64], t: f64) -> f64 {
    let k = time.partition_point(|&x| x < t).clamp(1, time.len() - 1);
    let (t0, t1) = (time[k - 1], time[k]);
    v[k - 1] + (v[k] - v[k - 1]) * (t - t0) / (t1 - t0)
}

// ── PWL r= / td= ──

#[test]
fn a_repeating_pwl_two_periods_on_equals_its_first_period() {
    // r=0 td=0.5m: the 2 ms triangle from 0.5 ms on, forever. Two periods
    // later the value is the same, and both are the closed-form triangle.
    let ir = bench(source(false, triangle(Some(0.0.into()), Some(ms(0.5)))), 1e-5, 7e-3);
    let deck = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(deck.contains("PWL(0 0 0.001 1 0.002 0) r=0 td=0.0005"), "{deck}");
    let raw = ngspice(&deck);
    let (time, v) = (col(&raw, "time"), col(&raw, "a"));
    for k in 0..=200 {
        let tau = 2e-3 * k as f64 / 200.0;
        let want = if tau <= 1e-3 { tau / 1e-3 } else { (2e-3 - tau) / 1e-3 };
        let first = at(time, v, 0.5e-3 + tau);
        let third = at(time, v, 0.5e-3 + tau + 4e-3);
        assert!((first - want).abs() < 1e-9, "t=0.5m+{tau}: {first}, want {want}");
        assert!((third - first).abs() < 1e-9, "t=4.5m+{tau}: {third}, first period {first}");
    }
    // Before the delay the source holds its first value.
    assert!(at(time, v, 0.25e-3).abs() < 1e-12);
}

#[test]
fn a_pwl_without_repeat_holds_its_last_value() {
    // The neighbour of the test above: no r= means once, then the final 0 V.
    let ir = bench(source(false, triangle(None, None)), 1e-5, 5e-3);
    let deck = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(deck.contains("PWL(0 0 0.001 1 0.002 0)\n"), "{deck}");
    let raw = ngspice(&deck);
    assert!(at(col(&raw, "time"), col(&raw, "a"), 4.5e-3).abs() < 1e-12);
}

#[test]
fn pwl_repeat_and_delay_are_refused_where_no_dialect_form_exists() {
    let tri = || triangle(Some(0.0.into()), Some(ms(0.5)));
    // ngspice reads r=/td= on a V source only (manual §4.1.4; ngspice 45
    // answers an I source with "unknown parameter (r)"); ESPice reads both.
    assert!(netlist(&bench(source(true, tri()), 1e-5, 1e-3), Dialect::Ngspice).unwrap_err().to_string().contains("PWL"));
    assert!(netlist(&bench(source(true, tri()), 1e-5, 1e-3), Dialect::Espice).unwrap().contains(") r=0 td=0.0005"));
    for d in [Dialect::Ltspice, Dialect::Hspice, Dialect::Xyce, Dialect::Spectre, Dialect::Vacask] {
        let e = netlist(&bench(source(false, tri()), 1e-5, 1e-3), d).unwrap_err().to_string();
        assert!(e.contains("PWL"), "{d:?}: {e}");
    }
    // Without r=/td= every dialect but VACASK (no PWL at all) writes it.
    for d in [Dialect::Ltspice, Dialect::Hspice, Dialect::Xyce, Dialect::Spectre] {
        netlist(&bench(source(false, triangle(None, None)), 1e-5, 1e-3), d).unwrap_or_else(|e| panic!("{d:?}: {e}"));
    }
}

#[test]
fn espice_reads_the_repeating_pwl_current_source() {
    // ESPice takes r=/td= on an I source too: 1 A into 1 Ω, two periods on.
    let ir = bench(source(true, triangle(Some(0.0.into()), Some(ms(0.5)))), 1e-5, 7e-3);
    let deck = netlist(&ir, Dialect::Espice).unwrap();
    let raw = Dialect::Espice.run(&deck).unwrap_or_else(|e| panic!("espice: {e}\n{deck}"));
    let (time, v) = (col(&raw, "time"), col(&raw, "a"));
    // ESPice 0.3.0 does not land a timepoint on a repeated period's corner
    // (ngspice does), so a corner read by interpolation is off by up to one
    // max step times the slope: 10 µs × 1 V/ms = 0.01 V.
    let tol = 1e-5 * 1e3;
    for tau in [0.25e-3, 1e-3, 1.5e-3] {
        let want = if tau <= 1e-3 { tau / 1e-3 } else { (2e-3 - tau) / 1e-3 };
        // I flows n+ -> n- through the source, so into `a` from 0 is -I.
        let first = -at(time, v, 0.5e-3 + tau);
        let third = -at(time, v, 4.5e-3 + tau);
        assert!((first - want).abs() < tol, "{tau}: {first}, want {want}");
        assert!((third - first).abs() < tol, "{tau}: {third} vs {first}");
    }
}
