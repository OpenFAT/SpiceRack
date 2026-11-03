#![cfg(feature = "backend")]
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
        Component::VoltageSource { name, np, nm, value, ac_magnitude, ac_phase, waveform: Some(wf), port: None }
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
    for d in [Dialect::Ltspice, Dialect::Hspice, Dialect::Xyce, Dialect::Vacask] {
        let e = netlist(&bench(source(false, tri()), 1e-5, 1e-3), d).unwrap_err().to_string();
        assert!(e.contains("PWL"), "{d:?}: {e}");
    }
    // Spectre: `delay` and `pwlperiod`, but only for an r= that is exactly a
    // period (test_spectre_golden.rs); r=1m repeats from mid-waveform.
    assert!(netlist(&bench(source(false, tri()), 1e-5, 1e-3), Dialect::Spectre).unwrap().contains("delay=500u pwlperiod=2m"));
    let mid = triangle(Some(ms(1.0)), None);
    assert!(netlist(&bench(source(false, mid), 1e-5, 1e-3), Dialect::Spectre).unwrap_err().to_string().contains("PWL"));
    // Without r=/td= every dialect but VACASK (no PWL at all) writes it.
    for d in [Dialect::Ltspice, Dialect::Hspice, Dialect::Xyce, Dialect::Spectre] {
        netlist(&bench(source(false, triangle(None, None)), 1e-5, 1e-3), d).unwrap_or_else(|e| panic!("{d:?}: {e}"));
    }
}

// ── TRNOISE / TRRANDOM (manual §4.1.7, §4.1.8) ──
//
// Statistical oracles. ngspice seeds its generator with 1 unless told
// otherwise (manual §18.1); the deck sets `seed=1` so the run is the same
// every time, and each tolerance below is five standard errors of the
// estimate for N independent samples, so a correct source fails with
// probability under 1e-6 whatever the seed.

const N: usize = 4000;

/// `.options seed=1`, and the source `src` alone into 1 Ω.
fn seeded(src: Component, step: f64, stop: f64) -> CircuitIR {
    let mut ir = bench(src, step, stop);
    ir.testbench.as_mut().unwrap().options.portable.push(("seed".into(), "1".into()));
    ir
}

#[test]
fn trnoise_white_noise_has_rms_na() {
    // TRNOISE(NA NT 0 0): a Gaussian sample of rms NA every NT. Read at the
    // sample times (breakpoints), the N values are i.i.d. N(0, NA^2): the
    // sample std s has standard error NA/sqrt(2(N-1)), the mean NA/sqrt(N).
    let (na, nt) = (0.1, 1e-6);
    let noise = IrWaveform::TrNoise { na: na.into(), nt: nt.into(), nalpha: 0.0.into(), namp: 0.0.into(), rtsam: 0.0.into(), rtscapt: 0.0.into(), rtsemt: 0.0.into() };
    let ir = seeded(source(false, noise), nt, nt * (N + 1) as f64);
    let deck = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(deck.contains("TRNOISE(0.1 0.000001 0 0)"), "{deck}");
    let raw = ngspice(&deck);
    let (time, v) = (col(&raw, "time"), col(&raw, "a"));
    let xs: Vec<f64> = (1..=N).map(|k| at(time, v, k as f64 * nt)).collect();
    let mean = xs.iter().sum::<f64>() / N as f64;
    let s = (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (N - 1) as f64).sqrt();
    assert!(mean.abs() < 5.0 * na / (N as f64).sqrt(), "mean {mean}");
    assert!((s - na).abs() < 5.0 * na / (2.0 * (N - 1) as f64).sqrt(), "std {s}, want {na}");
}

