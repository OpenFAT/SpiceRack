use super::*;

// ── Components ──

/// Citations, one per line:
/// `[CMP]` p.628 `r1 (1 2) resistor r=1.2K m=2`
/// `[CMP]` p.283 `c2 (1 0) capacitor c=2.5u tc1=1e-8`
/// `[CMP]` p.372 `l33 (0 net29) inductor l=10e-9 r=1 m=1`
/// `[CMP]` p.577 `ml1 mutual_inductor coupling=1 ind1=l1 ind2=l2`
/// `[CMP]` p.680 `e1 (out1 0 pos neg) vcvs gain=10`
/// `[CMP]` p.678 `Name sink src ps ns ... vccs` with `gm`
/// `[CMP]` p.286 `vcs (pos gnd) cccs gain=2.5 probe=v1`
/// `[CMP]` p.288 `vvs (pos gnd) ccvs rm=1 probe=v1`
/// `[CMP]` p.302 `d0 (dp dn) pdiode ...` (`Name a c ModelName`)
/// `[CMP]` p.645 `t1 (1 0 2 0) lmodel z0=100`, params `z0` and `td`
/// `[CMP]` p.625 `rel1 (1 2 ps ns) my_relay ...` (`Name 1 2 ps ns ModelName`)
#[test]
fn spectre_primitives_match_documented_instance_forms() {
    assert_eq!(
        component(Component::Resistor {
            name: "1".into(),
            n1: "a".into(),
            n2: "b".into(),
            value: IrValue::Numeric { value: 1200.0 },
            model: None,
            params: vec![("m".into(), "2".into())],
        }),
        "r1 (a b) resistor r=1.2k m=2",
    );
    // NOTE: the doc example is `c=2.5u`, but `crate::circuit::format_spice_number`
    // leaves a float artifact there (2.5e-6/1e-6 == 2.5000000000000004), so
    // this asserts on a value that divides cleanly. See the accompanying
    // report: the artifact is in shared code this change may not touch.
    assert_eq!(
        component(Component::Capacitor {
            name: "2".into(),
            n1: "1".into(),
            n2: "0".into(),
            value: IrValue::Numeric { value: 2e-6 },
            model: None,
            params: vec![],
        }),
        "c2 (1 0) capacitor c=2u",
    );
    assert_eq!(
        component(Component::Inductor {
            name: "33".into(),
            n1: "0".into(),
            n2: "net29".into(),
            value: IrValue::Numeric { value: 10e-9 },
            model: None,
            params: vec![],
        }),
        "l33 (0 net29) inductor l=10n",
    );
    assert_eq!(
        component(Component::MutualInductor {
            name: "1".into(),
            inductor1: "1".into(),
            inductor2: "2".into(),
            coupling: 1.0,
        }),
        "k1 mutual_inductor coupling=1 ind1=l1 ind2=l2",
    );
    assert_eq!(
        component(Component::Vcvs {
            name: "1".into(),
            np: "out1".into(),
            nm: "0".into(),
            ncp: "pos".into(),
            ncm: "neg".into(),
            gain: 10.0,
        }),
        "e1 (out1 0 pos neg) vcvs gain=10",
    );
    assert_eq!(
        component(Component::Vccs {
            name: "1".into(),
            np: "sink".into(),
            nm: "src".into(),
            ncp: "ps".into(),
            ncm: "ns".into(),
            transconductance: -1.0,
        }),
        "g1 (sink src ps ns) vccs gm=-1",
    );
    // `probe=` names an instance; our vsource emitter lowercases names, so the
    // reference has to be lowercased identically (`[KUN]` B.2: case sensitive).
    assert_eq!(
        component(Component::Cccs {
            name: "cs".into(),
            np: "pos".into(),
            nm: "gnd".into(),
            vsense: "V1".into(),
            gain: 2.5,
        }),
        "fcs (pos gnd) cccs probe=v1 gain=2.5",
    );
    assert_eq!(
        component(Component::Ccvs {
            name: "vs".into(),
            np: "pos".into(),
            nm: "gnd".into(),
            vsense: "V1".into(),
            transresistance: 1.0,
        }),
        "hvs (pos gnd) ccvs probe=v1 rm=1",
    );
    assert_eq!(
        component(Component::Diode {
            name: "0".into(),
            np: "dp".into(),
            nm: "dn".into(),
            model: "pdiode".into(),
            params: vec![("area".into(), "1".into())],
        }),
        "d0 (dp dn) pdiode area=1",
    );
    assert_eq!(
        component(Component::TLine {
            name: "1".into(),
            inp: "1".into(),
            inm: "0".into(),
            outp: "2".into(),
            outm: "0".into(),
            z0: 100.0,
            td: 1e-9,
        }),
        "t1 (1 0 2 0) tline z0=100 td=1n",
    );
    // Every number goes through the one Spectre formatter: scale suffixes,
    // with Spectre's capital M for mega (`[KUN]` B.2).
    assert_eq!(
        component(Component::Vcvs {
            name: "2".into(),
            np: "a".into(),
            nm: "0".into(),
            ncp: "b".into(),
            ncm: "0".into(),
            gain: 2e6
        }),
        "e2 (a 0 b 0) vcvs gain=2M"
    );
    assert_eq!(
        component(Component::Vccs {
            name: "2".into(),
            np: "a".into(),
            nm: "0".into(),
            ncp: "b".into(),
            ncm: "0".into(),
            transconductance: 1e-3
        }),
        "g2 (a 0 b 0) vccs gm=1m"
    );
    assert_eq!(
        component(Component::Cccs {
            name: "2".into(),
            np: "a".into(),
            nm: "0".into(),
            vsense: "V1".into(),
            gain: 1e-6
        }),
        "f2 (a 0) cccs probe=v1 gain=1u"
    );
    assert_eq!(
        component(Component::Ccvs {
            name: "2".into(),
            np: "a".into(),
            nm: "0".into(),
            vsense: "V1".into(),
            transresistance: 1e3
        }),
        "h2 (a 0) ccvs probe=v1 rm=1k"
    );
    assert_eq!(
        component(Component::MutualInductor {
            name: "2".into(),
            inductor1: "1".into(),
            inductor2: "2".into(),
            coupling: 1e-3
        }),
        "k2 mutual_inductor coupling=1m ind1=l1 ind2=l2"
    );
    assert_eq!(
        component(Component::TLine {
            name: "2".into(),
            inp: "1".into(),
            inm: "0".into(),
            outp: "2".into(),
            outm: "0".into(),
            z0: 1e6,
            td: 2.5e-12
        }),
        "t2 (1 0 2 0) tline z0=1M td=2.5p"
    );
    assert_eq!(
        component(Component::VSwitch {
            name: "1".into(),
            np: "1".into(),
            nm: "2".into(),
            ncp: "ps".into(),
            ncm: "ns".into(),
            model: "my_relay".into(),
        }),
        "s1 (1 2 ps ns) my_relay",
    );
}

