//! Simulator backends: a closed set, one `enum`, one `match` per question.
//!
//! A backend executes netlist text verbatim. Turning IR into that text is the
//! codegen's job; [`BackendKind::dialect`] names which dialect to ask it for.

pub mod ngspice;
pub mod ltspice;
pub mod vacask;
pub mod spectre;
pub mod espice;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::codegen::{CodeGen, CodeGenError};
use crate::ir::{CircuitIR, FeatureFlags};
use crate::rawfile;
use crate::result::RawData;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("No simulator backend found. Install ngspice: sudo apt install ngspice")]
    NoBackend,

    #[error("Analysis '{analysis}' not supported by {backend}.\n\n\
        This analysis requires {required}. Either:\n  \
        1. Install {required}: {install_cmd}\n  \
        2. Alternative: {alternative}")]
    UnsupportedAnalysis {
        analysis: String,
        backend: String,
        required: String,
        install_cmd: String,
        alternative: String,
    },

    #[error("Simulation failed: {0}")]
    SimulationError(String),

    #[error("Raw file parse error: {0}")]
    RawParseError(#[from] rawfile::RawFileError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Netlist dialects, one per simulator family.
// ponytail: stand-in for `codegen::Dialect`; on merge delete this enum and
// `netlist` below, then `use crate::codegen::{Dialect, netlist}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect { Ngspice, Ltspice, Spectre, Vacask, Espice }

/// IR → netlist text in dialect `d`. Same signature as `codegen::netlist`.
pub fn netlist(ir: &CircuitIR, d: Dialect) -> Result<String, CodeGenError> {
    use crate::codegen::spice3::{Spice3CodeGen, Spice3Dialect};
    match d {
        Dialect::Ngspice => Spice3CodeGen { dialect: Spice3Dialect::Ngspice }.emit_netlist(ir),
        Dialect::Ltspice => Spice3CodeGen { dialect: Spice3Dialect::Ltspice }.emit_netlist(ir),
        Dialect::Spectre => crate::codegen::spectre::SpectreCodeGen.emit_netlist(ir),
        Dialect::Vacask => crate::codegen::vacask::VacaskCodeGen.emit_netlist(ir),
        Dialect::Espice => crate::codegen::espice::EspiceCodeGen.emit_netlist(ir),
    }
}

/// Every simulator SpiceRack can drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// `ngspice -b` subprocess.
    Ngspice,
    Ltspice,
    Vacask,
    Spectre,
    Espice,
}

/// What a simulator can do, checked against `ir::FeatureFlags`.
#[derive(Debug, Clone, Copy)]
pub struct BackendCapabilities {
    pub xspice: bool,
    pub osdi: bool,
    pub measures: bool,
    pub step_params: bool,
    pub control_blocks: bool,
    pub laplace_sources: bool,
    pub verilog_cosim: bool,
}

impl BackendCapabilities {
    pub fn supports_features(&self, f: &FeatureFlags) -> bool {
        (!f.has_xspice || self.xspice)
            && (!f.has_osdi || self.osdi)
            && (!f.has_measures || self.measures)
            && (!f.has_step_params || self.step_params)
            && (!f.has_control_blocks || self.control_blocks)
            && (!f.has_laplace_sources || self.laplace_sources)
            && (!f.has_verilog_cosim || self.verilog_cosim)
    }
}

const fn caps(xspice: bool, osdi: bool, measures: bool, step_params: bool, control_blocks: bool, laplace_sources: bool, verilog_cosim: bool) -> BackendCapabilities {
    BackendCapabilities { xspice, osdi, measures, step_params, control_blocks, laplace_sources, verilog_cosim }
}

//                                           xspice osdi  meas  step  ctrl  laplace cosim
const NGSPICE_CAPS: BackendCapabilities = caps(true,  true, true, false, true,  true,  true);
const LTSPICE_CAPS: BackendCapabilities = caps(false, false, true, true, false, true,  false);
// VACASK: `load "x.osdi"`; no measurement statement ("Command not found");
// `var` + `sweep` does parameter sweeps; its `control` is not ngspice's.
const VACASK_CAPS: BackendCapabilities = caps(false, true, false, true, false, false, false);
// Spectre: `.measure` per Spectre Reference 19.1 p.17; xrun co-simulation.
const SPECTRE_CAPS: BackendCapabilities = caps(false, true, true, true, false, false, true);
// ESPice: Verilog-A via VerA `.hdl` (counts as osdi); `.step` lands every
// point as a separate plot in one raw file, which parse_raw does not split.
const ESPICE_CAPS: BackendCapabilities = caps(false, true, true, false, false, false, false);

