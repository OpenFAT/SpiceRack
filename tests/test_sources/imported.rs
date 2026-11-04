//! Decks read and written as ngspice: ngspice PWL `r=`/`td=`, RF ports and
//! `.sp`, and Spectre PWL `delay`/`pwlperiod`, each written as ngspice's
//! card (or refused) and run against the original deck or the closed form.
//!
//! Cites: ngspice 45 manual §4.1.4, §4.1.11, §11.3.11; Spectre Reference 5.0
//! `vsource` #4, #17, #18, p.686 (`[CMP]`); real ngspice.
//!
//! Contents: none (tests only).

use super::*;

/// The Spectre PWL sources the Spectre-cards tests below read.
const SPECTRE_PWLS: &str = "// pwl\nsimulator lang=spectre\n\
    v1 (a 0) vsource type=pwl wave=[0 0 1m 1 2m 0] delay=0.5m\n\
    v2 (b 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=2m\n\
    v3 (c 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=2m delay=0.5m\n\
    r1 (a 0) resistor r=1\nr2 (b 0) resistor r=1\nr3 (c 0) resistor r=1\n\
    tr tran step=10u stop=7m maxstep=10u\n";

// ---- private ----

// ── ngspice cards in: PWL r=/td= (§4.1.4), RF port (§4.1.11), .sp (§11.3.11) ──

/// `deck` on real ngspice, and the deck the renderer writes from it: every
/// vector of every plot the same, to 1e-12 relative. No card may be kept
/// as written (a note), so the second deck is the renderer's own.
fn ngspice_round_trip(deck: &str) {
    let w = render_text(deck, Dialect::Ngspice, "rt", Dialect::Ngspice)
        .unwrap_or_else(|e| panic!("{}", e.messages));
    assert!(w.notes.diags.is_empty(), "{}", w.notes.messages);
    let emitted = w.text;
    let run = |d: &str| {
        Dialect::Ngspice
            .run_all(d)
            .unwrap_or_else(|e| panic!("ngspice: {e}\n{d}"))
    };
    let (want, got) = (run(deck), run(&emitted));
    assert_eq!(want.len(), got.len(), "{emitted}");
    let same = |a: f64, b: f64| (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1e-30);
    for (w, g) in want.iter().zip(&got) {
        assert_eq!(w.plot_name, g.plot_name);
        for v in &w.variables {
            let gi = g
                .variables
                .iter()
                .position(|x| x.name == v.name)
                .unwrap_or_else(|| panic!("{} missing\n{emitted}", v.name));
            let (a, b) = (&w.re(v.index), &g.re(gi));
            assert_eq!(a.len(), b.len(), "{} in {}", v.name, w.plot_name);
            assert!(
                a.iter().zip(b.iter()).all(|(&a, &b)| same(a, b)),
                "{} in {}\n{emitted}",
                v.name,
                w.plot_name
            );
            let (a, b) = (
                w.complex(v.index).unwrap_or_default(),
                g.complex(gi).unwrap_or_default(),
            );
            assert!(
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a.im, b.im)),
                "{} in {}",
                v.name,
                w.plot_name
            );
        }
    }
}

#[test]
fn an_imported_repeating_pwl_runs_as_the_original_deck() {
    ngspice_round_trip(
        "pwl round trip\nV1 a 0 pwl(0 0 1m 1 2m 0) r=0 td=0.5m\nR1 a 0 1\nV2 b 0 PWL(0 0 1m 1 1.5m 0.2) TD=1m\nR2 b 0 1\n\
         V3 c 0 pwl(0 0 0.5m 1 1m 0.5 2m 0) r 0.5m\nR3 c 0 1\n.tran 10u 7m 0 10u\n.end\n",
    );
}

#[test]
fn an_imported_rf_port_runs_as_the_original_deck() {
    // A T pad between ports 1 (z0 given) and 2 (z0 by default, `=` form):
    // the .op sees each port's z0 in series, the .sp its S-parameters.
    ngspice_round_trip(
        "port round trip\nV1 in 0 dc 1 ac 1 portnum 1 z0 50\nV2 out 0 dc 0 ac 1 portnum=2\nR1 in mid 16.666666666666668\n\
         R2 mid out 16.666666666666668\nR3 mid 0 66.66666666666667\n.op\n.sp lin 3 1k 3k\n.end\n",
    );
}