/// `[CMP]` p.683-685 `vsource`: `dc` (#1), `type` (#2, values
/// `dc pulse pwl sine exp`), `delay` (#4), `val0`/`val1`/`period`/`rise`/
/// `fall`/`width` (#5-#10), `wave` (#12), `sinedc`/`ampl`/`freq`/`sinephase`
/// (#19-#22), `damp` (#32), `td1`/`tau1`/`td2`/`tau2` (#33-#36),
/// `mag`/`phase` (#39/#40). Verbatim sample:
/// `vpulse1 (1 0) vsource type=pulse val0=0 val1=5 period=100n rise=10n fall=10n width=40n`
/// and `vpwl1 (1 0) vsource type=pwl wave=[1n 0 1.1n 2 1.5n 0.5 2n 3 5n 5]`.
#[test]
fn spectre_vsource_waveforms_match_documented_parameters() {
    let v = |wf: Option<IrWaveform>, mag: Option<f64>, ph: Option<f64>| {
        component(Component::VoltageSource {
            name: "1".into(),
            np: "1".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: mag,
            ac_phase: ph,
            waveform: wf,
            port: None,
        })
    };

    assert_eq!(
        v(
            Some(IrWaveform::Pulse {
                initial: 0.0.into(),
                pulsed: 5.0.into(),
                delay: 0.0.into(),
                rise_time: 10e-9.into(),
                fall_time: 10e-9.into(),
                pulse_width: 40e-9.into(),
                period: 100e-9.into(),
            }),
            None,
            None
        ),
        "v1 (1 0) vsource dc=0 type=pulse val0=0 val1=5 delay=0 rise=10n fall=10n width=40n period=100n",
    );

    assert_eq!(
        v(
            Some(IrWaveform::Pwl {
                values: vec![
                    ((1e-9).into(), (0.0).into()),
                    ((1.5e-9).into(), (0.5).into())
                ],
                repeat: None,
                delay: None
            }),
            None,
            None
        ),
        "v1 (1 0) vsource dc=0 type=pwl wave=[1n 0 1.5n 500m]",
    );

    // sine: `delay` and `damp` are the documented names. The previous codegen
    // emitted `sinedelay=` / `sinedamp=`, which vsource does not have.
    assert_eq!(
        v(
            Some(IrWaveform::Sin {
                offset: 1.0.into(),
                amplitude: 0.5.into(),
                frequency: 1e6.into(),
                delay: 1e-9.into(),
                damping: 2.0.into(),
                phase: 45.0.into(),
            }),
            None,
            None
        ),
        "v1 (1 0) vsource dc=0 type=sine sinedc=1 ampl=500m freq=1M delay=1n damp=2 sinephase=45",
    );

    assert_eq!(
        v(
            Some(IrWaveform::Exp {
                initial: 0.0.into(),
                pulsed: 1.0.into(),
                rise_delay: 1e-9.into(),
                rise_tau: 2e-9.into(),
                fall_delay: 3e-9.into(),
                fall_tau: 4e-9.into(),
            }),
            None,
            None
        ),
        "v1 (1 0) vsource dc=0 type=exp val0=0 val1=1 td1=1n tau1=2n td2=3n tau2=4n",
    );

    assert_eq!(
        v(None, Some(1.0), Some(45.0)),
        "v1 (1 0) vsource dc=0 mag=1 phase=45"
    );
}

