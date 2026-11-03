//! IR numbers and values -> their spelling in one SPICE3-family dialect, and
//! the dialect predicates every spice3 writer shares.
//!
//! Contents:
//! - `pub(super) fn num(d: Dialect, x: bool, v: f64) -> String`
//! - `pub(super) fn dec(d: Dialect, x: bool, v: f64) -> String`
//! - `pub(super) fn key(d: Dialect) -> &'static str`
//! - `pub(super) fn ngspice(d: Dialect) -> bool`
//! - `pub(super) fn value(d: Dialect, x: bool, v: &IrValue) -> Cow<'_, str>`
//! - `pub(super) fn spelled(d: Dialect) -> fn(&str) -> Cow<'_, str>`

use std::borrow::Cow;

use crate::codegen::Dialect;
use crate::ir::*;

/// A number as `d` reads it back: ngspice (and ESPice, reading its decks)
/// computes a mantissa times `pow(10, e)`, so its spelling is picked to land
/// on exactly the same double ([`crate::unit::ngspice_spelling`]).
pub(super) fn num(d: Dialect, x: bool, v: f64) -> String {
    if x && ngspice(d) {
        crate::unit::ngspice_spelling(v)
    } else {
        crate::codegen::format_spice_number(v)
    }
}

/// `v` as plain decimal, unless ngspice would read that back as another
/// double (then [`num`]'s exact spelling).
pub(super) fn dec(d: Dialect, x: bool, v: f64) -> String {
    let s = format!("{v}");
    if !(x && ngspice(d)) || crate::unit::ngspice_value(&s) == Some(v) {
        s
    } else {
        crate::unit::ngspice_spelling(v)
    }
}

/// ESPice reads ngspice decks, so everywhere but the model loads it is ngspice
/// — including the `backend_paths`/`backend_specific` key it looks up.
pub(super) fn key(d: Dialect) -> &'static str {
    d.family().name()
}

/// ngspice proper, or ESPice reading an ngspice deck.
pub(super) fn ngspice(d: Dialect) -> bool {
    matches!(d, Dialect::Ngspice | Dialect::Espice)
}

/// An IR value as `d` reads it: a number through [`num`], an expression in
/// `d`'s brackets (`{..}`, HSPICE `'..'`), raw text as written.
pub(super) fn value(d: Dialect, x: bool, v: &IrValue) -> Cow<'_, str> {
    match v {
        IrValue::Numeric { value } => num(d, x, *value).into(),
        // HSPICE reads braces as name bytes; only quotes make an expression.
        IrValue::Expression { expr } if d == Dialect::Hspice => format!("'{}'", expr).into(),
        IrValue::Expression { expr } => format!("{{{}}}", expr).into(),
        IrValue::Raw { text } => text.into(),
    }
}

/// How `d` spells a parameter value kept as SPICE text: as written, but for
/// HSPICE, which reads braces as name bytes (#203) and an unquoted value as
/// one word (blanks only inside parentheses) starting with a letter, or a
/// number (hspice.peg X1, CR10 p.238), any other expression is quoted (`{a*2}` is `'a*2'`, `{a} + b` is
/// `'(a) + b'`, `a * b` is `'a * b'`); a run of numbers (a matrix) is not.
pub(super) fn spelled(d: Dialect) -> fn(&str) -> Cow<'_, str> {
    if d == Dialect::Hspice {
        hspice_value
    } else {
        crate::codegen::verbatim
    }
}

// ---- private ----

/// [`spelled`] for HSPICE.
fn hspice_value(v: &str) -> Cow<'_, str> {
    let t = v.trim();
    let number = |s: &str| {
        let m = s.strip_prefix(['-', '+']).unwrap_or(s);
        crate::unit::parse_spice_number(m)
            .is_some_and(|(_, end, _)| m[end..].bytes().all(|c| c.is_ascii_alphabetic()))
    };
    let word = |s: &str| s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
    let quoted = (t.starts_with('\'') || t.starts_with('"')) && t.len() > 1;
    // Words split at blanks outside parentheses (`unif(1k, 0.1)` is one); a
    // run of numbers is a vector (a CPL model's matrix), not an expression.
    let mut depth = 0i32;
    let words: Vec<&str> = t
        .split(|c: char| {
            depth += i32::from(c == '(') - i32::from(c == ')');
            depth == 0 && c.is_whitespace()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let plain = match words[..] {
        [w] => number(w) || word(w),
        _ => words.iter().all(|w| number(w)),
    };
    if !t.contains('{') && (quoted || plain) {
        return v.into();
    }
    let t = t
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
        .unwrap_or(t);
    let mut depth = 0i32;
    let closes = t.char_indices().find(|&(_, c)| {
        depth += i32::from(c == '{') - i32::from(c == '}');
        depth == 0
    });
    let single = t.starts_with('{') && closes.is_some_and(|(i, _)| i + 1 == t.len());
    let e = if single { &t[1..t.len() - 1] } else { t };
    format!("'{}'", e.trim().replace('{', "(").replace('}', ")")).into()
}
