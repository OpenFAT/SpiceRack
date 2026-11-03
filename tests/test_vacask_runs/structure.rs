//! Deck structure on VACASK: subcircuits and instances, `.step` parameters,
//! initial conditions, and several analyses in one deck in order.
//!
//! Cites: real VACASK.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

#[test]
fn subcircuits_and_their_instances_run() {
    let mut divider = empty_sub("divider");
    divider.ports = vec![
        Port {
            name: "in".into(),
            direction: PortDirection::Input,
        },
        Port {
            name: "out".into(),
            direction: PortDirection::Output,
        },
    ];
    divider.parameters = vec![
        ParamDef {
            name: "rtop".into(),
            default: Some("1k".into()),
        },
        ParamDef {
            name: "rbot".into(),
            default: Some("1k".into()),
        },
    ];
    divider.components = vec![
        Component::Resistor {
            name: "1".into(),
            n1: "in".into(),
            n2: "out".into(),
            value: IrValue::Expression {
                expr: "rtop".into(),
            },
            model: None,
            params: vec![],
        },
        Component::Resistor {
            name: "2".into(),
            n1: "out".into(),
            n2: "0".into(),
            value: IrValue::Expression {
                expr: "rbot".into(),
            },
            model: None,
            params: vec![],
        },
    ];

    let mut top = empty_sub("with_subckt");
    top.instances = vec![Instance {
        name: "1".into(),
        subcircuit: "divider".into(),
        port_mapping: vec!["vdd".into(), "mid".into()],
        parameters: vec![("rtop".into(), "1k".into()), ("rbot".into(), "3k".into())],
        ..Default::default()
    }];
    let mut tb = empty_tb(vec![Analysis::Op]);
    tb.stimulus = vec![vsrc("dd", "vdd", "0", 4.0)];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        subcircuit_defs: vec![divider],
        ..Default::default()
    };
    let raw = run(&ir);
    // 4 V * 3k/(1k+3k) = 3 V.
    close(col(&raw, "mid")[0], 3.0, 1e-9);
}

#[test]
fn step_params_sweep_a_netlist_variable() {
    // VACASK's capability table claimed `step_params: false`. A `var` declared
    // in the control block IS visible to the netlist, so it is not.
    let mut top = empty_sub("stepped");
    top.parameters = vec![ParamDef {
        name: "rtop".into(),
        default: Some("1k".into()),
    }];
    top.components = vec![
        Component::Resistor {
            name: "1".into(),
            n1: "vdd".into(),
            n2: "out".into(),
            value: IrValue::Expression {
                expr: "rtop".into(),
            },
            model: None,
            params: vec![],
        },
        r("2", "out", "0", 1000.0),
    ];
    let mut tb = empty_tb(vec![Analysis::Op]);
    tb.stimulus = vec![vsrc("dd", "vdd", "0", 2.0)];
    tb.step_params = vec![StepParam {
        param: "rtop".into(),
        start: 1000.0,
        stop: 3000.0,
        step: 1000.0,
        sweep_type: None,
    }];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let raw = run(&ir);
    let out = col(&raw, "out");
    assert_eq!(out.len(), 3, "{:?}", out);
    close(out[0], 2.0 * 1000.0 / 2000.0, 1e-9);
    close(out[1], 2.0 * 1000.0 / 3000.0, 1e-9);
    close(out[2], 2.0 * 1000.0 / 4000.0, 1e-9);
}

#[test]
fn initial_conditions_reach_the_analysis() {
    let mut top = empty_sub("ic");
    top.components = vec![r("1", "vin", "out", 1000.0), c("1", "out", "0", 1e-6)];
    let mut tb = empty_tb(vec![Analysis::Transient {
        step: 2.5e-5,
        stop: 1e-3,
        start: None,
        max_step: Some(2.5e-5),
        uic: true,
    }]);
    tb.stimulus = vec![vsrc("1", "vin", "0", 1.0)];
    tb.initial_conditions = vec![("out".into(), 0.5)];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let netlist = emit(&ir, Dialect::Vacask).unwrap();
    assert!(netlist.contains("ic=[\"out\"; 5e-1]"), "{}", netlist);
    let raw = run(&ir);
    close(col(&raw, "out")[0], 0.5, 1e-9);
}

#[test]
fn run_all_returns_every_analysis_in_deck_order() {
    // VACASK writes one raw file per analysis; run_all reads each, in the
    // order the deck declares them. Oracle: the divider's closed forms,
    // out = vdd/2 in every analysis (1.5 V at the 3 V rail).
    let ir = rc_ir(vec![
        Analysis::Op,
        Analysis::Dc {
            sweeps: vec![DcSweep {
                source: "Vdd".into(),
                start: 0.0,
                stop: 2.0,
                step: 1.0,
            }],
        },
        Analysis::Transient {
            step: 1e-4,
            stop: 1e-3,
            start: None,
            max_step: Some(1e-4),
            uic: false,
        },
    ]);
    let netlist = emit(&ir, Dialect::Vacask).expect("emit vacask deck");
    assert!(
        have_vacask(),
        "vacask not on PATH: run the tests in `nix develop .#testing`"
    );
    let plots = Dialect::Vacask
        .run_all(&netlist)
        .unwrap_or_else(|e| panic!("{e}\n{netlist}"));
    assert_eq!(
        plots.len(),
        3,
        "{:?}",
        plots.iter().map(|p| &p.plot_name).collect::<Vec<_>>()
    );
    close(col(&plots[0], "out")[0], 1.5, 1e-9);
    assert_eq!(col(&plots[1], "vsweep"), vec![0.0, 1.0, 2.0]);
    for (v, want) in col(&plots[1], "out").into_iter().zip([0.0, 0.5, 1.0]) {
        assert!((v - want).abs() <= 1e-9, "dc out {v}, want {want}");
    }
    let time = col(&plots[2], "time");
    close(*time.last().unwrap(), 1e-3, 1e-9);
    assert!(
        col(&plots[2], "out")
            .iter()
            .all(|v| (v - 1.5).abs() <= 1e-9)
    );
    // `run` is the first analysis's plot, as for every backend.
    assert_eq!(run(&ir).plot_name, plots[0].plot_name);
    assert_eq!(col(&run(&ir), "out").len(), 1);
}
