//! The source waveforms VACASK does run (sine with delay and damping, a
//! pulse train), emitted and checked against their closed forms.
//!
//! Cites: real VACASK; ngspice manual §4.1 for the waveform definitions.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

#[test]
fn a_sine_source_runs_with_delay_and_damping() {
    let raw = run("sine\nR1 vin 0 1k\nV1 vin 0 0 SIN(0.5 2 1k 0 0 0)\n.tran 10u 2m 0 10u\n.end\n");
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
    let raw = run(
        "pulse\nR1 vin 0 1k\nV1 vin 0 0 PULSE(0 1 100u 1u 1u 100u 200u)\n.tran 1u 500u 0 1u\n.end\n",
    );
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
