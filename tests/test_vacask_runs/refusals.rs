//! What VACASK cannot do (measures, PWL, behavioural sources, switches,
//! transmission lines, XSPICE, ...) -> a refusal code, never a broken deck.
//!
//! Cites: real VACASK, for what it refuses or runs wrong.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

#[test]
fn refusals_are_errors_not_broken_decks() {
    let refuses = |deck: &str| emit(deck).is_err();
    let with = |card: &str| format!("c\n{card}\n.end\n");

    // No measurement statement at all: `measure` is "Command not found".
    assert!(
        refuses("m\n.meas tran vmax MAX V(out)\n.op\n.end\n"),
        "measures must refuse"
    );

    // PWL parses and then returns an identically-zero transient.
    assert!(refuses(&with("V1 a 0 0 PWL(0 0 1m 1)")), "PWL must refuse");

    // No behavioural sources, switches, transmission lines or XSPICE; a SIN
    // phase VACASK accepts and then ignores.
    for card in [
        "B1 a 0 V=v(b)*2",
        "S1 a 0 c 0 sw\n.model sw SW",
        "T1 a 0 b 0 Z0=50 TD=1n",
        "A1 a d_and\n.model d_and d_and",
        "V1 a 0 0 SIN(0 1 1k 0 0 90)",
    ] {
        assert!(refuses(&with(card)), "{card} must refuse");
    }

    // Analyses VACASK does not implement.
    for card in [".pz a 0 b 0 vol pz", ".disto dec 10 1 1k", ".sens V(out)"] {
        assert!(refuses(&rc(card)), "{card} must refuse");
    }
}