/// Analysis kinds (`ir::Analysis::kind_str`) → dialects that run them, in
/// preference order, and the workaround to suggest without one. A kind not
/// listed runs on every dialect in `ANYWHERE`.
const ROUTES: &[(&[&str], &[Dialect], &str)] = {
    use Dialect::*;
    &[
        (&["pz"], &[Ngspice], ".ac() + post-process for poles/zeros"),
        (&["disto"], &[Ngspice], ".transient() + FFT post-processing"),
        (&["tf"], &[Ngspice, Ltspice, Vacask, Spectre, Espice], ".dc() with small-signal sweep"),
        (&["sens"], &[Ngspice, Spectre, Espice], "N/A"),
        (&["sens_ac"], &[Ngspice, Spectre], "N/A"),
        (&["hb"], &[Vacask, Spectre], "long .tran() + discard startup + FFT"),
        (&["sp", "s_param"], &[Vacask, Spectre, Ngspice], "N/A"),
        (&["stb", "stability"], &[Vacask, Spectre], ".ac() + Middlebrook method"),
        (&["pss"], &[Spectre, Ngspice], "long .tran() + discard startup"),
        (&["trannoise"], &[Vacask], ".tran() with trnoise sources"),
        (&["sampling", "pce", "embedded_sampling"], &[Spectre], ".control Monte Carlo loops (ngspice)"),
        (&["sens_tran", "sens_adjoint"], &[Spectre], "N/A"),
        (&["pac", "pnoise", "pxf", "pstb", "psp", "pdisto", "hbac", "hbnoise", "hbsp"], &[Spectre], "N/A"),
    ]
};
const ANYWHERE: &[Dialect] = &[Dialect::Ngspice, Dialect::Ltspice, Dialect::Vacask, Dialect::Spectre, Dialect::Espice];

fn route(analysis: &str) -> (&'static [Dialect], &'static str) {
    ROUTES
        .iter()
        .find(|(kinds, _, _)| kinds.contains(&analysis))
        .map_or((ANYWHERE, "N/A"), |&(_, d, alt)| (d, alt))
}

impl BackendKind {
    /// Parse a user-facing backend name. `ngspice-shared` and
    /// `vacask-shared` are kept as aliases of the executables.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "ngspice" | "ngspice-subprocess" | "ngspice-shared" => Self::Ngspice,
            "ltspice" => Self::Ltspice,
            "vacask" | "vacask-shared" => Self::Vacask,
            "spectre" => Self::Spectre,
            "espice" => Self::Espice,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Ngspice => "ngspice",
            Self::Ltspice => "ltspice",
            Self::Vacask => "vacask",
            Self::Spectre => "spectre",
            Self::Espice => "espice",
        }
    }

    pub fn dialect(self) -> Dialect {
        match self {
            Self::Ngspice => Dialect::Ngspice,
            Self::Ltspice => Dialect::Ltspice,
            Self::Vacask => Dialect::Vacask,
            Self::Spectre => Dialect::Spectre,
            Self::Espice => Dialect::Espice,
        }
    }

    pub fn capabilities(self) -> BackendCapabilities {
        match self.dialect() {
            Dialect::Ngspice => NGSPICE_CAPS,
            Dialect::Ltspice => LTSPICE_CAPS,
            Dialect::Vacask => VACASK_CAPS,
            Dialect::Spectre => SPECTRE_CAPS,
            Dialect::Espice => ESPICE_CAPS,
        }
    }

    /// Execute `netlist` (already in `self.dialect()`) and read the results.
    pub fn run(self, netlist: &str) -> Result<RawData, BackendError> {
        match self {
            Self::Ngspice => ngspice::run(netlist),
            Self::Ltspice => ltspice::run(netlist),
            Self::Vacask => vacask::run(netlist),
            Self::Spectre => spectre::run(netlist),
            Self::Espice => espice::run(netlist),
        }
    }
}

/// Backends installed on this machine, in preference order. Cached.
pub fn available() -> &'static [BackendKind] {
    static CACHE: OnceLock<Vec<BackendKind>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let found = [
            (BackendKind::Ngspice, is_on_path("ngspice")),
            (BackendKind::Ltspice, ltspice::find().is_some()),
            (BackendKind::Vacask, is_on_path("vacask")),
            (BackendKind::Spectre, is_on_path("spectre")),
            (BackendKind::Espice, is_on_path("espice")),
        ];
        found.into_iter().filter(|&(_, ok)| ok).map(|(k, _)| k).collect()
    })
}

