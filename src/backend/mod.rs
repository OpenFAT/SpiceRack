//! Simulator backends: the [`RUNNABLE`] dialects, one `match` per question.
//!
//! A backend executes netlist text verbatim, in its own [`Dialect`]. Turning
//! a deck into that text is `syntax::render`'s job. Netlist text in, [`RawData`] plots
//! out, through one subprocess per run.
//!
//! Cites: Spectre Reference 19.1 p.17 (`.measure`); each simulator's file
//! cites its own CLI.
//!
//! Contents:
//! - `pub enum BackendError`
//! - `pub const RUNNABLE: [Dialect; 5]`
//! - `pub use crate::syntax::Dialect`
//! - `pub mod feature`: `XSPICE`, `OSDI`, `MEASURES`, `STEP_PARAMS`, `CONTROL_BLOCKS`,
//!   `LAPLACE`, `VERILOG_COSIM`, `ALL` (`u8` bits)
//! - `pub(crate) fn runs(d: Dialect, needs: u8) -> bool`
//! - `pub fn parse(name: &str) -> Option<Dialect>`
//! - `Dialect::capabilities(self) -> u8`, `Dialect::run(self, netlist: &str) -> Result<RawData, BackendError>`,
//!   `Dialect::run_all(self, netlist: &str) -> Result<Vec<RawData>, BackendError>`
//! - `pub fn available() -> &'static [Dialect]`
//! - `pub enum Kind`, `Kind::name(self) -> &'static str`
//! - `pub fn select(kind: Kind, override_backend: Option<&str>, needs: u8) -> Result<Dialect, BackendError>`
//! - `pub struct Issue`, `pub enum IssueSeverity`,
//!   `pub fn check(needs: u8, elements: usize, backend: &str) -> Vec<Issue>`
//! - `pub mod espice`, `ltspice`, `ngspice`, `spectre`, `vacask`

pub mod espice;
pub mod ltspice;
pub mod ngspice;
pub mod spectre;
pub mod vacask;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use feature::*;

use crate::rawfile;
use crate::result::RawData;
pub use crate::syntax::Dialect;

/// What a deck needs from its simulator, one bit per feature
/// ([`Dialect::capabilities`] is the same mask: what a simulator does).
pub mod feature {
    pub const XSPICE: u8 = 1 << 0;
    pub const OSDI: u8 = 1 << 1;
    pub const MEASURES: u8 = 1 << 2;
    pub const STEP_PARAMS: u8 = 1 << 3;
    pub const CONTROL_BLOCKS: u8 = 1 << 4;
    pub const LAPLACE: u8 = 1 << 5;
    pub const VERILOG_COSIM: u8 = 1 << 6;
    /// Every bit above.
    pub const ALL: u8 =
        XSPICE | OSDI | MEASURES | STEP_PARAMS | CONTROL_BLOCKS | LAPLACE | VERILOG_COSIM;
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("No simulator backend found. Install ngspice: sudo apt install ngspice")]
    NoBackend,

    #[error(
        "Analysis '{analysis}' not supported by {backend}.\n\n\
        This analysis requires {required}. Either:\n  \
        1. Install {required}\n  \
        2. Alternative: {alternative}"
    )]
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

/// The dialects a simulator here runs; a universal analysis runs on each, in
/// this order of preference.
pub const RUNNABLE: [Dialect; 5] = [
    Dialect::Ngspice,
    Dialect::Ltspice,
    Dialect::Vacask,
    Dialect::Spectre,
    Dialect::Espice,
];

/// An analysis's kind: what picks the simulator that runs it
/// ([`select`]), and the plot a run writes for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Op,
    Dc,
    Ac,
    Tran,
    Noise,
    Tf,
    /// `.sens` at DC.
    Sens,
    /// `.sens … AC …`.
    SensAc,
    Pz,
    Disto,
    Pss,
    Hb,
    Sp,
    Stb,
    TranNoise,
    /// `.four`: a table on stdout, no plot.
    Fourier,
    SpectreSweep,
    SpectreMonteCarlo,
    Pac,
    Pnoise,
    Pxf,
    Pstb,
}

/// One problem [`check`] finds with running a deck on a backend.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: IssueSeverity,
    pub message: String,
}

/// Whether an [`Issue`] stops the run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IssueSeverity {
    Error,
    Warning,
}

