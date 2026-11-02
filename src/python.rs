//! PyO3 Python bindings — identical API surface to original SpiceRack.
//!
//! ```python
//! from spicerack import Circuit
//! from spicerack.unit import *
//! ```
//!
//! Every class here builds `ir` data directly; netlists come only from codegen.


use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::exceptions::{PyKeyError, PyAttributeError, PyRuntimeError, PyValueError};
use pyo3::types::{PyDict, PyList};

use crate::ir::{self, Analysis, CircuitIR, Component, Dialect, IrNum, IrValue, IrWaveform, PssBase};
use crate::result::*;
use crate::unit as u;

/// Convert MeasureResult vec to a Python-friendly dict
fn measures_to_dict(measures: &[crate::result::MeasureResult]) -> HashMap<String, f64> {
    measures.iter().map(|m| (m.name.clone(), m.value)).collect()
}

fn runtime_err(e: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}


// ── Unit bindings ──

#[pyclass(name = "Unit")]
#[derive(Clone)]
struct PyUnit {
    mult: f64,
    symbol: &'static str,
}

#[pyclass(name = "UnitValue")]
#[derive(Clone)]
struct PyUnitValue {
    inner: u::UnitValue,
}

#[pymethods]
impl PyUnit {
    fn __rmatmul__(&self, value: f64) -> PyUnitValue {
        PyUnitValue { inner: u::UnitValue { value: u::scale(value, self.mult), symbol: self.symbol } }
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

// ── Argument helpers ──

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

/// `**kwargs` as ordered `(name, value)` pairs, values per [`param_value`].
fn kw_params(params: Option<Bound<'_, PyDict>>) -> PyResult<Vec<(String, String)>> {
    let mut out = vec![];
    if let Some(dict) = params {
        for (k, v) in dict.iter() {
            out.push((k.extract::<String>()?, param_value(&v)?));
        }
    }
    Ok(out)
}

/// `**kwargs` of node voltages (`out=1.65`) as ordered pairs.
fn kw_volts(kwargs: Option<Bound<'_, PyDict>>) -> PyResult<Vec<(String, f64)>> {
    let mut out = vec![];
    if let Some(dict) = kwargs {
        for (k, v) in dict.iter() {
            out.push((k.extract::<String>()?, v.extract::<f64>()?));
        }
    }
    Ok(out)
}

/// Refuse a backend name at selection time rather than at the first run.
fn check_backend_name(name: &str) -> PyResult<()> {
    if crate::backend::parse(name).is_some() {
        return Ok(());
    }
    let names = crate::backend::RUNNABLE.map(Dialect::name).into_iter().chain(Dialect::ALIASES.map(|a| a.0));
    Err(PyValueError::new_err(format!("unknown backend {name:?}; expected one of {}", names.collect::<Vec<_>>().join(", "))))
}

/// Extract DC sweep params: kwargs like Vinput=slice(-2, 5, 0.01)
fn dc_sweeps(kwargs: Option<Bound<'_, PyDict>>) -> PyResult<Analysis> {
    let mut sweeps = Vec::new();
    if let Some(dict) = kwargs {
        for (k, v) in dict.iter() {
            sweeps.push(ir::DcSweep {
                source: k.extract::<String>()?,
                start: v.getattr("start")?.extract::<f64>()?,
                stop: v.getattr("stop")?.extract::<f64>()?,
                step: v.getattr("step")?.extract::<f64>()?,
            });
        }
    }
    if sweeps.is_empty() {
        return Err(PyValueError::new_err("dc() requires at least one sweep parameter"));
    }
    Ok(Analysis::Dc { sweeps })
}

#[derive(FromPyObject)]
enum PyValueArg {
    Float(f64),
    Unit(PyUnitValue),
    /// An expression over parameters (`"rv*2"`, `"{rv*2}"`), written in each
    /// dialect's own quoting.
    Expr(String),
}

impl PyValueArg {
    fn ir(self) -> IrValue {
        match self {
            Self::Float(value) => IrValue::Numeric { value },
            Self::Unit(uv) => IrValue::Numeric { value: uv.inner.value },
            Self::Expr(e) => {
                let t = e.trim();
                let inner = t.strip_prefix('{').and_then(|t| t.strip_suffix('}')).unwrap_or(t);
                IrValue::Expression { expr: inner.trim().to_string() }
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

fn num(value: f64) -> IrValue {
    IrValue::Numeric { value }
}



/// One of two spellings of the same argument (`dc`/`dc_offset`), never both.
fn either(a: Option<f64>, b: Option<f64>, an: &str, bn: &str) -> PyResult<Option<f64>> {
    match (a, b) {
        (Some(_), Some(_)) => Err(pyo3::exceptions::PyTypeError::new_err(format!("pass {an} or {bn}, not both"))),
        _ => Ok(a.or(b)),
    }
}

/// A waveform source: its DC value is `dc`, else the waveform's t=0 value.
fn wave_source(
    current: bool, name: &str, np: String, nm: String, wf: IrWaveform,
    dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
) -> Component {
    let value = dc.map_or_else(|| wf.at_zero(), num);
    source(current, name, np, nm, value, ac, ac_phase, Some(wf))
}

/// A `V` (or, with `current`, an `I`) source.
fn source(
    current: bool, name: &str, np: String, nm: String, value: IrValue,
    ac_magnitude: Option<f64>, ac_phase: Option<f64>, waveform: Option<IrWaveform>,
) -> Component {
    let name = name.to_string();
    if current {
        Component::CurrentSource { name, np, nm, value, ac_magnitude, ac_phase, waveform }
    } else {
        Component::VoltageSource { name, np, nm, value, ac_magnitude, ac_phase, waveform, port: None }
    }
}

// ── Netlist emission and running ──

/// Expand every Verilog block into what simulates it, in `ir`'s top and definitions.
fn lowered(mut ir: CircuitIR) -> PyResult<CircuitIR> {
    for sc in std::iter::once(&mut ir.top).chain(&mut ir.subcircuit_defs) {
        lower_verilog(sc).map_err(runtime_err)?;
    }
    Ok(ir)
}

/// The deck `backend` runs for `ir`.
fn emit_ir_netlist(ir: CircuitIR, backend: &str) -> PyResult<String> {
    let ir = lowered(ir)?;
    // Every runnable dialect, and HSPICE, which is written but never run here.
    let d = Dialect::from_backend_name(&backend.to_ascii_lowercase())
        .filter(|d| *d == Dialect::Hspice || crate::backend::RUNNABLE.contains(d))
        .ok_or_else(|| PyValueError::new_err(format!("unknown backend '{backend}'")))?;
    crate::codegen::netlist(&ir, d).map_err(runtime_err)
}

fn run_deck(ir: CircuitIR, backend: Option<&str>) -> PyResult<RawData> {
    crate::simulation::run(&lowered(ir)?, backend).map_err(runtime_err)
}

fn issues(ir: &CircuitIR, backend: &str) -> Vec<String> {
    ir.check_backend(backend).into_iter()
        .map(|issue| format!("{:?}: {}", issue.severity, issue.message))
        .collect()
}

// ── Circuit: the top-level Subcircuit plus its definitions and directives ──

#[pyclass(name = "Circuit", extends = PySubcircuit)]
struct PyCircuit {
    defs: Vec<ir::Subcircuit>,
    /// Circuit-level `.options` / `.temp`; every simulator starts from these.
    tb: ir::Testbench,
}

#[pymethods]
impl PyCircuit {
    #[new]
    fn new(title: &str) -> (Self, PySubcircuit) {
        let top = ir::Subcircuit { name: title.to_string(), ..Default::default() };
        (Self { defs: vec![], tb: Default::default() }, PySubcircuit { inner: top, ground_alias: true })
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
            ir::Subcircuit { components: vec![c.clone()], ..Default::default() }
        } else if let Some(i) = top.instances.iter().find(|i| i.name == name) {
            ir::Subcircuit { instances: vec![i.clone()], ..Default::default() }
        } else {
            return Err(PyKeyError::new_err(format!("Element '{name}' not found")));
        };
        let deck = emit_ir_netlist(CircuitIR { top: one, ..Default::default() }, "ngspice")?;
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
                Component::VoltageSource { name: n, value: v, .. }
                | Component::CurrentSource { name: n, value: v, .. } if n == name => {
                    *v = num(value);
                    return Ok(());
                }
                c if c.name() == Some(name) => {
                    return Err(PyKeyError::new_err(format!("{name} is not a voltage or current source")));
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
        Ok(PySimulator { ir: Self::ir(&slf), backend: simulator.map(str::to_string), save_currents: false })
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

/// The definition to store for `child`, its `include`/`lib`/`osdi`
/// directives moved to the deck's top level (once each), which is where this
/// API places them. The IR itself keeps a definition's directives inside it:
/// SPICE scopes a `.lib` in a `.subckt` to that subcircuit.
fn hoisted(top: &mut ir::Subcircuit, child: &ir::Subcircuit) -> ir::Subcircuit {
    for inc in &child.includes {
        if !top.includes.contains(inc) { top.includes.push(inc.clone()); }
    }
    for lib in &child.libs {
        if !top.libs.contains(lib) { top.libs.push(lib.clone()); }
    }
    for osdi in &child.osdi_loads {
        if !top.osdi_loads.contains(osdi) { top.osdi_loads.push(osdi.clone()); }
    }
    let mut def = child.clone();
    def.includes.clear();
    def.libs.clear();
    def.osdi_loads.clear();
    def
}

/// One `Analysis` per card that both forms build: `CircuitSimulator.x()`
/// runs it, `Testbench.add_x()` queues it.
mod card {
    use pyo3::{exceptions::PyValueError, PyResult};

    use crate::ir::{Analysis, FreqSweep};

    /// Where a Python sweep name becomes a [`crate::ir::Sweep`]: anything
    /// but dec, oct, lin or log (any case) is a `ValueError` here.
    pub fn sweep(variation: &str, points: u32, start: f64, stop: f64) -> PyResult<FreqSweep> {
        Ok(FreqSweep { variation: variation.parse().map_err(PyValueError::new_err)?, points, start, stop })
    }

    pub fn ac(variation: &str, points: u32, start: f64, stop: f64) -> PyResult<Analysis> {
        Ok(Analysis::Ac { sweep: sweep(variation, points, start, stop)? })
    }

    pub fn transient(step: f64, stop: f64, start: Option<f64>, max_step: Option<f64>, uic: bool) -> Analysis {
        Analysis::Transient { step, stop, start, max_step, uic }
    }

    pub fn noise(output: &str, reference: &str, source: &str, variation: &str, points: u32, start: f64, stop: f64, points_per_summary: Option<u32>) -> PyResult<Analysis> {
        let sweep = sweep(variation, points, start, stop)?;
        Ok(Analysis::Noise { output: output.into(), reference: reference.into(), source: source.into(), sweep, points_per_summary })
    }

    pub fn tf(output: &str, source: &str) -> Analysis {
        Analysis::Tf { output: output.into(), source: source.into() }
    }

    /// `.sens`: DC without `ac`, else over that AC sweep.
    pub fn sensitivity(output: &str, ac: Option<FreqSweep>) -> Analysis {
        Analysis::Sensitivity { output: output.into(), ac }
    }

    /// `tf_type` is vol or cur, `pz_type` pol, zer or pz (any case); else
    /// a `ValueError`.
    pub fn polezero(nodes: [&str; 4], tf_type: &str, pz_type: &str) -> PyResult<Analysis> {
        let [node1, node2, node3, node4] = nodes.map(String::from);
        let (tf_type, pz_type) = (tf_type.parse().map_err(PyValueError::new_err)?, pz_type.parse().map_err(PyValueError::new_err)?);
        Ok(Analysis::PoleZero { node1, node2, node3, node4, tf_type, pz_type })
    }

    pub fn distortion(variation: &str, points: u32, start: f64, stop: f64, f2overf1: Option<f64>) -> PyResult<Analysis> {
        Ok(Analysis::Distortion { sweep: sweep(variation, points, start, stop)?, f2overf1 })
    }

    pub fn spectre_sweep(param: &str, start: f64, stop: f64, step: f64, inner: &str, inner_type: &str) -> PyResult<Analysis> {
        let inner_type = inner_type.parse().map_err(PyValueError::new_err)?;
        Ok(Analysis::SpectreSweep { param: param.into(), start, stop, step, inner: inner.into(), inner_type })
    }

    pub fn spectre_monte_carlo(iterations: u32, inner: &str, inner_type: &str, seed: Option<u64>) -> PyResult<Analysis> {
        let inner_type = inner_type.parse().map_err(PyValueError::new_err)?;
        Ok(Analysis::SpectreMonteCarlo { iterations, inner: inner.into(), inner_type, seed })
    }
}

// ── CircuitSimulator: an IR deck plus a backend choice ──

#[pyclass(name = "CircuitSimulator", subclass)]
struct PySimulator {
    /// `testbench` holds the config; each analysis call runs a copy with only its analysis.
    ir: CircuitIR,
    backend: Option<String>,
    save_currents: bool,
}

impl PySimulator {
    fn tb(&mut self) -> &mut ir::Testbench {
        self.ir.testbench.get_or_insert_with(Default::default)
    }

    fn stim(&mut self, c: Component) {
        self.tb().stimulus.push(c);
    }

    /// A copy of the deck that runs `analysis` alone.
    fn deck(&self, analysis: Analysis) -> CircuitIR {
        let mut ir = self.ir.clone();
        let tb = ir.testbench.get_or_insert_with(Default::default);
        tb.analyses = vec![analysis];
        if self.save_currents {
            tb.saves = vec!["all".into(), "@all[i]".into()];
        }
        ir
    }

    fn run(&self, analysis: Analysis) -> PyResult<RawData> {
        run_deck(self.deck(analysis), self.backend.as_deref())
    }

    /// Spectre-only analyses always run on Spectre.
    fn run_spectre(&self, analysis: Analysis) -> PyResult<RawData> {
        run_deck(self.deck(analysis), Some("spectre"))
    }
}

#[pymethods]
impl PySimulator {
    // ── Config ──

    #[pyo3(signature = (**kwargs))]
    fn options(&mut self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.tb().options.portable.extend(kw_params(kwargs)?);
        Ok(())
    }

    #[pyo3(signature = (**kwargs))]
    fn initial_condition(&mut self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.tb().initial_conditions.extend(kw_volts(kwargs)?);
        Ok(())
    }

    #[pyo3(signature = (**kwargs))]
    fn node_set(&mut self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.tb().node_sets.extend(kw_volts(kwargs)?);
        Ok(())
    }

    #[pyo3(signature = (*args))]
    fn save(&mut self, args: Vec<String>) {
        self.tb().saves.extend(args);
    }

    #[pyo3(signature = (*args))]
    fn measure(&mut self, args: Vec<String>) {
        self.tb().measures.push(args.join(" "));
    }

    #[setter]
    fn set_save_currents(&mut self, v: bool) {
        self.save_currents = v;
    }

    #[getter]
    fn get_save_currents(&self) -> bool {
        self.save_currents
    }

    #[setter]
    fn set_temperature(&mut self, temp: f64) {
        self.tb().temperature = Some(temp);
    }

    #[setter]
    fn set_nominal_temperature(&mut self, temp: f64) {
        self.tb().nominal_temperature = Some(temp);
    }

    #[pyo3(signature = (param, start, stop, step))]
    fn step(&mut self, param: &str, start: f64, stop: f64, step: f64) {
        self.tb().step_params.push(ir::StepParam { param: param.to_string(), start, stop, step, sweep_type: None });
    }

    #[pyo3(signature = (param, start, stop, step, sweep_type))]
    fn step_sweep(&mut self, param: &str, start: f64, stop: f64, step: f64, sweep_type: &str) -> PyResult<()> {
        let sweep_type = Some(sweep_type.parse().map_err(PyValueError::new_err)?);
        self.tb().step_params.push(ir::StepParam { param: param.to_string(), start, stop, step, sweep_type });
        Ok(())
    }

    // ── Analyses ──

    fn operating_point(&self) -> PyResult<PyOperatingPoint> {
        Ok(PyOperatingPoint { inner: Results::from_raw(self.run(Analysis::Op)?) })
    }

    #[pyo3(signature = (**kwargs))]
    fn dc(&self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<PyDcAnalysis> {
        Ok(PyDcAnalysis { inner: Results::from_raw(self.run(dc_sweeps(kwargs)?)?) })
    }

    #[pyo3(signature = (variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e9))]
    fn ac(&self, variation: &str, number_of_points: u32, start_frequency: f64, stop_frequency: f64) -> PyResult<PyAcAnalysis> {
        let a = card::ac(variation, number_of_points, start_frequency, stop_frequency)?;
        Ok(PyAcAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (step_time, end_time, start_time=None, max_time=None, use_initial_condition=false))]
    fn transient(
        &self, step_time: f64, end_time: f64, start_time: Option<f64>, max_time: Option<f64>,
        use_initial_condition: bool,
    ) -> PyResult<PyTransientAnalysis> {
        let a = card::transient(step_time, end_time, start_time, max_time, use_initial_condition);
        Ok(PyTransientAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (output_node, ref_node, src, variation="dec", points=10, start_frequency=1e3, stop_frequency=1e8, points_per_summary=None))]
    fn noise(
        &self, output_node: &str, ref_node: &str, src: &str, variation: &str, points: u32,
        start_frequency: f64, stop_frequency: f64, points_per_summary: Option<u32>,
    ) -> PyResult<PyNoiseAnalysis> {
        let a = card::noise(output_node, ref_node, src, variation, points, start_frequency, stop_frequency, points_per_summary)?;
        Ok(PyNoiseAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (outvar, insrc))]
    fn transfer_function(&self, outvar: &str, insrc: &str) -> PyResult<PyTransferFunctionAnalysis> {
        Ok(PyTransferFunctionAnalysis { inner: Results::from_raw(self.run(card::tf(outvar, insrc))?) })
    }

    #[pyo3(signature = (outvar, insrc))]
    fn tf(&self, outvar: &str, insrc: &str) -> PyResult<PyTransferFunctionAnalysis> {
        self.transfer_function(outvar, insrc)
    }

    #[pyo3(signature = (output_variable))]
    fn dc_sensitivity(&self, output_variable: &str) -> PyResult<PySensitivityAnalysis> {
        let a = card::sensitivity(output_variable, None);
        Ok(PySensitivityAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (output_variable, variation="dec", number_of_points=10, start_frequency=100.0, stop_frequency=1e5))]
    fn ac_sensitivity(
        &self, output_variable: &str, variation: &str, number_of_points: u32,
        start_frequency: f64, stop_frequency: f64,
    ) -> PyResult<PySensitivityAnalysis> {
        let a = card::sensitivity(output_variable, Some(card::sweep(variation, number_of_points, start_frequency, stop_frequency)?));
        Ok(PySensitivityAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (node1, node2, node3, node4, tf_type, pz_type))]
    fn polezero(
        &self, node1: &str, node2: &str, node3: &str, node4: &str, tf_type: &str, pz_type: &str,
    ) -> PyResult<PyPoleZeroAnalysis> {
        let a = card::polezero([node1, node2, node3, node4], tf_type, pz_type)?;
        Ok(PyPoleZeroAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (variation="dec", points=10, start_frequency=100.0, stop_frequency=1e8, f2overf1=None))]
    fn distortion(
        &self, variation: &str, points: u32, start_frequency: f64, stop_frequency: f64, f2overf1: Option<f64>,
    ) -> PyResult<PyDistortionAnalysis> {
        let a = card::distortion(variation, points, start_frequency, stop_frequency, f2overf1)?;
        Ok(PyDistortionAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    /// `sc_iter`, `steady_coeff`, `uic`: ngspice's shooting controls
    /// (`sciter`, `steadycoeff`, `uic`; defaults 50 and 1e-3 when unset).
    #[pyo3(signature = (fundamental_frequency, stabilization_time, observe_node, points_per_period=128, harmonics=10, sc_iter=None, steady_coeff=None, uic=false))]
    fn pss(
        &self, fundamental_frequency: f64, stabilization_time: f64, observe_node: &str,
        points_per_period: u32, harmonics: u32, sc_iter: Option<u32>, steady_coeff: Option<f64>, uic: bool,
    ) -> PyResult<PyPssAnalysis> {
        let a = Analysis::Pss {
            fundamental: fundamental_frequency, stabilization: stabilization_time,
            observe_node: observe_node.into(), points_per_period, harmonics, sc_iter, steady_coeff, uic,
        };
        Ok(PyPssAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (variation="dec", number_of_points=10, start_frequency=1e6, stop_frequency=1e10))]
    fn s_param(&self, variation: &str, number_of_points: u32, start_frequency: f64, stop_frequency: f64) -> PyResult<PySParamAnalysis> {
        let a = Analysis::SPar { sweep: card::sweep(variation, number_of_points, start_frequency, stop_frequency)? };
        Ok(PySParamAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    /// LTspice `.net` network-parameter analysis over an AC sweep.
    #[pyo3(signature = (output_current, input_source, z_in=50.0, z_out=50.0, variation="dec", points=100, start_freq=1e3, stop_freq=1e9))]
    fn network_params(
        &self, output_current: &str, input_source: &str, z_in: f64, z_out: f64,
        variation: &str, points: u32, start_freq: f64, stop_freq: f64,
    ) -> PyResult<PySParamAnalysis> {
        let mut ir = self.deck(card::ac(variation, points, start_freq, stop_freq)?);
        if let Some(tb) = &mut ir.testbench {
            tb.extra_lines.push(format!(".net {output_current} {input_source} Rout={z_out} Rin={z_in}"));
        }
        Ok(PySParamAnalysis { inner: Results::from_raw(run_deck(ir, self.backend.as_deref())?) })
    }

    #[pyo3(signature = (fundamental_frequencies, num_harmonics=None))]
    fn harmonic_balance(&self, fundamental_frequencies: Vec<f64>, num_harmonics: Option<Vec<u32>>) -> PyResult<PyHarmonicBalanceAnalysis> {
        let harmonics = num_harmonics.unwrap_or_else(|| vec![7; fundamental_frequencies.len()]);
        let a = Analysis::HarmonicBalance { frequencies: fundamental_frequencies, harmonics };
        Ok(PyHarmonicBalanceAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (probe, variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e10))]
    fn stability(
        &self, probe: &str, variation: &str, number_of_points: u32, start_frequency: f64, stop_frequency: f64,
    ) -> PyResult<PyStabilityAnalysis> {
        let a = Analysis::Stability {
            probe: probe.into(), sweep: card::sweep(variation, number_of_points, start_frequency, stop_frequency)?,
        };
        Ok(PyStabilityAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (step_time, end_time))]
    fn transient_noise(&self, step_time: f64, end_time: f64) -> PyResult<PyTransientNoiseAnalysis> {
        let a = Analysis::TransientNoise { step: step_time, stop: end_time };
        Ok(PyTransientNoiseAnalysis { inner: Results::from_raw(self.run(a)?) })
    }

    // ── Spectre-specific analyses ──

    /// Spectre parametric sweep wrapping an inner analysis (Spectre-native text).
    #[pyo3(signature = (param, start, stop, step, inner_analysis, inner_type="ac"))]
    fn spectre_sweep(
        &self, param: &str, start: f64, stop: f64, step: f64, inner_analysis: &str, inner_type: &str,
    ) -> PyResult<PyRawData> {
        Ok(PyRawData { inner: self.run_spectre(card::spectre_sweep(param, start, stop, step, inner_analysis, inner_type)?)? })
    }

    /// Spectre Monte Carlo wrapping an inner analysis.
    #[pyo3(signature = (num_iterations, inner_analysis, inner_type="ac", seed=None))]
    fn spectre_monte_carlo(&self, num_iterations: u32, inner_analysis: &str, inner_type: &str, seed: Option<u64>) -> PyResult<PyRawData> {
        Ok(PyRawData { inner: self.run_spectre(card::spectre_monte_carlo(num_iterations, inner_analysis, inner_type, seed)?)? })
    }

    /// Deprecated: [`Self::spectre_monte_carlo`], the spelling
    /// `Testbench.add_spectre_monte_carlo` uses. Warns `DeprecationWarning`.
    #[pyo3(signature = (num_iterations, inner_analysis, inner_type="ac", seed=None))]
    fn spectre_montecarlo(&self, py: Python<'_>, num_iterations: u32, inner_analysis: &str, inner_type: &str, seed: Option<u64>) -> PyResult<PyRawData> {
        let category = py.get_type::<pyo3::exceptions::PyDeprecationWarning>();
        PyErr::warn(py, &category, c"spectre_montecarlo is deprecated; use spectre_monte_carlo", 1)?;
        self.spectre_monte_carlo(num_iterations, inner_analysis, inner_type, seed)
    }

    /// SpectreRF Periodic AC (PAC) analysis with automatic PSS prerequisite.
    #[pyo3(signature = (pss_fundamental, pss_stabilization, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9, sweep_type="relative"))]
    fn spectre_pac(
        &self, pss_fundamental: f64, pss_stabilization: f64, pss_harmonics: u32,
        variation: &str, points: u32, start_freq: f64, stop_freq: f64, sweep_type: &str,
    ) -> PyResult<PyAcAnalysis> {
        let a = Analysis::SpectrePac {
            pss: PssBase { pss_fundamental, pss_stabilization, pss_harmonics },
            sweep: card::sweep(variation, points, start_freq, stop_freq)?, sweep_type: sweep_type.into(),
        };
        Ok(PyAcAnalysis { inner: Results::from_raw(self.run_spectre(a)?) })
    }

    /// SpectreRF Periodic Noise (PNoise) analysis with automatic PSS prerequisite.
    #[pyo3(signature = (pss_fundamental, pss_stabilization, output_node, ref_node, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9))]
    fn spectre_pnoise(
        &self, pss_fundamental: f64, pss_stabilization: f64, output_node: &str, ref_node: &str,
        pss_harmonics: u32, variation: &str, points: u32, start_freq: f64, stop_freq: f64,
    ) -> PyResult<PyNoiseAnalysis> {
        let a = Analysis::SpectrePnoise {
            pss: PssBase { pss_fundamental, pss_stabilization, pss_harmonics }, output: output_node.into(), reference: ref_node.into(),
            sweep: card::sweep(variation, points, start_freq, stop_freq)?,
        };
        Ok(PyNoiseAnalysis { inner: Results::from_raw(self.run_spectre(a)?) })
    }

    /// SpectreRF Periodic Transfer Function (PXF) analysis with automatic PSS prerequisite.
    #[pyo3(signature = (pss_fundamental, pss_stabilization, output_node, source, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9))]
    fn spectre_pxf(
        &self, pss_fundamental: f64, pss_stabilization: f64, output_node: &str, source: &str,
        pss_harmonics: u32, variation: &str, points: u32, start_freq: f64, stop_freq: f64,
    ) -> PyResult<PyAcAnalysis> {
        let a = Analysis::SpectrePxf {
            pss: PssBase { pss_fundamental, pss_stabilization, pss_harmonics }, output: output_node.into(), source: source.into(),
            sweep: card::sweep(variation, points, start_freq, stop_freq)?,
        };
        Ok(PyAcAnalysis { inner: Results::from_raw(self.run_spectre(a)?) })
    }

    /// SpectreRF Periodic Stability (PSTB) analysis with automatic PSS prerequisite.
    #[pyo3(signature = (pss_fundamental, pss_stabilization, probe, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9))]
    fn spectre_pstb(
        &self, pss_fundamental: f64, pss_stabilization: f64, probe: &str,
        pss_harmonics: u32, variation: &str, points: u32, start_freq: f64, stop_freq: f64,
    ) -> PyResult<PyStabilityAnalysis> {
        let a = Analysis::SpectrePstb {
            pss: PssBase { pss_fundamental, pss_stabilization, pss_harmonics }, probe: probe.into(),
            sweep: card::sweep(variation, points, start_freq, stop_freq)?,
        };
        Ok(PyStabilityAnalysis { inner: Results::from_raw(self.run_spectre(a)?) })
    }

    /// List all available simulator backends on this system
    #[staticmethod]
    fn available_backends() -> Vec<String> {
        crate::backend::available().iter().map(|b| b.name().to_string()).collect()
    }

    /// Check backend compatibility for this deck.
    fn check_backend(&self, name: &str) -> Vec<String> {
        issues(&self.ir, name)
    }

    fn __repr__(&self) -> &str {
        "CircuitSimulator"
    }
}

// ── Result bindings ──

/// Raw simulation data wrapper -- returned by spectre_sweep and spectre_montecarlo.
#[pyclass(name = "RawData")]
struct PyRawData {
    inner: crate::result::RawData,
}

#[pymethods]
impl PyRawData {
    #[getter]
    fn title(&self) -> String { self.inner.title.clone() }
    #[getter]
    fn plot_name(&self) -> String { self.inner.plot_name.clone() }
    #[getter]
    fn is_complex(&self) -> bool { self.inner.is_complex }
    #[getter]
    fn variable_names(&self) -> Vec<String> {
        self.inner.variables.iter().map(|v| v.name.clone()).collect()
    }
    fn __getitem__<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyList>> {
        float_list(py, &self.inner.re(self.column(name)?))
    }
    fn __getattr__<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyList>> {
        self.__getitem__(py, name)
    }
    /// Variable `name` as complex numbers; a real column comes back with zero imaginary parts.
    fn complex<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyList>> {
        let i = self.column(name)?;
        match self.inner.complex(i) {
            Some(c) => PyList::new(py, c),
            None => PyList::new(py, self.inner.re(i).iter().map(|&x| num_complex::Complex64::new(x, 0.0))),
        }
    }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.measures) }
}

impl PyRawData {
    fn column(&self, name: &str) -> PyResult<usize> {
        self.inner
            .variables
            .iter()
            .find(|v| v.name.eq_ignore_ascii_case(name))
            .map(|v| v.index)
            .ok_or_else(|| PyKeyError::new_err(format!("Variable '{}' not found", name)))
    }
}

/// Every plot of a `.raw` file, in file order (ngspice, ESPice, VACASK,
/// LTspice, Spectre nutbin; binary or ASCII).
///
/// ```python
/// from spicerack import read_raw
///
/// plots = read_raw("out.raw")
/// noise = next(p for p in plots if p.plot_name == "Integrated Noise")
/// ```
#[pyfunction]
fn read_raw(path: &str) -> PyResult<Vec<PyRawData>> {
    let bytes = std::fs::read(path)
        .map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("{path}: {e}")))?;
    let plots = crate::rawfile::parse_raw_all(&bytes)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("{path}: {e}")))?;
    Ok(plots.into_iter().map(|inner| PyRawData { inner }).collect())
}

#[pyclass(name = "OperatingPoint")]
struct PyOperatingPoint {
    inner: crate::result::Results,
}

#[pymethods]
impl PyOperatingPoint {
    fn __getitem__(&self, name: &str) -> PyResult<f64> {
        self.value(name).ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }

    fn __getattr__(&self, name: &str) -> PyResult<f64> {
        self.value(name).ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }

    /// Parsed .meas results as {name: value} dict
    #[getter]
    fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.raw.measures) }
}

impl PyOperatingPoint {
    fn value(&self, name: &str) -> Option<f64> {
        self.inner.get(name)?.first().copied()
    }
}

/// A column as a Python list of floats, built from the stored slice with no
/// intermediate `Vec` (pyo3 has no NumPy here, so a list is what callers
/// get, as before).
fn float_list<'py>(py: Python<'py>, xs: &[f64]) -> PyResult<Bound<'py, PyList>> {
    PyList::new(py, xs)
}

/// Per-point transform of a complex waveform, for `magnitude`/`magnitude_db`/`phase`.
fn complex_part(
    a: &crate::result::Results,
    name: &str,
    f: fn(num_complex::Complex64) -> f64,
) -> PyResult<Vec<f64>> {
    let i = a.column(name).ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))?;
    let c = a.raw.complex(i).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err(format!(
            "Node '{}' has no complex data; the backend returned a real-valued plot",
            name
        ))
    })?;
    Ok(c.iter().map(|&v| f(v)).collect())
}