#[test]
fn trrandom_uniform_stays_in_range_with_the_offset_as_mean() {
    // TRRANDOM(1 TS TD 2 3): uniform within 3 ± 2 for each TS, and 3 (the
    // offset) before TD. Uniform on [1, 5] has sigma 4/sqrt(12); the mean of
    // N draws is 3 within 5 sigma/sqrt(N). N draws all inside a 3.5 V slice
    // of the 4 V range has probability below 1e-200, so the spread pins the
    // range too (a half-range reading would cap it at 2 V).
    let (ts, td) = (1e-6, 1e-5);
    let rnd = IrWaveform::TrRandom { kind: RandomKind::Uniform, ts: ts.into(), td: td.into(), p1: 2.0.into(), p2: 3.0.into() };
    let ir = seeded(source(false, rnd), ts, td + ts * (N + 1) as f64);
    let deck = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(deck.contains("TRRANDOM(1 0.000001 0.00001 2 3)"), "{deck}");
    let raw = ngspice(&deck);
    let (time, v) = (col(&raw, "time"), col(&raw, "a"));
    assert!((at(time, v, td / 2.0) - 3.0).abs() < 1e-12, "before TD: the offset");
    for (t, x) in time.iter().zip(v) {
        assert!((1.0 - 1e-12..=5.0 + 1e-12).contains(x), "t={t}: {x} outside 3 ± 2");
    }
    let xs: Vec<f64> = (0..N).map(|k| at(time, v, td + (k as f64 + 0.5) * ts)).collect();
    let mean = xs.iter().sum::<f64>() / N as f64;
    let sigma = 4.0 / 12f64.sqrt();
    assert!((mean - 3.0).abs() < 5.0 * sigma / (N as f64).sqrt(), "mean {mean}");
    let spread = xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
    assert!(spread > 3.5, "spread {spread}");
}

#[test]
fn trnoise_and_trrandom_are_ngspice_only() {
    // No other dialect has a source form (dev/research/ir-gaps.md §2), and
    // ESPice 0.3.0 refuses both cards ("the trnoise source waveform is not
    // supported").
    let z = || IrNum::Num(0.0);
    let noise = IrWaveform::TrNoise { na: 1.0.into(), nt: 1e-6.into(), nalpha: z(), namp: z(), rtsam: z(), rtscapt: z(), rtsemt: z() };
    let rnd = IrWaveform::TrRandom { kind: RandomKind::Gaussian, ts: 1e-6.into(), td: z(), p1: 1.0.into(), p2: z() };
    for wf in [noise, rnd] {
        for current in [false, true] {
            let ir = bench(source(current, wf.clone()), 1e-6, 1e-5);
            netlist(&ir, Dialect::Ngspice).unwrap();
            for d in [Dialect::Espice, Dialect::Ltspice, Dialect::Hspice, Dialect::Xyce, Dialect::Spectre, Dialect::Vacask] {
                let e = netlist(&ir, d).unwrap_err().to_string();
                assert!(e.contains("TRNOISE") || e.contains("TRRANDOM"), "{d:?}: {e}");
            }
        }
    }
}