// What each simulator can do, as `feature` bits.
const NGSPICE_CAPS: u8 = XSPICE | OSDI | MEASURES | CONTROL_BLOCKS | LAPLACE | VERILOG_COSIM;
const LTSPICE_CAPS: u8 = MEASURES | STEP_PARAMS | LAPLACE;
// VACASK: `load "x.osdi"`; no measurement statement ("Command not found");
// `var` + `sweep` does parameter sweeps; its `control` is not ngspice's.
const VACASK_CAPS: u8 = OSDI | STEP_PARAMS;
// Spectre: `.measure` per Spectre Reference 19.1 p.17; xrun co-simulation.
const SPECTRE_CAPS: u8 = OSDI | MEASURES | STEP_PARAMS | VERILOG_COSIM;
// ESPice: Verilog-A via VerA `.hdl` (counts as osdi); `.step` lands every
// point as a separate plot in one raw file, which parse_raw does not split.
// Laplace: its own `E/G … LAPLACE` card (`render::spice::laplace`).
const ESPICE_CAPS: u8 = OSDI | MEASURES | LAPLACE;

// Seven features, one distinct bit each, inside the `u8`.
const _: () = assert!(feature::ALL.count_ones() == 7);

/// Each feature a backend may lack, how bad that is, and what to say
/// (`{}` is the backend).
const MISSING: &[(u8, IssueSeverity, &str)] = &[
    (
        XSPICE,
        IssueSeverity::Error,
        "Backend '{}' does not support XSPICE A-elements (only ngspice does)",
    ),
    (
        OSDI,
        IssueSeverity::Error,
        "Backend '{}' does not support OSDI/Verilog-A model loading",
    ),
    (
        MEASURES,
        IssueSeverity::Error,
        "Backend '{}' does not support .meas directives",
    ),
    (
        STEP_PARAMS,
        IssueSeverity::Error,
        "Backend '{}' does not support .step param sweeps",
    ),
    (
        CONTROL_BLOCKS,
        IssueSeverity::Error,
        "Backend '{}' does not support .control blocks (ngspice only)",
    ),
    (
        LAPLACE,
        IssueSeverity::Warning,
        "Backend '{}' does not support Laplace-domain B-sources",
    ),
    (
        VERILOG_COSIM,
        IssueSeverity::Error,
        "Backend '{}' does not support Verilog co-simulation",
    ),
];

// Exactly one `MISSING` row per feature bit, so a backend `select` refuses
// for a feature always has the issue `check` names for it.
const _: () = {
    let (mut seen, mut i) = (0u8, 0);
    while i < MISSING.len() {
        assert!(seen & MISSING[i].0 == 0, "a feature has two MISSING rows");
        assert!(
            MISSING[i].0.count_ones() == 1,
            "a MISSING row names one feature"
        );
        seen |= MISSING[i].0;
        i += 1;
    }
    assert!(seen == ALL, "a feature has no MISSING row");
};

/// True when `d`'s simulator has every feature in `needs` (`feature`
/// bits); [`check`] reports each bit this finds missing.
pub(crate) fn runs(d: Dialect, needs: u8) -> bool {
    needs & !d.capabilities() == 0
}

/// A user-facing backend name (a [`RUNNABLE`] dialect's name or one of
/// [`Dialect::ALIASES`]) as the dialect it runs; `None` for anything else.
pub fn parse(name: &str) -> Option<Dialect> {
    Dialect::from_backend_name(name).filter(|d| RUNNABLE.contains(d))
}

impl Dialect {
    /// What this dialect's simulator can do, as `feature` bits; nothing
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
        let first = self.run_all(netlist)?.into_iter().next();
        Ok(first.expect("a raw file read holds at least one plot"))
    }

    /// [`Self::run`], returning every plot of the run in file order, each
    /// with the run's stdout. ngspice writes one per analysis (`noise` and
    /// `disto` two) in its own order (`ac`, `dc`, `op`, `tran`, `tf`,
    /// `noise`, `sens` for a deck without a `.control` block), not the
    /// deck's; VACASK one raw file per analysis, read in deck order.
    ///
    /// # Errors
    /// As [`Self::run`].
    pub fn run_all(self, netlist: &str) -> Result<Vec<RawData>, BackendError> {
        match self {
            Dialect::Ngspice => ngspice::run(netlist),
            Dialect::Ltspice => ltspice::run(netlist),
            Dialect::Vacask => vacask::run(netlist),
            Dialect::Spectre => spectre::run(netlist),
            Dialect::Espice => espice::run(netlist),
            Dialect::Hspice | Dialect::Xyce | Dialect::Cdl | Dialect::Verilog => {
                Err(BackendError::SimulationError(format!(
                    "no {} simulator is driven by SpiceRack",
                    self.name()
                )))
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
        found
            .into_iter()
            .filter(|&(_, ok)| ok)
            .map(|(k, _)| k)
            .collect()
    })
}

