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
    let Some(raw) = run(
        "* noise\nV1 in 0 0 AC 1\nR1 in out 10k\nC1 out 0 1f\n.save all\n\
         .noise V(out) V1 lin 2 1 2\n.end\n",
    ) else {
        return;
    };
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
    let Some(raw) = run(
        "* disto\nV1 in 0 0 AC 1\nR1 in nb 10k\nD1 nb 0 dmod\n\
         .model dmod D(IS=1e-14)\n.save all\n.disto dec 5 1k 10k\n.end\n",
    ) else {
        return;
    };
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
    let Some(raw) = run(
        "* pz\nV1 in 0 0 AC 1\nR1 in out 1k\nC1 out 0 1u\n.pz in 0 out 0 vol pol\n.end\n",
    ) else {
        return;
    };
    assert_eq!(raw.plot_name, "Pole-Zero Analysis");
    let pole = raw.complex_data[0][0];
    assert!(close(pole.re, -1000.0, 1e-6), "pole {pole}");
    assert!(pole.im.abs() < 1e-6, "real pole expected, got {pole}");
}

// ── .four ──

#[test]
fn four_prints_the_harmonic_table_to_stdout() {
    // Under `-b -r` ngspice says ".fourier line ignored since rawfile was
    // produced." The backend rewrites the deck so `fourier` runs in a
    // `.control` block; the table then lands in stdout.
    let Some(raw) = run(
        "* four\nV1 in 0 SIN(0 1 1k)\nR1 in out 1k\nC1 out 0 1n\n\
         .tran 1u 10m\n.four 1k v(out)\n.end\n",
    ) else {
        return;
    };
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

/// A `.control` block holding only `pre_osdi` (what the codegen emits for OSDI
/// loads) must not block the measure rewrite.
#[test]
fn measure_survives_alongside_an_osdi_control_block() {
    let Some(raw) = run(
        "* osdi + meas\n.control\npre_osdi /nonexistent/model.osdi\n.endc\n\
         V1 in 0 0\nR1 in out 10k\nR2 out 0 10k\n\
         .meas dc vhalf FIND V(out) AT=10\n.dc V1 0 10 1\n.end\n",
    ) else {
        return;
    };
    assert!(
        measures(&raw).iter().any(|m| m.name == "vhalf"),
        "measure lost next to an osdi block:\n{}",
        raw.stdout
    );
}

// ── circuit features claimed in the capability table ──

#[test]
fn xspice_a_elements_run() {
    // `gain` code model, gain = 2 -> a 0.5 V input gives 1.0 V out.
    let Some(raw) = run(
        "* xspice\nV1 in 0 0.5\nA1 in out gainblk\n\
         .model gainblk gain(gain=2.0)\nRl out 0 1meg\n.op\n.end\n",
    ) else {
        return;
    };
    assert!(close(scalar(&raw, "out"), 1.0, 1e-6), "{}", scalar(&raw, "out"));
}

#[test]
fn control_blocks_run_verbatim() {
    let Some(raw) = run("* control\nV1 in 0 3\nR1 in 0 1k\n.control\nop\n.endc\n.end\n") else {
        return;
    };
    assert!(close(scalar(&raw, "in"), 3.0, 1e-9));
}

/// The XSPICE `s_xfer` code model is ngspice's Laplace facility. The
/// `Laplace(...)` B-source text that `Circuit::has_laplace_sources` looks for
/// is *not* ngspice syntax — see the report.
#[test]
fn xspice_s_xfer_is_the_working_laplace_path() {
    // 1/(1 + s*tau), tau = 159.1549431 us -> -3.0103 dB at 1 kHz.
    let Some(raw) = run(
        "* s_xfer\nV1 in 0 0 AC 1\nA1 in out filt\n\
         .model filt s_xfer(num_coeff=[1] den_coeff=[1.591549431e-4 1] int_ic=[0])\n\
         Rl out 0 1meg\n.ac lin 1 1000 1000\n.end\n",
    ) else {
        return;
    };
    let db = 20.0 * raw.complex_data[index_of(&raw, "out")][0].norm().log10();
    assert!((db + 3.0103).abs() < 1e-2, "s_xfer corner {db} dB");
}
