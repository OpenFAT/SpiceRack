//! Cadence Spectre subprocess backend.
//!
//! Spectre is licence-gated and is not installed here, so nothing in this
//! file has run against the real binary. Flags come from the *Spectre Circuit
//! Simulator Reference*, Product Version 19.1 (`[REF]`):
//! p.26 `-raw <dir>` saves results there; p.26 `-format nutbin` is the binary
//! Nutmeg format `crate::rawfile` parses; p.494 a `.scs` file is read in
//! Spectre input mode without a `simulator lang=spectre` line.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{exec, read_results, scratch, sorted_files, BackendError};
use crate::result::RawData;

pub(super) fn run(netlist: &str) -> Result<RawData, BackendError> {
    let (dir, scs) = scratch(netlist, "circuit.scs")?;
    let raw_dir = dir.path().join("raw");
    std::fs::create_dir(&raw_dir)?;
    let stdout = exec(
        Command::new("spectre").args(["-format", "nutbin", "-raw"]).arg(&raw_dir).arg(&scs),
        "spectre",
        None,
    )?;
    match find_output_file(&raw_dir) {
        Some(out) => read_results(&out, stdout),
        None => Err(BackendError::SimulationError("No output file produced by spectre".into())),
    }
}

/// The result file in Spectre's raw directory.
///
/// **Not documented** by Cadence: the layout under `-raw <dir>` — a `logFile`
/// plus one file per analysis whose extension is the analysis type — comes
/// from third-party write-ups (analogflavor.com, "The PSF and PSFXL file
/// structure"). A file named after an analysis type wins; otherwise the first
/// non-log file, here or in a `psf/` subdirectory (the Virtuoso layout).
/// Nutmeg vs PSF is sniffed from content by `read_results`, not the extension.
fn find_output_file(dir: &Path) -> Option<PathBuf> {
    const ANALYSIS_EXT: [&str; 14] = ["ac", "dc", "noise", "op", "pac", "pnoise", "pss", "pstb", "pxf", "raw", "sp", "stb", "tran", "xf"];
    let files: Vec<PathBuf> = sorted_files(dir).into_iter().chain(sorted_files(&dir.join("psf")))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n != "logFile" && !n.ends_with(".log")))
        .collect();
    let typed = files.iter().find(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| ANALYSIS_EXT.contains(&e)));
    typed.or(files.first()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_discovery_is_deterministic_and_prefers_analysis_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("logFile"), b"log").unwrap();
        std::fs::write(dir.path().join("tran1.tran"), b"Title: x\n").unwrap();
        std::fs::write(dir.path().join("ac1.ac"), b"Title: x\n").unwrap();
        assert_eq!(find_output_file(dir.path()).unwrap().file_name().unwrap(), "ac1.ac");
    }

    #[test]
    fn a_lone_log_is_no_output() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("logFile"), b"log").unwrap();
        assert!(find_output_file(dir.path()).is_none());
    }
}