/// Pick the backend for an analysis of `kind`. An explicit name (argument,
/// else `$SPICERACK_BACKEND`) wins if it can run the analysis; otherwise
/// the first installed backend, in `route` order, that has every `needs`
/// bit (`feature`).
pub fn select(
    kind: Kind,
    override_backend: Option<&str>,
    needs: u8,
) -> Result<Dialect, BackendError> {
    let (dialects, alternative) = route(kind);
    let analysis = kind.name();
    let env = std::env::var("SPICERACK_BACKEND")
        .ok()
        .filter(|s| !s.is_empty());
    if let Some(name) = override_backend.or(env.as_deref()) {
        let kind = parse(name)
            .ok_or_else(|| BackendError::SimulationError(format!("Unknown backend: {name}")))?;
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
        if let Some(&k) = available.iter().find(|&&k| k == *d && runs(k, needs)) {
            return Ok(k);
        }
    }
    if dialects[0] != Dialect::Ngspice {
        let names: Vec<_> = available.iter().map(|k| k.name()).collect();
        return Err(unsupported(
            analysis,
            &names.join(", "),
            dialects[0],
            alternative,
        ));
    }
    // ponytail: universal analysis, no backend matches the features — run on
    // the first one and let the simulator report what it cannot do.
    Ok(available[0])
}

impl Kind {
    /// The analysis's short name, as errors and `check` name it.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Op => "op",
            Kind::Dc => "dc",
            Kind::Ac => "ac",
            // `fourier`, `sweep` and `montecarlo` wrap a transient here.
            Kind::Tran | Kind::Fourier | Kind::SpectreSweep | Kind::SpectreMonteCarlo => "tran",
            Kind::Noise => "noise",
            Kind::Tf => "tf",
            Kind::Sens => "sens",
            Kind::SensAc => "sens_ac",
            Kind::Pz => "pz",
            Kind::Disto => "disto",
            Kind::Pss => "pss",
            Kind::Hb => "hb",
            Kind::Sp => "sp",
            Kind::Stb => "stb",
            Kind::TranNoise => "trannoise",
            Kind::Pac => "pac",
            Kind::Pnoise => "pnoise",
            Kind::Pxf => "pxf",
            Kind::Pstb => "pstb",
        }
    }
}

/// The issues `backend` raises against a deck that needs the `needs` bits
/// (`feature`) and holds `elements` element and instance cards: each
/// feature it lacks, and a large deck on ngspice. Pure: nothing runs.
pub fn check(needs: u8, elements: usize, backend: &str) -> Vec<Issue> {
    let Some(kind) = parse(backend) else {
        return vec![Issue {
            severity: IssueSeverity::Error,
            message: format!("Unknown backend '{backend}'"),
        }];
    };
    let missing = needs & !kind.capabilities();
    let mut issues: Vec<Issue> = MISSING
        .iter()
        .filter(|(f, _, _)| missing & f != 0)
        .map(|&(_, severity, message)| Issue {
            severity,
            message: message.replace("{}", backend),
        })
        .collect();
    if elements > 10_000 && kind == Dialect::Ngspice {
        issues.push(Issue {
            severity: IssueSeverity::Warning,
            message: format!("Large circuit ({elements} elements) may be slow on '{backend}'"),
        });
    }
    issues
}

// ---- private ----

/// The dialects that run `a`, in preference order, and the workaround to
/// suggest without one. A universal analysis runs on every [`RUNNABLE`].
fn route(kind: Kind) -> (&'static [Dialect], &'static str) {
    use Dialect::*;
    match kind {
        // `tf` runs everywhere, so its old ".dc() with small-signal sweep"
        // workaround could never be shown.
        Kind::Op
        | Kind::Dc
        | Kind::Ac
        | Kind::Tran
        | Kind::Noise
        | Kind::Tf
        | Kind::Fourier
        | Kind::SpectreSweep
        | Kind::SpectreMonteCarlo => (&RUNNABLE, "N/A"),
        Kind::Pz => (&[Ngspice], ".ac() + post-process for poles/zeros"),
        Kind::Disto => (&[Ngspice], ".transient() + FFT post-processing"),
        Kind::Sens => (&[Ngspice, Spectre, Espice], "N/A"),
        Kind::SensAc => (&[Ngspice, Spectre], "N/A"),
        Kind::Hb => (&[Vacask, Spectre], "long .tran() + discard startup + FFT"),
        Kind::Sp => (&[Vacask, Spectre, Ngspice], "N/A"),
        Kind::Stb => (&[Vacask, Spectre], ".ac() + Middlebrook method"),
        Kind::Pss => (&[Spectre, Ngspice], "long .tran() + discard startup"),
        Kind::TranNoise => (&[Vacask], ".tran() with trnoise sources"),
        Kind::Pac | Kind::Pnoise | Kind::Pxf | Kind::Pstb => (&[Spectre], "N/A"),
    }
}

