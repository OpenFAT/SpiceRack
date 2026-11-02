//! ngspice: `ngspice -b` as a subprocess.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use super::{exec, read_results, scratch, BackendError};
use crate::codegen::spice3::{is_four, is_meas, runs_in_control, NGBEHAVIOR};
use crate::result::RawData;

/// Run a deck through `ngspice -b` and read back every plot of its rawfile.
///
/// A deck whose `.control` block runs the analysis writes nothing under
/// `-b -r rawfile`, so it gets a block appended that writes every plot the
/// run made (the codegen puts a deck with `.measure` or `.four` in that
/// shape, and a hand-written one may be). This is run mechanics (where the
/// rawfile goes), not dialect translation.
///
/// ngspice runs with `-n`, so neither `./.spiceinit` nor `~/.spiceinit` is
/// read: the result depends on the deck alone. A deck's [`NGBEHAVIOR`] line
/// becomes `-D ngbehavior=<mode>`, the one way to set the mode before the
/// deck is parsed without a file.
///
/// # Errors
/// `SimulationError` for a deck with a `.meas` or `.four` card and no
/// `.control` block that runs its analysis: under `-r`, ngspice would drop
/// those results and exit 0. `SimulationError` when ngspice reports no
/// compatibility mode for a mode the deck asked for (ngspice itself only
/// notes it and reads the deck as plain ngspice).
pub(super) fn run(netlist: &str) -> Result<Vec<RawData>, BackendError> {
    if !runs_in_control(netlist.lines()) && let Some(card) = netlist.lines().find(|l| is_meas(l) || is_four(l)) {
        return Err(BackendError::SimulationError(format!(
            "'{}': ngspice drops .meas and .four results when it writes a rawfile (-r). \
             Run the analysis (ac, tran or dc) from a .control block, which the ngspice \
             netlist SpiceRack writes does, or remove the card.",
            card.trim()
        )));
    }
    let (dir, cir) = scratch(netlist, "deck.cir")?;
    let raw_path = dir.path().join("out.raw");
    let mut cmd = Command::new("ngspice");
    cmd.args(["-b", "-n"]);
    let mode = ngbehavior(netlist);
    if let Some(mode) = mode {
        cmd.arg("-D").arg(format!("ngbehavior={mode}"));
    }
    match append_write_block(netlist, &raw_path) {
        Some(deck) => std::fs::write(&cir, deck)?,
        None => {
            cmd.arg("-r").arg(&raw_path);
        }
    }
    let stdout = exec(cmd.arg(&cir), "ngspice", None)?;
    if let Some(mode) = mode
        && stdout.contains("No compatibility mode selected")
    {
        return Err(BackendError::SimulationError(format!(
            "ngspice did not accept ngbehavior={mode}: it reports no compatibility mode selected"
        )));
    }
    read_results(&raw_path, stdout)
}

/// The compatibility mode on the deck's [`NGBEHAVIOR`] line, if it has one.
fn ngbehavior(netlist: &str) -> Option<&str> {
    netlist
        .lines()
        .find_map(|l| l.strip_prefix(NGBEHAVIOR))
        .map(str::trim)
}

/// Give a `.control` deck somewhere to put its results.
///
/// A deck whose analysis lives in a `.control` block writes nothing under
/// `-b -r rawfile`: ngspice reserves the file for dot-card analyses, the
/// control block runs, and the rawfile is never created — the backend then
/// fails with "Failed to read raw file". A bare `write` keeps only the
/// current plot, which loses every analysis but the last (and `noise`'s
/// spectra). So the appended block writes each plot in `$plots` but `const`,
/// in run order, appending to the one file (ngspice manual §13.5.107,
/// `appendwrite`; `strcmp`, §13.5.92). Returns `None` when the deck has no
/// control block that runs anything, or already writes.
fn append_write_block(netlist: &str, raw_path: &Path) -> Option<String> {
    fn first(l: &str) -> &str {
        l.split_whitespace().next().unwrap_or("")
    }
    if !runs_in_control(netlist.lines()) || netlist.lines().any(|l| first(l).eq_ignore_ascii_case("write")) {
        return None;
    }
    let block = format!(
        ".control\nset appendwrite\nforeach p $plots\nstrcmp _sr_const $p const\nif $_sr_const ne 0\nsetplot $p\nwrite {}\nend\nend\n.endc\n",
        raw_path.display()
    );
    let mut out = String::with_capacity(netlist.len() + block.len() + 1);
    let mut inserted = false;
    for line in netlist.lines() {
        if !inserted && first(line).eq_ignore_ascii_case(".end") {
            out.push_str(&block);
            inserted = true;
        }
        writeln!(out, "{line}").expect("writing to a String cannot fail");
    }
    if !inserted {
        out.push_str(&block);
    }
    Some(out)
}

