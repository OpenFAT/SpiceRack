//! SPICE statistical calls and Spectre `statistics` tables across dialects:
//! the call scanner every pass
//! shares and the walk over every expression with its owner and scope;
//! `lift` turns calls into Spectre's normalized variables and back
//! (`unlift`), `lower` turns `statistics` tables into SPICE and VACASK
//! calls, `vacask` spells calls VACASK lacks.
//!
//! Cites: ngspice manual §2.11.5; VACASK `docs/cmd-analysis-mc.md`;
//! Spectre Reference 19.1 pp.175-179, p.481.
//!
//! Contents:
//! - `pub(crate) fn rewrite_calls(e: &str, f: &mut dyn FnMut(&str, &[String]) -> Option<String>) -> String`
//! - `pub(crate) fn has_statistical(e: &str) -> bool`
//! - `pub(crate) fn rename_ident(e: &str, from: &str, to: &str) -> String`,
//!   `pub(crate) fn rename_idents(e: &str, map: &[(&str, String)]) -> String`
//! - `pub(crate) fn exprs_mut(ir: &mut CircuitIR, f: &mut dyn FnMut(&Site, &mut String))`
//! - `pub(crate) fn sub_exprs_mut(sc: &mut Subcircuit, f: &mut dyn FnMut(&mut String))`
//! - `pub(crate) struct Site`, `pub(crate) const STATISTICAL: [&str; 5]`
//! - re-exported: `lift::{lift, unlift}`, `lower::lower`, `vacask::vacask_calls`

mod lift;
mod lower;
#[cfg(all(test, feature = "backend"))]
mod tests;
mod vacask;

pub(crate) use lift::{lift, unlift};
pub(crate) use lower::lower;
pub(crate) use vacask::vacask_calls;

use crate::ir::*;

/// Where an expression is evaluated: the owner and field that name its
/// variables, the subcircuit it sits in (`None`: a deck's or section's top,
/// sampled once per run: Spectre `process`), and the library section it
/// belongs to.
pub(crate) struct Site {
    pub(crate) owner: String,
    pub(crate) field: String,
    pub(crate) subckt: Option<String>,
    pub(crate) section: Option<usize>,
}

/// The five statistical functions (ngspice §2.11.5): a nominal value plus
/// a random draw.
pub(crate) const STATISTICAL: [&str; 5] = ["agauss", "gauss", "aunif", "unif", "limit"];

