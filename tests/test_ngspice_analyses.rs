#![cfg(feature = "backend")]
//! End-to-end coverage of every analysis the library exposes on ngspice:
//! the shared suite in `common/mod.rs`, then what only ngspice runs.
//!
//! Each test runs a real deck through `Dialect::Ngspice` and asserts a
//! closed-form number, not merely that the process exited 0. Verified against
//! ngspice 44.2.
//!
//! Skipped automatically when ngspice is not on PATH.

#[macro_use]
mod common;

use spicerack::backend::Dialect;

analysis_suite!(Dialect::Ngspice, tf_columns: ("output_impedance_at_v(out)", "v1#input_impedance"));

// ── .noise ──

#[test]
fn noise_returns_the_spectra_plot_and_matches_sqrt_4ktr() {
    // A 10k resistor into an open (1 fF) node: the whole 4kTR shows up at `out`.
    // sqrt(4 * k * 300.15 K * 10k) = 1.2874809e-8 V/sqrt(Hz).
    let raw = run(
        "* noise\nV1 in 0 0 AC 1\nR1 in out 10k\nC1 out 0 1f\n.save all\n\
         .noise V(out) V1 lin 2 1 2\n.end\n",
    );
    // The first plot in the raw file must be the spectra, not "Integrated
    // Noise" — `rawfile::parse_raw` only reads the first one.
    assert_eq!(raw.plot_name, "Noise Spectral Density Curves");
    let nsd = column(&raw, "onoise_spectrum");
    let expected = (4.0 * 1.380649e-23 * 300.15 * 10e3f64).sqrt();
    for v in nsd {
        assert!(close(*v, expected, 1e-4), "onoise {v}, want {expected}");
    }
}

// ── .disto ──

#[test]
fn disto_runs_and_yields_the_second_harmonic_plot() {
    let raw = run(
        "* disto\nV1 in 0 0 AC 1\nR1 in nb 10k\nD1 nb 0 dmod\n\
         .model dmod D(IS=1e-14)\n.save all\n.disto dec 5 1k 10k\n.end\n",
    );
    // KNOWN LOSS: ngspice writes "2nd harmonic" *and* "3rd harmonic" plots;
    // `rawfile::parse_raw` reads only the first, so the 3rd is dropped.
    assert_eq!(raw.plot_name, "DISTORTION - 2nd harmonic");
    assert!(raw.is_complex);
    assert!(!raw.complex_data.is_empty());
}

// ── .pz ──

#[test]
fn pz_finds_the_rc_pole_at_minus_one_over_rc() {
    // R = 1k, C = 1u -> pole at -1/(RC) = -1000 rad/s.
    let raw = run(
        "* pz\nV1 in 0 0 AC 1\nR1 in out 1k\nC1 out 0 1u\n.pz in 0 out 0 vol pol\n.end\n",
    );
    assert_eq!(raw.plot_name, "Pole-Zero Analysis");
    let pole = raw.complex_data[0][0];
    assert!(close(pole.re, -1000.0, 1e-6), "pole {pole}");
    assert!(pole.im.abs() < 1e-6, "real pole expected, got {pole}");
}

// ── .four ──

#[test]
fn four_prints_the_harmonic_table_to_stdout() {
    // Under `-b -r` ngspice says ".fourier line ignored since rawfile was
    // produced." The codegen runs the transient from a `.control` block with
    // `fourier` after it; the table then lands in stdout.
    let deck = common::emitted(
        Dialect::Ngspice,
        "* four\nV1 in 0 SIN(0 1 1k)\nR1 in out 1k\nC1 out 0 1n\n\
         .tran 1u 10m\n.four 1k v(out)\n.end\n",
        |_| {},
    );
    assert!(deck.contains(".control\ntran 1u 10m\nfourier 1k v(out)\n.endc\n"), "{deck}");
    assert!(!deck.contains(".four"), "a .four dot-card would run the transient twice:\n{deck}");
    let raw = run(&deck);
    assert!(
        !raw.stdout.contains("fourier line ignored"),
        "four must not be dropped:\n{}",
        raw.stdout
    );
    assert!(
        raw.stdout.contains("Fourier analysis for v(out)"),
        "no fourier table:\n{}",
        raw.stdout
    );
    // The fundamental of a clean 1 V sine is 1 V; a pure sine has ~0 THD.
    let thd: f64 = raw
        .stdout
        .split("THD: ")
        .nth(1)
        .and_then(|s| s.split(' ').next())
        .and_then(|s| s.parse().ok())
        .expect("THD in table");
    assert!(thd < 1e-6, "THD of a pure sine should be ~0, got {thd} %");
    // The transient data must still be in the raw file.
    assert_eq!(raw.plot_name, "Transient Analysis");
    assert!(column(&raw, "out").len() > 100);
}

