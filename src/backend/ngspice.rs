//! ngspice: `ngspice -b` as a subprocess.

use std::process::Command;

use super::{exec, read_results, scratch, BackendError};
use crate::result::RawData;

/// Run a deck through `ngspice -b` and read back its rawfile.
///
/// ngspice refuses `.measure` (and ignores `.four`) under `-b -r rawfile`, so
/// such a deck has its analysis moved into a `.control` block that writes the
/// rawfile itself; a deck that already owns a control block gets a `write`
/// appended, or it produces no rawfile at all. This is run mechanics (where
/// the rawfile goes), not dialect translation.
pub(super) fn run(netlist: &str) -> Result<RawData, BackendError> {
    let (dir, cir) = scratch(netlist, "deck.cir")?;
    let raw_path = dir.path().join("out.raw");
    let rewritten = control_block_deck(netlist, &raw_path).or_else(|| append_write_block(netlist, &raw_path));
    let mut cmd = Command::new("ngspice");
    cmd.arg("-b");
    match rewritten {
        Some(deck) => std::fs::write(&cir, deck)?,
        None => {
            cmd.arg("-r").arg(&raw_path);
        }
    }
    let stdout = exec(cmd.arg(&cir), "ngspice", None)?;
    read_results(&raw_path, stdout)
}

/// Analyses worth moving into a `.control` block.
///
/// Deliberately only the three that ngspice's `.measure` supports (it accepts
/// DC, AC, TRAN and SP). Rewriting anything else is pure downside: `noise`,
/// `disto` and `pz` emit *several* plots, and the `write` command captures
/// only the current one — for `.noise` that is "Integrated Noise", which would
/// silently drop the spectra that callers actually read.
const ANALYSIS_CARDS: [&str; 3] = ["ac", "tran", "dc"];

/// Control commands that mean an existing `.control` block owns the run.
///
/// The codegen emits its own `.control` block for `pre_osdi` loads, and that
/// block runs nothing — bailing on *any* `.control` would mean OSDI decks can
/// never report `.measure` results. ngspice accepts several `.control` blocks
/// per deck (verified on 44.2), so only a block that actually starts an
/// analysis is off limits.
const RUN_COMMANDS: [&str; 5] = ["run", "op", "ac", "tran", "dc"];

/// Lowercased first whitespace-separated token of a line.
fn first_word(line: &str) -> String {
    line.split_whitespace().next().unwrap_or("").to_lowercase()
}

/// True if some `.control`/`.endc` block in the deck issues a run command.
fn has_owning_control_block(netlist: &str) -> bool {
    let mut inside = false;
    for line in netlist.lines() {
        match first_word(line).as_str() {
            ".control" => inside = true,
            ".endc" => inside = false,
            word if inside && RUN_COMMANDS.contains(&word) => return true,
            _ => {}
        }
    }
    false
}

/// Split a `.four` card into `(fundamental, outputs)`.
///
/// SPICE3 syntax is `.four <freq> <ov1> [ov2 ...]`; there is no harmonic-count
/// field, and the codegen no longer emits one for this dialect.
fn parse_four_card(line: &str) -> Option<(String, Vec<String>)> {
    let mut parts = line.split_whitespace();
    if parts.next()?.to_lowercase() != ".four" {
        return None;
    }
    let fundamental = parts.next()?.to_string();
    let outputs: Vec<String> = parts.map(str::to_string).collect();
    if outputs.is_empty() {
        return None;
    }
    Some((fundamental, outputs))
}

/// Rewrite a deck so `.measure` and `.four` results reach stdout.
///
/// ngspice refuses both under `-b -r rawfile`:
///   - "No .measure possible in batch mode (-b) with -r rawfile set!"
///   - ".fourier line ignored since rawfile was produced."
///
/// The cure is the same for both: move the analysis card into a `.control`
/// block that writes the rawfile itself. Inside the block `.four` has to become
/// the `fourier` command — there is no `four` command ("four: no such command
/// available in ngspice").
///
/// Returns `None` when the deck needs no rewrite (nothing to recover, no
/// analysis card found, or a control block already runs the simulation).
fn control_block_deck(netlist: &str, raw_path: &std::path::Path) -> Option<String> {
    let has_measure = netlist.lines().any(|l| first_word(l).starts_with(".meas"));
    let four = netlist.lines().find_map(parse_four_card);
    if !has_measure && four.is_none() {
        return None;
    }
    if has_owning_control_block(netlist) {
        return None;
    }

    let analysis_idx = netlist.lines().position(|l| {
        let word = first_word(l);
        ANALYSIS_CARDS.iter().any(|c| word == format!(".{}", c))
    })?;

    let mut out = Vec::new();
    for (i, line) in netlist.lines().enumerate() {
        if i == analysis_idx {
            // `.ac dec 10 1 1k` -> bare `ac dec 10 1 1k` inside the block
            out.push(".control".to_string());
            out.push(line.trim_start().trim_start_matches('.').to_string());
            if let Some((fundamental, outputs)) = &four {
                out.push(format!("fourier {} {}", fundamental, outputs.join(" ")));
            }
            out.push(format!("write {}", raw_path.display()));
            out.push(".endc".to_string());
        } else if four.is_some() && first_word(line) == ".four" {
            // Leaving the dot-card in place makes ngspice run the analysis twice.
            continue;
        } else {
            out.push(line.to_string());
        }
    }
    out.push(String::new());
    Some(out.join("\n"))
}

