//! The one pipeline: `CircuitIR` → codegen in the selected backend's dialect
//! → backend runs the text verbatim → `RawData`.

use crate::backend::{detect_and_select_with_features, BackendError};
use crate::ir::CircuitIR;
use crate::result::RawData;

/// Run `ir`'s testbench on `backend` (or the best detected backend for its
/// first analysis and features), tagging the result with the backend name and
/// its parsed `.meas` values.
pub fn run(ir: &CircuitIR, backend: Option<&str>) -> Result<RawData, BackendError> {
    let kind = ir.testbench.as_ref()
        .and_then(|tb| tb.analyses.first())
        .map_or("op", |a| a.kind_str());
    let sim = detect_and_select_with_features(kind, backend, &(&ir.compute_features()).into())?;
    let netlist = sim.codegen().emit_netlist(ir)
        .map_err(|e| BackendError::SimulationError(e.to_string()))?;
    let mut raw = sim.run_netlist(&netlist)?;
    raw.backend_hint = sim.name().to_string();
    // LTspice writes `.meas` results to its log, everyone else to stdout.
    let text = if raw.backend_hint == "ltspice" && !raw.log_content.is_empty() { &raw.log_content } else { &raw.stdout };
    if !text.is_empty() {
        raw.measures = crate::measure_parse::parse_measures(text, &raw.backend_hint);
    }
    Ok(raw)
}
