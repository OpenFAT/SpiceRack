//! TRNOISE and TRRANDOM -> ngspice runs whose samples have the stated
//! distribution; every other dialect refuses; the importer lowers the cards.
//!
//! Cites: ngspice 45 manual §4.1.7, §4.1.8, §18.1; real ngspice.
//!
//! Statistical oracles. ngspice seeds its generator with 1 unless told
//! otherwise (manual §18.1); the deck sets `seed=1` so the run is the same
//! every time, and each tolerance below is five standard errors of the
//! estimate for N independent samples, so a correct source fails with
//! probability under 1e-6 whatever the seed.
//!
//! Contents: none (tests only).

use super::*;

const N: usize = 4000;

// ---- private ----

/// `.options seed=1`, and the source `src` alone into 1 Ω.
fn seeded(src: Component, step: f64, stop: f64) -> CircuitIR {
    let mut ir = bench(src, step, stop);
    ir.testbench
        .as_mut()
        .unwrap()
        .options
        .portable
        .push(("seed".into(), "1".into()));
    ir
}

#[test]
fn trnoise_white_noise_has_rms_na() {
    // TRNOISE(NA NT 0 0): a Gaussian sample of rms NA every NT. Read at the
    // sample times (breakpoints), the N values are i.i.d. N(0, NA^2): the
    // sample std s has standard error NA/sqrt(2(N-1)), the mean NA/sqrt(N).
    let (na, nt) = (0.1, 1e-6);
    let noise = IrWaveform::TrNoise {
        na: na.into(),
        nt: nt.into(),
        nalpha: 0.0.into(),
        namp: 0.0.into(),
        rtsam: 0.0.into(),
        rtscapt: 0.0.into(),
        rtsemt: 0.0.into(),
    };
    let ir = seeded(source(false, noise), nt, nt * (N + 1) as f64);
    let deck = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(deck.contains("TRNOISE(0.1 0.000001 0 0)"), "{deck}");
    let raw = ngspice(&deck);
    let (time, v) = (col(&raw, "time"), col(&raw, "a"));
    let xs: Vec<f64> = (1..=N).map(|k| at(time, v, k as f64 * nt)).collect();
    let mean = xs.iter().sum::<f64>() / N as f64;
    let s = (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (N - 1) as f64).sqrt();
    assert!(mean.abs() < 5.0 * na / (N as f64).sqrt(), "mean {mean}");
    assert!(
        (s - na).abs() < 5.0 * na / (2.0 * (N - 1) as f64).sqrt(),
        "std {s}, want {na}"
    );
}

#[test]
fn trrandom_uniform_stays_in_range_with_the_offset_as_mean() {
    // TRRANDOM(1 TS TD 2 3): uniform within 3 ± 2 for each TS, and 3 (the
    // offset) before TD. Uniform on [1, 5] has sigma 4/sqrt(12); the mean of
    // N draws is 3 within 5 sigma/sqrt(N). N draws all inside a 3.5 V slice
    // of the 4 V range has probability below 1e-200, so the spread pins the
    // range too (a half-range reading would cap it at 2 V).
    let (ts, td) = (1e-6, 1e-5);
    let rnd = IrWaveform::TrRandom {
        kind: RandomKind::Uniform,
        ts: ts.into(),
        td: td.into(),
        p1: 2.0.into(),
        p2: 3.0.into(),
    };
    let ir = seeded(source(false, rnd), ts, td + ts * (N + 1) as f64);
    let deck = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(deck.contains("TRRANDOM(1 0.000001 0.00001 2 3)"), "{deck}");
    let raw = ngspice(&deck);
    let (time, v) = (col(&raw, "time"), col(&raw, "a"));
    assert!(
        (at(time, v, td / 2.0) - 3.0).abs() < 1e-12,
        "before TD: the offset"
    );
    for (t, x) in time.iter().zip(v) {
        assert!(
            (1.0 - 1e-12..=5.0 + 1e-12).contains(x),
            "t={t}: {x} outside 3 ± 2"
        );
    }
    let xs: Vec<f64> = (0..N)
        .map(|k| at(time, v, td + (k as f64 + 0.5) * ts))
        .collect();
    let mean = xs.iter().sum::<f64>() / N as f64;
    let sigma = 4.0 / 12f64.sqrt();
    assert!(
        (mean - 3.0).abs() < 5.0 * sigma / (N as f64).sqrt(),
        "mean {mean}"
    );
    let spread =
        xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
    assert!(spread > 3.5, "spread {spread}");
}