/// Give a hand-written `.control` deck somewhere to put its results.
///
/// A deck whose analysis lives in a `.control` block writes nothing under
/// `-b -r rawfile`: ngspice reserves the file for dot-card analyses, the
/// control block runs, and the rawfile is never created — the backend then
/// fails with "Failed to read raw file". Appending a second control block that
/// just does `write` captures the current plot. Returns `None` when the deck
/// has no control block of its own or already writes one.
fn append_write_block(netlist: &str, raw_path: &std::path::Path) -> Option<String> {
    if !has_owning_control_block(netlist) {
        return None;
    }
    if netlist.lines().any(|l| first_word(l) == "write") {
        return None;
    }

    let block = format!(".control\nwrite {}\n.endc", raw_path.display());
    let mut out: Vec<String> = Vec::new();
    let mut inserted = false;
    for line in netlist.lines() {
        if !inserted && first_word(line) == ".end" {
            out.push(block.clone());
            inserted = true;
        }
        out.push(line.to_string());
    }
    if !inserted {
        out.push(block);
    }
    out.push(String::new());
    Some(out.join("\n"))
}

#[cfg(test)]
mod deck_tests {
    use super::control_block_deck;
    use std::path::Path;

    const RAW: &str = "/tmp/out.raw";

    #[test]
    fn rewrites_ac_deck_when_measures_present() {
        let deck = "* t\nR1 a b 1k\n.meas ac g find vdb(b) at=100\n.ac dec 10 1 1k\n.end\n";
        let out = control_block_deck(deck, Path::new(RAW)).expect("should rewrite");
        assert!(out.contains(".control\nac dec 10 1 1k\nwrite /tmp/out.raw\n.endc"));
        // The dot-card must be gone, or ngspice runs the analysis twice.
        assert!(!out.contains(".ac dec"));
    }

    #[test]
    fn leaves_deck_alone_without_measures() {
        let deck = "* t\nR1 a b 1k\n.ac dec 10 1 1k\n.end\n";
        assert!(control_block_deck(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn never_rewrites_noise_which_emits_multiple_plots() {
        // `write` would capture only "Integrated Noise" and drop the spectra.
        let deck = "* t\nR1 a b 1k\n.meas tran x find v(b) at=1\n.noise v(b) V1 dec 10 1 1k\n.end\n";
        let out = control_block_deck(deck, Path::new(RAW));
        assert!(out.is_none(), "noise must keep the -r path, got: {:?}", out);
    }

    #[test]
    fn defers_to_a_hand_written_control_block() {
        let deck = "* t\n.meas ac g find vdb(b) at=100\n.control\nrun\n.endc\n.ac dec 10 1 1k\n.end\n";
        assert!(control_block_deck(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn rewrites_around_an_osdi_only_control_block() {
        // The codegen's own pre_osdi block runs nothing, so it must not veto the
        // rewrite — otherwise OSDI decks can never report measures.
        let deck = "* t\n.control\npre_osdi /m.osdi\n.endc\nR1 a b 1k\n\
                    .meas tran x find v(b) at=1\n.tran 1n 1u\n.end\n";
        let out = control_block_deck(deck, Path::new(RAW)).expect("should rewrite");
        assert!(out.contains("pre_osdi /m.osdi"), "osdi block kept: {}", out);
        assert!(out.contains("tran 1n 1u\nwrite /tmp/out.raw"), "rewritten: {}", out);
    }

    #[test]
    fn routes_four_through_the_fourier_command() {
        // `.four` is ignored under `-b -r`, and there is no `four` command
        // inside a control block — it has to become `fourier`.
        let deck = "* t\nV1 a 0 SIN(0 1 1k)\n.tran 1u 10m\n.four 1k v(a)\n.end\n";
        let out = control_block_deck(deck, Path::new(RAW)).expect("should rewrite");
        assert!(out.contains("tran 1u 10m\nfourier 1k v(a)\n"), "order: {}", out);
        assert!(!out.contains(".four "), "dot-card removed: {}", out);
    }

    #[test]
    fn four_without_a_transient_card_is_left_alone() {
        let deck = "* t\n.four 1k v(a)\n.end\n";
        assert!(control_block_deck(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn appends_a_write_block_to_a_hand_written_control_deck() {
        let deck = "* t\nV1 a 0 3\n.control\nop\n.endc\n.end\n";
        let out = super::append_write_block(deck, Path::new(RAW)).expect("should append");
        assert!(out.contains(".control\nwrite /tmp/out.raw\n.endc\n.end"), "{}", out);
    }

    #[test]
    fn leaves_a_control_deck_that_already_writes_alone() {
        let deck = "* t\n.control\nop\nwrite mine.raw\n.endc\n.end\n";
        assert!(super::append_write_block(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn does_not_append_to_a_plain_dot_card_deck() {
        let deck = "* t\nV1 a 0 3\n.op\n.end\n";
        assert!(super::append_write_block(deck, Path::new(RAW)).is_none());
    }
}