/// Every swept analysis is the same `result::Results`; each Python class differs only
/// in its name and the name of its axis getter (`sweep`, `frequency`, `time`).
macro_rules! analysis_class {
    ($ty:ident, $name:literal $(, $axis:ident)?) => {
        #[pyclass(name = $name)]
        struct $ty {
            inner: crate::result::Results,
        }

        #[pymethods]
        impl $ty {
            fn __getitem__<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyList>> {
                let col = self.inner.get(name).ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))?;
                float_list(py, &col)
            }

            fn __getattr__<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyList>> {
                let col = self.inner.get(name).ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))?;
                float_list(py, &col)
            }

            $(
                #[getter]
                fn $axis<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> { float_list(py, &self.inner.axis()) }
            )?

            #[getter]
            fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.raw.measures) }

            #[getter]
            fn nodes(&self) -> Vec<String> { self.inner.nodes.keys().cloned().collect() }

            #[getter]
            fn branches(&self) -> Vec<String> { self.inner.branches.keys().cloned().collect() }

            /// |H| per point. `result[node]` returns the real part, not this.
            fn magnitude(&self, name: &str) -> PyResult<Vec<f64>> {
                complex_part(&self.inner, name, |v| v.norm())
            }

            /// 20*log10(|H|) per point.
            fn magnitude_db(&self, name: &str) -> PyResult<Vec<f64>> {
                complex_part(&self.inner, name, |v| 20.0 * v.norm().log10())
            }

            /// Phase in degrees (ngspice's own `vp()` reports radians; this does not).
            fn phase(&self, name: &str) -> PyResult<Vec<f64>> {
                complex_part(&self.inner, name, |v| v.arg().to_degrees())
            }
        }
    };
}

