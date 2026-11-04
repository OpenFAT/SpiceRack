//! A deck rendered for one simulator → the backend runs the text verbatim
//! → `RawData`, one per plot or, matched by plot name (ngspice and ESPice
//! `Plotname`s, checked on both), one per analysis. The simulator is
//! picked first (`backend::select`), since the deck's text is that
//! simulator's dialect.
//!
//! Cites: none (plot names are ngspice's and ESPice's `Plotname` lines).
//!
//! Contents:
//! - `pub fn run(sim: Dialect, deck: &str) -> Result<RawData, BackendError>`
//! - `pub fn run_all(sim: Dialect, deck: &str) -> Result<Vec<RawData>, BackendError>`
//! - `pub fn run_each(sim: Dialect, deck: &str, analyses: &[Kind]) -> Result<Vec<Option<RawData>>, BackendError>`

use crate::backend::{BackendError, Dialect, Kind};
use crate::result::RawData;

/// Runs `deck`, written in `sim`'s dialect, on `sim`, tagging the result
/// with the backend and its parsed `.meas` values. The first plot:
/// [`run_all`] returns them all.
pub fn run(sim: Dialect, deck: &str) -> Result<RawData, BackendError> {
    let first = run_all(sim, deck)?.into_iter().next();
    Ok(first.expect("a raw file read holds at least one plot"))
}

/// [`run`], returning every plot the run wrote (ngspice: one per analysis,
/// in run order), each tagged with the backend and the run's `.meas` values.
pub fn run_all(sim: Dialect, deck: &str) -> Result<Vec<RawData>, BackendError> {
    let mut plots = sim.run_all(deck)?;
    // LTspice writes `.meas` results to its log, everyone else to stdout;
    // every plot carries the same run's text.
    let p = &plots[0];
    let text = if sim == Dialect::Ltspice && !p.log_content.is_empty() {
        &p.log_content
    } else {
        &p.stdout
    };
    let measures = if text.is_empty() {
        vec![]
    } else {
        crate::measure_parse::parse_measures(text, Some(sim))
    };
    for p in &mut plots {
        p.backend_hint = Some(sim);
        p.measures.clone_from(&measures);
    }
    Ok(plots)
}

/// [`run_all`], one entry per analysis of the deck, `analyses` in its
/// order: that analysis's first plot, or `None` for a `Fourier`, which
/// prints its table and writes none. ngspice runs a deck's analyses in its
/// own order, so a plot is matched by its `Plotname` (ngspice's and
/// ESPice's), not by position; VACASK's are in deck order, one per
/// analysis. A lone analysis takes the first plot, as [`run`].
///
/// # Errors
/// As [`run_all`], and `SimulationError` when an analysis's plot is not
/// among those the run wrote, or it is an analysis whose plot is not
/// matched here (`pss`, `hb`, `stb`, transient noise, the Spectre-only
/// ones) run beside others.
pub fn run_each(
    sim: Dialect,
    deck: &str,
    analyses: &[Kind],
) -> Result<Vec<Option<RawData>>, BackendError> {
    let mut plots: Vec<Option<RawData>> = run_all(sim, deck)?.into_iter().map(Some).collect();
    assert!(!plots.is_empty(), "a run returns at least one plot");
    if analyses.len() <= 1 {
        plots.truncate(1);
        return Ok(plots);
    }
    let missing = |what: &str| {
        BackendError::SimulationError(format!("no {what} plot among those the run wrote"))
    };
    if sim == Dialect::Vacask {
        if plots.len() != analyses.len() {
            return Err(missing(&format!(
                "one-per-analysis ({} plots, {} analyses)",
                plots.len(),
                analyses.len()
            )));
        }
        return Ok(plots);
    }
    let mut out = Vec::with_capacity(analyses.len());
    for &a in analyses {
        let name = match a {
            Kind::Fourier => {
                out.push(None);
                continue;
            }
            Kind::Op => "Operating Point",
            Kind::Dc => "DC transfer characteristic",
            Kind::Ac => "AC Analysis",
            Kind::Tran => "Transient Analysis",
            Kind::Noise => "Noise Spectral Density Curves",
            Kind::Tf => "Transfer Function",
            Kind::Sens | Kind::SensAc => "Sensitivity Analysis",
            Kind::Pz => "Pole-Zero Analysis",
            Kind::Disto => "DISTORTION",
            Kind::Sp => "SP Analysis",
            Kind::Pss
            | Kind::Hb
            | Kind::Stb
            | Kind::TranNoise
            | Kind::SpectreSweep
            | Kind::SpectreMonteCarlo
            | Kind::Pac
            | Kind::Pnoise
            | Kind::Pxf
            | Kind::Pstb => {
                return Err(missing(&format!("matchable {}", unmatched(a))));
            }
        };
        let i = plots
            .iter()
            .position(|p| p.as_ref().is_some_and(|p| p.plot_name.starts_with(name)))
            .ok_or_else(|| missing(name))?;
        out.push(plots[i].take());
    }
    assert_eq!(out.len(), analyses.len());
    Ok(out)
}

// ---- private ----

/// The analysis's name in a `run_each` error, as the Python API names it.
fn unmatched(a: Kind) -> &'static str {
    match a {
        Kind::Pss => "Pss",
        Kind::Hb => "HarmonicBalance",
        Kind::Stb => "Stability",
        Kind::TranNoise => "TransientNoise",
        Kind::SpectreSweep => "SpectreSweep",
        Kind::SpectreMonteCarlo => "SpectreMonteCarlo",
        Kind::Pac => "SpectrePac",
        Kind::Pnoise => "SpectrePnoise",
        Kind::Pxf => "SpectrePxf",
        Kind::Pstb => "SpectrePstb",
        Kind::Op
        | Kind::Dc
        | Kind::Ac
        | Kind::Tran
        | Kind::Noise
        | Kind::Tf
        | Kind::Sens
        | Kind::SensAc
        | Kind::Pz
        | Kind::Disto
        | Kind::Sp
        | Kind::Fourier => a.name(),
    }
}
