//! ESPice: `espice deck.sp --rawfile out.raw --format=binary`.
//!
//! ESPice compiles Verilog-A through VerA at run time (`.hdl`), so a deck with
//! `veriloga()` models needs no OpenVAF. Its first load of a model builds a
//! shared library, cached under `$ESPICE_CACHE` (else `~/.cache/espice`).

use std::process::Command;

use super::{exec, read_results, scratch, BackendError};
use crate::result::RawData;

pub(super) fn run(netlist: &str) -> Result<RawData, BackendError> {
    let (dir, deck) = scratch(netlist, "deck.sp")?;
    let raw = dir.path().join("out.raw");
    let stdout = exec(
        Command::new("espice").arg(&deck).arg("--rawfile").arg(&raw).arg("--format=binary"),
        "espice",
        None,
    )?;
    read_results(&raw, stdout)
}
