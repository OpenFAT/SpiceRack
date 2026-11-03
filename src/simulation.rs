//! The one pipeline: `CircuitIR` → codegen in the selected backend's dialect
//! → backend runs the text verbatim → `RawData`, one per plot or, matched
//! by plot name (ngspice and ESPice `Plotname`s, checked on both), one per
//! analysis.
//!
//! Cites: none (plot names are ngspice's and ESPice's `Plotname` lines).
//!
//! Contents:
//! - `pub fn run(ir: &CircuitIR, backend: Option<&str>) -> Result<RawData, BackendError>`
//! - `pub fn run_all(ir: &CircuitIR, backend: Option<&str>) -> Result<Vec<RawData>, BackendError>`
//! - `pub fn run_each(ir: &CircuitIR, backend: Option<&str>) -> Result<Vec<Option<RawData>>, BackendError>`

use crate::backend::{BackendError, Dialect, netlist, select};
use crate::ir::{Analysis, CircuitIR};
use crate::result::RawData;

/// Run `ir`'s testbench on `backend` (or the best detected backend for its
/// first analysis and features), tagging the result with the backend name and
/// its parsed `.meas` values. The first plot: [`run_all`] returns them all.
pub fn run(ir: &CircuitIR, backend: Option<&str>) -> Result<RawData, BackendError> {
    let first = run_all(ir, backend)?.into_iter().next();
    Ok(first.expect("a raw file read holds at least one plot"))
}

/// [`run`], returning every plot the run wrote (ngspice: one per analysis,
/// in run order), each tagged with the backend and the run's `.meas` values.
pub fn run_all(ir: &CircuitIR, backend: Option<&str>) -> Result<Vec<RawData>, BackendError> {
    let op = Analysis::Op;
    let first = ir
        .testbench
        .as_ref()
        .and_then(|tb| tb.analyses.first())
        .unwrap_or(&op);
    let sim = select(first, backend, &ir.compute_features())?;
    let deck = netlist(ir, sim).map_err(|e| BackendError::SimulationError(e.to_string()))?;
    let mut plots = sim.run_all(&deck)?;
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

/// [`run_all`], one entry per analysis of `ir`'s testbench, in its order:
/// that analysis's first plot, or `None` for a `Fourier`, which prints its
/// table and writes none. ngspice runs a deck's analyses in its own order,
/// so a plot is matched by its `Plotname` (ngspice's and ESPice's), not by
/// position; VACASK's are in deck order, one per analysis. A lone analysis
/// takes the first plot, as [`run`].
///
/// # Errors
/// As [`run_all`], and `SimulationError` when an analysis's plot is not
/// among those the run wrote, or it is an analysis whose plot is not
/// matched here (`pss`, `hb`, `stb`, transient noise, the Spectre-only
/// ones) run beside others.
pub fn run_each(
    ir: &CircuitIR,
    backend: Option<&str>,
) -> Result<Vec<Option<RawData>>, BackendError> {
    let analyses = ir.testbench.as_ref().map_or(&[][..], |tb| &tb.analyses);
    let mut plots: Vec<Option<RawData>> = run_all(ir, backend)?.into_iter().map(Some).collect();
    assert!(!plots.is_empty(), "a run returns at least one plot");
    if analyses.len() <= 1 {
        plots.truncate(1);
        return Ok(plots);
    }
    let missing = |what: &str| {
        BackendError::SimulationError(format!("no {what} plot among those the run wrote"))
    };
    if plots[0].as_ref().and_then(|p| p.backend_hint) == Some(Dialect::Vacask) {
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
    for a in analyses {
        let name = match a {
            Analysis::Fourier { .. } => {
                out.push(None);
                continue;
            }
            Analysis::Op => "Operating Point",
            Analysis::Dc { .. } => "DC transfer characteristic",
            Analysis::Ac { .. } => "AC Analysis",
            Analysis::Transient { .. } => "Transient Analysis",
            Analysis::Noise { .. } => "Noise Spectral Density Curves",
            Analysis::Tf { .. } => "Transfer Function",
            Analysis::Sensitivity { .. } => "Sensitivity Analysis",
            Analysis::PoleZero { .. } => "Pole-Zero Analysis",
            Analysis::Distortion { .. } => "DISTORTION",
            Analysis::SPar { .. } => "SP Analysis",
            Analysis::Pss { .. }
            | Analysis::HarmonicBalance { .. }
            | Analysis::Stability { .. }
            | Analysis::TransientNoise { .. }
            | Analysis::SpectreSweep { .. }
            | Analysis::SpectreMonteCarlo { .. }
            | Analysis::SpectrePac { .. }
            | Analysis::SpectrePnoise { .. }
            | Analysis::SpectrePxf { .. }
            | Analysis::SpectrePstb { .. } => {
                let kind = format!("{a:?}");
                return Err(missing(&format!(
                    "matchable {}",
                    kind.split([' ', '{']).next().unwrap_or_default()
                )));
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