fn unsupported(
    analysis: &str,
    backend: &str,
    required: Dialect,
    alternative: &str,
) -> BackendError {
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
    Command::new("which")
        .arg(program)
        .output()
        .is_ok_and(|o| o.status.success())
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
        std::io::ErrorKind::NotFound => {
            BackendError::SimulationError(format!("{program} not found on $PATH"))
        }
        _ => BackendError::Io(e),
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let log = log
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let errors: Vec<&str> = stdout
        .lines()
        .chain(stderr.lines())
        .chain(log.lines())
        .filter(|l| l.contains("rror") || l.contains("failed"))
        .collect();
    let detail = if errors.is_empty() {
        format!(
            "stdout: {}\nstderr: {}\nlog: {}",
            tail(&stdout),
            tail(&stderr),
            tail(&log)
        )
    } else {
        errors.join("\n")
    };
    Err(BackendError::SimulationError(format!(
        "{program} exited with {}\n{detail}",
        out.status
    )))
}

fn tail(s: &str) -> &str {
    let mut start = s.len().saturating_sub(1000);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

/// Read a result file, nutmeg raw or PSF (sniffed from its content).
fn read_results(path: &Path, stdout: String) -> Result<Vec<RawData>, BackendError> {
    let bytes = std::fs::read(path).map_err(|e| {
        BackendError::SimulationError(format!(
            "no result file '{}': {e}\nstdout: {}",
            path.display(),
            tail(&stdout)
        ))
    })?;
    // Spectre's PSF keeps its magic in the trailer. It is not read: the
    // Spectre backend asks for `-format nutbin`.
    if bytes.len() >= 12 && &bytes[bytes.len() - 12..bytes.len() - 4] == b"Clarissa" {
        return Err(BackendError::SimulationError(
            "PSF output is not read; run Spectre with -format nutbin".into(),
        ));
    }
    let mut plots = rawfile::parse_raw_all(&bytes)?;
    plots.iter_mut().for_each(|p| p.stdout.clone_from(&stdout));
    Ok(plots)
}

/// Regular files directly inside `dir`, sorted (read_dir order is not stable).
fn sorted_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
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

    #[test]
    fn routes_match_the_analysis() {
        use Dialect::*;
        assert_eq!(route(Kind::Pz).0, &[Ngspice]);
        assert_eq!(route(Kind::Hb).0, &[Vacask, Spectre]);
        assert_eq!(route(Kind::TranNoise).0, &[Vacask]);
        assert_eq!(route(Kind::Sens).0, &[Ngspice, Spectre, Espice]);
        assert_eq!(route(Kind::SensAc).0, &[Ngspice, Spectre]);
        assert_eq!(route(Kind::Tran).0, RUNNABLE);
        assert_eq!(
            route(Kind::Tran).0.last(),
            Some(&Espice),
            "espice is never preferred first"
        );
    }

    #[test]
    fn explicit_backend_must_run_the_analysis() {
        assert_eq!(
            select(Kind::Pz, Some("ngspice"), 0).unwrap(),
            Dialect::Ngspice
        );
        assert_eq!(
            select(Kind::Dc, Some("vacask-shared"), 0).unwrap(),
            Dialect::Vacask
        );
        assert!(
            matches!(select(Kind::Pz, Some("ltspice"), 0), Err(BackendError::UnsupportedAnalysis { analysis, .. }) if analysis == "pz")
        );
        assert!(matches!(
            select(Kind::Hb, Some("ngspice"), 0),
            Err(BackendError::UnsupportedAnalysis { .. })
        ));
        assert!(select(Kind::Op, Some("xyce"), 0).is_err());
    }

    #[test]
    fn check_names_each_missing_feature() {
        let issues = check(XSPICE | LAPLACE, 1, "vacask");
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].severity, IssueSeverity::Error);
        assert!(issues[0].message.contains("XSPICE"), "{issues:?}");
        assert_eq!(issues[1].severity, IssueSeverity::Warning);
        assert!(check(XSPICE, 1, "ngspice").is_empty());
        assert!(
            check(0, 20_000, "ngspice")[0]
                .message
                .contains("Large circuit")
        );
        assert_eq!(check(0, 1, "nope")[0].message, "Unknown backend 'nope'");
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
