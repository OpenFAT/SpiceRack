//! `CircuitIR` -> netlist text, one dialect per simulator: the dispatch
//! (`netlist`, `card`) and the helpers more than one dialect writer shares.
//! Cites: none here; each dialect module cites its simulator's manual.
//!
//! Contents:
//! - `pub enum CodeGenError`
//! - `pub use crate::ir::Dialect`
//! - `pub fn netlist(ir: &CircuitIR, d: Dialect) -> Result<String, CodeGenError>`
//! - `pub fn card(comp: &Component, d: Dialect) -> Result<String, CodeGenError>`
//! - `pub fn format_spice_number(v: f64) -> String`
//! - `pub(crate) fn model_refuse(backend: &str, name: &str, model: &Option<String>) -> Result<(), CodeGenError>`

#[macro_use]
mod append;
pub mod laplace;
mod spectre;
pub(crate) mod spice3;
pub(crate) mod statistics;
mod vacask;
pub mod xschem;

use std::borrow::Cow;

pub use crate::ir::Dialect;
use crate::ir::*;

/// Why no deck was written: every variant names what the target cannot
/// spell, never a silently different deck.
#[derive(Debug, thiserror::Error)]
pub enum CodeGenError {
    /// A component, model, flag or raw line `backend` has no spelling for; no
    /// deck is written.
    #[error("Unsupported component for backend {backend}: {component}")]
    UnsupportedComponent { backend: String, component: String },
    /// An analysis or option `backend` cannot run or spell; no deck is written.
    #[error("Unsupported analysis for backend {backend}: {analysis}")]
    UnsupportedAnalysis { backend: String, analysis: String },
    /// Any other refusal (an unreadable Laplace expression, a card the target
    /// dialect would misread), with its reason; no deck is written.
    #[error("Code generation error: {0}")]
    Other(String),
}

/// [`params`], written straight into the deck by its `Display`.
struct Params<'a>(&'a [(String, String)], fn(&str) -> Cow<'_, str>);

/// The whole deck for `d`. An analysis or component the simulator cannot run
/// is an error, never a silently different deck.
pub fn netlist(ir: &CircuitIR, d: Dialect) -> Result<String, CodeGenError> {
    // Statistical calls and `statistics` tables in the target's own form
    // (`docs/research/monte-carlo-spectre.md` §10): Spectre lifts the calls
    // into normalized variables, SPICE and VACASK lower the tables to calls.
    let spice = !matches!(
        d,
        Dialect::Spectre | Dialect::Vacask | Dialect::Cdl | Dialect::Verilog
    );
    let lowered = match d {
        Dialect::Spectre => statistics::lift(ir)?,
        Dialect::Vacask => {
            let calls = statistics::vacask_calls(ir)?;
            let ir = calls.as_ref().unwrap_or(ir);
            statistics::lower(ir, true)?.or(calls)
        }
        _ if spice => statistics::lower(ir, false)?,
        _ => None,
    };
    let ir = lowered.as_ref().unwrap_or(ir);
    check_caret_meaning(ir, d)?;
    check_functions(ir, d)?;
    match d {
        Dialect::Ngspice | Dialect::Ltspice | Dialect::Espice | Dialect::Hspice => {
            spice3::netlist(d, ir)
        }
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
        Dialect::Ngspice | Dialect::Ltspice | Dialect::Espice | Dialect::Hspice => {
            let mut s = String::new();
            spice3::component(&mut s, d, false, comp)?;
            Ok(s)
        }
        _ => Err(CodeGenError::Other(format!(
            "card: {d:?} has no single-card writer"
        ))),
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

/// An R/C/L naming a `.model` (ngspice's `R1 a b rmod`) has no spelling in
/// `backend`'s element line; only spice3-family dialects write it.
pub(crate) fn model_refuse(
    backend: &str,
    name: &str,
    model: &Option<String>,
) -> Result<(), CodeGenError> {
    match model {
        Some(m) => Err(CodeGenError::UnsupportedComponent {
            backend: backend.into(),
            component: format!("{name}: model reference '{m}'"),
        }),
        None => Ok(()),
    }
}

impl std::fmt::Display for Params<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (k, v) in self.0 {
            if v.is_empty() {
                write!(f, " {k}")?
            } else {
                write!(f, " {k}={}", (self.1)(v))?
            }
        }
        Ok(())
    }
}

// ---- private ----

