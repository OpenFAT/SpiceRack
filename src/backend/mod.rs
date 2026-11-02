//! Simulator backends: the [`RUNNABLE`] dialects, one `match` per question.
//!
//! A backend executes netlist text verbatim, in its own [`Dialect`]. Turning
//! IR into that text is the codegen's job.

pub mod ngspice;
pub mod ltspice;
pub mod vacask;
pub mod spectre;
pub mod espice;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::ir::{Analysis, FeatureFlags};
use crate::rawfile;
use crate::result::RawData;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("No simulator backend found. Install ngspice: sudo apt install ngspice")]
    NoBackend,

    #[error("Analysis '{analysis}' not supported by {backend}.\n\n\
        This analysis requires {required}. Either:\n  \
        1. Install {required}\n  \
        2. Alternative: {alternative}")]
    UnsupportedAnalysis {
        analysis: String,
        backend: String,
        required: String,
        alternative: String,
    },

    #[error("Simulation failed: {0}")]
    SimulationError(String),

    #[error("Raw file parse error: {0}")]
    RawParseError(#[from] rawfile::RawFileError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub use crate::codegen::{netlist, Dialect};

/// The dialects a simulator here runs; a universal analysis runs on each, in
/// this order of preference.
pub const RUNNABLE: [Dialect; 5] = [Dialect::Ngspice, Dialect::Ltspice, Dialect::Vacask, Dialect::Spectre, Dialect::Espice];

use crate::ir::feature::*;

// What each simulator can do, as `ir::feature` bits.
const NGSPICE_CAPS: u8 = XSPICE | OSDI | MEASURES | CONTROL_BLOCKS | LAPLACE | VERILOG_COSIM;
const LTSPICE_CAPS: u8 = MEASURES | STEP_PARAMS | LAPLACE;
// VACASK: `load "x.osdi"`; no measurement statement ("Command not found");
// `var` + `sweep` does parameter sweeps; its `control` is not ngspice's.
const VACASK_CAPS: u8 = OSDI | STEP_PARAMS;
// Spectre: `.measure` per Spectre Reference 19.1 p.17; xrun co-simulation.
const SPECTRE_CAPS: u8 = OSDI | MEASURES | STEP_PARAMS | VERILOG_COSIM;
// ESPice: Verilog-A via VerA `.hdl` (counts as osdi); `.step` lands every
// point as a separate plot in one raw file, which parse_raw does not split.
// Laplace: its own `E/G … LAPLACE` card (codegen `spice3::laplace_source`).
const ESPICE_CAPS: u8 = OSDI | MEASURES | LAPLACE;

/// The dialects that run `a`, in preference order, and the workaround to
/// suggest without one. A universal analysis runs on every [`RUNNABLE`].
fn route(a: &Analysis) -> (&'static [Dialect], &'static str) {
    use Dialect::*;
    match a {
        // `tf` runs everywhere, so its old ".dc() with small-signal sweep"
        // workaround could never be shown.
        Analysis::Op | Analysis::Dc { .. } | Analysis::Ac { .. } | Analysis::Transient { .. } | Analysis::Noise { .. }
        | Analysis::Tf { .. } | Analysis::Fourier { .. } | Analysis::SpectreSweep { .. } | Analysis::SpectreMonteCarlo { .. } => {
            (&RUNNABLE, "N/A")
        }
        Analysis::PoleZero { .. } => (&[Ngspice], ".ac() + post-process for poles/zeros"),
        Analysis::Distortion { .. } => (&[Ngspice], ".transient() + FFT post-processing"),
        Analysis::Sensitivity { ac: None, .. } => (&[Ngspice, Spectre, Espice], "N/A"),
        Analysis::Sensitivity { ac: Some(_), .. } => (&[Ngspice, Spectre], "N/A"),
        Analysis::HarmonicBalance { .. } => (&[Vacask, Spectre], "long .tran() + discard startup + FFT"),
        Analysis::SPar { .. } => (&[Vacask, Spectre, Ngspice], "N/A"),
        Analysis::Stability { .. } => (&[Vacask, Spectre], ".ac() + Middlebrook method"),
        Analysis::Pss { .. } => (&[Spectre, Ngspice], "long .tran() + discard startup"),
        Analysis::TransientNoise { .. } => (&[Vacask], ".tran() with trnoise sources"),
        Analysis::SpectrePac { .. } | Analysis::SpectrePnoise { .. } | Analysis::SpectrePxf { .. } | Analysis::SpectrePstb { .. } => {
            (&[Spectre], "N/A")
        }
    }
}

/// True when `d`'s simulator has every feature in `needs` (`ir::feature`
/// bits); `CircuitIR::check_backend` reports each bit this finds missing.
pub(crate) fn runs(d: Dialect, needs: u8) -> bool {
    needs & !d.capabilities() == 0
}

/// A user-facing backend name (a [`RUNNABLE`] dialect's name or one of
/// [`Dialect::ALIASES`]) as the dialect it runs; `None` for anything else.
pub fn parse(name: &str) -> Option<Dialect> {
    Dialect::from_backend_name(name).filter(|d| RUNNABLE.contains(d))
}

impl Dialect {
    /// What this dialect's simulator can do, as `ir::feature` bits; nothing
    /// for a dialect no simulator here runs.
    pub fn capabilities(self) -> u8 {
        match self {
            Dialect::Ngspice => NGSPICE_CAPS,
            Dialect::Ltspice => LTSPICE_CAPS,
            Dialect::Vacask => VACASK_CAPS,
            Dialect::Spectre => SPECTRE_CAPS,
            Dialect::Espice => ESPICE_CAPS,
            Dialect::Hspice | Dialect::Xyce | Dialect::Cdl | Dialect::Verilog => 0,
        }
    }

    /// Executes `netlist` (already in this dialect) and reads the results.
    ///
    /// # Errors
    /// `SimulationError` for a dialect outside [`RUNNABLE`] (HSPICE, Xyce,
    /// CDL, Verilog), and whatever the simulator run returns.
    pub fn run(self, netlist: &str) -> Result<RawData, BackendError> {
        match self {
            Dialect::Ngspice => ngspice::run(netlist),
            Dialect::Ltspice => ltspice::run(netlist),
            Dialect::Vacask => vacask::run(netlist),
            Dialect::Spectre => spectre::run(netlist),
            Dialect::Espice => espice::run(netlist),
            Dialect::Hspice | Dialect::Xyce | Dialect::Cdl | Dialect::Verilog => {
                Err(BackendError::SimulationError(format!("no {} simulator is driven by SpiceRack", self.name())))
            }
        }
    }
}

/// Backends installed on this machine, in preference order. Cached.
pub fn available() -> &'static [Dialect] {
    static CACHE: OnceLock<Vec<Dialect>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let found = [
            (Dialect::Ngspice, is_on_path("ngspice")),
            (Dialect::Ltspice, ltspice::find().is_some()),
            (Dialect::Vacask, is_on_path("vacask")),
            (Dialect::Spectre, is_on_path("spectre")),
            (Dialect::Espice, is_on_path("espice")),
        ];
        found.into_iter().filter(|&(_, ok)| ok).map(|(k, _)| k).collect()
    })
}