// ── .measure ──

/// The `.control` block holding only `pre_osdi` (what the codegen emits for
/// OSDI loads) runs nothing, so the measure still gets its own block.
#[test]
fn measure_survives_alongside_an_osdi_control_block() {
    let deck = common::emitted(
        Dialect::Ngspice,
        "* osdi + meas\nV1 in 0 0\nR1 in out 10k\nR2 out 0 10k\n\
         .meas dc vhalf FIND V(out) AT=10\n.dc V1 0 10 1\n.end\n",
        |ir| ir.top.osdi_loads.push("/nonexistent/model.osdi".into()),
    );
    assert!(deck.contains("\n.control\npre_osdi /nonexistent/model.osdi\n.endc\n"), "{deck}");
    assert!(deck.contains(".control\ndc V1 0 10 1\n.endc\n"), "{deck}");
    let raw = run(&deck);
    assert!(
        measures(&raw).iter().any(|m| m.name == "vhalf"),
        "measure lost next to an osdi block:\n{}",
        raw.stdout
    );
}

/// A hand-written deck that runs its own analysis from `.control` (the shape
/// the codegen writes for `.measure`) gets the backend's `write` block: the
/// measure reaches stdout and the sweep reaches the raw file.
#[test]
fn hand_written_control_deck_reports_measures_and_data() {
    let raw = run(
        "* meas in control\nV1 in 0 0\nR1 in out 10k\nR2 out 0 10k\n\
         .meas dc vhalf FIND V(out) AT=10\n.control\ndc V1 0 10 1\n.endc\n.end\n",
    );
    let m = measures(&raw);
    assert!(m.iter().any(|m| m.name == "vhalf" && close(m.value, 5.0, 1e-9)), "{m:?}\n{}", raw.stdout);
    assert_eq!(column(&raw, "out").len(), 11);
    assert!(close(column(&raw, "out")[10], 5.0, 1e-9));
}

/// A hand-written deck with top-level `.meas` or `.four` and no `.control`
/// block that runs it would go to ngspice with `-r`, which drops those
/// results and exits 0. The backend refuses it, naming the card, before
/// ngspice runs.
#[test]
fn top_level_meas_or_four_without_a_control_block_is_refused() {
    let meas = "* t\nV1 in 0 1\nR1 in 0 1k\n.MEASURE dc i1 find i(V1) at=1\n.dc V1 0 1 1\n.end\n";
    let four = "* t\nV1 in 0 SIN(0 1 1k)\nR1 in 0 1k\n.tran 1u 2m\n.four 1k v(in)\n.end\n";
    for (deck, card) in [(meas, ".MEASURE dc i1"), (four, ".four 1k v(in)")] {
        let err = Dialect::Ngspice.run(deck).expect_err("results would be lost").to_string();
        assert!(err.contains(card) && err.contains(".control block"), "{err}");
    }
}

/// A raw `.four` line (here a testbench line with a `+` continuation) runs
/// as `fourier` in the codegen's `.control` block: the table reaches stdout.
#[test]
fn raw_four_line_reports_its_fourier_table() {
    let deck = common::emitted(
        Dialect::Ngspice,
        "* four\nV1 in 0 SIN(0 1 1k)\nR1 in out 1k\nC1 out 0 1n\n.tran 1u 10m\n.end\n",
        |ir| ir.testbench.as_mut().unwrap().extra_lines.extend([".four 1k".to_string(), "+ v(in)".to_string()]),
    );
    assert!(deck.contains(".control\ntran 1u 10m\nfourier 1k v(in)\n.endc\n"), "{deck}");
    let raw = run(&deck);
    // A clean 1 V sine: fundamental 1 V, THD ~0 (closed form).
    assert!(raw.stdout.contains("Fourier analysis for v(in)"), "{}", raw.stdout);
    let thd: f64 = raw.stdout.split("THD: ").nth(1).and_then(|s| s.split(' ').next()).and_then(|s| s.parse().ok()).expect("THD in table");
    assert!(thd < 1e-6, "THD {thd} %");
}

// ── circuit features claimed in the capability table ──

