//! The one pipeline: `CircuitIR` → codegen in the selected backend's dialect
//! → backend runs the text verbatim → `RawData`.

use crate::backend::{netlist, select, BackendError, Dialect};
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
    let first = ir.testbench.as_ref().and_then(|tb| tb.analyses.first()).unwrap_or(&op);
    let sim = select(first, backend, &ir.compute_features())?;
    let deck = netlist(ir, sim).map_err(|e| BackendError::SimulationError(e.to_string()))?;
    let mut plots = sim.run_all(&deck)?;
    // LTspice writes `.meas` results to its log, everyone else to stdout;
    // every plot carries the same run's text.
    let p = &plots[0];
    let text = if sim == Dialect::Ltspice && !p.log_content.is_empty() { &p.log_content } else { &p.stdout };
    let measures = if text.is_empty() { vec![] } else { crate::measure_parse::parse_measures(text, Some(sim)) };
    for p in &mut plots {
        p.backend_hint = Some(sim);
        p.measures.clone_from(&measures);
    }
    Ok(plots)
}
