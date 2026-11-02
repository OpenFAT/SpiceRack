//! VACASK: run `vacask` on a native deck, read back every per-analysis `.raw`.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{exec, read_results, scratch, sorted_files, BackendError};
use crate::result::RawData;

pub(super) fn run(netlist: &str) -> Result<Vec<RawData>, BackendError> {
    // The deck runs in a scratch dir so the per-analysis raw files land
    // somewhere disposable, which moves any relative `load`/`include` path
    // out from under the user's cwd. Resolve those before writing.
    let (dir, sim) = scratch(&absolutize_paths(netlist), "circuit.sim")?;
    // -se/-sp: a deck may carry `embed`/`postprocess` steps, which write
    // files and shell out to Python. Neither produces a raw file.
    let stdout = exec(Command::new("vacask").args(["-se", "-sp"]).arg(&sim).current_dir(dir.path()), "vacask", None)?;
    // A failed analysis can exit 0 with nothing written, so each analysis's
    // raw file is the real success test.
    let files = raw_files_for(dir.path(), netlist).map_err(|missing| {
        BackendError::SimulationError(format!("vacask wrote no {missing}\nstdout: {}", super::tail(&stdout)))
    })?;
    let mut plots = Vec::with_capacity(files.len());
    for raw in &files {
        plots.extend(read_results(raw, stdout.clone())?);
    }
    assert!(plots.len() >= files.len(), "every raw file read holds at least one plot");
    Ok(plots)
}

/// Analysis names declared by a deck, in order: `analysis <name> <kind> ...`.
fn analysis_names(netlist: &str) -> impl Iterator<Item = &str> {
    netlist.lines().filter_map(|l| {
        let mut w = l.split_whitespace();
        (w.next()? == "analysis").then(|| w.next())?
    })
}

/// VACASK writes one raw file per analysis, named after it: each declared
/// analysis's, in deck order; for a deck that declares none this can see,
/// every `.raw` in name order. `Err` names the first declared analysis's
/// file that is missing, or `.raw` when nothing was written.
fn raw_files_for(dir: &Path, netlist: &str) -> Result<Vec<PathBuf>, String> {
    let declared: Vec<PathBuf> = analysis_names(netlist).map(|n| dir.join(format!("{n}.raw"))).collect();
    if let Some(missing) = declared.iter().find(|p| !p.is_file()) {
        return Err(missing.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned()));
    }
    if !declared.is_empty() {
        return Ok(declared);
    }
    let all: Vec<PathBuf> = sorted_files(dir).into_iter().filter(|p| p.extension().is_some_and(|e| e == "raw")).collect();
    if all.is_empty() { Err(".raw".into()) } else { Ok(all) }
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
    fn raw_files_are_every_declared_analysis_in_deck_order() {
        let dir = tempfile::TempDir::new().unwrap();
        let names = |r: Vec<PathBuf>| r.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        for f in ["tr1.raw", "op1.raw", "zz.raw"] {
            std::fs::write(dir.path().join(f), "").unwrap();
        }
        assert_eq!(names(raw_files_for(dir.path(), "analysis tr1 tran\nanalysis op1 op\n").unwrap()), ["tr1.raw", "op1.raw"]);
        assert_eq!(names(raw_files_for(dir.path(), "").unwrap()), ["op1.raw", "tr1.raw", "zz.raw"]);
        assert_eq!(raw_files_for(dir.path(), "analysis op1 op\nanalysis ac1 ac\n"), Err("ac1.raw".into()));
        let empty = tempfile::TempDir::new().unwrap();
        assert_eq!(raw_files_for(empty.path(), ""), Err(".raw".into()));
    }
}
