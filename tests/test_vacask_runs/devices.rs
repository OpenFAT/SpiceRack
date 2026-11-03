//! Each SPICE device VACASK runs through its `sp_*` modules (diode, BJT,
//! level-1 MOSFET, JFET, coupled inductors, controlled sources), emitted and
//! run.
//!
//! Cites: real VACASK; the closed forms named in each test.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

#[test]
fn a_diode_runs_under_the_spice_device_set() {
    let mut top = empty_sub("diode");
    top.components = vec![
        r("1", "vin", "a", 1000.0),
        Component::Diode {
            name: "1".into(),
            np: "a".into(),
            nm: "0".into(),
            model: "dmod".into(),
            params: vec![],
        },
    ];
    top.models = vec![ModelDef {
        name: "dmod".into(),
        kind: "D".into(),
        parameters: vec![("IS".into(), "1e-14".into()), ("N".into(), "1.0".into())],
    }];
    let mut tb = empty_tb(vec![Analysis::Op]);
    tb.stimulus = vec![vsrc("1", "vin", "0", 1.0)];
    tb.temperature = Some(27.0);
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let netlist = emit(&ir, Dialect::Vacask).unwrap();
    assert!(netlist.contains("load \"spice/diode.osdi\""), "{}", netlist);
    assert!(netlist.contains("model dmod sp_diode"), "{}", netlist);
    let raw = run(&ir);
    // 1 V through 1k into a diode: a forward drop somewhere in the 0.4-0.8 V
    // band, and the rest across the resistor.
    let va = col(&raw, "a")[0];
    assert!(
        (0.4..0.8).contains(&va),
        "diode drop {} V is not physical",
        va
    );
}

#[test]
fn a_bjt_runs_and_carries_its_polarity() {
    let mut top = empty_sub("bjt");
    top.components = vec![
        r("b", "vb", "base", 10_000.0),
        r("c", "vcc", "coll", 1000.0),
        Component::Bjt {
            name: "1".into(),
            nc: "coll".into(),
            nb: "base".into(),
            ne: "0".into(),
            extra_nodes: vec![],
            model: "qn".into(),
            params: vec![],
        },
    ];
    top.models = vec![ModelDef {
        name: "qn".into(),
        kind: "NPN".into(),
        parameters: vec![("IS".into(), "1e-15".into()), ("BF".into(), "100".into())],
    }];
    let mut tb = empty_tb(vec![Analysis::Op]);
    tb.stimulus = vec![vsrc("cc", "vcc", "0", 5.0), vsrc("b", "vb", "0", 1.0)];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let netlist = emit(&ir, Dialect::Vacask).unwrap();
    assert!(netlist.contains("model qn sp_bjt (type=1"), "{}", netlist);
    let raw = run(&ir);
    let vbe = col(&raw, "base")[0];
    assert!(
        (0.5..0.9).contains(&vbe),
        "Vbe {} V is not a forward-biased NPN",
        vbe
    );
    // With beta=100 the stage is on, so the collector is pulled well below Vcc.
    assert!(col(&raw, "coll")[0] < 4.9);
}

#[test]
fn a_level_1_mosfet_runs() {
    let mut top = empty_sub("mos");
    top.components = vec![
        r("d", "vdd", "drain", 1000.0),
        Component::Mosfet {
            name: "1".into(),
            nd: "drain".into(),
            ng: "gate".into(),
            ns: "0".into(),
            nb: Some("0".into()),
            extra_nodes: vec![],
            model: "nm".into(),
            params: vec![("W".into(), "10u".into()), ("L".into(), "1u".into())],
        },
    ];
    top.models = vec![ModelDef {
        name: "nm".into(),
        kind: "NMOS".into(),
        parameters: vec![
            ("LEVEL".into(), "1".into()),
            ("VTO".into(), "1.0".into()),
            ("KP".into(), "2e-5".into()),
            ("LAMBDA".into(), "0.05".into()),
        ],
    }];
    let mut tb = empty_tb(vec![Analysis::Op]);
    tb.stimulus = vec![vsrc("dd", "vdd", "0", 5.0), vsrc("g", "gate", "0", 3.0)];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let netlist = emit(&ir, Dialect::Vacask).unwrap();
    assert!(netlist.contains("load \"spice/mos1.osdi\""), "{}", netlist);
    assert!(netlist.contains("model nm sp_mos1 (type=1"), "{}", netlist);
    assert!(netlist.contains("w=10u l=1u"), "{}", netlist);
    let raw = run(&ir);
    let vd = col(&raw, "drain")[0];
    assert!(
        vd < 5.0 && vd > 0.0,
        "drain {} V is not a conducting NMOS",
        vd
    );
}