#[cfg(test)]
mod deck_tests {
    use super::append_write_block;
    use std::path::Path;

    const RAW: &str = "/tmp/out.raw";

    /// Every plot but `const`, appended to the one raw file.
    const WRITE: &str = ".control\nset appendwrite\nforeach p $plots\nstrcmp _sr_const $p const\nif $_sr_const ne 0\nsetplot $p\nwrite /tmp/out.raw\nend\nend\n.endc\n";

    #[test]
    fn appends_a_write_block_to_a_hand_written_control_deck() {
        let deck = "* t\nV1 a 0 3\n.control\nop\n.endc\n.end\n";
        let out = append_write_block(deck, Path::new(RAW)).expect("should append");
        assert_eq!(out, format!("* t\nV1 a 0 3\n.control\nop\n.endc\n{WRITE}.end\n"));
    }

    #[test]
    fn appends_after_a_deck_with_no_end_card() {
        let out = append_write_block("* t\n.control\nrun\n.endc", Path::new(RAW)).expect("should append");
        assert_eq!(out, format!("* t\n.control\nrun\n.endc\n{WRITE}"));
    }

    #[test]
    fn leaves_a_control_deck_that_already_writes_alone() {
        let deck = "* t\n.control\nop\nWRITE mine.raw\n.endc\n.end\n";
        assert!(append_write_block(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn ngbehavior_reaches_the_ngspice_run_and_no_other_dialect() {
        use crate::codegen::{Dialect, netlist};
        use crate::ir::{Analysis, CircuitIR, Testbench};
        // One analysis, and two with a measure: the plain and the `.control` deck shapes.
        let bench = |analyses, measures| CircuitIR {
            testbench: Some(Testbench {
                analyses,
                measures,
                ngbehavior: Some("hsa".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let tran = Analysis::Transient { step: 1e-6, stop: 1e-3, start: None, max_step: None, uic: false };
        for ir in [bench(vec![Analysis::Op], vec![]), bench(vec![Analysis::Op, tran], vec!["tran m max v(a)".into()])] {
            let deck = netlist(&ir, Dialect::Ngspice).unwrap();
            assert_eq!(super::ngbehavior(&deck), Some("hsa"), "{deck}");
            for d in [Dialect::Espice, Dialect::Ltspice, Dialect::Hspice] {
                let deck = netlist(&ir, d).unwrap();
                assert_eq!(super::ngbehavior(&deck), None, "{}: {deck}", d.name());
            }
        }
        let plain = netlist(&CircuitIR::default(), Dialect::Ngspice).unwrap();
        assert_eq!(super::ngbehavior(&plain), None, "{plain}");
    }

    #[test]
    fn does_not_append_to_a_plain_dot_card_deck() {
        let deck = "* t\nV1 a 0 3\n.op\n.end\n";
        assert!(append_write_block(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn does_not_append_to_a_block_that_runs_nothing() {
        // The codegen's `pre_osdi` block loads models and runs no analysis.
        let deck = "* t\n.control\npre_osdi /m.osdi\n.endc\nR1 a 0 1k\n.op\n.end\n";
        assert!(append_write_block(deck, Path::new(RAW)).is_none());
    }
}
