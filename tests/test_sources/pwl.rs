//! PWL `r=`/`td=` -> ngspice (and ESPice) runs that repeat and delay as the
//! closed form says; each dialect without a form refuses.
//!
//! Cites: ngspice 45 manual §4.1.4; real ngspice and ESPice.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

#[test]
fn a_repeating_pwl_two_periods_on_equals_its_first_period() {
    // r=0 td=0.5m: the 2 ms triangle from 0.5 ms on, forever. Two periods
    // later the value is the same, and both are the closed-form triangle.
    let ir = bench(
        &source(false, &triangle(Some("0"), Some("0.5m"))),
        1e-5,
        7e-3,
        "",
    );
    let deck = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(
        deck.contains("PWL(0 0 0.001 1 0.002 0) r=0 td=0.0005"),
        "{deck}"
    );
    let raw = ngspice(&deck);
    let (time, v) = (col(&raw, "time"), col(&raw, "a"));
    for k in 0..=200 {
        let tau = 2e-3 * k as f64 / 200.0;
        let want = if tau <= 1e-3 {
            tau / 1e-3
        } else {
            (2e-3 - tau) / 1e-3
        };
        let first = at(time, v, 0.5e-3 + tau);
        let third = at(time, v, 0.5e-3 + tau + 4e-3);
        assert!(
            (first - want).abs() < 1e-9,
            "t=0.5m+{tau}: {first}, want {want}"
        );
        assert!(
            (third - first).abs() < 1e-9,
            "t=4.5m+{tau}: {third}, first period {first}"
        );
    }
    // Before the delay the source holds its first value.
    assert!(at(time, v, 0.25e-3).abs() < 1e-12);
}

#[test]
fn a_pwl_without_repeat_holds_its_last_value() {
    // The neighbour of the test above: no r= means once, then the final 0 V.
    let ir = bench(&source(false, &triangle(None, None)), 1e-5, 5e-3, "");
    let deck = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(deck.contains("PWL(0 0 0.001 1 0.002 0)\n"), "{deck}");
    let raw = ngspice(&deck);
    assert!(at(col(&raw, "time"), col(&raw, "a"), 4.5e-3).abs() < 1e-12);
}

#[test]
fn pwl_repeat_and_delay_are_refused_where_no_dialect_form_exists() {
    let tri = || triangle(Some("0"), Some("0.5m"));
    let v = bench(&source(false, &tri()), 1e-5, 1e-3, "");
    let i = bench(&source(true, &tri()), 1e-5, 1e-3, "");
    // ngspice reads r=/td= on a V source only (manual §4.1.4; ngspice 45
    // answers an I source with "unknown parameter (r)"), `W0008`; ESPice
    // reads both.
    assert!(refuses(netlist(&i, Dialect::Ngspice), Code::W0008));
    let espice = netlist(&i, Dialect::Espice).unwrap();
    assert!(espice.contains(") r=0 td=0.0005"), "{espice}");
    for d in [Dialect::Ltspice, Dialect::Hspice, Dialect::Xyce] {
        assert!(refuses(netlist(&v, d), Code::W0008), "{d:?}");
    }
    // VACASK has no PWL at all (`W0108`, it runs as zero there).
    assert!(refuses(netlist(&v, Dialect::Vacask), Code::W0108));
    // Spectre: `delay` and `pwlperiod`, but only for an r= that is exactly a
    // period (test_spectre_golden/); r=1m repeats from mid-waveform.
    let spectre = netlist(&v, Dialect::Spectre).unwrap();
    assert!(spectre.contains("delay=500u pwlperiod=2m"), "{spectre}");
    let mid = bench(&source(false, &triangle(Some("1m"), None)), 1e-5, 1e-3, "");
    assert!(netlist(&mid, Dialect::Spectre).is_err());
    // Without r=/td= every dialect but VACASK (no PWL at all) writes it.
    let plain = bench(&source(false, &triangle(None, None)), 1e-5, 1e-3, "");
    for d in [
        Dialect::Ltspice,
        Dialect::Hspice,
        Dialect::Xyce,
        Dialect::Spectre,
    ] {
        netlist(&plain, d).unwrap_or_else(|e| panic!("{d:?}: {}", e.messages));
    }
}

#[test]
fn espice_reads_the_repeating_pwl_current_source() {
    // ESPice takes r=/td= on an I source too: 1 A into 1 Ω, two periods on.
    let ir = bench(
        &source(true, &triangle(Some("0"), Some("0.5m"))),
        1e-5,
        7e-3,
        "",
    );
    let deck = netlist(&ir, Dialect::Espice).unwrap();
    let raw = Dialect::Espice
        .run(&deck)
        .unwrap_or_else(|e| panic!("espice: {e}\n{deck}"));
    let (time, v) = (col(&raw, "time"), col(&raw, "a"));
    // ESPice 0.3.0 does not land a timepoint on a repeated period's corner
    // (ngspice does), so a corner read by interpolation is off by up to one
    // max step times the slope: 10 µs × 1 V/ms = 0.01 V.
    let tol = 1e-5 * 1e3;
    for tau in [0.25e-3, 1e-3, 1.5e-3] {
        let want = if tau <= 1e-3 {
            tau / 1e-3
        } else {
            (2e-3 - tau) / 1e-3
        };
        // I flows n+ -> n- through the source, so into `a` from 0 is -I.
        let first = -at(time, v, 0.5e-3 + tau);
        let third = -at(time, v, 4.5e-3 + tau);
        assert!((first - want).abs() < tol, "{tau}: {first}, want {want}");
        assert!((third - first).abs() < tol, "{tau}: {third} vs {first}");
    }
}