#[test]
fn trnoise_and_trrandom_are_ngspice_only() {
    // No other dialect has a source form (HSPICE only the `.TRANNOISE`
    // analysis, Command Reference E-2010.12 p.320; Spectre's `noisefile` is
    // small-signal, Spectre Reference 5.0 pp.685-686; VACASK's trannoise is
    // device noise, its `docs/cmd-analysis-trannoise.md`), and
    // ESPice 0.3.0 refuses both cards ("the trnoise source waveform is not
    // supported").
    let z = || IrNum::Num(0.0);
    let noise = IrWaveform::TrNoise {
        na: 1.0.into(),
        nt: 1e-6.into(),
        nalpha: z(),
        namp: z(),
        rtsam: z(),
        rtscapt: z(),
        rtsemt: z(),
    };
    let rnd = IrWaveform::TrRandom {
        kind: RandomKind::Gaussian,
        ts: 1e-6.into(),
        td: z(),
        p1: 1.0.into(),
        p2: z(),
    };
    for wf in [noise, rnd] {
        for current in [false, true] {
            let ir = bench(source(current, wf.clone()), 1e-6, 1e-5);
            netlist(&ir, Dialect::Ngspice).unwrap();
            for d in [
                Dialect::Espice,
                Dialect::Ltspice,
                Dialect::Hspice,
                Dialect::Xyce,
                Dialect::Spectre,
                Dialect::Vacask,
            ] {
                let e = netlist(&ir, d).unwrap_err().to_string();
                assert!(
                    e.contains("TRNOISE") || e.contains("TRRANDOM"),
                    "{d:?}: {e}"
                );
            }
        }
    }
}

/// The importer lowers an ngspice deck's TRNOISE/TRRANDOM cards (with
/// TRRANDOM's defaults filled in, manual §4.1.8) and keeps a TRRANDOM type
/// outside 1..=4 raw.
#[test]
fn ngspice_trnoise_and_trrandom_cards_lower_into_the_ir() {
    let deck = "t\nV1 a 0 dc 0 trnoise(20n 0.5n 0 0)\nI1 b 0 dc 0 TRNOISE(1m 1u 1.0 0.1m 15m 22u 50u)\n\
                V2 c 0 dc 0 trrandom (2 10m 0 1)\nV3 d 0 trrandom(4 1u)\nV4 e 0 trrandom(7 1u)\n.end\n";
    let (ir, report) = spicerack::import::parse(deck, "t", Dialect::Ngspice).unwrap();
    let wf = |i: usize| match &ir.top.components[i] {
        Component::VoltageSource { waveform, .. } | Component::CurrentSource { waveform, .. } => {
            waveform.clone()
        }
        other => panic!("{other:?}"),
    };
    // Each number as ngspice reads its spelling (`20n` is 20 * 10^-9 there).
    let n = |s: &str| IrNum::Num(spicerack::unit::ngspice_value(s).unwrap());
    assert_eq!(
        wf(0),
        Some(IrWaveform::TrNoise {
            na: n("20n"),
            nt: n("0.5n"),
            nalpha: n("0"),
            namp: n("0"),
            rtsam: n("0"),
            rtscapt: n("0"),
            rtsemt: n("0")
        })
    );
    assert_eq!(
        wf(1),
        Some(IrWaveform::TrNoise {
            na: n("1m"),
            nt: n("1u"),
            nalpha: n("1.0"),
            namp: n("0.1m"),
            rtsam: n("15m"),
            rtscapt: n("22u"),
            rtsemt: n("50u")
        })
    );
    assert_eq!(
        wf(2),
        Some(IrWaveform::TrRandom {
            kind: RandomKind::Gaussian,
            ts: n("10m"),
            td: n("0"),
            p1: n("1"),
            p2: n("0")
        })
    );
    assert_eq!(
        wf(3),
        Some(IrWaveform::TrRandom {
            kind: RandomKind::Poisson,
            ts: n("1u"),
            td: n("0"),
            p1: n("1"),
            p2: n("0")
        })
    );
    assert!(
        matches!(&ir.top.components[4], Component::RawSpice { .. }),
        "{:?}",
        ir.top.components[4]
    );
    assert_eq!(report.raw, 1);
    // Written back, a card with no DC value gets 0, not the waveform's start:
    // these two add to the DC value.
    let out = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(out.contains("V3 d 0 0 TRRANDOM(4 0.000001 0 1 0)"), "{out}");
}