#[test]
fn xspice_a_elements_run() {
    // `gain` code model, gain = 2 -> a 0.5 V input gives 1.0 V out.
    let raw = run(
        "* xspice\nV1 in 0 0.5\nA1 in out gainblk\n\
         .model gainblk gain(gain=2.0)\nRl out 0 1meg\n.op\n.end\n",
    );
    assert!(close(scalar(&raw, "out"), 1.0, 1e-6), "{}", scalar(&raw, "out"));
}

#[test]
fn control_blocks_run_verbatim() {
    let raw = run("* control\nV1 in 0 3\nR1 in 0 1k\n.control\nop\n.endc\n.end\n");
    assert!(close(scalar(&raw, "in"), 3.0, 1e-9));
}

/// The XSPICE `s_xfer` code model is ngspice's Laplace facility. The
/// `Laplace(...)` B-source text that `Circuit::has_laplace_sources` looks for
/// is *not* ngspice syntax — see the report.
#[test]
fn xspice_s_xfer_is_the_working_laplace_path() {
    // 1/(1 + s*tau), tau = 159.1549431 us -> -3.0103 dB at 1 kHz.
    let raw = run(
        "* s_xfer\nV1 in 0 0 AC 1\nA1 in out filt\n\
         .model filt s_xfer(num_coeff=[1] den_coeff=[1.591549431e-4 1] int_ic=[0])\n\
         Rl out 0 1meg\n.ac lin 1 1000 1000\n.end\n",
    );
    let db = 20.0 * raw.complex_data[index_of(&raw, "out")][0].norm().log10();
    assert!((db + 3.0103).abs() < 1e-2, "s_xfer corner {db} dB");
}

// ── several analyses beside a .meas ──

/// Regression: with a `.meas`, the codegen moved one analysis into the
/// `.control` block and wrote only its plot; the `.op` ran afterwards and
/// never reached the result. Closed forms: a 1k/1k divider from 2 V is
/// 1 V at `out`; `a` follows the 1 V, 1 kHz sine exactly (a source across a
/// resistor); its sampled maximum at a 1 µs step is within
/// 1 - cos(pi * 1e-3) = 4.9e-6 of 1.
#[test]
fn every_analysis_of_a_measured_testbench_reaches_the_result() {
    use spicerack::ir::*;
    let v = |name: &str, np: &str, value: f64, waveform: Option<IrWaveform>| Component::VoltageSource {
        name: name.into(), np: np.into(), nm: "0".into(), value: IrValue::Numeric { value }, ac_magnitude: None, ac_phase: None, waveform, port: None,
    };
    let r = |name: &str, n1: &str, n2: &str| Component::Resistor { name: name.into(), n1: n1.into(), n2: n2.into(), value: IrValue::Numeric { value: 1e3 }, model: None, params: vec![] };
    let sin = IrWaveform::Sin { offset: 0.0.into(), amplitude: 1.0.into(), frequency: 1e3.into(), delay: 0.0.into(), damping: 0.0.into(), phase: 0.0.into() };
    let top = Subcircuit {
        name: "op and tran".into(),
        components: vec![v("1", "in", 2.0, None), r("1", "in", "out"), r("2", "out", "0"), v("2", "a", 0.0, Some(sin)), r("3", "a", "0")],
        ..Default::default()
    };
    let tb = Testbench {
        analyses: vec![Analysis::Op, Analysis::Transient { step: 1e-6, stop: 1e-3, start: None, max_step: Some(1e-6), uic: false }],
        measures: vec!["tran vmax MAX v(a)".into()],
        ..Default::default()
    };
    let ir = CircuitIR { top, testbench: Some(tb), ..Default::default() };
    let deck = spicerack::codegen::netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(deck.contains(".control\nop\ntran 1u 1m 0 1u\n.endc\n"), "every analysis runs in the block:\n{deck}");
    let plots = spicerack::simulation::run_all(&ir, Some("ngspice")).unwrap_or_else(|e| panic!("{e}\n{deck}"));
    let names: Vec<&str> = plots.iter().map(|p| p.plot_name.as_str()).collect();
    assert_eq!(names, ["Operating Point", "Transient Analysis"]);
    assert!(close(scalar(&plots[0], "out"), 1.0, 1e-12), "op v(out) = {}", scalar(&plots[0], "out"));
    let (time, a) = (column(&plots[1], "time"), column(&plots[1], "a"));
    for (t, x) in time.iter().zip(a) {
        assert!((x - (2.0 * std::f64::consts::PI * 1e3 * t).sin()).abs() < 1e-9, "t={t}: v(a) = {x}");
    }
    let vmax = plots[0].measures.iter().find(|m| m.name == "vmax").unwrap_or_else(|| panic!("{:?}", plots[0].measures));
    assert!((vmax.value - 1.0).abs() < 5e-6, "vmax = {}", vmax.value);
}