analysis_class!(PyDcAnalysis, "DcAnalysis", sweep);
analysis_class!(PyAcAnalysis, "AcAnalysis", frequency);
analysis_class!(PyTransientAnalysis, "TransientAnalysis", time);
analysis_class!(PyNoiseAnalysis, "NoiseAnalysis", frequency);
analysis_class!(PyTransferFunctionAnalysis, "TransferFunctionAnalysis");
analysis_class!(PySensitivityAnalysis, "SensitivityAnalysis");
analysis_class!(PyPoleZeroAnalysis, "PoleZeroAnalysis");
analysis_class!(PyDistortionAnalysis, "DistortionAnalysis", frequency);
analysis_class!(PyPssAnalysis, "PssAnalysis", time);
analysis_class!(PySParamAnalysis, "SParamAnalysis", frequency);
analysis_class!(PyHarmonicBalanceAnalysis, "HarmonicBalanceAnalysis", frequency);
analysis_class!(PyStabilityAnalysis, "StabilityAnalysis", frequency);
analysis_class!(PyTransientNoiseAnalysis, "TransientNoiseAnalysis", time);

// ── Module-level functions ──

/// Lint a SPICE netlist for common issues and backend-specific warnings.
///
/// Returns a dict with "warnings" and "errors" lists.
#[pyfunction]
#[pyo3(signature = (netlist, backend=None))]
fn lint<'py>(py: Python<'py>, netlist: &str, backend: Option<&str>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let r = crate::lint::lint_netlist(netlist, backend);
    let rows = |fs: &[crate::lint::Finding], warn: bool| -> PyResult<Vec<Bound<'py, pyo3::types::PyDict>>> {
        fs.iter()
            .map(|f| {
                let d = pyo3::types::PyDict::new(py);
                d.set_item("line", f.line)?;
                d.set_item("message", &f.message)?;
                if let Some(s) = &f.suggestion {
                    d.set_item("suggestion", s)?;
                }
                if warn {
                    d.set_item("backends_affected", f.backends)?;
                }
                Ok(d)
            })
            .collect()
    };
    let out = pyo3::types::PyDict::new(py);
    out.set_item("warnings", rows(&r.warnings, true)?)?;
    out.set_item("errors", rows(&r.errors, false)?)?;
    Ok(out)
}