/// The importer lowers an ngspice deck's TRNOISE/TRRANDOM cards (with
/// TRRANDOM's defaults filled in, manual §4.1.8) and keeps a TRRANDOM type
/// outside 1..=4 raw.
#[test]
fn ngspice_trnoise_and_trrandom_cards_lower_into_the_ir() {
    let deck = "t\nV1 a 0 dc 0 trnoise(20n 0.5n 0 0)\nI1 b 0 dc 0 TRNOISE(1m 1u 1.0 0.1m 15m 22u 50u)\n\
                V2 c 0 dc 0 trrandom (2 10m 0 1)\nV3 d 0 trrandom(4 1u)\nV4 e 0 trrandom(7 1u)\n.end\n";
    let (ir, report) = spicerack::import::parse(deck, "t", Dialect::Ngspice).unwrap();
    let wf = |i: usize| match &ir.top.components[i] {
        Component::VoltageSource { waveform, .. } | Component::CurrentSource { waveform, .. } => waveform.clone(),
        other => panic!("{other:?}"),
    };
    // Each number as ngspice reads its spelling (`20n` is 20 * 10^-9 there).
    let n = |s: &str| IrNum::Num(spicerack::unit::ngspice_value(s).unwrap());
    assert_eq!(wf(0), Some(IrWaveform::TrNoise { na: n("20n"), nt: n("0.5n"), nalpha: n("0"), namp: n("0"), rtsam: n("0"), rtscapt: n("0"), rtsemt: n("0") }));
    assert_eq!(wf(1), Some(IrWaveform::TrNoise { na: n("1m"), nt: n("1u"), nalpha: n("1.0"), namp: n("0.1m"), rtsam: n("15m"), rtscapt: n("22u"), rtsemt: n("50u") }));
    assert_eq!(wf(2), Some(IrWaveform::TrRandom { kind: RandomKind::Gaussian, ts: n("10m"), td: n("0"), p1: n("1"), p2: n("0") }));
    assert_eq!(wf(3), Some(IrWaveform::TrRandom { kind: RandomKind::Poisson, ts: n("1u"), td: n("0"), p1: n("1"), p2: n("0") }));
    assert!(matches!(&ir.top.components[4], Component::RawSpice { .. }), "{:?}", ir.top.components[4]);
    assert_eq!(report.raw, 1);
    // Written back, a card with no DC value gets 0, not the waveform's start:
    // these two add to the DC value.
    let out = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(out.contains("V3 d 0 0 TRRANDOM(4 0.000001 0 1 0)"), "{out}");
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

// ── ngspice cards in: PWL r=/td= (§4.1.4), RF port (§4.1.11), .sp (§11.3.11) ──

/// Each card form lowers into the IR's field; a form it has no field for
/// stays raw (an `r=` off a PWL, a port on an I source, a z0 expression,
/// `.sp`'s `donoise` flag).
#[test]
fn ngspice_pwl_repeat_delay_and_rf_port_cards_lower_into_the_ir() {
    let deck = "t\nV1 a 0 pwl(0 0 1m 1 2m 0) r=0 td=0.5m\nV2 b 0 PWL(0 0 1m 1) TD=1m\nV3 c 0 pwl 0 0 1m 1 2m 0 r=1m\n\
                I1 d 0 pwl(0 0 1m 1) r=0\nV4 e 0 dc 0 ac 1 portnum 1 z0 100\nV5 f 0 ac 1 portnum=2\nV6 j 0 pwl(0 0 1m 1) r={rr}\n\
                V7 g 0 sin(0 1 1k) r=0\nI2 h 0 ac 1 portnum 1\nV8 i 0 ac 1 portnum 1 z0 {zz}\n.sp lin 3 1k 3k\n.sp dec 10 1k 1g 1\n.end\n";
    let (ir, report) = spicerack::import::parse(deck, "t", Dialect::Ngspice).unwrap();
    let n = |s: &str| IrNum::Num(spicerack::unit::ngspice_value(s).unwrap());
    let pwl = |i: usize| match &ir.top.components[i] {
        Component::VoltageSource { waveform: Some(IrWaveform::Pwl { repeat, delay, .. }), .. }
        | Component::CurrentSource { waveform: Some(IrWaveform::Pwl { repeat, delay, .. }), .. } => (repeat.clone(), delay.clone()),
        other => panic!("{other:?}"),
    };
    let port = |i: usize| match &ir.top.components[i] {
        Component::VoltageSource { port, .. } => *port,
        other => panic!("{other:?}"),
    };
    assert_eq!(pwl(0), (Some(n("0")), Some(n("0.5m"))));
    assert_eq!(pwl(1), (None, Some(n("1m"))));
    assert_eq!(pwl(2), (Some(n("1m")), None));
    assert_eq!(pwl(3), (Some(n("0")), None));
    assert_eq!(port(4), Some(RfPort { num: 1, z0: 100.0 }));
    assert_eq!(port(5), Some(RfPort { num: 2, z0: 50.0 }));
    assert_eq!(pwl(6), (Some(IrNum::Expr("rr".into())), None));
    for i in 7..10 {
        assert!(matches!(&ir.top.components[i], Component::RawSpice { .. }), "{:?}", ir.top.components[i]);
    }
    let sweep = FreqSweep { variation: Sweep::Lin, points: 3, start: 1e3, stop: 3e3 };
    assert_eq!(ir.testbench.unwrap().analyses, [Analysis::SPar { sweep }]);
    assert_eq!(report.raw, 4);
}

/// `deck` on real ngspice, and the deck the importer and codegen make of
/// it: every vector of every plot the same, to 1e-12 relative. Every card
/// must have lowered, so the second deck is the IR's.
fn ngspice_round_trip(deck: &str) {
    let (ir, report) = spicerack::import::parse(deck, "rt", Dialect::Ngspice).unwrap();
    assert_eq!((report.raw, report.unparsed.len()), (0, 0), "{report:?}");
    let emitted = netlist(&ir, Dialect::Ngspice).unwrap();
    let run = |d: &str| Dialect::Ngspice.run_all(d).unwrap_or_else(|e| panic!("ngspice: {e}\n{d}"));
    let (want, got) = (run(deck), run(&emitted));
    assert_eq!(want.len(), got.len(), "{emitted}");
    let same = |a: f64, b: f64| (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1e-30);
    for (w, g) in want.iter().zip(&got) {
        assert_eq!(w.plot_name, g.plot_name);
        for v in &w.variables {
            let gi = g.variables.iter().position(|x| x.name == v.name).unwrap_or_else(|| panic!("{} missing\n{emitted}", v.name));
            let (a, b) = (&w.re(v.index), &g.re(gi));
            assert_eq!(a.len(), b.len(), "{} in {}", v.name, w.plot_name);
            assert!(a.iter().zip(b.iter()).all(|(&a, &b)| same(a, b)), "{} in {}\n{emitted}", v.name, w.plot_name);
            let (a, b) = (w.complex(v.index).unwrap_or_default(), g.complex(gi).unwrap_or_default());
            assert!(a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a.im, b.im)), "{} in {}", v.name, w.plot_name);
        }
    }
}

