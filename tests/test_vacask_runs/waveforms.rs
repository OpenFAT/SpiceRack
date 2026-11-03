use super::*;

// ── Waveforms that DO translate ──

#[test]
fn a_sine_source_runs_with_delay_and_damping() {
    let mut top = empty_sub("sine");
    top.components = vec![r("1", "vin", "0", 1000.0)];
    let mut tb = empty_tb(vec![Analysis::Transient {
        step: 1e-5,
        stop: 2e-3,
        start: None,
        max_step: Some(1e-5),
        uic: false,
    }]);
    tb.stimulus = vec![Component::VoltageSource {
        name: "1".into(),
        np: "vin".into(),
        nm: "0".into(),
        value: IrValue::Numeric { value: 0.0 },
        ac_magnitude: None,
        ac_phase: None,
        waveform: Some(IrWaveform::Sin {
            offset: 0.5.into(),
            amplitude: 2.0.into(),
            frequency: 1e3.into(),
            delay: 0.0.into(),
            damping: 0.0.into(),
            phase: 0.0.into(),
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
    let v = col(&raw, "vin");
    // A quarter period into a 1 kHz sine: offset + amplitude.
    let (i, _) = t
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1 - 2.5e-4)
                .abs()
                .partial_cmp(&(b.1 - 2.5e-4).abs())
                .unwrap()
        })
        .unwrap();
    close(v[i], 2.5, 5e-3);
    close(v[0], 0.5, 1e-6);
}

#[test]
fn a_pulse_train_runs() {
    let mut top = empty_sub("pulse");
    top.components = vec![r("1", "vin", "0", 1000.0)];
    let mut tb = empty_tb(vec![Analysis::Transient {
        step: 1e-6,
        stop: 5e-4,
        start: None,
        max_step: Some(1e-6),
        uic: false,
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
            delay: 1e-4.into(),
            rise_time: 1e-6.into(),
            fall_time: 1e-6.into(),
            pulse_width: 1e-4.into(),
            period: 2e-4.into(),
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
    let v = col(&raw, "vin");
    let at = |want: f64| {
        let (i, _) = t
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - want).abs().partial_cmp(&(b.1 - want).abs()).unwrap())
            .unwrap();
        v[i]
    };
    close(at(5e-5), 0.0, 1e-6); // before the delay
    close(at(1.5e-4), 1.0, 1e-3); // mid-pulse
    close(at(2.5e-4), 0.0, 1e-3); // after the fall
    close(at(3.5e-4), 1.0, 1e-3); // second period
}
