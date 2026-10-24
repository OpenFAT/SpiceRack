//! One circuit dumped in every dialect, read back by ESPice.
//!
//! ESPice reads ngspice, HSPICE and Spectre (`espice --tokenizer=<d>`), so it
//! is the oracle for those three and for its own deck: each must give the same
//! operating point. LTspice and VACASK decks are only checked to emit; ESPice
//! reads neither. The HSPICE card forms follow ESPice's
//! `tests/frontend/grammar/hspice*.peg` and `tests/conformance/tests/netlist/hspice`.
//!
//! The ESPice runs skip when `espice` is not on PATH (`$ESPICE` overrides).

use spicerack::codegen::{netlist, Dialect};
use spicerack::ir::*;

fn sub(name: &str) -> Subcircuit {
    Subcircuit {
        name: name.into(),
        ports: vec![],
        parameters: vec![],
        components: vec![],
        instances: vec![],
        models: vec![],
        raw_spice: vec![],
        includes: vec![],
        libs: vec![],
        osdi_loads: vec![],
        verilog_blocks: vec![],
    }
}

fn tb(analyses: Vec<Analysis>) -> Testbench {
    Testbench { prints: vec![],
        dut: "dialects".into(),
        stimulus: vec![],
        analyses,
        options: SimOptions::default(),
        saves: vec![],
        measures: vec![],
        temperature: None,
        nominal_temperature: None,
        initial_conditions: vec![],
        node_sets: vec![],
        step_params: vec![],
        extra_lines: vec![],
    }
}

/// v(mid) = 0.5 (a parameter expression over a parametrised subcircuit),
/// v(b) = 2 v(mid) + 1 = 2, v(lp) = DC gain of 1/(1 + 1e-3 s) = 1.
fn circuit(analyses: Vec<Analysis>) -> CircuitIR {
    let mut load = sub("load");
    load.ports = vec![Port { name: "n".into(), direction: PortDirection::InOut }];
    load.parameters = vec![ParamDef { name: "r".into(), default: Some("1k".into()) }];
    load.components = vec![Component::Resistor {
        name: "1".into(),
        n1: "n".into(),
        n2: "0".into(),
        value: IrValue::Expression { expr: "r".into() },
        model: None, params: vec![],
    }];

    let mut top = sub("dialects");
    top.parameters = vec![ParamDef { name: "rv".into(), default: Some("2k".into()) }];
    top.components = vec![
        Component::VoltageSource {
            name: "1".into(),
            np: "in".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 1.0 },
            ac_magnitude: Some(1.0),
            ac_phase: None,
            waveform: None,
        },
        Component::Resistor {
            name: "1".into(),
            n1: "in".into(),
            n2: "mid".into(),
            value: IrValue::Expression { expr: "rv".into() },
            model: None, params: vec![],
        },
        Component::BehavioralVoltage {
            name: "1".into(),
            np: "b".into(),
            nm: "0".into(),
            expression: "2*v(mid)+1".into(),
        },
        Component::BehavioralVoltage {
            name: "2".into(),
            np: "lp".into(),
            nm: "0".into(),
            expression: "Laplace(v(in), 1/(1+s*1e-3))".into(),
        },
    ];
    top.instances = vec![Instance {
        name: "1".into(),
        subcircuit: "load".into(),
        port_mapping: vec!["mid".into()],
        parameters: vec![("r".into(), "2k".into())],
    }];
    CircuitIR { raw_dialect: None, globals: vec![], functions: vec![], top, testbench: Some(tb(analyses)), subcircuit_defs: vec![load], model_libraries: vec![] }
}

#[test]
fn every_dialect_dumps() {
    // VACASK has no behavioural-source language; the rest is common ground.
    let mut ir = circuit(vec![Analysis::Op]);
    ir.top.components.retain(|c| !matches!(c, Component::BehavioralVoltage { .. }));
    for d in [Dialect::Ngspice, Dialect::Hspice, Dialect::Ltspice, Dialect::Vacask, Dialect::Spectre, Dialect::Espice] {
        let text = netlist(&ir, d).unwrap_or_else(|e| panic!("{d:?}: {e}"));
        assert!(text.contains("mid"), "{d:?}:\n{text}");
    }
}

