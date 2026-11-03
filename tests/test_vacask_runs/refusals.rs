//! What VACASK cannot do (measures, PWL, behavioural sources, switches,
//! transmission lines, XSPICE, ...) -> a codegen error, never a broken deck.
//!
//! Cites: real VACASK, for what it refuses or runs wrong.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

#[test]
fn refusals_are_errors_not_broken_decks() {
    let refuses = |ir: &CircuitIR| emit(ir, Dialect::Vacask).is_err();
    let with = |comp: Component| {
        let mut top = empty_sub("c");
        top.components = vec![comp];
        CircuitIR {
            top,
            ..Default::default()
        }
    };

    // No measurement statement at all: `measure` is "Command not found".
    let mut tb = empty_tb(vec![Analysis::Op]);
    tb.measures = vec!["tran vmax MAX V(out)".into()];
    let ir = CircuitIR {
        top: empty_sub("m"),
        testbench: Some(tb),
        ..Default::default()
    };
    assert!(refuses(&ir), "measures must refuse");

    // PWL parses and then returns an identically-zero transient.
    let mut top = empty_sub("pwl");
    top.components = vec![Component::VoltageSource {
        name: "1".into(),
        np: "a".into(),
        nm: "0".into(),
        value: IrValue::Numeric { value: 0.0 },
        ac_magnitude: None,
        ac_phase: None,
        waveform: Some(IrWaveform::Pwl {
            values: vec![((0.0).into(), (0.0).into()), ((1e-3).into(), (1.0).into())],
            repeat: None,
            delay: None,
        }),
        port: None,
    }];
    let ir = CircuitIR {
        top,
        ..Default::default()
    };
    assert!(refuses(&ir), "PWL must refuse");

    // No behavioural sources, switches, transmission lines or XSPICE.
    for comp in [
        Component::BehavioralVoltage {
            name: "1".into(),
            np: "a".into(),
            nm: "0".into(),
            expression: "v(b)*2".into(),
        },
        Component::VSwitch {
            name: "1".into(),
            np: "a".into(),
            nm: "0".into(),
            ncp: "c".into(),
            ncm: "0".into(),
            model: "sw".into(),
        },
        Component::TLine {
            name: "1".into(),
            inp: "a".into(),
            inm: "0".into(),
            outp: "b".into(),
            outm: "0".into(),
            z0: 50.0,
            td: 1e-9,
        },
        Component::Xspice {
            name: "1".into(),
            connections: vec!["a".into()],
            model: "d_and".into(),
        },
    ] {
        assert!(refuses(&with(comp.clone())), "{:?} must refuse", comp);
    }

    // A SIN phase is accepted by VACASK and then ignored.
    let sin = Component::VoltageSource {
        name: "1".into(),
        np: "a".into(),
        nm: "0".into(),
        value: IrValue::Numeric { value: 0.0 },
        ac_magnitude: None,
        ac_phase: None,
        waveform: Some(IrWaveform::Sin {
            offset: 0.0.into(),
            amplitude: 1.0.into(),
            frequency: 1e3.into(),
            delay: 0.0.into(),
            damping: 0.0.into(),
            phase: 90.0.into(),
        }),
        port: None,
    };
    assert!(refuses(&with(sin)), "a nonzero SIN phase must refuse");

    // Analyses VACASK does not implement.
    for a in [
        Analysis::PoleZero {
            node1: "a".into(),
            node2: "0".into(),
            node3: "b".into(),
            node4: "0".into(),
            tf_type: PzTransfer::Vol,
            pz_type: PzKind::Pz,
        },
        Analysis::Distortion {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1.0,
                stop: 1e3,
            },
            f2overf1: None,
        },
        Analysis::Sensitivity {
            output: "v(out)".into(),
            ac: None,
        },
    ] {
        assert!(refuses(&rc_ir(vec![a.clone()])), "{:?} must refuse", a);
    }
}
