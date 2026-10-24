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
        // A Xyce deck with its analyses; `xschem::write` is the xschem layout.
        Dialect::Xyce => spice3::netlist(d, ir),
        Dialect::Cdl | Dialect::Verilog => xschem::write(&xschem::Deck::new(ir), d),
    }
}

/// One component's card in a SPICE-family dialect (ngspice, LTspice,
/// HSPICE, ESPice): what `netlist` writes for it.
pub fn card(comp: &Component, d: Dialect) -> Result<String, CodeGenError> {
    match d {
        Dialect::Ngspice | Dialect::Ltspice | Dialect::Espice | Dialect::Hspice => spice3::component(d, comp),
        _ => Err(CodeGenError::Other(format!("card: {d:?} has no single-card writer"))),
    }
}

/// `raw_spice` lines are SPICE text; a non-SPICE dialect cannot place them,
/// and dropping them would be a silently different deck.
/// Raw lines imported from `backend`'s own language are its text and pass.
fn refuse_raw_spice(ir: &CircuitIR, backend: &str) -> Result<(), CodeGenError> {
    refuse_flags(ir, backend)?;
    if ir.raw_dialect.as_deref() == Some(backend) {
        return Ok(());
    }
    let sub = std::iter::once(&ir.top).chain(&ir.subcircuit_defs).find(|s| !s.raw_spice.is_empty());
    match sub {
        Some(s) => Err(CodeGenError::UnsupportedComponent {
            backend: backend.into(),
            component: format!("raw SPICE line '{}' in {}", s.raw_spice[0].trim(), s.name),
        }),
        None => Ok(()),
    }
}

/// `v` in SPICE notation, with an SI suffix when that spelling reads back as
/// exactly `v` (`10k`, `2.5n`), else the shortest exact exponent form
/// (`2.0000000000000002e-7`). Rounding the mantissa (`200n` for that value)
/// would hand the simulator a different number.
pub fn format_spice_number(v: f64) -> String {
    let (mult, suffix) = crate::unit::si_prefix(v);
    let exp = mult.log10().round() as i32; // the prefixes are powers of ten
    let m = crate::unit::unscale(v, mult);
    let rounded = m.round();
    let whole = (rounded.abs() < 1e15).then(|| format!("{}", rounded as i64));
    for mantissa in whole.into_iter().chain([format!("{m}")]) {
        if format!("{mantissa}e{exp}").parse::<f64>() == Ok(v) {
            return mantissa + suffix;
        }
    }
    format!("{v:e}")
}

/// ` k=v k=v` — instance parameters in SPICE and Spectre alike.
/// An empty value is a flag (`off`, VDMOS `pchan`), written bare.
fn params(params: &[(String, String)]) -> String {
    params.iter().map(|(k, v)| if v.is_empty() { format!(" {k}") } else { format!(" {k}={v}") }).collect()
}

/// Spectre and VACASK have no valueless parameter: refuse a flag rather than
/// write `off=` or drop it.
fn refuse_flags(ir: &CircuitIR, backend: &str) -> Result<(), CodeGenError> {
    for sc in std::iter::once(&ir.top).chain(&ir.subcircuit_defs) {
        let model_flag = sc.models.iter().flat_map(|m| m.parameters.iter().map(move |(k, v)| (&m.name, k, v))).find(|(_, _, v)| v.is_empty());
        if let Some((m, k, _)) = model_flag {
            return Err(CodeGenError::UnsupportedComponent { backend: backend.into(), component: format!("model {m}: flag '{k}' has no {backend} spelling") });
        }
        for c in &sc.components {
            let ps = match c {
                Component::Resistor { params, .. } | Component::Capacitor { params, .. } | Component::Inductor { params, .. }
                | Component::Diode { params, .. } | Component::Bjt { params, .. } | Component::Mosfet { params, .. }
                | Component::Jfet { params, .. } | Component::Mesfet { params, .. } => params,
                _ => continue,
            };
            if let Some((k, _)) = ps.iter().find(|(_, v)| v.is_empty()) {
                return Err(CodeGenError::UnsupportedComponent { backend: backend.into(), component: format!("{}: flag '{k}' has no {backend} spelling", c.name().unwrap_or("device")) });
            }
        }
    }
    Ok(())
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
/// `key=value`, or the bare `key` of a flag (an empty value: `.options noacct`).
fn option_pairs(opts: &SimOptions, backend: &str, map: fn(&str) -> &str) -> Vec<String> {
    let portable = opts.portable.iter().map(|(k, v)| if v.is_empty() { map(k).to_string() } else { format!("{}={}", map(k), v) });
    let specific = opts.backend_specific.get(backend).into_iter().flatten().map(|(k, v)| format!("{k}={v}"));
    portable.chain(specific).collect()
}

