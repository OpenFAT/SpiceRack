//! Arguments: Python values and `**kwargs` → IR values (`IrValue`,
//! `IrNum`, `(name, text)` parameter pairs). Cites no spec: SPICE's own
//! number syntax is why a `UnitValue` is written in SI form.
//!
//! Contents:
//! - `pub(super) enum PyValueArg`, `PyValueArg::ir(self) -> IrValue`
//! - `impl FromPyObject for ir::IrNum`
//! - `pub(super) fn kw_params(params: Option<Bound<'_, PyDict>>) -> PyResult<Vec<(String, String)>>`
//! - `pub(super) fn kw_volts(kwargs: Option<Bound<'_, PyDict>>) -> PyResult<Vec<(String, f64)>>`
//! - `pub(super) fn num(value: f64) -> IrValue`

use pyo3::prelude::*;
use pyo3::types::PyDict;

use super::unit::PyUnitValue;
use crate::ir::{self, IrValue};

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

/// Shorthand for `IrValue::Numeric { value }`.
pub(super) fn num(value: f64) -> IrValue {
    IrValue::Numeric { value }
}

impl PyValueArg {
    /// The IR value; an expression loses one pair of enclosing braces.
    pub(super) fn ir(self) -> IrValue {
        match self {
            Self::Float(value) => IrValue::Numeric { value },
            Self::Unit(uv) => IrValue::Numeric {
                value: uv.inner.value,
            },
            Self::Expr(e) => {
                let t = e.trim();
                let inner = t
                    .strip_prefix('{')
                    .and_then(|t| t.strip_suffix('}'))
                    .unwrap_or(t);
                IrValue::Expression {
                    expr: inner.trim().to_string(),
                }
            }
        }
    }
}

/// A waveform parameter: a number, a unit value, or an expression string.
impl<'py> FromPyObject<'py> for ir::IrNum {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(match ob.extract::<PyValueArg>()?.ir() {
            IrValue::Numeric { value } => ir::IrNum::Num(value),
            IrValue::Expression { expr } | IrValue::Raw { text: expr } => ir::IrNum::Expr(expr),
        })
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
