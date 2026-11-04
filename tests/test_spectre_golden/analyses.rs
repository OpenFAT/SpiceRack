//! Each analysis -> its documented Spectre statement (dc, ac, tran, noise,
//! xf, sens, sp, fourier), SI scale factors, and the analyses Spectre
//! refuses.
//!
//! Cites: `[REF]`, `[CMP]` and `[KUN]` (see `main.rs`), per test.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

/// `[REF]` p.20143 verbatim `dc1 dc` (a `dc` without a sweep parameter is the
/// operating point).
/// `[REF]` p.64: temperature is `param=temp`, a netlist parameter is a bare
/// `param=<name>`, and a device parameter needs `dev=<instance>` alongside
/// `param=<parameter>`. SPICE `.dc Vin ...` sweeps the source's `dc` value
/// (`[CMP]` p.683 vsource parameter #1).
#[test]
fn spectre_dc_sweeps_pick_the_documented_selector() {
    assert_eq!(analysis(".op"), "op1 dc");
    let dc = |src: &str| analysis(&format!(".dc {src} 0 5 0.1"));
    assert_eq!(
        dc("Vin"),
        "dc1 dc dev=vin param=dc start=0 stop=5 step=100m"
    );
    assert_eq!(
        dc("Iin"),
        "dc1 dc dev=iin param=dc start=0 stop=5 step=100m"
    );
    assert_eq!(dc("temp"), "dc1 dc param=temp start=0 stop=5 step=100m");
    assert_eq!(dc("rload"), "dc1 dc param=rload start=0 stop=5 step=100m");
}

/// `[REF]` p.43-44 (ac) and p.416-417 (tran): `Name ac ...` /
/// `Name tran ...`. tran parameters `stop` (#1), `start` (#3),
/// `maxstep` (#8), `step` (#9). Verbatim `[REF]` p.433
/// `tran1 tran stop=0.5u noisefmax=10G noiseseed=1`.
#[test]
fn spectre_ac_and_tran_match_documented_parameters() {
    assert_eq!(
        analysis(".ac dec 100 1 1g"),
        "ac1 ac start=1 stop=1G dec=100"
    );
    assert_eq!(
        analysis(".tran 1n 1u 100n 5n"),
        "tran1 tran step=1n stop=1u start=100n maxstep=5n",
    );
}

/// `[REF]` "Sweep interval parameters" (p.44, p.182, p.389, p.394, p.439, ...)
/// list `step lin dec log values valuesfile` only — Spectre has no octave
/// sweep, so SPICE's `oct` must be refused rather than emitted.
#[test]
fn spectre_rejects_octave_frequency_sweeps() {
    let r = try_analysis(".ac oct 10 1 1meg");
    assert!(r.is_err(), "{r:?}");
}

/// `[REF]` p.181-182 noise: `Name [p] [n] noise parameter=value ...`,
/// "The optional terminals (p and n) specify the output of the circuit",
/// `oprobe` (#16) and `iprobe` (#17) name *components*: an output is a
/// terminal pair, never `oprobe=V(out)`.
#[test]
fn spectre_noise_puts_the_output_in_the_terminal_list() {
    assert_eq!(
        analysis(".noise V(out) Vin dec 10 1 1meg"),
        "noise1 (out 0) noise start=1 stop=1M dec=10 iprobe=vin",
    );
}

/// `[REF]` p.438-439 xf: "you can simply specify a voltage to be the output by
/// giving a pair of nodes on the xf analysis statement". xf computes the
/// transfer function from *every* independent source, so there is no
/// `source=` parameter.
///
/// INFERRED, NOT VERIFIED: `freq=0` as the stand-in for SPICE's DC `.tf`.
#[test]
fn spectre_xf_has_no_source_parameter() {
    let s = analysis(".tf V(out) Vin");
    assert_eq!(s, "xf1 (out 0) xf freq=0");
    assert!(!s.contains("source="), "{s}");
}

/// `[REF]` p.524 verbatim: `sens (1 n2 7) for (analAC)` — `sens` is a control
/// statement naming an analysis, spelled
/// `sens (outputs) to (params) for (analyses)`.
#[test]
fn spectre_sens_matches_documented_control_statement() {
    assert_eq!(
        analysis(".sens V(out)"),
        "sensdc1 dc\nsens (out) for (sensdc1)"
    );
    assert_eq!(
        analysis(".sens V(out) AC dec 10 1 1meg"),
        "sensac1 ac start=1 stop=1M dec=10\nsens (out) for (sensac1)",
    );
}

/// `[REF]` p.387 sp: `Name sp parameter=value ...` with the sweep interval
/// parameters.
#[test]
fn spectre_sp_matches_documented_parameters() {
    assert_eq!(
        analysis(".sp dec 20 1meg 1g"),
        "sp1 sp start=1M stop=1G dec=20"
    );
}

/// `[CMP]` p.323-325: Spectre's `fourier` is a *component* placed on a node
/// pair — `Name [p] [n] [pr] [nr] fourier parameter=value ...`, verbatim
/// `four1 (1 0) fourmod harms=50`, with `fund` as instance parameter #1. It is
/// not an analysis statement and has no `signal=` parameter. (ngspice's
/// `.four` names no harmonic count, so none is written.)
#[test]
fn spectre_fourier_is_emitted_as_a_component() {
    assert_eq!(
        analysis(".tran 1u 1m\n.four 1k V(out) mid"),
        "tran1 tran step=1u stop=1m\nfour1 (out 0) fourier fund=1k\nfour2 (mid 0) fourier fund=1k",
    );
}

/// Analyses belonging to other simulators' dialects must be refused:
/// Spectre's `pz` (`[REF]` p.312) does not take SPICE's node quadruple.
#[test]
fn spectre_refuses_foreign_analyses() {
    let r = try_analysis(".pz 1 0 3 0 vol pz");
    assert!(r.is_err(), "{r:?}");
}

/// Kundert Appendix B, Table B.1 (SI, used by Spectre) vs Table B.2 (SPICE):
/// SPICE's `meg`/`g`/`t` are not Spectre scale factors; SI spells them
/// `M`/`G`/`T`. `k m u n p f` mean the same in both.
#[test]
fn spectre_numbers_use_si_scale_factors() {
    let r = |v: &str| component(&format!("R1 a b {v}"));
    assert!(r("1meg").ends_with("r=1M"), "{}", r("1meg"));
    assert!(r("1g").ends_with("r=1G"), "{}", r("1g"));
    assert!(r("1t").ends_with("r=1T"), "{}", r("1t"));
    assert!(r("1k").ends_with("r=1k"), "{}", r("1k"));
    for v in ["1meg", "1g", "1t"] {
        let s = r(v);
        assert!(
            !s.contains("meg") && !s.ends_with('g') && !s.ends_with('t'),
            "{s}"
        );
    }
}