/// `^` read as power and `^` read as XOR are different expressions wearing
/// the same character; carrying one into the other dialect would silently
/// change what the deck computes. Refuse instead of guessing.
fn check_caret_meaning(ir: &CircuitIR, d: Dialect) -> Result<(), CodeGenError> {
    let from = ir.raw_dialect.unwrap_or(Dialect::Ngspice);
    if caret_means_power(from) == caret_means_power(d) {
        return Ok(());
    }
    let exprs = expressions(ir);
    if let Some(bad) = exprs.into_iter().find(|e| e.contains('^')) {
        return Err(CodeGenError::Other(format!(
            "expression '{bad}' uses '^', which means {} in {} but {} in {}; refusing to reinterpret it",
            if caret_means_power(from) {
                "power"
            } else {
                "XOR"
            },
            from.name(),
            if caret_means_power(d) { "power" } else { "XOR" },
            d.name(),
        )));
    }
    Ok(())
}

/// Every expression the deck's cards carry: functions, parameters (a
/// definition's own included), model, instance and component values, the
/// stimulus. Raw lines are not: they reach their own dialect only.
fn expressions(ir: &CircuitIR) -> Vec<&str> {
    let mut exprs: Vec<&str> = ir.functions.iter().map(|f| f.body.as_str()).collect();
    for sc in ir.subcircuits() {
        let params = sc.parameters.iter().chain(&sc.locals);
        exprs.extend(params.filter_map(|p| p.default.as_deref()));
        let models = sc.models.iter().flat_map(|m| &m.parameters);
        exprs.extend(models.map(|(_, v)| v.as_str()));
        let instances = sc.instances.iter().flat_map(|i| &i.parameters);
        exprs.extend(instances.map(|(_, v)| v.as_str()));
        exprs.extend(sc.components.iter().flat_map(Component::exprs));
    }
    if let Some(tb) = &ir.testbench {
        exprs.extend(tb.stimulus.iter().flat_map(Component::exprs));
    }
    exprs
}

/// Writing a statistical function where `d` has none would drop its random
/// term or call something else: refuse, naming the function. HSPICE,
/// ngspice (and ESPice) and Xyce have the five (ngspice manual §2.11.5);
/// Spectre gets them lifted and VACASK has four and a lowered `limit`
/// (`statistics`); LTspice's `gauss(x)` is a different function of one
/// argument, and it has none of the others.
fn check_functions(ir: &CircuitIR, d: Dialect) -> Result<(), CodeGenError> {
    let has = match d {
        Dialect::Ngspice | Dialect::Espice | Dialect::Hspice | Dialect::Xyce => true,
        // The xschem layouts (LVS) are not evaluated.
        Dialect::Cdl | Dialect::Verilog => true,
        Dialect::Spectre | Dialect::Vacask => true,
        Dialect::Ltspice => false,
    };
    if has {
        return Ok(());
    }
    for e in expressions(ir) {
        let stat = |f: &&str| {
            statistics::STATISTICAL
                .iter()
                .any(|s| s.eq_ignore_ascii_case(f))
        };
        if let Some(f) = calls(e).find(stat) {
            return Err(CodeGenError::Other(format!(
                "expression '{e}' calls the statistical function {f}(), which {} has no function for; refusing to drop its random term",
                d.name()
            )));
        }
    }
    Ok(())
}

/// The names `e` calls: each identifier followed by `(`.
fn calls(e: &str) -> impl Iterator<Item = &str> {
    let b = e.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    (0..b.len()).filter_map(move |i| {
        let starts = b[i].is_ascii_alphabetic() && (i == 0 || !ident(b[i - 1]) && b[i - 1] != b'.');
        if !starts {
            return None;
        }
        let end = i + b[i..].iter().take_while(|&&c| ident(c)).count();
        let open = end
            + b[end..]
                .iter()
                .take_while(|c| c.is_ascii_whitespace())
                .count();
        (b.get(open) == Some(&b'(')).then(|| &e[i..end])
    })
}

