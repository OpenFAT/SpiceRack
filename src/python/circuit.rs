//! Circuit: the top-level `Subcircuit` plus its child definitions and
//! circuit-wide `.options`/`.temp` → a `CircuitIR` for a simulator or the
//! ngspice deck `str()` prints.
//!
//! Contents:
//! - `pub(super) struct PyCircuit` (Python `Circuit`, extends `Subcircuit`)
//! - `pub(super) fn hoisted(top: &mut ir::Subcircuit, child: &ir::Subcircuit) -> ir::Subcircuit`

use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use super::args::{kw_params, num};
use super::deck::{check_backend_name, emit_ir_netlist};
use super::simulator::PySimulator;
use super::subcircuit::PySubcircuit;
use crate::ir::{self, CircuitIR, Component};

#[pyclass(name = "Circuit", extends = PySubcircuit)]
pub(super) struct PyCircuit {
    defs: Vec<ir::Subcircuit>,
    /// Circuit-level `.options` / `.temp`; every simulator starts from these.
    tb: ir::Testbench,
}

/// The definition to store for `child`, its `include`/`lib`/`osdi`
/// directives moved to the deck's top level (once each), which is where this
/// API places them. The IR itself keeps a definition's directives inside it:
/// SPICE scopes a `.lib` in a `.subckt` to that subcircuit.
pub(super) fn hoisted(top: &mut ir::Subcircuit, child: &ir::Subcircuit) -> ir::Subcircuit {
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
    for osdi in &child.osdi_loads {
        if !top.osdi_loads.contains(osdi) {
            top.osdi_loads.push(osdi.clone());
        }
    }
    let mut def = child.clone();
    def.includes.clear();
    def.libs.clear();
    def.osdi_loads.clear();
    def
}

// ---- private ----

#[pymethods]
impl PyCircuit {
    #[new]
    fn new(title: &str) -> (Self, PySubcircuit) {
        let top = ir::Subcircuit {
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
        self.tb.options.portable.extend(kw_params(kwargs)?);
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
        let top = &slf.as_super().inner;
        let one = if let Some(c) = top.components.iter().find(|c| c.name() == Some(name)) {
            ir::Subcircuit {
                components: vec![c.clone()],
                ..Default::default()
            }
        } else if let Some(i) = top.instances.iter().find(|i| i.name == name) {
            ir::Subcircuit {
                instances: vec![i.clone()],
                ..Default::default()
            }
        } else {
            return Err(PyKeyError::new_err(format!("Element '{name}' not found")));
        };
        let deck = emit_ir_netlist(
            CircuitIR {
                top: one,
                ..Default::default()
            },
            "ngspice",
        )?;
        // Drop the title line and `.end`.
        let lines: Vec<&str> = deck.lines().collect();
        Ok(lines[1..lines.len() - 1].join("\n"))
    }

    fn element(slf: PyRef<'_, Self>, name: &str) -> PyResult<String> {
        Self::__getitem__(slf, name)
    }

    fn set_source_value(mut slf: PyRefMut<'_, Self>, name: &str, value: f64) -> PyResult<()> {
        for c in &mut slf.as_super().inner.components {
            match c {
                Component::VoltageSource {
                    name: n, value: v, ..
                }
                | Component::CurrentSource {
                    name: n, value: v, ..
                } if n == name => {
                    *v = num(value);
                    return Ok(());
                }
                c if c.name() == Some(name) => {
                    return Err(PyKeyError::new_err(format!(
                        "{name} is not a voltage or current source"
                    )));
                }
                _ => {}
            }
        }
        Err(PyKeyError::new_err(format!("element '{name}' not found")))
    }

    fn node(&self, name: &str) -> String {
        name.to_string()
    }

    /// The ngspice deck, titled with `.title` (line 1 is the title either way).
    fn __str__(slf: PyRef<'_, Self>) -> PyResult<String> {
        let deck = emit_ir_netlist(Self::ir(&slf), "ngspice")?;
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
            ir: Self::ir(&slf),
            backend: simulator.map(str::to_string),
            save_currents: false,
        })
    }
}

impl PyCircuit {
    fn ir(slf: &PyRef<'_, Self>) -> CircuitIR {
        CircuitIR {
            top: slf.as_super().inner.clone(),
            testbench: Some(slf.tb.clone()),
            subcircuit_defs: slf.defs.clone(),
            ..Default::default()
        }
    }
}
