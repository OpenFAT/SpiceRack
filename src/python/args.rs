//! Arguments: Python values and `**kwargs` → card fields (`Value`, a
//! waveform field `Num`, `(name, text)` parameter pairs). Cites no spec:
//! SPICE's own number syntax is why a `UnitValue` is written in SI form.
//!
//! Contents:
//! - `pub(super) enum PyValueArg`, `PyValueArg::value(self) -> Value`
//! - `pub(super) struct Num`, `impl FromPyObject for Num`, `Num::of(v: f64) -> Num`, `Num::text(&self) -> String`,
//!   `Num::zero(&self) -> bool`, `Num::value(&self) -> Value`
//! - `pub(super) fn kw_params(params: Option<Bound<'_, PyDict>>) -> PyResult<Vec<(String, String)>>`
//! - `pub(super) fn kw_volts(kwargs: Option<Bound<'_, PyDict>>) -> PyResult<Vec<(String, f64)>>`

use pyo3::prelude::*;
use pyo3::types::PyDict;

use super::model::{Value, dec};
use super::unit::PyUnitValue;

/// A value argument (`R(..., value)`, `V(..., value)`): tried as a number,
/// then a `UnitValue`, then a string.
#[derive(FromPyObject)]
pub(super) enum PyValueArg {
    Float(f64),
    Unit(PyUnitValue),
    /// An expression over parameters (`"rv*2"`, `"{rv*2}"`), written in each
    /// dialect's own quoting.
    Expr(String),
}

/// One waveform field: a number, a unit value, or an expression string.
#[derive(Clone, Debug)]
pub(super) struct Num(pub(super) Value);

/// `**kwargs` as ordered `(name, value)` pairs, values per [`param_value`].
pub(super) fn kw_params(params: Option<Bound<'_, PyDict>>) -> PyResult<Vec<(String, String)>> {
    let mut out = vec![];
    if let Some(dict) = params {
        for (k, v) in dict.iter() {
            out.push((k.extract::<String>()?, param_value(&v)?));
        }
    }
    Ok(out)
}

/// `**kwargs` of node voltages (`out=1.65`) as ordered pairs.
pub(super) fn kw_volts(kwargs: Option<Bound<'_, PyDict>>) -> PyResult<Vec<(String, f64)>> {
    let mut out = vec![];
    if let Some(dict) = kwargs {
        for (k, v) in dict.iter() {
            out.push((k.extract::<String>()?, v.extract::<f64>()?));
        }
    }
    Ok(out)
}

impl PyValueArg {
    /// The card value; an expression loses one pair of enclosing braces.
    pub(super) fn value(self) -> Value {
        match self {
            Self::Float(v) => Value::Num(v),
            Self::Unit(uv) => Value::Num(uv.inner.value),
            Self::Expr(e) => {
                let t = e.trim();
                let inner = t
                    .strip_prefix('{')
                    .and_then(|t| t.strip_suffix('}'))
                    .unwrap_or(t);
                Value::Expr(inner.trim().to_string())
            }
        }
    }
}

impl<'py> FromPyObject<'py> for Num {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(Num(ob.extract::<PyValueArg>()?.value()))
    }
}

impl Num {
    /// A number field.
    pub(super) fn of(v: f64) -> Num {
        Num(Value::Num(v))
    }

    /// The field in a waveform's parentheses: a number as plain decimal,
    /// an expression braced.
    pub(super) fn text(&self) -> String {
        match &self.0 {
            Value::Num(v) => dec(*v),
            v => v.text(),
        }
    }

    /// Whether the field is the number 0.
    pub(super) fn zero(&self) -> bool {
        matches!(self.0, Value::Num(v) if v == 0.0)
    }

    /// The field as a card value (a source's DC value).
    pub(super) fn value(&self) -> Value {
        self.0.clone()
    }
}

// ---- private ----

/// Text of a `**kwargs` parameter value on an element, model or instance card.
///
/// A `UnitValue` is written as its SI value (`1e-6`), never its display form:
/// `str(1 @ u_MOhm)` is `1MOhm`, which every SPICE reads as 1 milliohm, and
/// `1F` reads as one femtofarad. Everything else keeps its Python `str()`, so
/// expressions such as `"{w*2}"` pass through verbatim.
fn param_value(v: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(uv) = v.extract::<PyUnitValue>() {
        return Ok(format!("{:e}", uv.inner.value));
    }
    Ok(v.str()?.to_string())
}
