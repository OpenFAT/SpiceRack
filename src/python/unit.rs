//! Units: `spicerack.unit` names → `Unit` objects, and `value @ unit` →
//! `UnitValue`. Cites `crate::unit::UNITS`, the table every name comes from.
//!
//! Contents:
//! - `pub(super) struct PyUnit` (Python `Unit`)
//! - `pub(super) struct PyUnitValue` (Python `UnitValue`)
//! - `pub(super) fn create_unit_module(m: &Bound<'_, PyModule>) -> PyResult<()>`

use pyo3::prelude::*;

use crate::unit as u;

#[pyclass(name = "Unit")]
#[derive(Clone)]
pub(super) struct PyUnit {
    mult: f64,
    symbol: &'static str,
}

#[pyclass(name = "UnitValue")]
#[derive(Clone)]
pub(super) struct PyUnitValue {
    pub(super) inner: u::UnitValue,
}

pub(super) fn create_unit_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let unit_mod = PyModule::new(m.py(), "unit")?;
    for &(name, mult, symbol) in u::UNITS {
        unit_mod.add(name, PyUnit { mult, symbol })?;
    }

    m.add_submodule(&unit_mod)?;

    // Register in sys.modules so `from spicerack.unit import ...` works
    let sys = m.py().import("sys")?;
    let modules = sys.getattr("modules")?;
    modules.set_item("spicerack.unit", &unit_mod)?;

    Ok(())
}

// ---- private ----

#[pymethods]
impl PyUnit {
    fn __rmatmul__(&self, value: f64) -> PyUnitValue {
        PyUnitValue {
            inner: u::UnitValue {
                value: u::scale(value, self.mult),
                symbol: self.symbol,
            },
        }
    }

    fn __repr__(&self) -> String {
        format!("Unit({}, {:?})", self.mult, self.symbol)
    }
}

#[pymethods]
impl PyUnitValue {
    #[getter]
    fn value(&self) -> f64 {
        self.inner.value
    }

    fn str_spice(&self) -> String {
        self.inner.str_spice()
    }

    fn __float__(&self) -> f64 {
        self.inner.value
    }

    fn __repr__(&self) -> String {
        self.inner.to_string()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }
}