#[test]
fn hspice_card_forms() {
    let mut ir = circuit(vec![
        Analysis::Transient { step: 1e-9, stop: 1e-6, start: Some(1e-7), max_step: Some(5e-9), uic: true },
        Analysis::Noise {
            output: "mid".into(),
            reference: "0".into(),
            source: "V1".into(),
            variation: "dec".into(),
            points: 2,
            start: 1e3,
            stop: 1e5,
            points_per_summary: Some(1),
        },
    ]);
    ir.top.libs = vec![("corners.lib".into(), "tt".into())];
    let n = netlist(&ir, Dialect::Hspice).unwrap();
    for want in [
        ".lib 'corners.lib' tt",
        ".subckt load n r=1k",
        "R1 n 0 'r'",
        "R1 in mid 'rv'",
        "E1 b 0 vol='2*v(mid)+1'",
        "EB2 lp 0 LAPLACE in 0 1e0 / 1e0 1e-3",
        ".option delmax=5n",
        ".tran 1n 1u START=100n UIC",
        ".ac dec 2 1k 100k\n.noise V(mid) V1 1",
    ] {
        assert!(n.contains(want), "missing `{want}` in:\n{n}");
    }
    // ESPice refuses these in an HSPICE deck (invalid/hspice_brace_*.sp).
    assert!(!n.contains('{') && !n.contains("PARAMS:") && !n.contains("\nB"), "{n}");

    let step = StepParam { param: "rv".into(), start: 1e3, stop: 3e3, step: 1e3, sweep_type: None };
    let mut stepped = circuit(vec![Analysis::Op]);
    stepped.testbench.as_mut().unwrap().step_params = vec![step];
    assert!(netlist(&stepped, Dialect::Hspice).is_err());
    let xspice = Component::Xspice { name: "1".into(), connections: vec!["a".into()], model: "m".into() };
    let mut with_a = circuit(vec![Analysis::Op]);
    with_a.top.components.push(xspice);
    assert!(netlist(&with_a, Dialect::Hspice).is_err());
}

/// Raw SPICE text, at the top or inside a subcircuit, is refused by the
/// non-SPICE dialects rather than dropped, and kept by the SPICE ones.
#[test]
fn raw_spice_is_refused_not_dropped() {
    for at_top in [true, false] {
        let mut ir = circuit(vec![Analysis::Op]);
        let line = "Gp 0 p poly(1) mid 0 0 1m".to_string();
        if at_top { ir.top.raw_spice.push(line.clone()) } else { ir.subcircuit_defs[0].raw_spice.push(line.clone()) }
        for d in [Dialect::Spectre, Dialect::Vacask] {
            let e = netlist(&ir, d).unwrap_err().to_string();
            assert!(e.contains("raw SPICE line"), "{d:?}: {e}");
        }
        assert!(netlist(&ir, Dialect::Ngspice).unwrap().contains(&line));
        // Imported ngspice text reaches ngspice and ESPice only.
        ir.raw_dialect = Some("ngspice".into());
        assert!(netlist(&ir, Dialect::Espice).is_ok());
        for d in [Dialect::Hspice, Dialect::Ltspice] {
            let e = netlist(&ir, d).unwrap_err().to_string();
            assert!(e.contains("ngspice line"), "{d:?}: {e}");
        }
    }
}

/// Spectre deck text that would parse but mean something else.
#[test]
fn spectre_keeps_spice_meaning() {
    let mut ir = circuit(vec![Analysis::Transient { step: 1e-9, stop: 1e-6, start: None, max_step: None, uic: true }]);
    ir.top.components.retain(|c| !matches!(c, Component::BehavioralVoltage { .. }));
    ir.top.models = vec![ModelDef { name: "qn".into(), kind: "NPN".into(), parameters: vec![("IKF".into(), "30M".into())] }];
    ir.testbench.as_mut().unwrap().saves = vec!["i(V1)".into()];
    let n = netlist(&ir, Dialect::Spectre).unwrap();
    // SPICE 30M is milli; Spectre's M is mega, its m milli.
    assert!(n.contains("model qn bjt type=npn ikf=30m"), "{n}");
    assert!(n.contains("skipdc=yes"), "uic: {n}");
    assert!(n.contains("save v1:p"), "current save: {n}");

    let tb = ir.testbench.as_mut().unwrap();
    let sweep = |s: &str| DcSweep { source: s.into(), start: 0.0, stop: 1.0, step: 0.1 };
    tb.analyses = vec![Analysis::Dc { sweeps: vec![sweep("V1"), sweep("temp")] }];
    assert!(netlist(&ir, Dialect::Spectre).is_err(), "a nested .dc must not lose a sweep");
    ir.top.models[0].kind = "VDMOS".into();
    ir.testbench.as_mut().unwrap().analyses = vec![Analysis::Op];
    assert!(netlist(&ir, Dialect::Spectre).unwrap_err().to_string().contains("no Spectre master"));
}

