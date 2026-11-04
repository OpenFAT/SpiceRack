//! Each analysis, emitted for VACASK and run: op, dc sweeps, ac, tran,
//! noise and dc transfer function against their closed forms.
//!
//! Cites: real VACASK; the closed forms named in each test.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

#[test]
fn op_solves_a_resistive_divider() {
    let raw = run(&rc(".op"));
    // 3 V across 1k+1k: out = 1.5 V, i(vdd) = -1.5 mA (VACASK's branch flow
    // points into the source's positive terminal).
    close(col(&raw, "out")[0], 1.5, 1e-9);
    close(col(&raw, "vdd")[0], 3.0, 1e-9);
    close(col(&raw, "vdd:flow(br)")[0], -1.5e-3, 1e-9);
}

#[test]
fn dc_sweeps_a_source_on_spices_own_grid() {
    let raw = run(&rc(".dc Vdd 0 10 0.5"));
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
    let raw = run(&rc(".dc Vdd 0 1.1 0.2"));
    let sweep = col(&raw, "vsweep");
    assert_eq!(sweep.len(), 6, "{:?}", sweep);
    close(sweep[5], 1.0, 1e-12);
    close(sweep[1], 0.2, 1e-12);
}

#[test]
fn two_source_dc_runs_inner_sweep_fastest() {
    // SPICE's FIRST source is the fast one.
    let deck = "two_src\nR1 a b 1k\nR2 b 0 1k\nR3 c 0 1k\nVa a 0 0\nVb c 0 0\n.dc Va 0 1 0.5 Vb 0 2 1\n.end\n";
    let raw = run(deck);
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
    let deck = "rc_ac\nR1 vdd out 1k\nR2 out 0 1k\nC1 out 0 1u\nVdd vdd 0 0 AC 1 0\n.ac dec 200 1 100k\n.end\n";
    let raw = run(deck);
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
    let raw = run("acphase\nR1 vin 0 1k\nV1 vin 0 0 AC 2 90\n.ac lin 2 1k 2k\n.end\n");
    let v = cplx(&raw, "vin");
    close(v[0].norm(), 2.0, 1e-9);
    close(v[0].arg().to_degrees(), 90.0, 1e-6);
}

#[test]
fn tran_follows_the_rc_step_response() {
    // tau = 1k * 1u = 1 ms, driven by a 1 V step at t=0.
    let deck = "rc_tran\nR1 vin out 1k\nC1 out 0 1u\nV1 vin 0 0 PULSE(0 1 0 1n 1n 1 0)\n.tran 10u 5m 0 10u UIC\n.end\n";
    let raw = run(deck);
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
    let deck = "grid\nR1 vin out 1k\nC1 out 0 1u\nV1 vin 0 1\n.tran 100u 10m\n.end\n";
    let netlist = emit(deck).unwrap();
    assert!(netlist.contains("maxstep="), "{}", netlist);
    let raw = run(deck);
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
    let deck = "rnoise\nR1 vin out 1k\nR2 out 0 1k\nV1 vin 0 0 AC 1\n.temp 27\n.noise V(out) V1 dec 5 1k 100k\n.end\n";
    let raw = run(deck);

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
    let raw = run(&rc(".tf V(out) Vdd"));
    close(col(&raw, "tf(vdd)")[0], 0.5, 1e-9);
    close(col(&raw, "zin(vdd)")[0], 2000.0, 1e-9);
}