/// Pick the backend for `analysis`. An explicit name (argument, else
/// `$SPICERACK_BACKEND`) wins if it can run the analysis; otherwise the first
/// installed backend, in `route` order, that supports the circuit's features.
pub fn select(analysis: &Analysis, override_backend: Option<&str>, features: &FeatureFlags) -> Result<Dialect, BackendError> {
    let (dialects, alternative) = route(analysis);
    let analysis = analysis.kind_str();
    let env = std::env::var("SPICERACK_BACKEND").ok().filter(|s| !s.is_empty());
    if let Some(name) = override_backend.or(env.as_deref()) {
        let kind = parse(name).ok_or_else(|| BackendError::SimulationError(format!("Unknown backend: {name}")))?;
        if !dialects.contains(&kind) {
            return Err(unsupported(analysis, name, dialects[0], alternative));
        }
        return Ok(kind);
    }

    let available = available();
    if available.is_empty() {
        return Err(BackendError::NoBackend);
    }
    for d in dialects {
        if let Some(&k) = available.iter().find(|&&k| k == *d && runs(k, features.needs)) {
            return Ok(k);
        }
    }
    if dialects[0] != Dialect::Ngspice {
        let names: Vec<_> = available.iter().map(|k| k.name()).collect();
        return Err(unsupported(analysis, &names.join(", "), dialects[0], alternative));
    }
    // ponytail: universal analysis, no backend matches the features — run on
    // the first one and let the simulator report what it cannot do.
    Ok(available[0])
}

fn unsupported(analysis: &str, backend: &str, required: Dialect, alternative: &str) -> BackendError {
    let required = required.name().to_string();
    BackendError::UnsupportedAnalysis {
        analysis: analysis.to_string(),
        backend: backend.to_string(),
        required,
        alternative: alternative.to_string(),
    }
}

// ── Subprocess plumbing shared by every backend ──

fn is_on_path(program: &str) -> bool {
    Command::new("which").arg(program).output().is_ok_and(|o| o.status.success())
}

/// A scratch directory holding `netlist` as `file_name`. The directory (and
/// everything the simulator writes into it) is deleted on drop.
fn scratch(netlist: &str, file_name: &str) -> Result<(tempfile::TempDir, PathBuf), BackendError> {
    let dir = tempfile::TempDir::new()?;
    let deck = dir.path().join(file_name);
    std::fs::write(&deck, netlist)?;
    Ok((dir, deck))
}

/// Run `cmd` and return its stdout. A nonzero exit becomes an error carrying
/// the lines that mention an error (from stdout, stderr and the `log` file
/// the simulator wrote), or the output tails when none do.
fn exec(cmd: &mut Command, program: &str, log: Option<&Path>) -> Result<String, BackendError> {
    let out = cmd.output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => BackendError::SimulationError(format!("{program} not found on $PATH")),
        _ => BackendError::Io(e),
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let log = log.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let errors: Vec<&str> = stdout.lines().chain(stderr.lines()).chain(log.lines())
        .filter(|l| l.contains("rror") || l.contains("failed"))
        .collect();
    let detail = if errors.is_empty() {
        format!("stdout: {}\nstderr: {}\nlog: {}", tail(&stdout), tail(&stderr), tail(&log))
    } else {
        errors.join("\n")
    };
    Err(BackendError::SimulationError(format!("{program} exited with {}\n{detail}", out.status)))
}