// ── .sp and RF ports ──

/// A voltage source as an RF port, written by the codegen (manual §4.1.11).
fn port(name: &str, node: &str, num: u32) -> spicerack::ir::Component {
    use spicerack::ir::*;
    Component::VoltageSource {
        name: name.into(), np: node.into(), nm: "0".into(), value: IrValue::Numeric { value: 0.0 },
        ac_magnitude: Some(1.0), ac_phase: None, waveform: None, port: Some(RfPort { num, z0: 50.0 }),
    }
}

#[test]
fn sp_of_a_matched_t_attenuator_reflects_nothing_and_passes_half() {
    // A T pad for K = 2 in Z0 = 50: series arms Z0 (K-1)/(K+1) = 50/3, shunt
    // 2 Z0 K/(K^2-1) = 200/3. Matched both ways, so S11 = S22 = 0 and
    // S21 = S12 = 1/K = 0.5, real, at every frequency.
    use spicerack::ir::*;
    let r = |name: &str, n1: &str, n2: &str, value: f64| Component::Resistor { name: name.into(), n1: n1.into(), n2: n2.into(), value: IrValue::Numeric { value }, model: None, params: vec![] };
    let top = Subcircuit {
        name: "t pad".into(),
        components: vec![port("1", "in", 1), port("2", "out", 2), r("1", "in", "mid", 50.0 / 3.0), r("2", "mid", "out", 50.0 / 3.0), r("3", "mid", "0", 200.0 / 3.0)],
        ..Default::default()
    };
    let sp = Analysis::SPar { sweep: FreqSweep { variation: Sweep::Lin, points: 3, start: 1e3, stop: 3e3 } };
    let ir = CircuitIR { top, testbench: Some(Testbench { analyses: vec![sp], ..Default::default() }), ..Default::default() };
    let deck = spicerack::codegen::netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(deck.contains("V1 in 0 0 AC 1 portnum 1 z0 50\n"), "{deck}");
    let raw = Dialect::Ngspice.run(&deck).unwrap_or_else(|e| panic!("{e}\n{deck}"));
    for (s, want) in [("S_1_1", 0.0), ("S_2_2", 0.0), ("S_2_1", 0.5), ("S_1_2", 0.5)] {
        for z in &raw.complex_data[index_of(&raw, s)] {
            assert!((z.re - want).abs() < 1e-9 && z.im.abs() < 1e-9, "{s} = {z}, want {want}");
        }
    }
}

/// An RF port puts Z0 in series with the source in every analysis (manual
/// §4.1.11), so a dialect that cannot write one refuses rather than drop it.
#[test]
fn rf_ports_are_refused_where_no_dialect_form_exists() {
    use spicerack::ir::*;
    let ir = CircuitIR { top: Subcircuit { name: "p".into(), components: vec![port("1", "in", 1)], ..Default::default() }, ..Default::default() };
    assert!(spicerack::codegen::netlist(&ir, Dialect::Espice).unwrap().contains("V1 in 0 0 AC 1 portnum 1 z0 50"));
    for d in [Dialect::Ltspice, Dialect::Hspice, Dialect::Xyce, Dialect::Spectre, Dialect::Vacask] {
        let e = spicerack::codegen::netlist(&ir, d).unwrap_err().to_string();
        assert!(e.contains("RF port"), "{d:?}: {e}");
    }
}

// ── .pss ──

