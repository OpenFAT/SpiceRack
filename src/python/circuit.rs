//! Circuit: the top-level `Subcircuit` plus its child definitions and
//! circuit-wide `.options`/`.temp` → a `Deck` for a simulator or the
//! ngspice deck `str()` prints.
//!
//! Contents:
//! - `pub(super) struct PyCircuit` (Python `Circuit`, extends `Subcircuit`)
//! - `pub(super) fn hoisted(top: &mut Body, child: &Body) -> Body`

use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use super::args::kw_params;
use super::deck::{Deck, check_backend_name};
use super::model::{Bench, Body};
use super::simulator::PySimulator;
use super::subcircuit::PySubcircuit;

#[pyclass(name = "Circuit", extends = PySubcircuit)]
pub(super) struct PyCircuit {
    defs: Vec<Body>,
    /// Circuit-level `.options` / `.temp`; every simulator starts from these.
    tb: Bench,
}

/// The definition to store for `child`, its `include`/`lib`/`osdi`
/// directives moved to the deck's top level (once each), which is where this
/// API places them. A definition's own directives would stay inside it:
/// SPICE scopes a `.lib` in a `.subckt` to that subcircuit.
pub(super) fn hoisted(top: &mut Body, child: &Body) -> Body {
    for inc in &child.includes {
        if !top.includes.contains(inc) {
            top.includes.push(inc.clone());
        }
    }
    for lib in &child.libs {
        if !top.libs.contains(lib) {
            top.libs.push(lib.clone());
        }
    }
    for osdi in &child.osdi {
        if !top.osdi.contains(osdi) {
            top.osdi.push(osdi.clone());
        }
    }
    let mut def = child.clone();
    def.includes.clear();
    def.libs.clear();
    def.osdi.clear();
    def
}

// ---- private ----

#[pymethods]
impl PyCircuit {
    #[new]
    fn new(title: &str) -> (Self, PySubcircuit) {
        let top = Body {
            name: title.to_string(),
            ..Default::default()
        };
        (
            Self {
                defs: vec![],
                tb: Default::default(),
            },
            PySubcircuit {
                inner: top,
                ground_alias: true,
            },
        )
    }

    #[pyo3(signature = (**kwargs))]
    fn options(&mut self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.tb.options.extend(kw_params(kwargs)?);
        Ok(())
    }

    fn temp(&mut self, temperature: f64) {
        self.tb.temperature = Some(temperature);
    }

    fn subcircuit(mut slf: PyRefMut<'_, Self>, subckt: &PySubcircuit) {
        let def = hoisted(&mut slf.as_super().inner, &subckt.inner);
        slf.defs.push(def);
    }

    /// The card of the first component or instance called `name` (without its letter).
    fn __getitem__(slf: PyRef<'_, Self>, name: &str) -> PyResult<String> {
        Deck::card(&slf.as_super().inner, name)
    }

    fn element(slf: PyRef<'_, Self>, name: &str) -> PyResult<String> {
        Self::__getitem__(slf, name)
    }

    fn set_source_value(mut slf: PyRefMut<'_, Self>, name: &str, value: f64) -> PyResult<()> {
        let top = &mut slf.as_super().inner;
        let Some(c) = top.elements.iter_mut().find(|c| c.name == name) else {
            return Err(PyKeyError::new_err(format!("element '{name}' not found")));
        };
        if !c.set_value(value) {
            return Err(PyKeyError::new_err(format!(
                "{name} is not a voltage or current source"
            )));
        }
        Ok(())
    }

    fn node(&self, name: &str) -> String {
        name.to_string()
    }

    /// The ngspice deck, titled with `.title` (line 1 is the title either way).
    fn __str__(slf: PyRef<'_, Self>) -> PyResult<String> {
        let deck = Self::deck(&slf).netlist("ngspice")?;
        Ok(format!(".title{}", deck.strip_prefix('*').unwrap_or(&deck)))
    }

    fn __repr__(slf: PyRef<'_, Self>) -> String {
        format!("Circuit('{}')", slf.as_super().inner.name)
    }

    #[pyo3(signature = (simulator=None))]
    fn simulator(slf: PyRef<'_, Self>, simulator: Option<&str>) -> PyResult<PySimulator> {
        if let Some(name) = simulator {
            check_backend_name(name)?;
        }
        Ok(PySimulator {
            deck: Self::deck(&slf),
            backend: simulator.map(str::to_string),
            save_currents: false,
        })
    }
}

impl PyCircuit {
    fn deck(slf: &PyRef<'_, Self>) -> Deck {
        Deck {
            top: slf.as_super().inner.clone(),
            bench: Some(slf.tb.clone()),
            defs: slf.defs.clone(),
            ..Default::default()
        }
    }
}