/// Compile a Verilog-A file path or inline source to OSDI with OpenVAF.
///
/// Returns the path to the compiled .osdi file.
///
/// ```python
/// from spicerack import compile_veriloga
///
/// osdi_path = compile_veriloga("models/comparator.va")
/// circuit.osdi(osdi_path)
/// ```
#[pyfunction]
fn compile_veriloga(source_or_path: &str) -> PyResult<String> {
    crate::veriloga::resolve(source_or_path).and_then(|p| crate::veriloga::compile_osdi(&p)).map_err(pyo3::exceptions::PyRuntimeError::new_err)
}

/// The IR block for a `verilog(...)` call; connection values are a net name
/// or a list of net names (a bus).
#[allow(clippy::too_many_arguments)]
fn verilog_block(
    py: pyo3::Python<'_>,
    source: &str,
    mode: &str,
    instance_name: &str,
    connections: &HashMap<String, pyo3::PyObject>,
    pdk: Option<&str>,
    liberty: Option<&str>,
    spice_models: Option<&str>,
) -> PyResult<crate::ir::VerilogBlock> {
    use crate::ir::{VerilogConnection, VerilogMode};
    let mode = match mode {
        "simulate" => VerilogMode::Simulate,
        "synthesize" => VerilogMode::Synthesize,
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Invalid mode '{}'. Must be 'simulate' or 'synthesize'.",
                mode
            )));
        }
    };
    let mut conns = std::collections::BTreeMap::new();
    for (port, obj) in connections {
        let conn = if let Ok(s) = obj.extract::<String>(py) {
            VerilogConnection::Single(s)
        } else if let Ok(v) = obj.extract::<Vec<String>>(py) {
            VerilogConnection::Bus(v)
        } else {
            return Err(pyo3::exceptions::PyTypeError::new_err("Connection value must be a string or list of strings"));
        };
        conns.insert(port.clone(), conn);
    }
    Ok(crate::ir::VerilogBlock {
        source: source.to_string(),
        mode,
        instance_name: instance_name.to_string(),
        connections: conns,
        pdk: pdk.map(str::to_string),
        liberty: liberty.map(str::to_string),
        spice_models: spice_models.map(str::to_string),
    })
}

// ── High-level waveform sources ──

