//! VACASK: run `vacask` on a native deck, read back the per-analysis `.raw`.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{exec, read_results, scratch, sorted_files, BackendError};
use crate::result::RawData;

pub(super) fn run(netlist: &str) -> Result<RawData, BackendError> {
    // The deck runs in a scratch dir so the per-analysis raw files land
    // somewhere disposable, which moves any relative `load`/`include` path
    // out from under the user's cwd. Resolve those before writing.
    let (dir, sim) = scratch(&absolutize_paths(netlist), "circuit.sim")?;
    // -se/-sp: a deck may carry `embed`/`postprocess` steps, which write
    // files and shell out to Python. Neither produces a raw file.
    let stdout = exec(Command::new("vacask").args(["-se", "-sp"]).arg(&sim).current_dir(dir.path()), "vacask", None)?;
    // A failed analysis can exit 0 with nothing written, so the raw file is
    // the real success test.
    match raw_file_for(dir.path(), netlist) {
        Some(raw) => read_results(&raw, stdout),
        None => Err(BackendError::SimulationError(format!("vacask produced no .raw output\nstdout: {}", super::tail(&stdout)))),
    }
}

/// Analysis names declared by a deck, in order: `analysis <name> <kind> ...`.
fn analysis_names(netlist: &str) -> impl DoubleEndedIterator<Item = &str> {
    netlist.lines().filter_map(|l| {
        let mut w = l.split_whitespace();
        (w.next()? == "analysis").then(|| w.next())?
    })
}

/// VACASK writes one raw file per analysis, named after it. Take the last
/// declared analysis that produced one — the one the caller asked for in the
/// single-analysis path — else the last `.raw` in name order.
fn raw_file_for(dir: &Path, netlist: &str) -> Option<PathBuf> {
    analysis_names(netlist).rev().map(|n| dir.join(format!("{n}.raw"))).find(|p| p.is_file())
        .or_else(|| sorted_files(dir).into_iter().rfind(|p| p.extension().is_some_and(|e| e == "raw")))
}

/// Rewrite relative `load "x"` / `include "x"` paths that exist under the
/// caller's cwd into absolute ones. A bare module name (`spice/diode.osdi`)
/// resolves through VACASK's own module path and is left alone.
fn absolutize_paths(netlist: &str) -> String {
    let Ok(cwd) = std::env::current_dir() else { return netlist.to_string() };
    let mut out = String::with_capacity(netlist.len());
    for line in netlist.lines() {
        let t = line.trim_start();
        let quoted = (t.starts_with("load \"") || t.starts_with("include \""))
            .then(|| line.split('"').nth(1))
            .flatten()
            .filter(|p| !Path::new(p).is_absolute() && cwd.join(p).is_file());
        match quoted {
            Some(p) => out.push_str(&line.replacen(p, &cwd.join(p).display().to_string(), 1)),
            None => out.push_str(line),
        }
        // The trailing newline is load-bearing: VACASK's `endc` wants one.
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_names_are_read_off_the_deck_in_order() {
        let deck = "x\ncontrol\n  analysis op1 op\n  sweep s instance=\"v1\" parameter=\"dc\"\n    analysis dc1 op\nendc\n";
        assert_eq!(analysis_names(deck).collect::<Vec<_>>(), vec!["op1", "dc1"]);
    }

    #[test]
    fn raw_file_is_the_last_declared_analysis() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("op1.raw"), "").unwrap();
        std::fs::write(dir.path().join("zz.raw"), "").unwrap();
        let deck = "analysis op1 op\nanalysis tr1 tran\n";
        assert_eq!(raw_file_for(dir.path(), deck).unwrap().file_name().unwrap(), "op1.raw");
        assert_eq!(raw_file_for(dir.path(), "").unwrap().file_name().unwrap(), "zz.raw");
    }
}
