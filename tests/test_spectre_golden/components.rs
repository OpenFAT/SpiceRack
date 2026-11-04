//! Each component and source waveform -> its documented Spectre instance
//! form; a construct with no documented spelling is a refusal code.
//!
//! Cites: `[CMP]` component chapters and `[REF]` (see `main.rs`), per test.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

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
    let cases = [
        ("R1 a b 1.2k m=2", "r1 (a b) resistor r=1.2k m=2"),
        ("C2 1 0 2u", "c2 (1 0) capacitor c=2u"),
        ("L33 0 net29 10n", "l33 (0 net29) inductor l=10n"),
        ("E1 out1 0 pos neg 10", "e1 (out1 0 pos neg) vcvs gain=10"),
        ("G1 sink src ps ns -1", "g1 (sink src ps ns) vccs gm=-1"),
        // `probe=` names an instance as the deck wrote it (`[KUN]` B.2: case
        // sensitive; the source's own name is lowercased with it).
        ("Fcs pos gnd V1 2.5", "fcs (pos gnd) cccs probe=v1 gain=2.5"),
        ("Hvs pos gnd V1 1", "hvs (pos gnd) ccvs probe=v1 rm=1"),
        ("D0 dp dn pdiode area=1", "d0 (dp dn) pdiode area=1"),
        ("T1 1 0 2 0 Z0=100 TD=1n", "t1 (1 0 2 0) tline z0=100 td=1n"),
        // Every number goes through the one Spectre formatter: scale
        // suffixes, with Spectre's capital M for mega (`[KUN]` B.2).
        ("E2 a 0 b 0 2meg", "e2 (a 0 b 0) vcvs gain=2M"),
        ("G2 a 0 b 0 1m", "g2 (a 0 b 0) vccs gm=1m"),
        ("F2 a 0 V1 1u", "f2 (a 0) cccs probe=v1 gain=1u"),
        ("H2 a 0 V1 1k", "h2 (a 0) ccvs probe=v1 rm=1k"),
        (
            "T2 1 0 2 0 Z0=1meg TD=2.5p",
            "t2 (1 0 2 0) tline z0=1M td=2.5p",
        ),
        ("S1 1 2 ps ns my_relay", "s1 (1 2 ps ns) my_relay"),
    ];
    for (card, want) in cases {
        assert_eq!(component(card), want, "{card}");
    }
    let k = |card: &str| component(&format!("L1 a b 1u\nL2 c d 1u\n{card}"));
    assert!(
        k("K1 L1 L2 1").ends_with("k1 mutual_inductor coupling=1 ind1=l1 ind2=l2"),
        "{}",
        k("K1 L1 L2 1")
    );
    assert!(
        k("K2 L1 L2 1m").ends_with("k2 mutual_inductor coupling=1m ind1=l1 ind2=l2"),
        "{}",
        k("K2 L1 L2 1m")
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
    let cases = [
        (
            "V1 1 0 0 PULSE(0 5 0 10n 10n 40n 100n)",
            "v1 (1 0) vsource dc=0 type=pulse val0=0 val1=5 delay=0 rise=10n fall=10n width=40n period=100n",
        ),
        (
            "V1 1 0 0 PWL(1n 0 1.5n 0.5)",
            "v1 (1 0) vsource dc=0 type=pwl wave=[1n 0 1.5n 500m]",
        ),
        // sine: `delay` and `damp` are the documented names, not
        // `sinedelay=` / `sinedamp=`, which vsource does not have.
        (
            "V1 1 0 0 SIN(1 0.5 1meg 1n 2 45)",
            "v1 (1 0) vsource dc=0 type=sine sinedc=1 ampl=500m freq=1M delay=1n damp=2 sinephase=45",
        ),
        (
            "V1 1 0 0 EXP(0 1 1n 2n 3n 4n)",
            "v1 (1 0) vsource dc=0 type=exp val0=0 val1=1 td1=1n tau1=2n td2=3n tau2=4n",
        ),
        ("V1 1 0 0 AC 1 45", "v1 (1 0) vsource dc=0 mag=1 phase=45"),
    ];
    for (card, want) in cases {
        assert_eq!(component(card), want, "{card}");
    }
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
    let tri = |end: &str, tail: &str| format!("V1 1 0 0 PWL(0 0 1m 1 2m {end}){tail}");
    assert_eq!(
        component(&tri("0", " td=0.5m")),
        "v1 (1 0) vsource dc=0 type=pwl wave=[0 0 1m 1 2m 0] delay=500u"
    );
    assert_eq!(
        component(&tri("0", " r=0 td=0.5m")),
        "v1 (1 0) vsource dc=0 type=pwl wave=[0 0 1m 1 2m 0] delay=500u pwlperiod=2m"
    );
    for (end, repeat) in [("0", "1m"), ("0.5", "0")] {
        let r = try_component(&tri(end, &format!(" r={repeat}")));
        assert!(r.is_err(), "end {end}, r={repeat}: {r:?}");
    }
}

/// Things SPICE can express that Spectre has no documented spelling for must
/// be refused, not silently become a comment or a guessed statement.
#[test]
fn spectre_refuses_constructs_without_a_documented_spelling() {
    let cases = [
        // XSPICE A-devices are ngspice-only.
        "A1 a d_and\n.model d_and d_and",
        // `[CMP]` p.856: Spectre bsource expressions are `v(a,b)`,
        // `i("inst:idx")`, `$time` — not SPICE's `V()`/`I()`/`time`, and
        // Spectre is case sensitive.
        "B1 o 0 V=V(a)*2",
        "B1 o 0 I=I(V1)*2",
        // `[CMP]` p.625 `relay` is voltage controlled; p.643 `switch` changes
        // position only between analyses. Neither is SPICE's W element.
        "W1 a b V1 csw",
        // `[CMP]` p.685 documents ammodindex/ammodfreq but no waveform
        // equation, so the SPICE AM() mapping cannot be established.
        "V1 a 0 0 AM(1 0.5 1k 1meg 0)",
    ];
    for card in cases {
        let r = try_component(card);
        assert!(r.is_err(), "{card}: expected a refusal, got {r:?}");
    }
}