/// `#[pymethods] impl $ty { ... }` with the sixteen waveform sources (eight
/// waveforms, each as a V and an I) appended: one table for Subcircuit and
/// Testbench, which each add the built source with their own `place_wave`.
/// Every source takes `dc=` (else its waveform's t=0 value, as SPICE reads a
/// card with none), `ac=`, `ac_phase=`, and PySpice's spellings where they
/// exist. One block, not two: pyo3 takes one `#[pymethods]` per class
/// without its `multiple-pymethods` feature. The caller writes the
/// `#[pymethods]` so the names pyo3 generates for `**kwargs` share the
/// body's hygiene.
macro_rules! pymethods_with_waves {
    ($hash:tt $attr:tt impl $ty:ty { $($body:tt)* }) => {
        pymethods_with_waves!(@emit $hash $attr $ty { $($body)* }
            [SinusoidalVoltageSource PulseVoltageSource PieceWiseLinearVoltageSource ExponentialVoltageSource SingleFrequencyFMVoltageSource AmplitudeModulatedVoltageSource TransientNoiseVoltageSource RandomVoltageSource false]
            [SinusoidalCurrentSource PulseCurrentSource PieceWiseLinearCurrentSource ExponentialCurrentSource SingleFrequencyFMCurrentSource AmplitudeModulatedCurrentSource TransientNoiseCurrentSource RandomCurrentSource true]);
    };
    (@emit $hash:tt $attr:tt $ty:ty { $($body:tt)* } $([$sin:ident $pulse:ident $pwl:ident $exp:ident $sffm:ident $am:ident $noise:ident $random:ident $current:literal])*) => {
        $hash $attr
        impl $ty {
            $($body)*
            $(
            #[pyo3(signature = (name, positive, negative, *, offset=IrNum::Num(0.0), amplitude=IrNum::Num(1.0), frequency=IrNum::Num(1000.0), delay=IrNum::Num(0.0), damping_factor=IrNum::Num(0.0), phase=IrNum::Num(0.0), dc_offset=None, ac_magnitude=None, dc=None, ac=None, ac_phase=None))]
            fn $sin(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                offset: IrNum, amplitude: IrNum, frequency: IrNum, delay: IrNum, damping_factor: IrNum, phase: IrNum, dc_offset: Option<f64>, ac_magnitude: Option<f64>, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Sin { offset, amplitude, frequency, delay, damping: damping_factor, phase };
                Self::place_wave(slf, $current, name, positive, negative, wf, either(dc, dc_offset, "dc", "dc_offset")?, either(ac, ac_magnitude, "ac", "ac_magnitude")?, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, initial_value=IrNum::Num(0.0), pulsed_value=IrNum::Num(1.0), delay=IrNum::Num(0.0), pulse_width=IrNum::Num(50e-9), period=IrNum::Num(100e-9), rise_time=IrNum::Num(1e-9), fall_time=IrNum::Num(1e-9), delay_time=None, dc_offset=None, dc=None, ac=None, ac_phase=None))]
            fn $pulse(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                initial_value: IrNum, pulsed_value: IrNum, delay: IrNum, pulse_width: IrNum, period: IrNum, rise_time: IrNum, fall_time: IrNum, delay_time: Option<IrNum>, dc_offset: Option<f64>, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Pulse { initial: initial_value, pulsed: pulsed_value, delay: delay_time.unwrap_or(delay), rise_time, fall_time, pulse_width, period };
                Self::place_wave(slf, $current, name, positive, negative, wf, either(dc, dc_offset, "dc", "dc_offset")?, ac, ac_phase);
                Ok(())
            }

            // `repeat_time`/`delay_time`: PySpice's names for ngspice's `r=`/`td=`.
            #[pyo3(signature = (name, positive, negative, *, values, repeat_time=None, delay_time=None, dc=None, ac=None, ac_phase=None))]
            fn $pwl(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                values: Vec<(IrNum, IrNum)>, repeat_time: Option<IrNum>, delay_time: Option<IrNum>, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Pwl { values, repeat: repeat_time, delay: delay_time };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, initial_value, pulsed_value, rise_time_constant, fall_delay_time, fall_time_constant, rise_delay_time=IrNum::Num(0.0), dc=None, ac=None, ac_phase=None))]
            fn $exp(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                initial_value: IrNum, pulsed_value: IrNum, rise_time_constant: IrNum, fall_delay_time: IrNum, fall_time_constant: IrNum, rise_delay_time: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Exp { initial: initial_value, pulsed: pulsed_value, rise_delay: rise_delay_time, rise_tau: rise_time_constant, fall_delay: fall_delay_time, fall_tau: fall_time_constant };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, offset, amplitude, carrier_frequency, modulation_index, signal_frequency, dc=None, ac=None, ac_phase=None))]
            fn $sffm(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                offset: IrNum, amplitude: IrNum, carrier_frequency: IrNum, modulation_index: IrNum, signal_frequency: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Sffm { offset, amplitude, carrier_freq: carrier_frequency, modulation_index, signal_freq: signal_frequency };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, offset, amplitude, modulating_frequency, carrier_frequency, signal_delay=IrNum::Num(0.0), dc=None, ac=None, ac_phase=None))]
            fn $am(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                offset: IrNum, amplitude: IrNum, modulating_frequency: IrNum, carrier_frequency: IrNum, signal_delay: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Am { amplitude, offset, modulating_freq: modulating_frequency, carrier_freq: carrier_frequency, delay: signal_delay };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            // ngspice TRNOISE(NA NT NALPHA NAMP RTSAM RTSCAPT RTSEMT), by meaning.
            #[pyo3(signature = (name, positive, negative, *, noise_rms, time_step, flicker_exponent=IrNum::Num(0.0), flicker_rms=IrNum::Num(0.0), rts_amplitude=IrNum::Num(0.0), rts_capture_time=IrNum::Num(0.0), rts_emission_time=IrNum::Num(0.0), dc=None, ac=None, ac_phase=None))]
            fn $noise(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                noise_rms: IrNum, time_step: IrNum, flicker_exponent: IrNum, flicker_rms: IrNum, rts_amplitude: IrNum, rts_capture_time: IrNum, rts_emission_time: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::TrNoise { na: noise_rms, nt: time_step, nalpha: flicker_exponent, namp: flicker_rms, rtsam: rts_amplitude, rtscapt: rts_capture_time, rtsemt: rts_emission_time };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            // ngspice TRRANDOM(TYPE TS TD PARAM1 PARAM2), PySpice's names;
            // `random_type` is uniform, gaussian, exponential or poisson.
            #[pyo3(signature = (name, positive, negative, *, random_type, duration, time_delay=IrNum::Num(0.0), parameter1=IrNum::Num(1.0), parameter2=IrNum::Num(0.0), dc=None, ac=None, ac_phase=None))]
            fn $random(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                random_type: &str, duration: IrNum, time_delay: IrNum, parameter1: IrNum, parameter2: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let kind = random_type.parse().map_err(PyValueError::new_err)?;
                let wf = IrWaveform::TrRandom { kind, ts: duration, td: time_delay, p1: parameter1, p2: parameter2 };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }
            )*
        }
    };
}

// ── Subcircuit: the element builder (Circuit extends it) ──

#[pyclass(name = "Subcircuit", subclass)]
#[derive(Clone)]
struct PySubcircuit {
    inner: ir::Subcircuit,
    /// `gnd` names the global ground `0`: true only for the top-level `Circuit`.
    ground_alias: bool,
}

/// A node name at the top level: `gnd` (any case) is the global ground `0`.
fn top_net(n: &str) -> String {
    if n.eq_ignore_ascii_case("gnd") { "0".into() } else { n.into() }
}

impl PySubcircuit {
    fn net(&self, n: &str) -> String {
        if self.ground_alias { top_net(n) } else { n.into() }
    }

    fn push(&mut self, c: Component) {
        self.inner.components.push(c);
    }

    /// Adds one waveform source; the generated `*Source` methods call it.
    fn place_wave(mut slf: PyRefMut<'_, Self>, current: bool, name: &str, p: &str, n: &str, wf: IrWaveform,
                  dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>) {
        let c = wave_source(current, name, slf.net(p), slf.net(n), wf, dc, ac, ac_phase);
        slf.push(c);
    }

    fn src(&mut self, current: bool, name: &str, p: &str, n: &str, value: IrValue,
           ac: Option<f64>, ac_phase: Option<f64>, wf: Option<IrWaveform>) {
        let c = source(current, name, self.net(p), self.net(n), value, ac, ac_phase, wf);
        self.push(c);
    }
}