// ── Spectre cards in: PWL `delay`, `pwlperiod` ([CMP] p.683 #4, #17, #18; p.686) ──
//
// `delay` is the general waveform delay, ngspice's `td=`. `pwlperiod=P`
// repeats the waveform from t = 0 with period P, and before each repeat
// ramps over `twidth` (default P/1000) from its value at P - twidth to its
// value at 0. That is ngspice's `r=0` exactly when the points start at 0,
// end at P on the first point's value, and the last segment is at least
// twidth long (the ramp is then the segment itself); any other form stays raw.

#[test]
fn spectre_pwl_delay_and_an_exact_pwlperiod_are_ngspice_td_and_r() {
    // v1..v3 are ngspice's `td=`, `r=0` and both; each of v4..v11 has no
    // ngspice card (ends that differ, a twidth ramp; a period past the
    // points; a given twidth; points from 0.1 ms; a last segment shorter
    // than the default twidth; an FM sine's and an exp's delay; a
    // pwlperiod off a PWL), so it stays Spectre text, which ngspice
    // refuses (`W0001`).
    let ng = |src: &str| render_text(src, Dialect::Spectre, "t", Dialect::Ngspice);
    let deck = ng(SPECTRE_PWLS)
        .unwrap_or_else(|e| panic!("{}", e.messages))
        .text;
    for card in [
        "V1 a 0 0 PWL(0 0 0.001 1 0.002 0) td=0.0005",
        "V2 b 0 0 PWL(0 0 0.001 1 0.002 0) r=0",
        "V3 c 0 0 PWL(0 0 0.001 1 0.002 0) r=0 td=0.0005",
    ] {
        assert!(deck.lines().any(|l| l == card), "{card}\n{deck}");
    }
    let extra = [
        "v4 (d 0) vsource type=pwl wave=[0 0 1m 1 2m 0.5] pwlperiod=2m",
        "v5 (e 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=3m",
        "v6 (f 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=2m twidth=1u",
        "v7 (g 0) vsource type=pwl wave=[0.1m 0 1m 1 2m 0] pwlperiod=2m",
        "v8 (h 0) vsource type=pwl wave=[0 0 1m 1 2m 0 2.0000001m 0] pwlperiod=2.0000001m",
        "v9 (i 0) vsource type=sine sinedc=0 ampl=1 freq=1k fmmodindex=2 fmmodfreq=100 delay=1m",
        "v10 (j 0) vsource type=exp val0=0 val1=1 td1=1m tau1=1m delay=1m",
        "v11 (k 0) vsource type=pulse val0=0 val1=1 pwlperiod=2m",
    ];
    for line in extra {
        let e = ng(&format!("{SPECTRE_PWLS}{line}\n")).expect_err(line);
        assert!(
            e.diags.iter().any(|d| d.code == Code::W0001),
            "{line}: {}",
            e.messages
        );
    }
}

#[test]
fn an_imported_spectre_pwl_runs_on_ngspice_as_the_manual_says() {
    // The manual's semantics as the closed form: `tri` once (1 V at 1 ms,
    // 0 from 2 ms), shifted by `delay` (holding its first value before) and
    // repeated with period `pwlperiod`.
    let from = |d: Dialect| {
        render_text(SPECTRE_PWLS, Dialect::Spectre, "t", d)
            .unwrap_or_else(|e| panic!("{}", e.messages))
            .text
    };
    // Written back as Spectre, the cards say what they said.
    let back = from(Dialect::Spectre);
    assert!(
        back.contains("wave=[0 0 1m 1 2m 0] pwlperiod=2m\n")
            && back.contains("wave=[0 0 1m 1 2m 0] delay=500u pwlperiod=2m\n"),
        "{back}"
    );
    let deck = from(Dialect::Ngspice);
    let raw = ngspice(&deck);
    let time = col(&raw, "time");
    let tri = |t: f64| {
        if t <= 0.0 || t >= 2e-3 {
            0.0
        } else if t <= 1e-3 {
            t / 1e-3
        } else {
            (2e-3 - t) / 1e-3
        }
    };
    let delayed = |t: f64| {
        if t < 0.5e-3 {
            0.0
        } else {
            (t - 0.5e-3).rem_euclid(2e-3)
        }
    };
    for (node, want) in [
        ("a", &(|t: f64| tri(t - 0.5e-3)) as &dyn Fn(f64) -> f64),
        ("b", &|t: f64| tri(t.rem_euclid(2e-3))),
        ("c", &|t: f64| tri(delayed(t))),
    ] {
        let v = col(&raw, node);
        for k in 0..=700 {
            let t = 7e-3 * k as f64 / 700.0;
            assert!(
                (at(time, v, t) - want(t)).abs() < 1e-9,
                "{node} at {t}: {}, want {}\n{deck}",
                at(time, v, t),
                want(t)
            );
        }
    }
}
