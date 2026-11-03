use super::*;

// ── Analyses ──

#[test]
fn op_solves_a_resistive_divider() {
    let raw = run(&rc_ir(vec![Analysis::Op]));
    // 3 V across 1k+1k: out = 1.5 V, i(vdd) = -1.5 mA (VACASK's branch flow
    // points into the source's positive terminal).
    close(col(&raw, "out")[0], 1.5, 1e-9);
    close(col(&raw, "vdd")[0], 3.0, 1e-9);
    close(col(&raw, "vdd:flow(br)")[0], -1.5e-3, 1e-9);
}

#[test]
fn dc_sweeps_a_source_on_spices_own_grid() {
    let ir = rc_ir(vec![Analysis::Dc {
        sweeps: vec![DcSweep {
            source: "Vdd".into(),
            start: 0.0,
            stop: 10.0,
            step: 0.5,
        }],
    }]);
    let raw = run(&ir);
    let sweep = col(&raw, "vsweep");
    // 0..10 by 0.5 is 21 SPICE samples, not 21 intervals.
    assert_eq!(sweep.len(), 21, "sweep grid: {:?}", sweep);
    close(sweep[0], 0.0, 1e-12);
    close(sweep[20], 10.0, 1e-12);
    let out = col(&raw, "out");
    for (i, v) in out.iter().enumerate() {
        close(*v, sweep[i] / 2.0, 1e-9);
    }
}

#[test]
fn dc_sweep_stops_before_passing_the_endpoint() {
    // SPICE walks from START by STEP and never reaches 1.1; VACASK always
    // lands ON `to`, so `to` has to be recomputed or the grids diverge.
    let ir = rc_ir(vec![Analysis::Dc {
        sweeps: vec![DcSweep {
            source: "Vdd".into(),
            start: 0.0,
            stop: 1.1,
            step: 0.2,
        }],
    }]);
    let raw = run(&ir);
    let sweep = col(&raw, "vsweep");
    assert_eq!(sweep.len(), 6, "{:?}", sweep);
    close(sweep[5], 1.0, 1e-12);
    close(sweep[1], 0.2, 1e-12);
}

#[test]
fn two_source_dc_runs_inner_sweep_fastest() {
    let mut top = empty_sub("two_src");
    top.components = vec![r("1", "a", "b", 1000.0), r("2", "b", "0", 1000.0)];
    let mut tb = empty_tb(vec![Analysis::Dc {
        sweeps: vec![
            // SPICE's FIRST source is the fast one.
            DcSweep {
                source: "Va".into(),
                start: 0.0,
                stop: 1.0,
                step: 0.5,
            },
            DcSweep {
                source: "Vb".into(),
                start: 0.0,
                stop: 2.0,
                step: 1.0,
            },
        ],
    }]);
    tb.stimulus = vec![vsrc("a", "a", "0", 0.0), vsrc("b", "c", "0", 0.0)];
    top.components.push(r("3", "c", "0", 1000.0));
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let raw = run(&ir);
    // 3 outer x 3 inner = 9 points, inner fastest.
    let a = col(&raw, "a");
    assert_eq!(a.len(), 9, "{:?}", a);
    close(a[0], 0.0, 1e-12);
    close(a[1], 0.5, 1e-12);
    close(a[2], 1.0, 1e-12);
    close(a[3], 0.0, 1e-12);
    let cc = col(&raw, "c");
    close(cc[0], 0.0, 1e-12);
    close(cc[3], 1.0, 1e-12);
    close(cc[6], 2.0, 1e-12);
}

#[test]
fn ac_finds_the_rc_corner() {
    // 1k || 1k = 500 ohm driving 1 uF: f_3dB = 1/(2*pi*500*1e-6) = 318.31 Hz.
    let mut top = empty_sub("rc_ac");
    top.components = vec![
        r("1", "vdd", "out", 1000.0),
        r("2", "out", "0", 1000.0),
        c("1", "out", "0", 1e-6),
    ];
    let mut tb = empty_tb(vec![Analysis::Ac {
        sweep: FreqSweep {
            variation: Sweep::Dec,
            points: 200,
            start: 1.0,
            stop: 1e5,
        },
    }]);
    tb.stimulus = vec![Component::VoltageSource {
        name: "dd".into(),
        np: "vdd".into(),
        nm: "0".into(),
        value: IrValue::Numeric { value: 0.0 },
        ac_magnitude: Some(1.0),
        ac_phase: Some(0.0),
        waveform: None,
        port: None,
    }];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let raw = run(&ir);
    assert!(raw.is_complex, "AC must come back as a complex plot");

    let f = col(&raw, "frequency");
    let out = cplx(&raw, "out");
    // DC gain is the divider ratio.
    close(out[0].norm(), 0.5, 1e-3);

    // -3 dB relative to the 0.5 plateau.
    let target = 0.5 / std::f64::consts::SQRT_2;
    let (i, _) = out
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1.norm() - target)
                .abs()
                .partial_cmp(&(b.1.norm() - target).abs())
                .unwrap()
        })
        .unwrap();
    let f3db = 1.0 / (2.0 * std::f64::consts::PI * 500.0 * 1e-6);
    close(f[i], f3db, 2e-2);
}