pymethods_with_waves! {
#[pymethods]
impl PySubcircuit {
    #[new]
    #[pyo3(signature = (name, ports=None, **params))]
    fn new(name: &str, ports: Option<Vec<String>>, params: Option<Bound<'_, PyDict>>) -> PyResult<Self> {
        let inner = ir::Subcircuit {
            name: name.to_string(),
            ports: ports.unwrap_or_default().into_iter()
                .map(|name| ir::Port { name, direction: ir::PortDirection::InOut })
                .collect(),
            parameters: kw_params(params)?.into_iter()
                .map(|(name, v)| ir::ParamDef { name, default: Some(v) })
                .collect(),
            ..Default::default()
        };
        Ok(Self { inner, ground_alias: false })
    }

    #[getter]
    fn gnd(&self) -> String {
        "0".to_string()
    }

    /// The subcircuit's name, as `X` instances refer to it.
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// Port names in declaration order: the order an instance connects nets in.
    #[getter]
    fn ports(&self) -> Vec<String> {
        self.inner.ports.iter().map(|p| p.name.clone()).collect()
    }

    // ── Elements ──

    /// `value` is a number, a `UnitValue` or an expression string; `**kwargs`
    /// are instance parameters (`tc1=1e-3`, `m=2`).
    #[pyo3(signature = (name, positive, negative, value, *, raw_spice=None, **kwargs))]
    fn R(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg, raw_spice: Option<&str>, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        let value = match raw_spice {
            Some(text) => IrValue::Raw { text: text.to_string() },
            None => value.ir(),
        };
        self.push(Component::Resistor { name: name.into(), n1: self.net(positive), n2: self.net(negative), value, model: None, params: kw_params(kwargs)? });
        Ok(())
    }

    #[pyo3(signature = (name, positive, negative, value, **kwargs))]
    fn C(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.push(Component::Capacitor { name: name.into(), n1: self.net(positive), n2: self.net(negative), value: value.ir(), model: None, params: kw_params(kwargs)? });
        Ok(())
    }

    #[pyo3(signature = (name, positive, negative, value, **kwargs))]
    fn L(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.push(Component::Inductor { name: name.into(), n1: self.net(positive), n2: self.net(negative), value: value.ir(), model: None, params: kw_params(kwargs)? });
        Ok(())
    }

    #[pyo3(signature = (name, inductor1, inductor2, *, coupling))]
    fn K(&mut self, name: &str, inductor1: &str, inductor2: &str, coupling: f64) {
        self.push(Component::MutualInductor { name: name.into(), inductor1: inductor1.into(), inductor2: inductor2.into(), coupling });
    }

    /// `portnum` makes the source RF port `portnum` (from 1) for
    /// `s_param`, with `z0` ohms in series in every analysis (ngspice's
    /// `portnum n z0 x`; ngspice and ESPice only).
    #[pyo3(signature = (name, positive, negative, value, *, ac=None, ac_phase=None, portnum=None, z0=50.0))]
    fn V(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg, ac: Option<f64>, ac_phase: Option<f64>, portnum: Option<u32>, z0: f64) {
        self.src(false, name, positive, negative, value.ir(), ac, ac_phase, None);
        if let Some(Component::VoltageSource { port, .. }) = self.inner.components.last_mut() {
            *port = portnum.map(|num| ir::RfPort { num, z0 });
        }
    }

    #[pyo3(signature = (name, positive, negative, value, *, ac=None, ac_phase=None))]
    fn I(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg, ac: Option<f64>, ac_phase: Option<f64>) {
        self.src(true, name, positive, negative, value.ir(), ac, ac_phase, None);
    }

    #[pyo3(signature = (name, positive, negative, *, expression))]
    fn BV(&mut self, name: &str, positive: &str, negative: &str, expression: &str) {
        self.push(Component::BehavioralVoltage { name: name.into(), np: self.net(positive), nm: self.net(negative), expression: expression.into() });
    }

    #[pyo3(signature = (name, positive, negative, *, expression))]
    fn BI(&mut self, name: &str, positive: &str, negative: &str, expression: &str) {
        self.push(Component::BehavioralCurrent { name: name.into(), np: self.net(positive), nm: self.net(negative), expression: expression.into() });
    }

    #[pyo3(signature = (name, positive, negative, *, control_positive, control_negative, voltage_gain))]
    fn E(&mut self, name: &str, positive: &str, negative: &str, control_positive: &str, control_negative: &str, voltage_gain: f64) {
        self.push(Component::Vcvs {
            name: name.into(), np: self.net(positive), nm: self.net(negative),
            ncp: self.net(control_positive), ncm: self.net(control_negative), gain: voltage_gain,
        });
    }

    #[pyo3(signature = (name, positive, negative, *, control_positive, control_negative, transconductance))]
    fn G(&mut self, name: &str, positive: &str, negative: &str, control_positive: &str, control_negative: &str, transconductance: f64) {
        self.push(Component::Vccs {
            name: name.into(), np: self.net(positive), nm: self.net(negative),
            ncp: self.net(control_positive), ncm: self.net(control_negative), transconductance,
        });
    }

    #[pyo3(signature = (name, positive, negative, *, vsense, current_gain))]
    fn F(&mut self, name: &str, positive: &str, negative: &str, vsense: &str, current_gain: f64) {
        self.push(Component::Cccs { name: name.into(), np: self.net(positive), nm: self.net(negative), vsense: vsense.into(), gain: current_gain });
    }

    #[pyo3(signature = (name, positive, negative, *, vsense, transresistance))]
    fn H(&mut self, name: &str, positive: &str, negative: &str, vsense: &str, transresistance: f64) {
        self.push(Component::Ccvs { name: name.into(), np: self.net(positive), nm: self.net(negative), vsense: vsense.into(), transresistance });
    }

    /// `**kwargs` are instance parameters (`area=2`, `m=4`), as on `M`.
    #[pyo3(signature = (name, anode, cathode, *, model, **kwargs))]
    fn D(&mut self, name: &str, anode: &str, cathode: &str, model: &str, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.push(Component::Diode { name: name.into(), np: self.net(anode), nm: self.net(cathode), model: model.into(), params: kw_params(kwargs)? });
        Ok(())
    }

    /// `substrate` and `thermal` are the optional 4th and 5th terminals.
    #[pyo3(signature = (name, collector, base, emitter, *, model, substrate=None, thermal=None, **kwargs))]
    fn Q(
        &mut self, name: &str, collector: &str, base: &str, emitter: &str, model: &str,
        substrate: Option<&str>, thermal: Option<&str>, kwargs: Option<Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        if thermal.is_some() && substrate.is_none() {
            return Err(pyo3::exceptions::PyValueError::new_err("Q: a thermal node needs a substrate node before it"));
        }
        let extra_nodes = substrate.into_iter().chain(thermal).map(|n| self.net(n)).collect();
        self.push(Component::Bjt {
            name: name.into(), nc: self.net(collector), nb: self.net(base), ne: self.net(emitter), extra_nodes,
            model: model.into(), params: kw_params(kwargs)?,
        });
        Ok(())
    }

    #[pyo3(signature = (name, collector, base, emitter, *, model, substrate=None, thermal=None, **kwargs))]
    fn BJT(
        &mut self, name: &str, collector: &str, base: &str, emitter: &str, model: &str,
        substrate: Option<&str>, thermal: Option<&str>, kwargs: Option<Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        self.Q(name, collector, base, emitter, model, substrate, thermal, kwargs)
    }

    /// `bulk=None` is a 3-terminal device (VDMOS); `extra_nodes` follow the
    /// bulk (VDMOS thermal nodes, SOI body and back gate).
    #[pyo3(signature = (name, drain, gate, source, bulk=None, *, model, extra_nodes=None, **kwargs))]
    fn M(
        &mut self, name: &str, drain: &str, gate: &str, source: &str, bulk: Option<&str>, model: &str,
        extra_nodes: Option<Vec<String>>, kwargs: Option<Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let extra_nodes: Vec<String> = extra_nodes.unwrap_or_default().iter().map(|n| self.net(n)).collect();
        if bulk.is_none() && !extra_nodes.is_empty() {
            return Err(PyValueError::new_err("M: extra_nodes follow the bulk node; give bulk too"));
        }
        self.push(Component::Mosfet {
            name: name.into(), nd: self.net(drain), ng: self.net(gate), ns: self.net(source), nb: bulk.map(|b| self.net(b)), extra_nodes,
            model: model.into(), params: kw_params(kwargs)?,
        });
        Ok(())
    }

    #[pyo3(signature = (name, drain, gate, source, bulk=None, *, model, extra_nodes=None, **kwargs))]
    fn MOSFET(
        &mut self, name: &str, drain: &str, gate: &str, source: &str, bulk: Option<&str>, model: &str,
        extra_nodes: Option<Vec<String>>, kwargs: Option<Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        self.M(name, drain, gate, source, bulk, model, extra_nodes, kwargs)
    }

    #[pyo3(signature = (name, drain, gate, source, *, model, **kwargs))]
    fn J(&mut self, name: &str, drain: &str, gate: &str, source: &str, model: &str, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.push(Component::Jfet { name: name.into(), nd: self.net(drain), ng: self.net(gate), ns: self.net(source), model: model.into(), params: kw_params(kwargs)? });
        Ok(())
    }

    #[pyo3(signature = (name, drain, gate, source, *, model, **kwargs))]
    fn Z(&mut self, name: &str, drain: &str, gate: &str, source: &str, model: &str, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.push(Component::Mesfet { name: name.into(), nd: self.net(drain), ng: self.net(gate), ns: self.net(source), model: model.into(), params: kw_params(kwargs)? });
        Ok(())
    }

    #[pyo3(signature = (name, positive, negative, *, control_positive, control_negative, model))]
    fn S(&mut self, name: &str, positive: &str, negative: &str, control_positive: &str, control_negative: &str, model: &str) {
        self.push(Component::VSwitch {
            name: name.into(), np: self.net(positive), nm: self.net(negative),
            ncp: self.net(control_positive), ncm: self.net(control_negative), model: model.into(),
        });
    }

    #[pyo3(signature = (name, positive, negative, *, vcontrol, model))]
    fn W(&mut self, name: &str, positive: &str, negative: &str, vcontrol: &str, model: &str) {
        self.push(Component::ISwitch { name: name.into(), np: self.net(positive), nm: self.net(negative), vcontrol: vcontrol.into(), model: model.into() });
    }

    #[pyo3(signature = (name, input_positive, input_negative, output_positive, output_negative, *, Z0, TD))]
    fn T(&mut self, name: &str, input_positive: &str, input_negative: &str, output_positive: &str, output_negative: &str, Z0: f64, TD: f64) {
        self.push(Component::TLine {
            name: name.into(), inp: self.net(input_positive), inm: self.net(input_negative),
            outp: self.net(output_positive), outm: self.net(output_negative), z0: Z0, td: TD,
        });
    }

    /// A polynomial controlled source, SPICE `POLY(n)`: `kind` is "E", "G",
    /// "F" or "H"; `controls` are node pairs (E/G) or sensing V sources
    /// (F/H); `coefficients` in SPICE's term order.
    #[pyo3(signature = (name, kind, positive, negative, controls, coefficients))]
    fn POLY(&mut self, name: &str, kind: &str, positive: &str, negative: &str, controls: Vec<String>, coefficients: Vec<f64>) -> PyResult<()> {
        let letter = match kind.to_ascii_uppercase().as_str() {
            k @ ("E" | "G" | "F" | "H") => k.chars().next().unwrap(),
            _ => return Err(PyValueError::new_err("POLY kind is E, G, F or H")),
        };
        if matches!(letter, 'E' | 'G') && controls.len() % 2 != 0 {
            return Err(PyValueError::new_err("POLY E/G controls are node pairs"));
        }
        let controls = if matches!(letter, 'E' | 'G') { controls.iter().map(|c| self.net(c)).collect() } else { controls };
        self.push(Component::Poly { name: name.into(), letter, np: self.net(positive), nm: self.net(negative), controls, coeffs: coefficients });
        Ok(())
    }

    /// XSPICE code model instance (A-element).
    ///
    /// ```python
    /// circuit.A(name="adc1", connections=["[vin]", "[dout]"], model="adc_buf")
    /// circuit.A(name="dff1", connections=["[d clk $d_hi $d_hi]", "[q qbar]"], model="dff_model")
    /// ```
    #[pyo3(signature = (*, name, connections, model))]
    fn A(&mut self, name: &str, connections: Vec<String>, model: &str) {
        self.push(Component::Xspice { name: name.into(), connections, model: model.into() });
    }

    // ── Subcircuit instances ──

    #[pyo3(signature = (subckt, name, *nodes, **params))]
    fn instance(&mut self, subckt: &PySubcircuit, name: &str, nodes: Vec<String>, params: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        let subckt_name = subckt.inner.name.clone();
        self.X(name, &subckt_name, nodes, params)
    }

    /// Subcircuit instance by name (e.g. a PDK device). `**params` land on the card as `name=value`.
    #[pyo3(signature = (name, subcircuit_name, *nodes, **params))]
    fn X(&mut self, name: &str, subcircuit_name: &str, nodes: Vec<String>, params: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        let port_mapping = nodes.iter().map(|n| self.net(n)).collect();
        self.inner.instances.push(ir::Instance {
            name: name.into(), subcircuit: subcircuit_name.into(), port_mapping, parameters: kw_params(params)?, at: None,
        });
        Ok(())
    }

    // ── Directives ──

    #[pyo3(signature = (name, kind, **kwargs))]
    fn model(&mut self, name: &str, kind: &str, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.inner.models.push(ir::ModelDef { name: name.into(), kind: kind.into(), parameters: kw_params(kwargs)? });
        Ok(())
    }

    fn raw_spice(&mut self, line: &str) {
        self.inner.raw_spice.push(line.to_string());
    }

    fn include(&mut self, path: &str) {
        self.inner.includes.push(path.to_string());
    }

    fn lib(&mut self, path: &str, section: &str) {
        self.inner.libs.push((path.to_string(), section.to_string()));
    }

    /// Load a compiled Verilog-A model (OSDI binary).
    fn osdi(&mut self, path: &str) {
        self.inner.osdi_loads.push(path.to_string());
    }

    fn parameter(&mut self, name: &str, value: &str) {
        self.inner.parameters.push(ir::ParamDef { name: name.into(), default: Some(value.into()) });
    }

    /// Load a Verilog-A model (file path or inline source). The `.va` is
    /// recorded, not compiled: the backend's codegen compiles it with OpenVAF
    /// for ngspice/VACASK, includes it for Spectre, and hands it to ESPice as
    /// `.hdl` (VerA). Returns the resolved `.va` path.
    fn veriloga(&mut self, source_or_path: &str) -> PyResult<String> {
        let va = crate::veriloga::resolve(source_or_path)
            .map_err(runtime_err)?
            .to_string_lossy().to_string();
        if !self.inner.osdi_loads.contains(&va) { self.inner.osdi_loads.push(va.clone()); }
        Ok(va)
    }

    /// Add a Verilog module, lowered when the netlist is emitted.
    ///
    /// mode="simulate": compile with iverilog, use ngspice d_cosim XSPICE model.
    /// mode="synthesize": synthesize with Yosys to gate-level SPICE subcircuit calls.
    ///
    /// ```python
    /// circuit.verilog(
    ///     source="counter.v",
    ///     mode="simulate",
    ///     instance_name="cnt1",
    ///     connections={"clk": "clk_net", "count": ["bit0", "bit1", "bit2"]},
    /// )
    /// ```
    #[pyo3(signature = (*, source, mode="simulate", instance_name, connections, pdk=None, liberty=None, spice_models=None))]
    fn verilog(
        &mut self,
        py: pyo3::Python<'_>,
        source: &str,
        mode: &str,
        instance_name: &str,
        connections: HashMap<String, pyo3::PyObject>,
        pdk: Option<&str>,
        liberty: Option<&str>,
        spice_models: Option<&str>,
    ) -> PyResult<()> {
        let vb = verilog_block(py, source, mode, instance_name, &connections, pdk, liberty, spice_models)?;
        self.inner.verilog_blocks.push(vb);
        Ok(())
    }

    // ── Serialization ──

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string_pretty(&self.inner).map_err(runtime_err)
    }

    #[staticmethod]
    fn from_json(json: &str) -> PyResult<Self> {
        let inner = serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self { inner, ground_alias: false })
    }

    fn save_json(&self, path: &str) -> PyResult<()> {
        std::fs::write(path, self.to_json()?).map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    #[staticmethod]
    fn load_json(path: &str) -> PyResult<Self> {
        let json = std::fs::read_to_string(path).map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
        Self::from_json(&json)
    }

    fn __str__(&self) -> String {
        format!("Subcircuit('{}', ports={:?})", self.inner.name,
            self.inner.ports.iter().map(|p| &p.name).collect::<Vec<_>>())
    }

    fn __repr__(&self) -> String {
        self.__str__()
    }
}
}

