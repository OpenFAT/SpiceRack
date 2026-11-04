//! LTspice dialect audit.
//!
//! UNVERIFIED BY EXECUTION: no LTspice binary exists on this machine (wine is
//! present, LTspice is not). Every assertion here checks the emitted text
//! against LTspice's *documented* syntax. Nothing below was run through
//! LTspice.
//!
//! Cites: the LTspice help, per test.
//!
//! Contents: none (tests only).

use spicerack::syntax::Dialect;
use spicerack::syntax::diag::Code;
use spicerack::syntax::render::{Diags, render_text};

/// One target, probed one piece at a time through a minimal deck.
struct Cg(Dialect);

/// A noise deck that saves `V(out)`.
const NOISE: &str = "* nz\nR1 in out 10k\n.save V(out)\n.noise V(out) V1 dec 10 1 1meg\n.end\n";

// ---- private ----

fn lt() -> Cg {
    Cg(Dialect::Ltspice)
}

fn ng() -> Cg {
    Cg(Dialect::Ngspice)
}

impl Cg {
    /// `deck`, read as `from`, written for this target.
    fn emit_netlist(&self, deck: &str, from: Dialect) -> Result<String, Diags> {
        render_text(deck, from, "t", self.0).map(|w| w.text)
    }

    /// The deck of `cards` (read as `from`) minus its `* t` title and
    /// `.end`.
    fn body(&self, cards: &str, from: Dialect) -> Result<String, Diags> {
        let n = self.emit_netlist(&format!("t\n{cards}\n.end\n"), from)?;
        Ok(n.strip_prefix("* t\n")
            .unwrap()
            .strip_suffix("\n.end")
            .unwrap()
            .to_string())
    }

    /// The last line of an ngspice deck holding `card`: ngspice puts
    /// `.save all` ahead of a noise card.
    fn emit_analysis(&self, card: &str) -> Result<String, Diags> {
        let b = self.body(card, Dialect::Ngspice)?;
        Ok(b.lines().last().unwrap().to_string())
    }
}

// ── .step ──

/// LTspice: `.step [lin|oct|dec] param <name> <start> <stop> <inc>`.
/// The `param` keyword is required in the lin/oct/dec form too (an old
/// writer dropped it, producing `.step dec rval ...`, which LTspice
/// rejects). ngspice has no `.step`, so the cards are LTspice's own.
#[test]
fn step_keeps_the_param_keyword_in_every_form() {
    for sweep in ["lin", "oct", "dec"] {
        let card = format!(".step {sweep} param rval 1000 10000 10");
        let emitted = lt().body(&card, Dialect::Ltspice).unwrap();
        assert_eq!(emitted, card);
    }
}

/// ngspice has no `.step`: a stepped LTspice deck is refused (`W0016`, whose
/// citation names `.dc` sweeps), never written commented out, which would
/// run once.
#[test]
fn ngspice_refuses_step_instead_of_commenting_it_out() {
    let e = ng()
        .body(".step param rval 1000 10000 1000", Dialect::Ltspice)
        .unwrap_err();
    assert!(
        e.diags.iter().any(|d| d.code == Code::W0016),
        "{}",
        e.messages
    );
    let cite = spicerack::syntax::diag::info(Code::W0016).cite;
    assert!(cite.contains(".dc"), "the working alternative: {cite}");
}

// ── .four ──

/// LTspice's `.four` takes an optional harmonic count; SPICE3/ngspice's does
/// not (ngspice reads the count as an output vector and fails the analysis).
#[test]
fn four_harmonic_count_is_ltspice_only() {
    let four = ".four 1k 9 V(out)";
    assert_eq!(lt().body(four, Dialect::Ltspice).unwrap(), four);
    assert_eq!(
        ng().body(".four 1k V(out)", Dialect::Ngspice).unwrap(),
        ".four 1k V(out)"
    );
}

// ── .noise ──

/// `pts_per_summary` is a SPICE3/ngspice extension; LTspice's `.noise` card
/// ends at the stop frequency.
#[test]
fn noise_points_per_summary_is_ngspice_only() {
    let noise = ".noise V(out) V1 dec 10 1 1meg 5";
    assert_eq!(
        lt().emit_analysis(noise).unwrap(),
        ".noise V(out) V1 dec 10 1 1meg"
    );
    assert_eq!(ng().emit_analysis(noise).unwrap(), noise);
}

// ── analyses LTspice does not have ──

#[test]
fn pz_disto_and_sens_are_rejected_for_ltspice() {
    for a in [
        ".pz 1 0 3 0 vol pz",
        ".disto dec 10 1k 1meg",
        ".sens V(out)",
    ] {
        assert!(lt().emit_analysis(a).is_err(), "ltspice must reject {a}");
        assert!(ng().emit_analysis(a).is_ok(), "ngspice must accept {a}");
    }
}

// ── .save ──

/// A node-name save list cannot name the noise output vectors, so it must not
/// be emitted. ngspice needs an explicit `.save all` to override; LTspice has
/// no `.save all` and saves everything by default, so it gets no save line.
#[test]
fn noise_never_emits_a_node_save_list() {
    let ng_netlist = ng().emit_netlist(NOISE, Dialect::Ngspice).unwrap();
    assert!(ng_netlist.contains(".save all"), "{ng_netlist}");
    assert!(!ng_netlist.contains(".save V(out)"), "{ng_netlist}");

    let lt_netlist = lt().emit_netlist(NOISE, Dialect::Ngspice).unwrap();
    assert!(
        !lt_netlist.contains(".save"),
        "ltspice has no .save all: {lt_netlist}"
    );
}

// ── XSPICE and OSDI are ngspice-only ──

/// LTspice has no XSPICE `A` element: an ngspice deck's (kept as ngspice
/// text) is refused (`W0001`), never dropped or commented out; ngspice
/// copies it.
#[test]
fn xspice_is_refused_for_ltspice() {
    let a = "A1 in out gainblk\n.model gainblk gain(gain=2.0)";
    let e = lt().body(a, Dialect::Ngspice).unwrap_err();
    assert!(
        e.diags.iter().any(|d| d.code == Code::W0001),
        "{}",
        e.messages
    );
    assert!(
        ng().body(a, Dialect::Ngspice)
            .unwrap()
            .contains("A1 in out gainblk")
    );
}

// ── options ──

#[test]
fn ltspice_uses_the_spice3_option_spellings() {
    let s = lt()
        .body(
            ".options reltol=1e-4 abstol=1e-12 max_iterations=200",
            Dialect::Ngspice,
        )
        .unwrap();
    assert!(s.contains("reltol=1e-4"), "{s}");
    assert!(s.contains("abstol=1e-12"), "{s}");
    // LTspice documents itl1/itl2/itl4 like SPICE3.
    assert!(s.contains("ITL1=200"), "{s}");
}