/// An empty option value is a flag: bare in SPICE, refused by Spectre, whose
/// `options` statement takes `name=value` only.
#[test]
fn option_flags() {
    let mut ir = circuit(vec![Analysis::Op]);
    ir.top.components.retain(|c| !matches!(c, Component::BehavioralVoltage { .. }));
    let tb = ir.testbench.as_mut().unwrap();
    tb.options.portable = vec![("noacct".into(), String::new()), ("reltol".into(), "1e-4".into())];
    assert!(netlist(&ir, Dialect::Ngspice).unwrap().contains(".options noacct reltol=1e-4"));
    let e = netlist(&ir, Dialect::Spectre).unwrap_err().to_string();
    assert!(e.contains("option flag 'noacct'"), "{e}");
}

// ── ESPice as the reader ──

fn espice() -> Option<String> {
    let bin = std::env::var("ESPICE").unwrap_or_else(|_| "espice".into());
    let ok = std::process::Command::new(&bin).arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    ok.then_some(bin)
}

fn op_point(bin: &str, deck: &str, tokenizer: &str) -> spicerack::result::RawData {
    let dir = tempfile::TempDir::new().unwrap();
    let (path, raw) = (dir.path().join("deck.sp"), dir.path().join("out.raw"));
    std::fs::write(&path, deck).unwrap();
    let out = std::process::Command::new(bin)
        .arg(format!("--tokenizer={tokenizer}"))
        .arg(&path)
        .arg("--rawfile")
        .arg(&raw)
        .arg("--format=binary")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "espice --tokenizer={tokenizer} refused the deck:\n{}\n{}\n{deck}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    spicerack::rawfile::parse_raw(&std::fs::read(&raw).unwrap()).unwrap()
}

fn node(raw: &spicerack::result::RawData, name: &str) -> f64 {
    let wrapped = format!("v({name})");
    let i = raw
        .variables
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case(name) || v.name.eq_ignore_ascii_case(&wrapped))
        .unwrap_or_else(|| panic!("no {name} in {:?}", raw.variables.iter().map(|v| &v.name).collect::<Vec<_>>()));
    raw.real_data[i][0]
}

#[test]
fn espice_reads_each_dialect_to_the_same_op() {
    let Some(bin) = espice() else {
        eprintln!("espice not on PATH — skipping");
        return;
    };
    let ir = circuit(vec![Analysis::Op]);
    let gens = [
        (Dialect::Ngspice, "ngspice"),
        (Dialect::Hspice, "hspice"),
        (Dialect::Espice, "ngspice"),
        (Dialect::Spectre, "spectre"),
    ];
    for (g, tokenizer) in gens {
        let deck = netlist(&ir, g).unwrap();
        let raw = op_point(&bin, &deck, tokenizer);
        for (name, want) in [("mid", 0.5), ("b", 2.0), ("lp", 1.0)] {
            let got = node(&raw, name);
            assert!((got - want).abs() < 1e-6, "{:?}: v({name}) = {got}, want {want}\n{deck}", g);
        }
    }
}

/// `^` is power in ngspice but XOR in LTspice. An expression imported from
/// LTspice carries that XOR meaning; emitting it as ngspice (power) would
/// silently change the deck, so codegen refuses. Emitting it back to
/// LTspice, where it came from, is fine.
#[test]
fn caret_meaning_refused_across_dialects() {
    let mut top = sub("dialects");
    top.components = vec![Component::Resistor {
        name: "1".into(),
        n1: "a".into(),
        n2: "0".into(),
        value: IrValue::Expression { expr: "a^b".into() },
        model: None, params: vec![],
    }];
    let ir = CircuitIR { raw_dialect: Some("ltspice".into()), top, ..Default::default() };

    let err = netlist(&ir, Dialect::Ngspice).unwrap_err();
    assert!(err.to_string().contains("a^b"), "{err}");

    netlist(&ir, Dialect::Ltspice).expect("same dialect's own meaning is fine");
}