/// A SPICE value without its expression delimiters (`{e}`, `'e'`): Spectre
/// and VACASK read the bare expression, and a quote there is a string
/// (Spectre) or a quoted name (VACASK).
fn undelimited(v: &str) -> &str {
    let t = v.trim();
    let inner = t.strip_prefix('{').and_then(|r| r.strip_suffix('}'));
    inner
        .or_else(|| t.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
        .map_or(t, str::trim)
}

/// `^` means power in ngspice, HSPICE, ESPice and Spectre (as SpiceRack
/// writes it), but XOR in LTspice, Xyce and VACASK. `true` here means
/// "power".
fn caret_means_power(d: Dialect) -> bool {
    !matches!(d, Dialect::Ltspice | Dialect::Xyce | Dialect::Vacask)
}

/// `raw_spice` lines are SPICE text; a non-SPICE dialect cannot place them,
/// and dropping them would be a silently different deck.
/// Raw lines imported from `backend`'s own language are its text and pass.
fn refuse_raw_spice(ir: &CircuitIR, backend: &str) -> Result<(), CodeGenError> {
    refuse_flags(ir, backend)?;
    if ir.raw_dialect.map(Dialect::name) == Some(backend) {
        return Ok(());
    }
    let sub = ir.subcircuits().find(|s| !s.raw_spice.is_empty());
    match sub {
        Some(s) => Err(CodeGenError::UnsupportedComponent {
            backend: backend.into(),
            component: format!("raw SPICE line '{}' in {}", s.raw_spice[0].trim(), s.name),
        }),
        None => Ok(()),
    }
}

/// Spectre and VACASK have no valueless parameter: refuse a flag rather than
/// write `off=` or drop it.
fn refuse_flags(ir: &CircuitIR, backend: &str) -> Result<(), CodeGenError> {
    for sc in ir.subcircuits() {
        let model_flag = sc
            .models
            .iter()
            .flat_map(|m| m.parameters.iter().map(move |(k, v)| (&m.name, k, v)))
            .find(|(_, _, v)| v.is_empty());
        if let Some((m, k, _)) = model_flag {
            return Err(CodeGenError::UnsupportedComponent {
                backend: backend.into(),
                component: format!("model {m}: flag '{k}' has no {backend} spelling"),
            });
        }
        for c in &sc.components {
            if let Some((k, _)) = c.params().iter().find(|(_, v)| v.is_empty()) {
                return Err(CodeGenError::UnsupportedComponent {
                    backend: backend.into(),
                    component: format!(
                        "{}: flag '{k}' has no {backend} spelling",
                        c.name().unwrap_or("device")
                    ),
                });
            }
        }
    }
    Ok(())
}

/// A SPICE signal (`V(out)`, `i(v1)`) as the bare node or device name;
/// other text trimmed. Spectre and VACASK name nodes bare: `[REF]` p.502
/// `nodeset 7=0 out=1`, p.524 `sens (q1:betadc 2 Out) ...`.
fn bare_node(s: &str) -> &str {
    let t = s.trim();
    let inner = ["V(", "v(", "I(", "i("]
        .into_iter()
        .find_map(|p| t.strip_prefix(p))
        .and_then(|r| r.strip_suffix(')'));
    inner.map_or(t, str::trim)
}

/// ` k=v k=v` — instance parameters in SPICE and Spectre alike, each value
/// through `respell`. An empty value is a flag (`off`, VDMOS `pchan`),
/// written bare (Spectre refuses flags before it gets here).
fn params(params: &[(String, String)], respell: fn(&str) -> Cow<'_, str>) -> Params<'_> {
    Params(params, respell)
}

/// A value respelled as itself, for [`params`] and [`param_defs`].
fn verbatim(v: &str) -> Cow<'_, str> {
    Cow::Borrowed(v)
}

/// ` a=1 b` — parameter declarations (SPICE `PARAMS:`, Spectre `parameters`),
/// defaults through `respell` (a SPICE `1meg` reads as milli in Spectre),
/// written onto `out`.
fn param_defs(out: &mut String, defs: &[ParamDef], respell: fn(&str) -> Cow<'_, str>) {
    for p in defs {
        match &p.default {
            Some(d) => w!(out, " {}={}", p.name, respell(d)),
            None => w!(out, " {}", p.name),
        }
    }
}

/// `key=value` pairs of an options card: portable names through `map`, then
/// the ones only `backend` understands, verbatim.
/// `key=value`, or the bare `key` of a flag (an empty value: `.options noacct`).
fn option_pairs(opts: &SimOptions, backend: &str, map: fn(&str) -> &str) -> Vec<String> {
    let portable = opts.portable.iter().map(|(k, v)| {
        if v.is_empty() {
            map(k).to_string()
        } else {
            format!("{}={}", map(k), v)
        }
    });
    let specific = opts
        .backend_specific
        .get(backend)
        .into_iter()
        .flatten()
        .map(|(k, v)| format!("{k}={v}"));
    portable.chain(specific).collect()
}