fn tail(s: &str) -> &str {
    let mut start = s.len().saturating_sub(1000);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

/// Read a result file, nutmeg raw or PSF (sniffed from its content).
fn read_results(path: &Path, stdout: String) -> Result<RawData, BackendError> {
    let bytes = std::fs::read(path).map_err(|e| {
        BackendError::SimulationError(format!("no result file '{}': {e}\nstdout: {}", path.display(), tail(&stdout)))
    })?;
    // Spectre's PSF keeps its magic in the trailer. It is not read: the
    // Spectre backend asks for `-format nutbin`.
    if bytes.len() >= 12 && &bytes[bytes.len() - 12..bytes.len() - 4] == b"Clarissa" {
        return Err(BackendError::SimulationError("PSF output is not read; run Spectre with -format nutbin".into()));
    }
    let mut raw = rawfile::parse_raw(&bytes)?;
    raw.stdout = stdout;
    Ok(raw)
}

/// Regular files directly inside `dir`, sorted (read_dir order is not stable).
fn sorted_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten()
        .map(|e| e.path()).filter(|p| p.is_file()).collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_table_filters_by_feature() {
        assert!(runs(Dialect::Ngspice, XSPICE));
        assert!(!runs(Dialect::Vacask, XSPICE));
        assert!(!runs(Dialect::Spectre, XSPICE));
        assert!(!runs(Dialect::Ltspice, OSDI));
        assert!(runs(Dialect::Espice, OSDI));
        assert!(!runs(Dialect::Ngspice, STEP_PARAMS | MEASURES));
        assert!(!runs(Dialect::Vacask, STEP_PARAMS | MEASURES));
        assert!(runs(Dialect::Spectre, STEP_PARAMS | MEASURES));
        assert!(runs(Dialect::Ltspice, STEP_PARAMS | MEASURES));
        assert!(runs(Dialect::Ngspice, 0));
    }

    fn pz() -> Analysis {
        Analysis::PoleZero { node1: "a".into(), node2: "0".into(), node3: "b".into(), node4: "0".into(), tf_type: crate::ir::PzTransfer::Vol, pz_type: crate::ir::PzKind::Pz }
    }

    #[test]
    fn routes_match_the_analysis() {
        use Dialect::*;
        let tran = Analysis::Transient { step: 1e-9, stop: 1e-6, start: None, max_step: None, uic: false };
        let sens = |ac| Analysis::Sensitivity { output: "v(b)".into(), ac };
        let ac = crate::ir::FreqSweep { variation: crate::ir::Sweep::Dec, points: 10, start: 1.0, stop: 1e6 };
        assert_eq!(route(&pz()).0, &[Ngspice]);
        assert_eq!(route(&Analysis::HarmonicBalance { frequencies: vec![1e6], harmonics: vec![5] }).0, &[Vacask, Spectre]);
        assert_eq!(route(&Analysis::TransientNoise { step: 1e-9, stop: 1e-6 }).0, &[Vacask]);
        assert_eq!(route(&sens(None)).0, &[Ngspice, Spectre, Espice]);
        assert_eq!(route(&sens(Some(ac))).0, &[Ngspice, Spectre]);
        assert_eq!(route(&tran).0, RUNNABLE);
        assert_eq!(route(&tran).0.last(), Some(&Espice), "espice is never preferred first");
    }

    #[test]
    fn explicit_backend_must_run_the_analysis() {
        let none = FeatureFlags::default();
        let hb = Analysis::HarmonicBalance { frequencies: vec![1e6], harmonics: vec![5] };
        assert_eq!(select(&pz(), Some("ngspice"), &none).unwrap(), Dialect::Ngspice);
        assert_eq!(select(&Analysis::Dc { sweeps: vec![] }, Some("vacask-shared"), &none).unwrap(), Dialect::Vacask);
        assert!(matches!(select(&pz(), Some("ltspice"), &none), Err(BackendError::UnsupportedAnalysis { analysis, .. }) if analysis == "pz"));
        assert!(matches!(select(&hb, Some("ngspice"), &none), Err(BackendError::UnsupportedAnalysis { .. })));
        assert!(select(&Analysis::Op, Some("xyce"), &none).is_err());
    }

    #[test]
    fn names_round_trip() {
        for d in RUNNABLE {
            assert_eq!(parse(d.name()), Some(d));
        }
        for (alias, d) in Dialect::ALIASES {
            assert_eq!(parse(alias), Some(d));
        }
        assert_eq!(parse("hspice"), None, "emitted, never run");
        assert_eq!(parse("Ngspice"), None, "backend names are exact");
        assert!(Dialect::Xyce.run("").is_err());
        assert_eq!(Dialect::Cdl.capabilities(), 0);
    }
}