// ── Testbench: a simulator around a DUT, with stimulus and multi-analysis decks ──

#[pyclass(name = "Testbench", extends = PySimulator)]
struct PyTestbench {}

impl PyTestbench {
    /// Adds one waveform source to the stimulus; the generated `*Source`
    /// methods call it.
    fn place_wave(mut slf: PyRefMut<'_, Self>, current: bool, name: &str, p: &str, n: &str, wf: IrWaveform,
                  dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>) {
        slf.as_super().stim(wave_source(current, name, top_net(p), top_net(n), wf, dc, ac, ac_phase));
    }
}

pymethods_with_waves! {
#[pymethods]
impl PyTestbench {
    #[new]
    fn new(dut: &PySubcircuit) -> (Self, PySimulator) {
        let ir = CircuitIR { top: dut.inner.clone(), testbench: Some(Default::default()), ..Default::default() };
        (Self {}, PySimulator { ir, backend: None, save_currents: false })
    }

    /// Register a child definition (its include/lib/osdi are hoisted to the top level).
    fn add_subcircuit(mut slf: PyRefMut<'_, Self>, subckt: &PySubcircuit) {
        let ir = &mut slf.as_super().ir;
        let def = hoisted(&mut ir.top, &subckt.inner);
        ir.subcircuit_defs.push(def);
    }

    /// Attach a PDK model library. Codegen resolves the per-backend path and corner.
    fn use_pdk(mut slf: PyRefMut<'_, Self>, pdk: &PyModelLibrary) {
        slf.as_super().ir.model_libraries.push(pdk.inner.clone());
    }

    // ── Stimulus ──

    #[pyo3(signature = (name, positive, negative, value, *, ac=None, ac_phase=None, portnum=None, z0=50.0))]
    fn V(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, value: PyValueArg, ac: Option<f64>, ac_phase: Option<f64>, portnum: Option<u32>, z0: f64) {
        let mut v = source(false, name, top_net(positive), top_net(negative), value.ir(), ac, ac_phase, None);
        if let Component::VoltageSource { port, .. } = &mut v {
            *port = portnum.map(|num| ir::RfPort { num, z0 });
        }
        slf.as_super().stim(v);
    }

    #[pyo3(signature = (name, positive, negative, value, *, ac=None, ac_phase=None))]
    fn I(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, value: PyValueArg, ac: Option<f64>, ac_phase: Option<f64>) {
        slf.as_super().stim(source(true, name, top_net(positive), top_net(negative), value.ir(), ac, ac_phase, None));
    }

    #[pyo3(signature = (name, positive, negative, value))]
    fn R(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, value: PyValueArg) {
        slf.as_super().stim(Component::Resistor { name: name.into(), n1: top_net(positive), n2: top_net(negative), value: value.ir(), model: None, params: vec![] });
    }

    #[pyo3(signature = (name, positive, negative, value))]
    fn C(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, value: PyValueArg) {
        slf.as_super().stim(Component::Capacitor { name: name.into(), n1: top_net(positive), n2: top_net(negative), value: value.ir(), model: None, params: vec![] });
    }

    // ── Config ──

    fn with_backend(mut slf: PyRefMut<'_, Self>, name: &str) -> PyResult<()> {
        check_backend_name(name)?;
        slf.as_super().backend = Some(name.to_string());
        Ok(())
    }

    fn extra_line(mut slf: PyRefMut<'_, Self>, line: &str) {
        slf.as_super().tb().extra_lines.push(line.to_string());
    }

    /// A `.print` card: `add_print("tran", "v(out)", "i(vdd)")`; `analysis`
    /// may be `""`. Simulators that print nothing in a rawfile run omit it.
    #[pyo3(signature = (analysis, *outputs))]
    fn add_print(mut slf: PyRefMut<'_, Self>, analysis: &str, outputs: Vec<String>) {
        slf.as_super().tb().prints.push(ir::PrintCard { analysis: analysis.to_ascii_lowercase(), outputs });
    }

    /// A `.func`: `add_function("f", ["x"], "x*2")`.
    #[pyo3(signature = (name, args, body))]
    fn add_function(mut slf: PyRefMut<'_, Self>, name: &str, args: Vec<String>, body: &str) {
        slf.as_super().ir.functions.push(ir::FuncDef { name: name.into(), args, body: body.into() });
    }

    /// `.global` nodes: one net under these names in every subcircuit.
    #[pyo3(signature = (*nodes))]
    fn global_nodes(mut slf: PyRefMut<'_, Self>, nodes: Vec<String>) {
        let g = &mut slf.as_super().ir.globals;
        g.extend(nodes.into_iter().filter(|n| !g.contains(n)).collect::<Vec<_>>());
    }

    // ── Analysis builders for multi-analysis decks ──

    fn add_operating_point(mut slf: PyRefMut<'_, Self>) {
        slf.as_super().tb().analyses.push(Analysis::Op);
    }

    #[pyo3(signature = (**kwargs))]
    fn add_dc(mut slf: PyRefMut<'_, Self>, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        let a = dc_sweeps(kwargs)?;
        slf.as_super().tb().analyses.push(a);
        Ok(())
    }

    #[pyo3(signature = (variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e9))]
    fn add_ac(mut slf: PyRefMut<'_, Self>, variation: &str, number_of_points: u32, start_frequency: f64, stop_frequency: f64) -> PyResult<()> {
        slf.as_super().tb().analyses.push(card::ac(variation, number_of_points, start_frequency, stop_frequency)?);
        Ok(())
    }

    #[pyo3(signature = (step_time, end_time, start_time=None, max_time=None, use_initial_condition=false))]
    fn add_transient(mut slf: PyRefMut<'_, Self>, step_time: f64, end_time: f64, start_time: Option<f64>, max_time: Option<f64>, use_initial_condition: bool) {
        slf.as_super().tb().analyses.push(card::transient(step_time, end_time, start_time, max_time, use_initial_condition));
    }

    #[pyo3(signature = (outvar, insrc))]
    fn add_transfer_function(mut slf: PyRefMut<'_, Self>, outvar: &str, insrc: &str) {
        slf.as_super().tb().analyses.push(card::tf(outvar, insrc));
    }

    #[pyo3(signature = (outvar, insrc))]
    fn add_tf(slf: PyRefMut<'_, Self>, outvar: &str, insrc: &str) {
        Self::add_transfer_function(slf, outvar, insrc)
    }

    #[pyo3(signature = (output_variable))]
    fn add_dc_sensitivity(mut slf: PyRefMut<'_, Self>, output_variable: &str) {
        slf.as_super().tb().analyses.push(card::sensitivity(output_variable, None));
    }

    #[pyo3(signature = (output_variable, variation="dec", number_of_points=10, start_frequency=100.0, stop_frequency=1e5))]
    fn add_ac_sensitivity(mut slf: PyRefMut<'_, Self>, output_variable: &str, variation: &str, number_of_points: u32, start_frequency: f64, stop_frequency: f64) -> PyResult<()> {
        let a = card::sensitivity(output_variable, Some(card::sweep(variation, number_of_points, start_frequency, stop_frequency)?));
        slf.as_super().tb().analyses.push(a);
        Ok(())
    }

    #[pyo3(signature = (node1, node2, node3, node4, tf_type, pz_type))]
    fn add_polezero(mut slf: PyRefMut<'_, Self>, node1: &str, node2: &str, node3: &str, node4: &str, tf_type: &str, pz_type: &str) -> PyResult<()> {
        slf.as_super().tb().analyses.push(card::polezero([node1, node2, node3, node4], tf_type, pz_type)?);
        Ok(())
    }

    #[pyo3(signature = (variation="dec", points=10, start_frequency=100.0, stop_frequency=1e8, f2overf1=None))]
    fn add_distortion(mut slf: PyRefMut<'_, Self>, variation: &str, points: u32, start_frequency: f64, stop_frequency: f64, f2overf1: Option<f64>) -> PyResult<()> {
        slf.as_super().tb().analyses.push(card::distortion(variation, points, start_frequency, stop_frequency, f2overf1)?);
        Ok(())
    }

    #[pyo3(signature = (output_node, ref_node, src, variation="dec", points=10, start_frequency=1e3, stop_frequency=1e8, points_per_summary=None))]
    fn add_noise(
        mut slf: PyRefMut<'_, Self>, output_node: &str, ref_node: &str, src: &str, variation: &str, points: u32,
        start_frequency: f64, stop_frequency: f64, points_per_summary: Option<u32>,
    ) -> PyResult<()> {
        let a = card::noise(output_node, ref_node, src, variation, points, start_frequency, stop_frequency, points_per_summary)?;
        slf.as_super().tb().analyses.push(a);
        Ok(())
    }

    #[pyo3(signature = (fundamental_frequency, outputs, num_harmonics=None))]
    fn add_fourier(mut slf: PyRefMut<'_, Self>, fundamental_frequency: f64, outputs: Vec<String>, num_harmonics: Option<u32>) {
        slf.as_super().tb().analyses.push(Analysis::Fourier { fundamental: fundamental_frequency, outputs, num_harmonics });
    }

    #[pyo3(signature = (param, start, stop, step, inner_analysis, inner_type))]
    fn add_spectre_sweep(mut slf: PyRefMut<'_, Self>, param: &str, start: f64, stop: f64, step: f64, inner_analysis: &str, inner_type: &str) -> PyResult<()> {
        slf.as_super().tb().analyses.push(card::spectre_sweep(param, start, stop, step, inner_analysis, inner_type)?);
        Ok(())
    }

    #[pyo3(signature = (num_iterations, inner_analysis, inner_type, seed=None))]
    fn add_spectre_monte_carlo(mut slf: PyRefMut<'_, Self>, num_iterations: u32, inner_analysis: &str, inner_type: &str, seed: Option<u64>) -> PyResult<()> {
        slf.as_super().tb().analyses.push(card::spectre_monte_carlo(num_iterations, inner_analysis, inner_type, seed)?);
        Ok(())
    }

    // ── Serialization ──

    fn to_json(slf: PyRef<'_, Self>) -> PyResult<String> {
        serde_json::to_string_pretty(&slf.as_super().ir).map_err(runtime_err)
    }

    #[pyo3(signature = (backend="ngspice"))]
    fn netlist(slf: PyRef<'_, Self>, backend: &str) -> PyResult<String> {
        emit_ir_netlist(slf.as_super().ir.clone(), backend)
    }

    fn __str__(slf: PyRef<'_, Self>) -> String {
        let ir = &slf.as_super().ir;
        let n = ir.testbench.as_ref().map_or(0, |tb| tb.analyses.len());
        format!("Testbench(dut='{}', analyses={})", ir.top.name, n)
    }

    fn __repr__(slf: PyRef<'_, Self>) -> String {
        Self::__str__(slf)
    }
}
}

