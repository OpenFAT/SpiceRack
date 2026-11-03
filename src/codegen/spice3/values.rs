//! IR numbers and values -> their spelling in one SPICE3-family dialect, and
//! the dialect predicates every spice3 writer shares.

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

pub(super) fn value(d: Dialect, x: bool, v: &IrValue) -> Cow<'_, str> {
    match v {
        IrValue::Numeric { value } => num(d, x, *value).into(),
        // HSPICE reads braces as name bytes; only quotes make an expression.
        IrValue::Expression { expr } if d == Dialect::Hspice => format!("'{}'", expr).into(),
        IrValue::Expression { expr } => format!("{{{}}}", expr).into(),
        IrValue::Raw { text } => text.into(),
    }
}
