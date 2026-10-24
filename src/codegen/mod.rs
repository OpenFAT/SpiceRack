//! `CircuitIR` -> netlist text, one dialect per simulator.

pub mod laplace;
pub(crate) mod spice3;
mod spectre;
mod vacask;
pub mod xschem;

use crate::ir::*;

#[derive(Debug, thiserror::Error)]
pub enum CodeGenError {
    #[error("Unsupported component for backend {backend}: {component}")]
    UnsupportedComponent { backend: String, component: String },
    #[error("Unsupported analysis for backend {backend}: {analysis}")]
    UnsupportedAnalysis { backend: String, analysis: String },
    #[error("Code generation error: {0}")]
    Other(String),
}

/// The netlist language a simulator reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Ngspice,
    Ltspice,
    Spectre,
    Vacask,
    /// ngspice text with Verilog-A loaded as `.hdl` instead of `pre_osdi`.
    Espice,
    /// HSPICE: `'expr'` quoting, `E`/`G` `vol=`/`cur=` behavioural sources,
    /// `.hdl` Verilog-A, and the card forms in ESPice's
    /// `tests/frontend/grammar/hspice.peg` and `tests/conformance/tests/netlist/hspice`.
    Hspice,
    /// Xyce: the xschem layout only ([`xschem`]).
    Xyce,
    /// CDL for LVS: the xschem layout only ([`xschem`]).
    Cdl,
    /// Structural Verilog: the xschem layout only ([`xschem`]).
    Verilog,
}

/// The whole deck for `d`. An analysis or component the simulator cannot run
/// is an error, never a silently different deck.
pub fn netlist(ir: &CircuitIR, d: Dialect) -> Result<String, CodeGenError> {
    match d {
        Dialect::Ngspice | Dialect::Ltspice | Dialect::Espice | Dialect::Hspice => spice3::netlist(d, ir),
        Dialect::Spectre => spectre::netlist(ir),
        Dialect::Vacask => vacask::netlist(ir),
        Dialect::Xyce | Dialect::Cdl | Dialect::Verilog => xschem::write(&xschem::Deck::new(ir), d),
    }
}

/// Format f64 in SPICE notation (use SI prefix when clean)
pub fn format_spice_number(v: f64) -> String {
    let (mult, suffix) = crate::unit::si_prefix(v);
    let mantissa = v / mult;
    // Round to avoid floating point artifacts (10e-6/1e-6 = 10.000000000000002)
    let rounded = mantissa.round();
    if (mantissa - rounded).abs() < 1e-9 && rounded.abs() < 1e15 {
        format!("{}{}", rounded as i64, suffix)
    } else {
        format!("{}{}", mantissa, suffix)
    }
}

/// ` k=v k=v` — instance parameters in SPICE and Spectre alike.
fn params(params: &[(String, String)]) -> String {
    params.iter().map(|(k, v)| format!(" {k}={v}")).collect()
}

/// ` a=1 b` — parameter declarations (SPICE `PARAMS:`, Spectre `parameters`).
fn param_defs(defs: &[ParamDef]) -> String {
    defs.iter()
        .map(|p| match &p.default {
            Some(d) => format!(" {}={}", p.name, d),
            None => format!(" {}", p.name),
        })
        .collect()
}

/// `key=value` pairs of an options card: portable names through `map`, then
/// the ones only `backend` understands, verbatim.
fn option_pairs(opts: &SimOptions, backend: &str, map: fn(&str) -> &str) -> Vec<String> {
    let portable = opts.portable.iter().map(|(k, v)| format!("{}={}", map(k), v));
    let specific = opts.backend_specific.get(backend).into_iter().flatten().map(|(k, v)| format!("{k}={v}"));
    portable.chain(specific).collect()
}