#[test]
fn an_imported_repeating_pwl_runs_as_the_original_deck() {
    ngspice_round_trip(
        "pwl round trip\nV1 a 0 pwl(0 0 1m 1 2m 0) r=0 td=0.5m\nR1 a 0 1\nV2 b 0 PWL(0 0 1m 1 1.5m 0.2) TD=1m\nR2 b 0 1\n\
         V3 c 0 pwl(0 0 0.5m 1 1m 0.5 2m 0) r 0.5m\nR3 c 0 1\n.tran 10u 7m 0 10u\n.end\n",
    );
}

#[test]
fn an_imported_rf_port_runs_as_the_original_deck() {
    // A T pad between ports 1 (z0 given) and 2 (z0 by default, `=` form):
    // the .op sees each port's z0 in series, the .sp its S-parameters.
    ngspice_round_trip(
        "port round trip\nV1 in 0 dc 1 ac 1 portnum 1 z0 50\nV2 out 0 dc 0 ac 1 portnum=2\nR1 in mid 16.666666666666668\n\
         R2 mid out 16.666666666666668\nR3 mid 0 66.66666666666667\n.op\n.sp lin 3 1k 3k\n.end\n",
    );
}

// ── Spectre cards in: PWL `delay`, `pwlperiod` ([CMP] p.683 #4, #17, #18; p.686) ──
//
// `delay` is the general waveform delay, ngspice's `td=`. `pwlperiod=P`
// repeats the waveform from t = 0 with period P, and before each repeat
// ramps over `twidth` (default P/1000) from its value at P - twidth to its
// value at 0. That is ngspice's `r=0` exactly when the points start at 0,
// end at P on the first point's value, and the last segment is at least
// twidth long (the ramp is then the segment itself); any other form stays raw.

const SPECTRE_PWLS: &str = "// pwl\nsimulator lang=spectre\n\
    v1 (a 0) vsource type=pwl wave=[0 0 1m 1 2m 0] delay=0.5m\n\
    v2 (b 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=2m\n\
    v3 (c 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=2m delay=0.5m\n\
    r1 (a 0) resistor r=1\nr2 (b 0) resistor r=1\nr3 (c 0) resistor r=1\n\
    tr tran step=10u stop=7m maxstep=10u\n";

