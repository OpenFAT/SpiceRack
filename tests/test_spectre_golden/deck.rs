//! Spectre deck structure (preamble, includes, subcircuits, models, ic,
//! nodeset, save, measures, options) -> text checked against the manual.
//!
//! Cites: `[REF]`, `[CMP]` and `[KUN]` (see `main.rs`), per test.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

/// `[KUN]` B.2: "all Spectre netlists must begin with a `lang=spectre`
/// statement"; `//` starts a comment.
/// `[REF]` p.494 `include "filename"`, p.493 `include "file" section=name`.
/// `[REF]` p.541 / `[CMP]` p.888 `ahdl_include "VerilogAfile.va"`.
/// `[REF]` p.506 `parameters <param=value> [param=value]...`.
#[test]
fn spectre_deck_preamble_matches_documented_statements() {
    let deck =
        "Preamble\n.include /pdk/models.scs\n.lib /pdk/corners.scs tt\n.param vdd=1.8\n.end\n";
    // An ngspice deck has no load card: the load comes beside the text.
    let netlist = spectre_with(deck, &["/m/bsimcmg.va".to_string()]).unwrap();

    assert!(netlist.starts_with("// Preamble\n"), "{netlist}");
    assert!(netlist.contains("\nsimulator lang=spectre\n"), "{netlist}");
    assert!(
        netlist.contains("ahdl_include \"/m/bsimcmg.va\""),
        "{netlist}"
    );
    assert!(netlist.contains("include \"/pdk/models.scs\""), "{netlist}");
    assert!(
        netlist.contains("include \"/pdk/corners.scs\" section=tt"),
        "{netlist}"
    );
    assert!(netlist.contains("parameters vdd=1.8"), "{netlist}");
    // Spectre has no `.end`.
    assert!(!netlist.contains(".end"), "{netlist}");
}

/// `[REF]` p.535 verbatim:
/// ```text
/// subckt coax (i1 o1 i2 o2)
///      parameters zin=50 zout=50 vin=1 vout=1 len=0
/// ends coax
/// ```
/// plus the instance form `Coax1 pin nin out gnd coax zin=75 zout=150 len=35m`.
#[test]
fn spectre_subckt_and_instance_match_documented_forms() {
    let emitted = spectre("defs\n.subckt coax i1 o1 PARAMS: zin=50\n.ends coax\n.end\n").unwrap();
    assert!(
        emitted.contains("\n\nsubckt coax (i1 o1)\nparameters zin=50\nends coax\n"),
        "{emitted}"
    );
    let netlist = spectre("inst\nXCoax1 pin nin coax zin=75\n.end\n").unwrap();
    assert!(
        netlist.contains("xcoax1 (pin nin) coax zin=75"),
        "{netlist}"
    );
}

/// `[CMP]` p.628 `model resmod resistor rsh=150 l=2u w=2u etch=0.05u tc1=0.1`,
/// `[REF]` p.498 `model nch bsim3v3 type=n mobmod=1 capmod=2 version=3.1`.
/// No documented Cadence example parenthesises model parameters.
#[test]
fn spectre_model_statement_has_no_parentheses() {
    let netlist = spectre("models\n.model nch bsim3v3 type=n version=3.1\n.end\n").unwrap();
    assert!(
        netlist.contains("model nch bsim3v3 type=n version=3.1"),
        "{netlist}"
    );
    assert!(
        !netlist.contains("bsim3v3 ("),
        "model params must not be parenthesised: {netlist}"
    );
}

/// `[REF]` p.490 `ic 7=0 out=1 ...`, p.502 `nodeset 7=0 out=1 ...`,
/// p.518 `save 7 out OpAmp1.comp ...` — bare signal names, never `V(x)`.
#[test]
fn spectre_ic_nodeset_save_use_bare_signal_names() {
    let netlist =
        spectre("t\n.ic V(out)=1.2\n.nodeset V(mid)=0.5\n.save V(out)\n.op\n.end\n").unwrap();
    assert!(netlist.contains("\nic out=1.2\n"), "{netlist}");
    assert!(netlist.contains("\nnodeset mid=500m\n"), "{netlist}");
    assert!(netlist.contains("\nsave out\n"), "{netlist}");
}

/// `[REF]` p.17: Spectre supports "standard SPICE measurement functions
/// (.measure)". `.measure` is SPICE syntax, so it must sit inside a
/// `simulator lang=spice` region (`[REF]` p.493, `[KUN]` B.2).
#[test]
fn spectre_measure_is_fenced_by_language_switches() {
    let netlist = spectre("t\n.meas tran vmax MAX V(out)\n.tran 1u 1m\n.end\n").unwrap();
    assert!(
        netlist
            .contains("simulator lang=spice\n.meas tran vmax MAX V(out)\nsimulator lang=spectre"),
        "{netlist}",
    );
}

/// `[REF]` p.236 verbatim `myopt options temp=27`; options parameters
/// `reltol` (#1), `vabstol` (#3), `iabstol` (#4), `gmin` (#128),
/// `dcmaxiters` (#56, SPICE's `itl1`). The p.237-240 parameter index has no
/// plain `maxiters`.
#[test]
fn spectre_options_use_documented_parameter_names() {
    let deck = spectre(
        "t\n.options reltol=1e-4 abstol=1e-13 vntol=1e-7 gmin=1e-13 max_iterations=150\n.op\n.end\n",
    )
    .unwrap();
    let s = deck.lines().find(|l| l.starts_with("myopts")).unwrap();
    assert_eq!(
        s,
        "myopts options reltol=100u iabstol=100f vabstol=100n gmin=100f dcmaxiters=150",
    );
    assert!(
        !s.contains(" maxiters="),
        "maxiters is a dc parameter, not an option: {s}"
    );
}