/// Pick the backend for one analysis kind. An explicit name (argument, else
/// `$SPICERACK_BACKEND`) wins if it can run the analysis; otherwise the first
/// installed backend, in `ROUTES` order, that supports the circuit's features.
pub fn select(analysis: &str, override_backend: Option<&str>, features: &FeatureFlags) -> Result<BackendKind, BackendError> {
    let (dialects, alternative) = route(analysis);
    let env = std::env::var("SPICERACK_BACKEND").ok().filter(|s| !s.is_empty());
    if let Some(name) = override_backend.or(env.as_deref()) {
        let kind = BackendKind::from_str(name)
            .ok_or_else(|| BackendError::SimulationError(format!("Unknown backend: {name}")))?;
        if !dialects.contains(&kind.dialect()) {
            return Err(unsupported(analysis, name, dialects[0], alternative));
        }
        return Ok(kind);
    }

    let available = available();
    if available.is_empty() {
        return Err(BackendError::NoBackend);
    }
    for d in dialects {
        if let Some(&k) = available.iter().find(|k| k.dialect() == *d && k.capabilities().supports_features(features)) {
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
    let required = format!("{required:?}").to_lowercase();
    BackendError::UnsupportedAnalysis {
        analysis: analysis.to_string(),
        backend: backend.to_string(),
        install_cmd: format!("See docs/backends/{required}.md"),
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
    let mut raw = if crate::psf::is_psf(&bytes) {
        crate::psf::parse_psf(&bytes).map_err(|e| BackendError::SimulationError(format!("PSF parse error: {e}")))?
    } else {
        rawfile::parse_raw(&bytes)?
    };
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
        let f = |g: fn(&mut FeatureFlags)| { let mut f = FeatureFlags::default(); g(&mut f); f };
        let xspice = f(|f| f.has_xspice = true);
        assert!(BackendKind::Ngspice.capabilities().supports_features(&xspice));
        assert!(!BackendKind::Vacask.capabilities().supports_features(&xspice));
        assert!(!BackendKind::Spectre.capabilities().supports_features(&xspice));

        let osdi = f(|f| f.has_osdi = true);
        assert!(!BackendKind::Ltspice.capabilities().supports_features(&osdi));
        assert!(BackendKind::Espice.capabilities().supports_features(&osdi));

        let step_meas = f(|f| { f.has_step_params = true; f.has_measures = true; });
        assert!(!BackendKind::Ngspice.capabilities().supports_features(&step_meas));
        assert!(!BackendKind::Vacask.capabilities().supports_features(&step_meas));
        assert!(BackendKind::Spectre.capabilities().supports_features(&step_meas));
        assert!(BackendKind::Ltspice.capabilities().supports_features(&step_meas));
        assert!(BackendKind::Ngspice.capabilities().supports_features(&FeatureFlags::default()));
    }

    #[test]
    fn routes_are_data() {
        use Dialect::*;
        assert_eq!(route("pz").0, &[Ngspice]);
        assert_eq!(route("hb").0, &[Vacask, Spectre]);
        assert_eq!(route("trannoise").0, &[Vacask]);
        assert_eq!(route("pnoise").0, &[Spectre]);
        assert_eq!(route("tran").0, ANYWHERE);
        assert_eq!(route("tran").0.last(), Some(&Espice), "espice is never preferred first");
    }

    #[test]
    fn explicit_backend_must_run_the_analysis() {
        let none = FeatureFlags::default();
        assert_eq!(select("pz", Some("ngspice"), &none).unwrap(), BackendKind::Ngspice);
        assert_eq!(select("dc", Some("vacask-shared"), &none).unwrap(), BackendKind::Vacask);
        assert!(matches!(select("pz", Some("ltspice"), &none), Err(BackendError::UnsupportedAnalysis { .. })));
        assert!(matches!(select("hb", Some("ngspice"), &none), Err(BackendError::UnsupportedAnalysis { .. })));
        assert!(select("op", Some("xyce"), &none).is_err());
    }

    #[test]
    fn names_round_trip() {
        for k in [BackendKind::Ngspice, BackendKind::Ltspice,
                  BackendKind::Vacask, BackendKind::Spectre, BackendKind::Espice] {
            assert_eq!(BackendKind::from_str(k.name()), Some(k));
        }
        assert_eq!(BackendKind::from_str("ngspice-subprocess"), Some(BackendKind::Ngspice));
    }
}
