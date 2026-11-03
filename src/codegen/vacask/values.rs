//! IR numbers, expressions and names -> their VACASK spelling, and the two
//! refusals every VACASK writer returns.
//!
//! Contents:
//! - `pub(super) struct Q<'a>(pub(super) &'a str, pub(super) bool)` (its `Display` is the spelling)
//! - `pub(super) fn num(v: f64) -> String`
//! - `pub(crate) fn parse_spice_num(t: &str) -> Option<(f64, String)>`
//! - `pub(super) fn respell(s: &str) -> String`
//! - `pub(super) fn value_of(s: &str) -> Option<f64>`
//! - `pub(super) fn expr(s: &str, low: bool) -> String`
//! - `pub(super) fn val(v: &IrValue, low: bool) -> String`
//! - `pub(super) fn wave_num(x: &IrNum, low: bool) -> String`
//! - `pub(super) fn inst_params(out: &mut String, params: &[(String, String)], low: bool)`
//! - `pub(super) fn ac_spec(out: &mut String, mag: &Option<f64>, phase: &Option<f64>)`
//! - `pub(super) fn sanitize(s: &str) -> String`
//! - `pub(super) fn q(name: &str) -> Q<'_>`
//! - `pub(super) fn unsupported_comp(what: impl Into<String>) -> CodeGenError`
//! - `pub(super) fn unsupported_analysis(what: impl Into<String>) -> CodeGenError`

use crate::codegen::CodeGenError;
use crate::ir::*;
use std::fmt::Write as _;

/// A name as a VACASK identifier: `[a-zA-Z_$][0-9a-zA-Z_$]*`, a number, or
/// else quoted, `''` for a quote (docs/input-identifiers.md). SPICE names
/// carry `#` `/` `-` `+` freely (`mnd#/mp1`, `Net-_R1-Pad2_`). Lowercased
/// when `.1`: SPICE is case-insensitive, VACASK is not. Written as each name
/// is, so `netlist` renames nothing in a copy of the IR.
#[derive(Clone, Copy)]
pub(super) struct Q<'a>(pub(super) &'a str, pub(super) bool);

/// A plain number VACASK cannot misread. SPICE scale suffixes are
/// case-INSENSITIVE and VACASK's are not (`M` is milli in SPICE and MEGA in
/// VACASK: a factor of 1e9 with nothing to notice it), so nothing computed here
/// is ever re-spelled with a suffix.
pub(super) fn num(v: f64) -> String {
    if v == 0.0 {
        "0".to_string()
    } else {
        format!("{:e}", v)
    }
}

/// A whole token that is one signed SPICE number-with-suffix, as `(value,
/// vacask_spelling)`. A trailing unit (`5V`, `1ohm`) is ignored, as SPICE
/// ignores it.
pub(crate) fn parse_spice_num(t: &str) -> Option<(f64, String)> {
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    let (v, len, scale) = crate::unit::parse_spice_number(body)?;
    let lead = t.len() - body.len() + len; // sign and mantissa
    // A scale and a unit, nothing else: `1k*(1+x)` is an expression, and
    // reading it as `1k` would drop the rest.
    if !t[lead..].bytes().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    // VACASK spells tera and giga in capitals (`t` is not a scale there).
    let vacask = match scale {
        "t" => "T",
        "g" => "G",
        s => s,
    };
    let sign = if t.starts_with('-') { -1.0 } else { 1.0 };
    Some((sign * v, format!("{}{vacask}", &t[..lead])))
}

/// A value written as text: a number gets re-spelled in VACASK's suffixes,
/// anything else (a parameter name, an arithmetic expression — VACASK has
/// those) passes through minus the ngspice `{}` wrapper it has no use for.
pub(super) fn respell(s: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .unwrap_or(t)
        .trim();
    match parse_spice_num(t) {
        Some((_, spelled)) => spelled,
        // Case is settled by `prepared` (lowercased for SPICE-origin text).
        None => t.to_string(),
    }
}

/// Numeric value of a text field, when one is needed to compute with.
pub(super) fn value_of(s: &str) -> Option<f64> {
    parse_spice_num(
        s.trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim(),
    )
    .map(|(v, _)| v)
}

/// An expression from the IR, lowercased when `low` (a SPICE-origin IR:
/// SPICE names are case-blind, `Rload` and `rload` are one parameter, and
/// VACASK's are not) unless it is a number; a VACASK-origin IR keeps its
/// case (`M_PI` is not `m_pi`). Then [`respell`]ed.
pub(super) fn expr(s: &str, low: bool) -> String {
    // A SPICE `'e'` is an expression; a VACASK one a quoted name, kept.
    let s = if low {
        crate::codegen::undelimited(s)
    } else {
        s
    };
    if low && parse_spice_num(s.trim()).is_none() {
        respell(&s.to_ascii_lowercase())
    } else {
        respell(s)
    }
}

/// An IR value: a number through [`num`], an expression through [`expr`],
/// raw text [`respell`]ed.
pub(super) fn val(v: &IrValue, low: bool) -> String {
    match v {
        IrValue::Numeric { value } => num(*value),
        IrValue::Expression { expr: e } => expr(e, low),
        IrValue::Raw { text } => respell(text),
    }
}

/// A waveform number: [`num`], or an [`expr`].
pub(super) fn wave_num(x: &IrNum, low: bool) -> String {
    match x {
        IrNum::Num(v) => num(*v),
        IrNum::Expr(e) => expr(e, low),
    }
}

/// Instance parameters, written onto `out`. `m` is SPICE's device
/// multiplier, which VACASK spells with the Verilog-A builtin.
pub(super) fn inst_params(out: &mut String, params: &[(String, String)], low: bool) {
    for (k, v) in params {
        let k = k.to_ascii_lowercase();
        let k = if k == "m" { "$mfactor" } else { &k };
        w!(out, " {}={}", k, expr(v, low));
    }
}

/// `mag=`/`phase=` on an independent source.
pub(super) fn ac_spec(out: &mut String, mag: &Option<f64>, phase: &Option<f64>) {
    if let Some(m) = mag {
        w!(out, " mag={}", num(*m));
    }
    if let Some(p) = phase {
        w!(out, " phase={}", num(*p));
    }
}

/// VACASK identifiers cannot carry punctuation.
pub(super) fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// [`Q`], lowercased: how every name reaches the deck.
pub(super) fn q(name: &str) -> Q<'_> {
    Q(name, true)
}

impl std::fmt::Display for Q<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.0;
        let plain = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
            && (name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
                // A number node is `0` or has no leading zero (`06` is not one).
                || (name.chars().all(|c| c.is_ascii_digit()) && (name == "0" || !name.starts_with('0'))));
        let quoted = !plain && !name.is_empty();
        if quoted {
            f.write_char('\'')?;
        }
        for c in name.chars() {
            if quoted && c == '\'' {
                f.write_char('\'')?;
            }
            f.write_char(if self.1 { c.to_ascii_lowercase() } else { c })?;
        }
        if quoted {
            f.write_char('\'')?;
        }
        Ok(())
    }
}

/// The refusal of a component (or model, source, line) VACASK cannot spell.
pub(super) fn unsupported_comp(what: impl Into<String>) -> CodeGenError {
    CodeGenError::UnsupportedComponent {
        backend: "vacask".into(),
        component: what.into(),
    }
}

/// The refusal of an analysis, sweep or option VACASK cannot spell.
pub(super) fn unsupported_analysis(what: impl Into<String>) -> CodeGenError {
    CodeGenError::UnsupportedAnalysis {
        backend: "vacask".into(),
        analysis: what.into(),
    }
}