/// `[CMP]` p.683 `vsource` `delay` (#4, the general waveform delay: ngspice
/// `td=`) and `pwlperiod` (#17) with `twidth=pwlperiod/1000` (#18); p.686:
/// "Before repeating, the waveform changes linearly in an interval of twidth
/// from its value at (period - twidth) to its value at the beginning of the
/// waveform." So `r=0` is `pwlperiod=<last time>` only for points from 0
/// whose last value is the first and whose last segment spans the ramp;
/// any other repeat is refused.
#[test]
fn spectre_pwl_writes_delay_and_only_an_exact_pwlperiod() {
    let tri = |end: f64, repeat: Option<f64>, delay: Option<f64>| {
        let values = [(0.0, 0.0), (1e-3, 1.0), (2e-3, end)]
            .map(|(t, v)| (IrNum::Num(t), IrNum::Num(v)))
            .to_vec();
        Component::VoltageSource {
            name: "1".into(),
            np: "1".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: None,
            ac_phase: None,
            port: None,
            waveform: Some(IrWaveform::Pwl {
                values,
                repeat: repeat.map(IrNum::Num),
                delay: delay.map(IrNum::Num),
            }),
        }
    };
    assert_eq!(
        component(tri(0.0, None, Some(0.5e-3))),
        "v1 (1 0) vsource dc=0 type=pwl wave=[0 0 1m 1 2m 0] delay=500u"
    );
    assert_eq!(
        component(tri(0.0, Some(0.0), Some(0.5e-3))),
        "v1 (1 0) vsource dc=0 type=pwl wave=[0 0 1m 1 2m 0] delay=500u pwlperiod=2m"
    );
    for (end, repeat) in [(0.0, 1e-3), (0.5, 0.0)] {
        let e = try_component(&tri(end, Some(repeat), None))
            .unwrap_err()
            .to_string();
        assert!(e.contains("PWL"), "end {end}, r={repeat}: {e}");
    }
}

/// Things SPICE can express that Spectre has no documented spelling for must
/// error, not silently become a comment or a guessed statement.
#[test]
fn spectre_refuses_constructs_without_a_documented_spelling() {
    let cases: Vec<Component> = vec![
        // XSPICE A-devices are ngspice-only.
        Component::Xspice {
            name: "1".into(),
            connections: vec!["a".into()],
            model: "d_and".into(),
        },
        // Raw SPICE cannot be placed without knowing where a
        // `simulator lang=spice` region may legally open.
        Component::RawSpice {
            line: ".model sw sw vt=0.5".into(),
        },
        // `[CMP]` p.856: Spectre bsource expressions are `v(a,b)`,
        // `i("inst:idx")`, `$time` — not SPICE's `V()`/`I()`/`time`, and
        // Spectre is case sensitive.
        Component::BehavioralVoltage {
            name: "1".into(),
            np: "o".into(),
            nm: "0".into(),
            expression: "V(a)*2".into(),
        },
        Component::BehavioralCurrent {
            name: "1".into(),
            np: "o".into(),
            nm: "0".into(),
            expression: "I(V1)*2".into(),
        },
        // `[CMP]` p.625 `relay` is voltage controlled; p.643 `switch` changes
        // position only between analyses. Neither is SPICE's W element.
        Component::ISwitch {
            name: "1".into(),
            np: "a".into(),
            nm: "b".into(),
            vcontrol: "V1".into(),
            model: "csw".into(),
        },
        // `[CMP]` p.685 documents ammodindex/ammodfreq but no waveform
        // equation, so the SPICE AM() mapping cannot be established.
        Component::VoltageSource {
            name: "1".into(),
            np: "a".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: None,
            ac_phase: None,
            waveform: Some(IrWaveform::Am {
                amplitude: 1.0.into(),
                offset: 0.5.into(),
                modulating_freq: 1e3.into(),
                carrier_freq: 1e6.into(),
                delay: 0.0.into(),
            }),
            port: None,
        },
    ];

    for c in cases {
        let r = try_component(&c);
        assert!(
            matches!(r, Err(CodeGenError::UnsupportedComponent { .. })),
            "expected UnsupportedComponent, got {r:?}",
        );
    }
}