// ── PyModelLibrary ──

#[pyclass(name = "ModelLibrary")]
struct PyModelLibrary {
    inner: ir::ModelLibrary,
}

#[pymethods]
impl PyModelLibrary {
    #[new]
    #[pyo3(signature = (path, corner=None, setup_includes=None, **backend_paths))]
    fn new(path: &str, corner: Option<&str>, setup_includes: Option<Vec<String>>, backend_paths: Option<Bound<'_, PyDict>>) -> PyResult<Self> {
        let mut bp = std::collections::BTreeMap::new();
        if let Some(dict) = backend_paths {
            for (k, v) in dict.iter() {
                bp.insert(k.extract::<String>()?, v.extract::<String>()?);
            }
        }
        Ok(Self {
            inner: ir::ModelLibrary {
                name: std::path::Path::new(path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("library")
                    .to_string(),
                path: path.to_string(),
                corner: corner.map(str::to_string),
                backend_paths: bp,
                setup_includes: setup_includes.unwrap_or_default(),
            },
        })
    }

    #[getter]
    fn name(&self) -> String { self.inner.name.clone() }

    #[getter]
    fn path(&self) -> String { self.inner.path.clone() }

    #[getter]
    fn corner(&self) -> Option<String> { self.inner.corner.clone() }

    fn __str__(&self) -> String {
        format!("ModelLibrary('{}', corner={:?})", self.inner.path, self.inner.corner)
    }

    fn __repr__(&self) -> String {
        self.__str__()
    }
}

// ── Module registration ──

fn create_unit_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
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

#[pymodule(name = "_native")]
pub fn spicerack(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyCircuit>()?;
    m.add_class::<PyUnit>()?;
    m.add_class::<PyUnitValue>()?;
    m.add_class::<PySimulator>()?;
    m.add_class::<PyOperatingPoint>()?;
    m.add_class::<PyDcAnalysis>()?;
    m.add_class::<PyAcAnalysis>()?;
    m.add_class::<PyTransientAnalysis>()?;
    m.add_class::<PyNoiseAnalysis>()?;
    m.add_class::<PyTransferFunctionAnalysis>()?;
    m.add_class::<PySensitivityAnalysis>()?;
    m.add_class::<PyPoleZeroAnalysis>()?;
    m.add_class::<PyDistortionAnalysis>()?;
    // New analysis types
    m.add_class::<PyPssAnalysis>()?;
    m.add_class::<PySParamAnalysis>()?;
    m.add_class::<PyHarmonicBalanceAnalysis>()?;
    m.add_class::<PyStabilityAnalysis>()?;
    m.add_class::<PyTransientNoiseAnalysis>()?;
    // Spectre raw data type
    m.add_class::<PyRawData>()?;

    // IR types
    m.add_class::<PySubcircuit>()?;
    m.add_class::<PyTestbench>()?;
    m.add_class::<PyModelLibrary>()?;

    // Module-level functions
    m.add_function(wrap_pyfunction!(lint, m)?)?;
    m.add_function(wrap_pyfunction!(read_raw, m)?)?;
    m.add_function(wrap_pyfunction!(compile_veriloga, m)?)?;

    create_unit_module(m)?;
    Ok(())
}

/// Expand each Verilog block of `sub` into what simulates it: a `d_cosim`
/// XSPICE instance (simulate) or the synthesized standard-cell instances
/// (synthesize).
fn lower_verilog(sub: &mut ir::Subcircuit) -> Result<(), String> {
    for vb in std::mem::take(&mut sub.verilog_blocks) {
        match crate::veriloga::lower(&vb)? {
            crate::veriloga::Lowered::Cosim { vvp, ports } => {
                let bridges = [
                    ("adc_buf", "adc_bridge", [("in_low", "0.5"), ("in_high", "0.5")]),
                    ("dac_buf", "dac_bridge", [("out_low", "0.0"), ("out_high", "1.8")]),
                ];
                for (name, kind, params) in bridges {
                    if !sub.models.iter().any(|m| m.kind == kind) {
                        sub.models.push(ir::ModelDef {
                            name: name.into(),
                            kind: kind.into(),
                            parameters: params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                        });
                    }
                }
                let model = format!("cosim_{}", vb.instance_name);
                sub.raw_spice.push(format!(".model {model} d_cosim(delay=1n simulation_id=\"{vvp}\")"));
                sub.components.push(Component::Xspice { name: vb.instance_name.clone(), connections: ports, model });
            }
            crate::veriloga::Lowered::Gates { spice_models, cells } => {
                sub.includes.push(spice_models);
                sub.instances.extend(cells.into_iter().map(|c| ir::Instance {
                    name: c.name, subcircuit: c.cell, port_mapping: c.nodes, parameters: vec![], at: None,
                }));
            }
        }
    }
    Ok(())
}