/// `e` with each call `name(args)` replaced by `f(name, args)` when that is
/// `Some`, inner calls first; a call `f` leaves alone keeps its text.
pub(crate) fn rewrite_calls(
    e: &str,
    f: &mut dyn FnMut(&str, &[String]) -> Option<String>,
) -> String {
    let b = e.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = String::with_capacity(e.len());
    let mut i = 0;
    while i < b.len() {
        let starts = b[i].is_ascii_alphabetic() && (i == 0 || !ident(b[i - 1]) && b[i - 1] != b'.');
        if !starts {
            let ch = e[i..].chars().next().expect("i is a char boundary");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let end = i + b[i..].iter().take_while(|&&c| ident(c)).count();
        let open = end
            + b[end..]
                .iter()
                .take_while(|c| c.is_ascii_whitespace())
                .count();
        let close = (b.get(open) == Some(&b'('))
            .then(|| matching(b, open))
            .flatten();
        let Some(close) = close else {
            out.push_str(&e[i..end]);
            i = end;
            continue;
        };
        let args: Vec<String> = split_args(&e[open + 1..close])
            .into_iter()
            .map(|a| rewrite_calls(a, f))
            .collect();
        match f(&e[i..end], &args) {
            Some(r) => out.push_str(&r),
            None => {
                out.push_str(&e[i..open]);
                out.push('(');
                out.push_str(&args.join(","));
                out.push(')');
            }
        }
        i = close + 1;
    }
    out
}

/// Whether `e` calls one of [`STATISTICAL`] (a three-argument `limit`, Xyce's
/// clamp, is not one).
pub(crate) fn has_statistical(e: &str) -> bool {
    let mut found = false;
    rewrite_calls(e, &mut |name, args| {
        let n = name.to_ascii_lowercase();
        found |= STATISTICAL.contains(&n.as_str()) && !(n == "limit" && args.len() == 3);
        None
    });
    found
}

/// `e` with every identifier `from` (as a whole word) renamed `to`.
pub(crate) fn rename_ident(e: &str, from: &str, to: &str) -> String {
    rename_idents(e, &[(from, to.to_string())])
}

/// `e` with each identifier in `map` renamed at once, so one renaming's
/// result is never renamed again.
pub(crate) fn rename_idents(e: &str, map: &[(&str, String)]) -> String {
    let b = e.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = String::with_capacity(e.len());
    let mut i = 0;
    while i < b.len() {
        if ident(b[i]) && (i == 0 || !ident(b[i - 1])) {
            let end = i + b[i..].iter().take_while(|&&c| ident(c)).count();
            let word = &e[i..end];
            out.push_str(
                map.iter()
                    .find(|(f, _)| *f == word)
                    .map_or(word, |(_, t)| t.as_str()),
            );
            i = end;
        } else {
            let ch = e[i..].chars().next().expect("i is a char boundary");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Calls `f` on every expression of `ir` with its [`Site`]: parameters and
/// locals, model, instance and component values, in every body (the top,
/// its definitions, each section's body and definitions) and the stimulus.
pub(crate) fn exprs_mut(ir: &mut CircuitIR, f: &mut dyn FnMut(&Site, &mut String)) {
    body(&mut ir.top, None, None, f);
    for sc in &mut ir.subcircuit_defs {
        let name = sc.name.clone();
        body(sc, Some(name), None, f);
    }
    for (k, s) in ir.sections.iter_mut().enumerate() {
        body(&mut s.body, None, Some(k), f);
        for sc in &mut s.subcircuit_defs {
            let name = sc.name.clone();
            body(sc, Some(name), Some(k), f);
        }
    }
    if let Some(tb) = &mut ir.testbench {
        for c in &mut tb.stimulus {
            component(c, &None, None, f);
        }
    }
}

/// Calls `f` on every expression of one body (`exprs_mut`'s walk).
pub(crate) fn sub_exprs_mut(sc: &mut Subcircuit, f: &mut dyn FnMut(&mut String)) {
    body(sc, None, None, &mut |_, e| f(e));
}

// ---- private ----

/// The `)` matching the `(` at `open`, or `None`.
fn matching(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (k, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// A call's argument text split at its top-level commas, blanks kept.
fn split_args(s: &str) -> Vec<&str> {
    if s.trim().is_empty() {
        return vec![];
    }
    let (mut depth, mut start, mut out) = (0i32, 0, Vec::new());
    for (k, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

fn body(
    sc: &mut Subcircuit,
    subckt: Option<String>,
    section: Option<usize>,
    f: &mut dyn FnMut(&Site, &mut String),
) {
    let site = |owner: &str, field: &str| Site {
        owner: owner.to_string(),
        field: field.to_string(),
        subckt: subckt.clone(),
        section,
    };
    for p in sc.parameters.iter_mut().chain(&mut sc.locals) {
        if let Some(d) = &mut p.default {
            f(&site(&p.name, ""), d);
        }
    }
    for m in &mut sc.models {
        for (k, v) in &mut m.parameters {
            f(&site(&m.name, k), v);
        }
    }
    for i in &mut sc.instances {
        for (k, v) in &mut i.parameters {
            f(&site(&format!("x{}", i.name), k), v);
        }
    }
    for c in &mut sc.components {
        component(c, &subckt, section, f);
    }
    for c in &mut sc.conditionals {
        for b in &mut c.branches {
            body(&mut b.body, subckt.clone(), section, f);
        }
    }
}

fn component(
    c: &mut Component,
    subckt: &Option<String>,
    section: Option<usize>,
    f: &mut dyn FnMut(&Site, &mut String),
) {
    let letter = match c {
        Component::Resistor { .. } => "r",
        Component::Capacitor { .. } | Component::ChargeCapacitor { .. } => "c",
        Component::Inductor { .. } => "l",
        Component::VoltageSource { .. } => "v",
        Component::CurrentSource { .. } => "i",
        Component::Diode { .. } => "d",
        Component::Bjt { .. } => "q",
        Component::Mosfet { .. } => "m",
        _ => "e",
    };
    let owner = format!("{letter}{}", c.name().unwrap_or_default());
    let site = |field: &str| Site {
        owner: owner.clone(),
        field: field.to_string(),
        subckt: subckt.clone(),
        section,
    };
    match c {
        Component::Resistor { value, params, .. }
        | Component::Capacitor { value, params, .. }
        | Component::Inductor { value, params, .. } => {
            if let IrValue::Expression { expr } = value {
                f(&site(letter), expr);
            }
            for (k, v) in params {
                f(&site(k), v);
            }
        }
        Component::ChargeCapacitor { charge, params, .. } => {
            f(&site("q"), charge);
            for (k, v) in params {
                f(&site(k), v);
            }
        }
        c => {
            for e in c.exprs_mut() {
                f(&site("e"), e);
            }
        }
    }
}