#[test]
fn ac_phase_on_a_source_is_honoured() {
    let mut top = empty_sub("acphase");
    top.components = vec![r("1", "vin", "0", 1000.0)];
    let mut tb = empty_tb(vec![Analysis::Ac {
        sweep: FreqSweep {
            variation: Sweep::Lin,
            points: 2,
            start: 1e3,
            stop: 2e3,
        },
    }]);
    tb.stimulus = vec![Component::VoltageSource {
        name: "1".into(),
        np: "vin".into(),
        nm: "0".into(),
        value: IrValue::Numeric { value: 0.0 },
        ac_magnitude: Some(2.0),
        ac_phase: Some(90.0),
        waveform: None,
        port: None,
    }];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let raw = run(&ir);
    let v = cplx(&raw, "vin");
    close(v[0].norm(), 2.0, 1e-9);
    close(v[0].arg().to_degrees(), 90.0, 1e-6);
}

#[test]
fn tran_follows_the_rc_step_response() {
    // tau = 1k * 1u = 1 ms, driven by a 1 V step at t=0.
    let mut top = empty_sub("rc_tran");
    top.components = vec![r("1", "vin", "out", 1000.0), c("1", "out", "0", 1e-6)];
    let mut tb = empty_tb(vec![Analysis::Transient {
        step: 1e-5,
        stop: 5e-3,
        start: None,
        max_step: Some(1e-5),
        uic: true,
    }]);
    tb.stimulus = vec![Component::VoltageSource {
        name: "1".into(),
        np: "vin".into(),
        nm: "0".into(),
        value: IrValue::Numeric { value: 0.0 },
        ac_magnitude: None,
        ac_phase: None,
        waveform: Some(IrWaveform::Pulse {
            initial: 0.0.into(),
            pulsed: 1.0.into(),
            delay: 0.0.into(),
            rise_time: 1e-9.into(),
            fall_time: 1e-9.into(),
            pulse_width: 1.0.into(),
            period: 0.0.into(),
        }),
        port: None,
    }];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let raw = run(&ir);
    let t = col(&raw, "time");
    let out = col(&raw, "out");
    // 1 - exp(-1) = 0.6321 at one time constant.
    let (i, _) = t
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - 1e-3).abs().partial_cmp(&(b.1 - 1e-3).abs()).unwrap())
        .unwrap();
    close(out[i], 1.0 - (-1.0f64).exp(), 5e-3);
    close(*out.last().unwrap(), 1.0 - (-5.0f64).exp(), 5e-3);
}

#[test]
fn tran_emits_maxstep_so_the_grid_is_spices() {
    // Without `maxstep` VACASK's `step` is only a STARTING step and it strides
    // far past SPICE's output interval.
    let mut top = empty_sub("grid");
    top.components = vec![r("1", "vin", "out", 1000.0), c("1", "out", "0", 1e-6)];
    let mut tb = empty_tb(vec![Analysis::Transient {
        step: 1e-4,
        stop: 1e-2,
        start: None,
        max_step: None,
        uic: false,
    }]);
    tb.stimulus = vec![vsrc("1", "vin", "0", 1.0)];
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let netlist = emit(&ir, Dialect::Vacask).unwrap();
    assert!(netlist.contains("maxstep="), "{}", netlist);
    let raw = run(&ir);
    let t = col(&raw, "time");
    // 10 ms / 100 us = 100 intervals; without maxstep VACASK returns far fewer.
    assert!(
        t.len() >= 100,
        "only {} points — maxstep is not being honoured",
        t.len()
    );
    let dt_max = t.windows(2).map(|w| w[1] - w[0]).fold(0.0f64, f64::max);
    assert!(
        dt_max <= 1e-4 * 1.001,
        "max step {:e} exceeds the requested 1e-4",
        dt_max
    );
}

#[test]
fn noise_matches_a_resistors_thermal_floor() {
    // A single 1k resistor into a unity-gain probe: the output noise density
    // is 4kTR = 1.66e-17 V^2/Hz at 27 C, flat with frequency.
    let mut top = empty_sub("rnoise");
    top.components = vec![r("1", "vin", "out", 1000.0), r("2", "out", "0", 1000.0)];
    let mut tb = empty_tb(vec![Analysis::Noise {
        output: "out".into(),
        reference: "0".into(),
        source: "V1".into(),
        sweep: FreqSweep {
            variation: Sweep::Dec,
            points: 5,
            start: 1e3,
            stop: 1e5,
        },
        points_per_summary: None,
    }]);
    tb.stimulus = vec![Component::VoltageSource {
        name: "1".into(),
        np: "vin".into(),
        nm: "0".into(),
        value: IrValue::Numeric { value: 0.0 },
        ac_magnitude: Some(1.0),
        ac_phase: None,
        waveform: None,
        port: None,
    }];
    tb.temperature = Some(27.0);
    let ir = CircuitIR {
        top,
        testbench: Some(tb),
        ..Default::default()
    };
    let raw = run(&ir);

    let onoise = col(&raw, "onoise");
    // Two 1k resistors in parallel (500 ohm) seen from `out`: 4kT*500.
    let k = 1.380649e-23;
    let want = 4.0 * k * (27.0 + 273.15) * 500.0;
    close(onoise[0], want, 2e-3);
    close(*onoise.last().unwrap(), want, 2e-3);
    // Power gain from the 1 V source through the divider is (1/2)^2.
    close(col(&raw, "gain")[0], 0.25, 1e-6);
}

#[test]
fn dc_transfer_function_runs() {
    // VACASK's `dcxf` reports the transfer function from EVERY source to `out`
    // in one plot, so SPICE's input-source argument is implicit.
    let ir = rc_ir(vec![Analysis::Tf {
        output: "V(out)".into(),
        source: "Vdd".into(),
    }]);
    let raw = run(&ir);
    close(col(&raw, "tf(vdd)")[0], 0.5, 1e-9);
    close(col(&raw, "zin(vdd)")[0], 2000.0, 1e-9);
}
