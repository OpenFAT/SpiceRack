//! Deck: a `CircuitIR` → its netlist text for one backend, or → the
//! `RawData` of running it. Verilog blocks are lowered here first, so
//! neither codegen nor a backend sees one.
//!
//! Contents:
//! - `pub(super) fn lowered(ir: CircuitIR) -> PyResult<CircuitIR>`
//! - `pub(super) fn emit_ir_netlist(ir: CircuitIR, backend: &str) -> PyResult<String>`
//! - `pub(super) fn run_deck(ir: CircuitIR, backend: Option<&str>) -> PyResult<RawData>`
//! - `pub(super) fn issues(ir: &CircuitIR, backend: &str) -> Vec<String>`
//! - `pub(super) fn check_backend_name(name: &str) -> PyResult<()>`
//! - `pub(super) fn runtime_err(e: impl std::fmt::Display) -> PyErr`

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use super::verilog::lower_verilog;
use crate::ir::{CircuitIR, Dialect};
use crate::result::RawData;

/// Expand every Verilog block into what simulates it, in `ir`'s top and definitions.
pub(super) fn lowered(mut ir: CircuitIR) -> PyResult<CircuitIR> {
    for sc in std::iter::once(&mut ir.top).chain(&mut ir.subcircuit_defs) {
        lower_verilog(sc).map_err(runtime_err)?;
    }
    Ok(ir)
}

/// The deck `backend` runs for `ir`.
pub(super) fn emit_ir_netlist(ir: CircuitIR, backend: &str) -> PyResult<String> {
    let ir = lowered(ir)?;
    // Every runnable dialect, and HSPICE, which is written but never run here.
    let d = Dialect::from_backend_name(&backend.to_ascii_lowercase())
        .filter(|d| *d == Dialect::Hspice || crate::backend::RUNNABLE.contains(d))
        .ok_or_else(|| PyValueError::new_err(format!("unknown backend '{backend}'")))?;
    crate::codegen::netlist(&ir, d).map_err(runtime_err)
}

/// Runs the lowered `ir`; `backend` `None` lets `simulation::run` choose.
pub(super) fn run_deck(ir: CircuitIR, backend: Option<&str>) -> PyResult<RawData> {
    crate::simulation::run(&lowered(ir)?, backend).map_err(runtime_err)
}

/// `ir.check_backend(backend)` as `"Severity: message"` lines.
pub(super) fn issues(ir: &CircuitIR, backend: &str) -> Vec<String> {
    ir.check_backend(backend)
        .into_iter()
        .map(|issue| format!("{:?}: {}", issue.severity, issue.message))
        .collect()
}

/// Refuse a backend name at selection time rather than at the first run.
pub(super) fn check_backend_name(name: &str) -> PyResult<()> {
    if crate::backend::parse(name).is_some() {
        return Ok(());
    }
    let names = crate::backend::RUNNABLE
        .map(Dialect::name)
        .into_iter()
        .chain(Dialect::ALIASES.map(|a| a.0));
    Err(PyValueError::new_err(format!(
        "unknown backend {name:?}; expected one of {}",
        names.collect::<Vec<_>>().join(", ")
    )))
}

/// Any error as a Python `RuntimeError` carrying its `Display` text.
pub(super) fn runtime_err(e: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}
