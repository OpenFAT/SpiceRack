//! LTspice: `LTspice -b deck.cir`, natively or under Wine.
//!
//! UNVERIFIED: no LTspice binary is installed here, so every claim in this
//! file comes from LTspice's documented syntax, not from a run.

use std::path::PathBuf;
use std::process::Command;

use super::{exec, is_on_path, read_results, scratch, BackendError};
use crate::result::RawData;

pub(super) fn run(netlist: &str) -> Result<RawData, BackendError> {
    let (exe, wine) = find().ok_or_else(|| BackendError::SimulationError("LTspice not found on this system".into()))?;
    // `plotwinsize=0` disables waveform compression (lossy decimation) and
    // `numdgt=15` switches the raw file to double precision. Raw-file format
    // is run mechanics, so it is set here rather than in the dialect. Both go
    // after the title: line 1 of a SPICE deck is a comment.
    let (title, body) = netlist.split_once('\n').unwrap_or((netlist, ""));
    let title = if title.is_empty() { "* spicerack" } else { title };
    let deck = format!("{title}\n.options plotwinsize=0\n.options numdgt=15\n{body}");
    let (_dir, cir) = scratch(&deck, "deck.cir")?;

    // `-b` is the headless switch for a netlist; it writes deck.raw and
    // deck.log beside the input.
    let mut cmd = if wine {
        let mut c = Command::new("wine");
        // UNVERIFIED: `-wine` is the LTspice XVII switch for the Windows build under Wine.
        c.arg(&exe).arg("-wine");
        c
    } else {
        Command::new(&exe)
    };
    cmd.arg("-b").arg(&cir);
    // LTspice reports simulation errors (and `.meas` results) in the log.
    let stdout = exec(&mut cmd, "LTspice", Some(&cir.with_extension("log")))?;
    let log = std::fs::read_to_string(cir.with_extension("log")).unwrap_or_default();
    let mut raw = read_results(&cir.with_extension("raw"), stdout)?;
    raw.log_content = log;
    Ok(raw)
}

/// The LTspice executable, and whether it needs Wine.
pub fn find() -> Option<(PathBuf, bool)> {
    if is_on_path("ltspice") {
        return Some((PathBuf::from("ltspice"), false));
    }
    let mac = PathBuf::from("/Applications/LTspice.app/Contents/MacOS/LTspice");
    if mac.exists() {
        return Some((mac, false));
    }
    #[cfg(target_os = "windows")]
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(format!("{local}/Programs/ADI/LTspice/LTspice.exe"));
        if p.exists() {
            return Some((p, false));
        }
    }
    let home = std::env::var("HOME").ok()?;
    let user = std::env::var("USER").unwrap_or_default();
    [
        format!("{home}/.wine/drive_c/users/{user}/AppData/Local/Programs/ADI/LTspice/LTspice.exe"),
        format!("{home}/.wine/drive_c/Program Files/ADI/LTspice/LTspice.exe"),
        format!("{home}/.wine/drive_c/Program Files/LTC/LTspiceXVII/XVIIx64.exe"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
    .filter(|_| is_on_path("wine"))
    .map(|p| (p, true))
}