#[test]
fn a_jfet_runs() {
    let mut top = empty_sub("jfet");
    top.components = vec![
        r("d", "vdd", "drain", 1000.0),
        Component::Jfet {
            name: "1".into(),
            nd: "drain".into(),
            ng: "0".into(),
            ns: "0".into(),
            model: "jn".into(),
            params: vec![],
        },
    ];
    top.models = vec![ModelDef {
        name: "jn".into(),
        kind: "NJF".into(),
        parameters: vec![("VTO".into(), "-2".into()), ("BETA".into(), "1e-3".into())],
    }];
    let mut tb = empty_tb(vec![Analysis::Op]);
    tb.stimulus = vec![vsrc("dd", "vdd", "0", 5.0)];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let netlist = emit(&ir, Dialect::Vacask).unwrap();
    assert!(netlist.contains("load \"spice/jfet1.osdi\""), "{}", netlist);
    let raw = run(&ir);
    assert!(col(&raw, "drain")[0] < 5.0);
}

#[test]
fn an_inductor_and_its_mutual_coupling_run() {
    let mut top = empty_sub("xfmr");
    top.components = vec![
        Component::Inductor {
            name: "1".into(),
            n1: "p".into(),
            n2: "0".into(),
            value: IrValue::Numeric { value: 1e-3 },
            model: None,
            params: vec![],
        },
        Component::Inductor {
            name: "2".into(),
            n1: "s".into(),
            n2: "0".into(),
            value: IrValue::Numeric { value: 4e-3 },
            model: None,
            params: vec![],
        },
        Component::MutualInductor {
            name: "1".into(),
            inductor1: "1".into(),
            inductor2: "2".into(),
            coupling: 0.99,
        },
        r("1", "drive", "p", 1.0),
        r("2", "s", "0", 1e6),
    ];
    let mut tb = empty_tb(vec![Analysis::Ac {
        sweep: FreqSweep {
            variation: Sweep::Dec,
            points: 5,
            start: 1e3,
            stop: 1e5,
        },
    }]);
    tb.stimulus = vec![Component::VoltageSource {
        name: "1".into(),
        np: "drive".into(),
        nm: "0".into(),
        value: IrValue::Numeric { value: 0.0 },
        ac_magnitude: Some(1.0),
        ac_phase: None,
        waveform: None,
        port: None,
    }];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let netlist = emit(&ir, Dialect::Vacask).unwrap();
    assert!(
        netlist.contains("k1 () mutual k=9.9e-1 ind1=\"l1\" ind2=\"l2\""),
        "{}",
        netlist
    );
    let raw = run(&ir);
    // Turns ratio sqrt(4m/1m) = 2, so the secondary rises above the primary.
    let p = cplx(&raw, "p");
    let s = cplx(&raw, "s");
    let ratio = s.last().unwrap().norm() / p.last().unwrap().norm();
    close(ratio, 2.0 * 0.99, 2e-2);
}

#[test]
fn controlled_sources_run() {
    let mut top = empty_sub("ctl");
    top.components = vec![
        r("0", "in", "0", 1000.0),
        Component::Vcvs {
            name: "1".into(),
            np: "e".into(),
            nm: "0".into(),
            ncp: "in".into(),
            ncm: "0".into(),
            gain: 2.0,
        },
        r("1", "e", "0", 1000.0),
        Component::Vccs {
            name: "1".into(),
            np: "0".into(),
            nm: "g".into(),
            ncp: "in".into(),
            ncm: "0".into(),
            transconductance: 2e-3,
        },
        r("2", "g", "0", 1000.0),
        Component::Cccs {
            name: "1".into(),
            np: "0".into(),
            nm: "f".into(),
            vsense: "V1".into(),
            gain: -2.0,
        },
        r("3", "f", "0", 1000.0),
        Component::Ccvs {
            name: "1".into(),
            np: "h".into(),
            nm: "0".into(),
            vsense: "V1".into(),
            transresistance: -2000.0,
        },
        r("4", "h", "0", 1000.0),
    ];
    let mut tb = empty_tb(vec![Analysis::Op]);
    tb.stimulus = vec![vsrc("1", "in", "0", 2.0)];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let raw = run(&ir);
    close(col(&raw, "e")[0], 4.0, 1e-9); // VCVS: 2 V * 2
    close(col(&raw, "g")[0], 2.0 * 2e-3 * 1e3, 1e-9); // VCCS: 2 V * 2 mS * 1k
    // i(V1) = -2 mA, so the CCCS drives -2 * -2 mA = 4 mA into 1k.
    close(col(&raw, "f")[0], 4.0, 1e-9);
    close(col(&raw, "h")[0], 4.0, 1e-9);
}