/// ngspice-45 as built here answers `.pss` with "unimplemented dot command"
/// (the manual calls PSS experimental, §11.3.12), so the oracle is the
/// manual's own example card, `.pss 624e6 1u v_plus 1024 10 150 5e-3 uic`:
/// the card written for the same fields reads, token by token, as the same
/// numbers ngspice would parse.
#[test]
fn pss_card_matches_the_manual_example() {
    use spicerack::ir::*;
    let pss = Analysis::Pss {
        fundamental: 624e6, stabilization: 1e-6, observe_node: "v_plus".into(), points_per_period: 1024, harmonics: 10,
        sc_iter: Some(150), steady_coeff: Some(5e-3), uic: true,
    };
    let ir = CircuitIR { testbench: Some(Testbench { analyses: vec![pss], ..Default::default() }), ..Default::default() };
    let deck = spicerack::codegen::netlist(&ir, Dialect::Ngspice).unwrap();
    let card = deck.lines().find(|l| l.starts_with(".pss")).unwrap_or_else(|| panic!("{deck}"));
    let manual = ".pss 624e6 1u v_plus 1024 10 150 5e-3 uic";
    let (got, want): (Vec<&str>, Vec<&str>) = (card.split_whitespace().collect(), manual.split_whitespace().collect());
    assert_eq!(got.len(), want.len(), "{card}");
    for (g, w) in got.iter().zip(&want) {
        let n = spicerack::unit::ngspice_value;
        match (n(g), n(w)) {
            (Some(a), Some(b)) => assert_eq!(a, b, "{card}"),
            _ => assert_eq!(g, w, "{card}"),
        }
    }
    // Positional: STEADYCOEFF alone still needs SCITER before it (default
    // 50), and `uic` alone both (default 1e-3).
    let only = |sc_iter, steady_coeff, uic| {
        let a = Analysis::Pss { fundamental: 1e3, stabilization: 1e-3, observe_node: "o".into(), points_per_period: 64, harmonics: 5, sc_iter, steady_coeff, uic };
        let ir = CircuitIR { testbench: Some(Testbench { analyses: vec![a], ..Default::default() }), ..Default::default() };
        spicerack::codegen::netlist(&ir, Dialect::Ngspice).unwrap().lines().find(|l| l.starts_with(".pss")).unwrap().to_string()
    };
    assert_eq!(only(None, None, false), ".pss 1000 0.001 o 64 5");
    assert_eq!(only(None, Some(5e-3), false), ".pss 1000 0.001 o 64 5 50 0.005");
    assert_eq!(only(Some(9), None, false), ".pss 1000 0.001 o 64 5 9 0.001");
    assert_eq!(only(None, None, true), ".pss 1000 0.001 o 64 5 50 0.001 uic");
}

/// ngspice-45 here has no `.pss` at all: pinned, so a build that grows it
/// fails this test and the manual-example golden above can become a run.
#[test]
fn this_ngspice_has_no_pss() {
    let deck = "* pss\nV1 in 0 SIN(0 1 1k)\nR1 in out 1k\nC1 out 0 159.15n\n.pss 1k 5m out 128 5 50 1e-3\n.end\n";
    let err = Dialect::Ngspice.run(deck).expect_err("ngspice-45 ran .pss: replace the golden with a run").to_string();
    // Line 5 is the `.pss` card ("unimplemented dot command '.pss'").
    assert!(err.contains("Error on line 5"), "{err}");
}

// ── ngbehavior ──

// ngspice's own "Compatibility modes selected" note is the oracle that the
// mode reached it before the deck was parsed.
#[test]
fn ngbehavior_line_sets_the_mode_before_the_deck_is_read() {
    let raw = run("* m\n*spicerack ngbehavior=hsa\nV1 a 0 1\nR1 a 0 1k\n.op\n.end\n");
    let note = "Compatibility modes selected: hs a";
    assert!(raw.stdout.contains(note), "{}", raw.stdout);
    assert!(close(scalar(&raw, "a"), 1.0, 1e-12));
}

#[test]
fn ngbehavior_ngspice_does_not_know_is_an_error() {
    let deck = "* m\n*spicerack ngbehavior=xyz\nV1 a 0 1\nR1 a 0 1k\n.op\n.end\n";
    let err = Dialect::Ngspice.run(deck).unwrap_err();
    assert!(err.to_string().contains("ngbehavior=xyz"), "{err}");
}

/// The `.control` path (several analyses, `run_all`) gets the mode too.
#[test]
fn ngbehavior_reaches_a_control_block_run() {
    let deck = "* m\n*spicerack ngbehavior=hsa\nV1 a 0 1\nR1 a 0 1k\n.control\nop\ndc V1 0 2 1\n.endc\n.end\n";
    let plots = Dialect::Ngspice.run_all(deck).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(plots.len(), 2, "op and dc plots");
    assert!(plots[0].stdout.contains("Compatibility modes selected: hs a"), "{}", plots[0].stdout);
    assert!(close(scalar(&plots[0], "a"), 1.0, 1e-12));
}