#[test]
fn spectre_pwl_delay_and_an_exact_pwlperiod_lower_into_td_and_r() {
    let extra = "v4 (d 0) vsource type=pwl wave=[0 0 1m 1 2m 0.5] pwlperiod=2m\n\
                 v5 (e 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=3m\n\
                 v6 (f 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=2m twidth=1u\n\
                 v7 (g 0) vsource type=pwl wave=[0.1m 0 1m 1 2m 0] pwlperiod=2m\n\
                 v8 (h 0) vsource type=pwl wave=[0 0 1m 1 2m 0 2.0000001m 0] pwlperiod=2.0000001m\n\
                 v9 (i 0) vsource type=sine sinedc=0 ampl=1 freq=1k fmmodindex=2 fmmodfreq=100 delay=1m\n\
                 v10 (j 0) vsource type=exp val0=0 val1=1 td1=1m tau1=1m delay=1m\n\
                 v11 (k 0) vsource type=pulse val0=0 val1=1 pwlperiod=2m\n";
    let (ir, _) = spicerack::import::parse(&format!("{SPECTRE_PWLS}{extra}"), "t", Dialect::Spectre).unwrap();
    let sources: Vec<_> = ir.top.components.iter().filter_map(|c| match c {
        Component::VoltageSource { name, waveform, .. } => Some((name.as_str(), waveform.clone())),
        _ => None,
    }).collect();
    let pwl = |repeat: Option<f64>, delay: Option<f64>| {
        let values = [(0.0, 0.0), (1e-3, 1.0), (2e-3, 0.0)].map(|(t, v)| (IrNum::Num(t), IrNum::Num(v))).to_vec();
        Some(IrWaveform::Pwl { values, repeat: repeat.map(IrNum::Num), delay: delay.map(IrNum::Num) })
    };
    assert_eq!(sources, [("1", pwl(None, Some(0.5e-3))), ("2", pwl(Some(0.0), None)), ("3", pwl(Some(0.0), Some(0.5e-3)))]);
    // v4..v11: ends differ (a twidth ramp), a period past the points, a given
    // twidth, points from 0.1 ms, a last segment shorter than the default
    // twidth, an FM sine's and an exp's delay (no IR field), a pwlperiod off
    // a PWL. All raw.
    assert_eq!(ir.top.raw_spice.len(), 8, "{:?}", ir.top.raw_spice);
}

#[test]
fn an_imported_spectre_pwl_runs_on_ngspice_as_the_manual_says() {
    // The manual's semantics as the closed form: `tri` once (1 V at 1 ms,
    // 0 from 2 ms), shifted by `delay` (holding its first value before) and
    // repeated with period `pwlperiod`.
    let (ir, report) = spicerack::import::parse(SPECTRE_PWLS, "t", Dialect::Spectre).unwrap();
    assert_eq!((report.raw, ir.top.raw_spice.len()), (0, 0), "{:?}", ir.top.raw_spice);
    // Written back as Spectre, the cards say what they said.
    let back = netlist(&ir, Dialect::Spectre).unwrap();
    assert!(back.contains("wave=[0 0 1m 1 2m 0] pwlperiod=2m\n") && back.contains("wave=[0 0 1m 1 2m 0] delay=500u pwlperiod=2m\n"), "{back}");
    let deck = netlist(&ir, Dialect::Ngspice).unwrap();
    let raw = ngspice(&deck);
    let time = col(&raw, "time");
    let tri = |t: f64| if t <= 0.0 || t >= 2e-3 { 0.0 } else if t <= 1e-3 { t / 1e-3 } else { (2e-3 - t) / 1e-3 };
    let delayed = |t: f64| if t < 0.5e-3 { 0.0 } else { (t - 0.5e-3).rem_euclid(2e-3) };
    for (node, want) in [
        ("a", &(|t: f64| tri(t - 0.5e-3)) as &dyn Fn(f64) -> f64),
        ("b", &|t: f64| tri(t.rem_euclid(2e-3))),
        ("c", &|t: f64| tri(delayed(t))),
    ] {
        let v = col(&raw, node);
        for k in 0..=700 {
            let t = 7e-3 * k as f64 / 700.0;
            assert!((at(time, v, t) - want(t)).abs() < 1e-9, "{node} at {t}: {}, want {}\n{deck}", at(time, v, t), want(t));
        }
    }
}
