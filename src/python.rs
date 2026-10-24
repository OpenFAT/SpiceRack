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
use pyo3::types::PyDict;

use crate::codegen::CodeGen;
use crate::ir::{self, Analysis, CircuitIR, Component, IrValue, IrWaveform};
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
    inner: u::Unit,
}

#[pyclass(name = "UnitValue")]
#[derive(Clone)]
struct PyUnitValue {
    inner: u::UnitValue,
}

#[pymethods]
impl PyUnit {
    fn __rmatmul__(&self, value: f64) -> PyUnitValue {
        PyUnitValue {
            inner: u::UnitValue::new(value, self.inner),
        }
    }

    fn __repr__(&self) -> String {
        format!("Unit({:?}, {:?})", self.inner.prefix, self.inner.kind)
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
        format!("{}", self.inner)
    }

    fn __str__(&self) -> String {
        format!("{}", self.inner)
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
    if crate::backend::BackendKind::from_str(name).is_none() {
        return Err(PyValueError::new_err(format!(
            "unknown backend {name:?}; expected one of ngspice, ngspice-subprocess, \
             ngspice-shared, ltspice, vacask, vacask-shared, spectre, espice")));
    }
    Ok(())
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
}

impl PyValueArg {
    fn ir(self) -> IrValue {
        match self {
            Self::Float(value) => IrValue::Numeric { value },
            Self::Unit(uv) => IrValue::Numeric { value: uv.inner.as_f64() },
        }
    }
}

fn num(value: f64) -> IrValue {
    IrValue::Numeric { value }
}

fn sin(offset: f64, amplitude: f64, frequency: f64) -> Option<IrWaveform> {
    Some(IrWaveform::Sin { offset, amplitude, frequency, delay: 0.0, damping: 0.0, phase: 0.0 })
}

fn pulse(initial: f64, pulsed: f64, pulse_width: f64, period: f64, rise_time: f64, fall_time: f64) -> Option<IrWaveform> {
    Some(IrWaveform::Pulse { initial, pulsed, delay: 0.0, rise_time, fall_time, pulse_width, period })
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
        Component::VoltageSource { name, np, nm, value, ac_magnitude, ac_phase, waveform }
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
    let emitted = match backend.to_ascii_lowercase().as_str() {
        "ngspice" | "ngspice-subprocess" | "ngspice-shared" => {
            crate::codegen::spice3::Spice3CodeGen {
                dialect: crate::codegen::spice3::Spice3Dialect::Ngspice,
            }.emit_netlist(&ir)
        }
        "ltspice" => {
            crate::codegen::spice3::Spice3CodeGen {
                dialect: crate::codegen::spice3::Spice3Dialect::Ltspice,
            }.emit_netlist(&ir)
        }
        "spectre" => crate::codegen::spectre::SpectreCodeGen.emit_netlist(&ir),
        "vacask" => crate::codegen::vacask::VacaskCodeGen.emit_netlist(&ir),
        "espice" => crate::codegen::espice::EspiceCodeGen.emit_netlist(&ir),
        _ => return Err(PyValueError::new_err(format!("unknown backend '{backend}'"))),
    };
    emitted.map_err(runtime_err)
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
        hoist_directives(&mut slf.as_super().inner, &subckt.inner);
        slf.defs.push(subckt.inner.clone());
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
            model_libraries: vec![],
        }
    }
}

/// Hoist a child definition's `include`/`lib`/`osdi` directives to the deck's
/// top level: every backend scopes them globally.
fn hoist_directives(top: &mut ir::Subcircuit, child: &ir::Subcircuit) {
    for inc in &child.includes {
        if !top.includes.contains(inc) { top.includes.push(inc.clone()); }
    }
    for lib in &child.libs {
        if !top.libs.contains(lib) { top.libs.push(lib.clone()); }
    }
    for osdi in &child.osdi_loads {
        if !top.osdi_loads.contains(osdi) { top.osdi_loads.push(osdi.clone()); }
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
    fn step_sweep(&mut self, param: &str, start: f64, stop: f64, step: f64, sweep_type: &str) {
        self.tb().step_params.push(ir::StepParam {
            param: param.to_string(), start, stop, step, sweep_type: Some(sweep_type.to_string()),
        });
    }

    // ── Analyses ──

    fn operating_point(&self) -> PyResult<PyOperatingPoint> {
        Ok(PyOperatingPoint { inner: OperatingPoint::from_raw(self.run(Analysis::Op)?) })
    }

    #[pyo3(signature = (**kwargs))]
    fn dc(&self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<PyDcAnalysis> {
        Ok(PyDcAnalysis { inner: DcAnalysis::from_raw(self.run(dc_sweeps(kwargs)?)?) })
    }

    #[pyo3(signature = (variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e9))]
    fn ac(&self, variation: &str, number_of_points: u32, start_frequency: f64, stop_frequency: f64) -> PyResult<PyAcAnalysis> {
        let a = Analysis::Ac { variation: variation.into(), points: number_of_points, start: start_frequency, stop: stop_frequency };
        Ok(PyAcAnalysis { inner: AcAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (step_time, end_time, start_time=None, max_time=None, use_initial_condition=false))]
    fn transient(
        &self, step_time: f64, end_time: f64, start_time: Option<f64>, max_time: Option<f64>,
        use_initial_condition: bool,
    ) -> PyResult<PyTransientAnalysis> {
        let a = Analysis::Transient { step: step_time, stop: end_time, start: start_time, max_step: max_time, uic: use_initial_condition };
        Ok(PyTransientAnalysis { inner: TransientAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (output_node, ref_node, src, variation="dec", points=10, start_frequency=1e3, stop_frequency=1e8, points_per_summary=None))]
    fn noise(
        &self, output_node: &str, ref_node: &str, src: &str, variation: &str, points: u32,
        start_frequency: f64, stop_frequency: f64, points_per_summary: Option<u32>,
    ) -> PyResult<PyNoiseAnalysis> {
        let a = Analysis::Noise {
            output: output_node.into(), reference: ref_node.into(), source: src.into(),
            variation: variation.into(), points, start: start_frequency, stop: stop_frequency, points_per_summary,
        };
        Ok(PyNoiseAnalysis { inner: NoiseAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (outvar, insrc))]
    fn transfer_function(&self, outvar: &str, insrc: &str) -> PyResult<PyTransferFunctionAnalysis> {
        let a = Analysis::Tf { output: outvar.into(), source: insrc.into() };
        Ok(PyTransferFunctionAnalysis { inner: TransferFunctionAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (outvar, insrc))]
    fn tf(&self, outvar: &str, insrc: &str) -> PyResult<PyTransferFunctionAnalysis> {
        self.transfer_function(outvar, insrc)
    }

    #[pyo3(signature = (output_variable))]
    fn dc_sensitivity(&self, output_variable: &str) -> PyResult<PySensitivityAnalysis> {
        let a = Analysis::Sensitivity { output: output_variable.into(), ac: None };
        Ok(PySensitivityAnalysis { inner: SensitivityAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (output_variable, variation="dec", number_of_points=10, start_frequency=100.0, stop_frequency=1e5))]
    fn ac_sensitivity(
        &self, output_variable: &str, variation: &str, number_of_points: u32,
        start_frequency: f64, stop_frequency: f64,
    ) -> PyResult<PySensitivityAnalysis> {
        let ac = ir::AcSweepParams { variation: variation.into(), points: number_of_points, start: start_frequency, stop: stop_frequency };
        let a = Analysis::Sensitivity { output: output_variable.into(), ac: Some(ac) };
        Ok(PySensitivityAnalysis { inner: SensitivityAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (node1, node2, node3, node4, tf_type, pz_type))]
    fn polezero(
        &self, node1: &str, node2: &str, node3: &str, node4: &str, tf_type: &str, pz_type: &str,
    ) -> PyResult<PyPoleZeroAnalysis> {
        let a = Analysis::PoleZero {
            node1: node1.into(), node2: node2.into(), node3: node3.into(), node4: node4.into(),
            tf_type: tf_type.into(), pz_type: pz_type.into(),
        };
        Ok(PyPoleZeroAnalysis { inner: PoleZeroAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (variation="dec", points=10, start_frequency=100.0, stop_frequency=1e8, f2overf1=None))]
    fn distortion(
        &self, variation: &str, points: u32, start_frequency: f64, stop_frequency: f64, f2overf1: Option<f64>,
    ) -> PyResult<PyDistortionAnalysis> {
        let a = Analysis::Distortion { variation: variation.into(), points, start: start_frequency, stop: stop_frequency, f2overf1 };
        Ok(PyDistortionAnalysis { inner: DistortionAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (fundamental_frequency, stabilization_time, observe_node, points_per_period=128, harmonics=10))]
    fn pss(
        &self, fundamental_frequency: f64, stabilization_time: f64, observe_node: &str,
        points_per_period: u32, harmonics: u32,
    ) -> PyResult<PyPssAnalysis> {
        let a = Analysis::Pss {
            fundamental: fundamental_frequency, stabilization: stabilization_time,
            observe_node: observe_node.into(), points_per_period, harmonics,
        };
        Ok(PyPssAnalysis { inner: PssAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (variation="dec", number_of_points=10, start_frequency=1e6, stop_frequency=1e10))]
    fn s_param(&self, variation: &str, number_of_points: u32, start_frequency: f64, stop_frequency: f64) -> PyResult<PySParamAnalysis> {
        let a = Analysis::SPar { variation: variation.into(), points: number_of_points, start: start_frequency, stop: stop_frequency };
        Ok(PySParamAnalysis { inner: SParamAnalysis::from_raw(self.run(a)?) })
    }

    /// LTspice `.net` network-parameter analysis over an AC sweep.
    #[pyo3(signature = (output_current, input_source, z_in=50.0, z_out=50.0, variation="dec", points=100, start_freq=1e3, stop_freq=1e9))]
    fn network_params(
        &self, output_current: &str, input_source: &str, z_in: f64, z_out: f64,
        variation: &str, points: u32, start_freq: f64, stop_freq: f64,
    ) -> PyResult<PySParamAnalysis> {
        let mut ir = self.deck(Analysis::Ac { variation: variation.into(), points, start: start_freq, stop: stop_freq });
        if let Some(tb) = &mut ir.testbench {
            tb.extra_lines.push(format!(".net {output_current} {input_source} Rout={z_out} Rin={z_in}"));
        }
        Ok(PySParamAnalysis { inner: SParamAnalysis::from_raw(run_deck(ir, self.backend.as_deref())?) })
    }

    #[pyo3(signature = (fundamental_frequencies, num_harmonics=None))]
    fn harmonic_balance(&self, fundamental_frequencies: Vec<f64>, num_harmonics: Option<Vec<u32>>) -> PyResult<PyHarmonicBalanceAnalysis> {
        let harmonics = num_harmonics.unwrap_or_else(|| vec![7; fundamental_frequencies.len()]);
        let a = Analysis::HarmonicBalance { frequencies: fundamental_frequencies, harmonics };
        Ok(PyHarmonicBalanceAnalysis { inner: HarmonicBalanceAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (probe, variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e10))]
    fn stability(
        &self, probe: &str, variation: &str, number_of_points: u32, start_frequency: f64, stop_frequency: f64,
    ) -> PyResult<PyStabilityAnalysis> {
        let a = Analysis::Stability {
            probe: probe.into(), variation: variation.into(), points: number_of_points, start: start_frequency, stop: stop_frequency,
        };
        Ok(PyStabilityAnalysis { inner: StabilityAnalysis::from_raw(self.run(a)?) })
    }

    #[pyo3(signature = (step_time, end_time))]
    fn transient_noise(&self, step_time: f64, end_time: f64) -> PyResult<PyTransientNoiseAnalysis> {
        let a = Analysis::TransientNoise { step: step_time, stop: end_time };
        Ok(PyTransientNoiseAnalysis { inner: TransientNoiseAnalysis::from_raw(self.run(a)?) })
    }

    // ── Spectre-specific analyses ──

    /// Spectre parametric sweep wrapping an inner analysis (Spectre-native text).
    #[pyo3(signature = (param, start, stop, step, inner_analysis, inner_type="ac"))]
    fn spectre_sweep(
        &self, param: &str, start: f64, stop: f64, step: f64, inner_analysis: &str, inner_type: &str,
    ) -> PyResult<PyRawData> {
        let a = Analysis::SpectreSweep {
            param: param.into(), start, stop, step, inner: inner_analysis.into(), inner_type: inner_type.into(),
        };
        Ok(PyRawData { inner: self.run_spectre(a)? })
    }

    /// Spectre Monte Carlo wrapping an inner analysis.
    #[pyo3(signature = (num_iterations, inner_analysis, inner_type="ac", seed=None))]
    fn spectre_montecarlo(&self, num_iterations: u32, inner_analysis: &str, inner_type: &str, seed: Option<u64>) -> PyResult<PyRawData> {
        let a = Analysis::SpectreMonteCarlo {
            iterations: num_iterations, inner: inner_analysis.into(), inner_type: inner_type.into(), seed,
        };
        Ok(PyRawData { inner: self.run_spectre(a)? })
    }

    /// SpectreRF Periodic AC (PAC) analysis with automatic PSS prerequisite.
    #[pyo3(signature = (pss_fundamental, pss_stabilization, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9, sweep_type="relative"))]
    fn spectre_pac(
        &self, pss_fundamental: f64, pss_stabilization: f64, pss_harmonics: u32,
        variation: &str, points: u32, start_freq: f64, stop_freq: f64, sweep_type: &str,
    ) -> PyResult<PyAcAnalysis> {
        let a = Analysis::SpectrePac {
            pss_fundamental, pss_stabilization, pss_harmonics, variation: variation.into(), points,
            start: start_freq, stop: stop_freq, sweep_type: sweep_type.into(),
        };
        Ok(PyAcAnalysis { inner: AcAnalysis::from_raw(self.run_spectre(a)?) })
    }

    /// SpectreRF Periodic Noise (PNoise) analysis with automatic PSS prerequisite.
    #[pyo3(signature = (pss_fundamental, pss_stabilization, output_node, ref_node, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9))]
    fn spectre_pnoise(
        &self, pss_fundamental: f64, pss_stabilization: f64, output_node: &str, ref_node: &str,
        pss_harmonics: u32, variation: &str, points: u32, start_freq: f64, stop_freq: f64,
    ) -> PyResult<PyNoiseAnalysis> {
        let a = Analysis::SpectrePnoise {
            pss_fundamental, pss_stabilization, pss_harmonics, output: output_node.into(), reference: ref_node.into(),
            variation: variation.into(), points, start: start_freq, stop: stop_freq,
        };
        Ok(PyNoiseAnalysis { inner: NoiseAnalysis::from_raw(self.run_spectre(a)?) })
    }

    /// SpectreRF Periodic Transfer Function (PXF) analysis with automatic PSS prerequisite.
    #[pyo3(signature = (pss_fundamental, pss_stabilization, output_node, source, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9))]
    fn spectre_pxf(
        &self, pss_fundamental: f64, pss_stabilization: f64, output_node: &str, source: &str,
        pss_harmonics: u32, variation: &str, points: u32, start_freq: f64, stop_freq: f64,
    ) -> PyResult<PyAcAnalysis> {
        let a = Analysis::SpectrePxf {
            pss_fundamental, pss_stabilization, pss_harmonics, output: output_node.into(), source: source.into(),
            variation: variation.into(), points, start: start_freq, stop: stop_freq,
        };
        Ok(PyAcAnalysis { inner: AcAnalysis::from_raw(self.run_spectre(a)?) })
    }

    /// SpectreRF Periodic Stability (PSTB) analysis with automatic PSS prerequisite.
    #[pyo3(signature = (pss_fundamental, pss_stabilization, probe, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9))]
    fn spectre_pstb(
        &self, pss_fundamental: f64, pss_stabilization: f64, probe: &str,
        pss_harmonics: u32, variation: &str, points: u32, start_freq: f64, stop_freq: f64,
    ) -> PyResult<PyStabilityAnalysis> {
        let a = Analysis::SpectrePstb {
            pss_fundamental, pss_stabilization, pss_harmonics, probe: probe.into(),
            variation: variation.into(), points, start: start_freq, stop: stop_freq,
        };
        Ok(PyStabilityAnalysis { inner: StabilityAnalysis::from_raw(self.run_spectre(a)?) })
    }

    /// List all available simulator backends on this system
    #[staticmethod]
    fn available_backends() -> Vec<String> {
        crate::backend::detect::detect_backends().iter().map(|b| b.display_name().to_string()).collect()
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
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        let lower = name.to_lowercase();
        for (i, var) in self.inner.variables.iter().enumerate() {
            if var.name.to_lowercase() == lower
                && i < self.inner.real_data.len() {
                    return Ok(self.inner.real_data[i].clone());
                }
        }
        Err(PyKeyError::new_err(format!("Variable '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.__getitem__(name)
    }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.measures) }
}

#[pyclass(name = "OperatingPoint")]
struct PyOperatingPoint {
    inner: crate::result::OperatingPoint,
}

#[pymethods]
impl PyOperatingPoint {
    fn __getitem__(&self, name: &str) -> PyResult<f64> {
        self.inner
            .get(name)
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }

    fn __getattr__(&self, name: &str) -> PyResult<f64> {
        self.inner
            .get(name)
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }

    /// Parsed .meas results as {name: value} dict
    #[getter]
    fn measures(&self) -> HashMap<String, f64> {
        measures_to_dict(&self.inner.base.measures)
    }
}

#[pyclass(name = "DcAnalysis")]
struct PyDcAnalysis {
    inner: crate::result::DcAnalysis,
}

#[pymethods]
impl PyDcAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner
            .base
            .get(name)
            .map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }

    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner
            .base
            .get(name)
            .map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }

    #[getter]
    fn sweep(&self) -> Vec<f64> {
        self.inner.sweep.clone()
    }

    #[getter]
    fn measures(&self) -> HashMap<String, f64> {
        measures_to_dict(&self.inner.base.measures)
    }

    #[getter]
    fn nodes(&self) -> Vec<String> {
        self.inner.base.nodes.keys().cloned().collect()
    }

    #[getter]
    fn branches(&self) -> Vec<String> {
        self.inner.base.branches.keys().cloned().collect()
    }
}

#[pyclass(name = "AcAnalysis")]
struct PyAcAnalysis {
    inner: crate::result::AcAnalysis,
}

#[pymethods]
impl PyAcAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner
            .base
            .get(name)
            .map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }

    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner
            .base
            .get(name)
            .map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }

    #[getter]
    fn frequency(&self) -> Vec<f64> {
        self.inner.frequency.clone()
    }

    /// |H| per frequency point. `ac[node]` returns the real part, not this.
    fn magnitude(&self, name: &str) -> PyResult<Vec<f64>> {
        self.complex_part(name, |wf| wf.magnitude())
    }

    /// 20*log10(|H|) per frequency point.
    fn magnitude_db(&self, name: &str) -> PyResult<Vec<f64>> {
        self.complex_part(name, |wf| wf.magnitude_db())
    }

    /// Phase in degrees (ngspice's own `vp()` reports radians; this does not).
    fn phase(&self, name: &str) -> PyResult<Vec<f64>> {
        self.complex_part(name, |wf| wf.phase_deg())
    }

    #[getter]
    fn measures(&self) -> HashMap<String, f64> {
        measures_to_dict(&self.inner.base.measures)
    }
}

impl PyAcAnalysis {
    fn complex_part(
        &self,
        name: &str,
        f: impl Fn(&crate::result::WaveForm) -> Option<Vec<f64>>,
    ) -> PyResult<Vec<f64>> {
        let wf = self
            .inner
            .base
            .get(name)
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))?;
        f(wf).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "Node '{}' has no complex data; the backend returned a real-valued plot",
                name
            ))
        })
    }
}

#[pyclass(name = "TransientAnalysis")]
struct PyTransientAnalysis {
    inner: crate::result::TransientAnalysis,
}

#[pymethods]
impl PyTransientAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner
            .base
            .get(name)
            .map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }

    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner
            .base
            .get(name)
            .map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }

    #[getter]
    fn time(&self) -> Vec<f64> {
        self.inner.time.clone()
    }

    #[getter]
    fn measures(&self) -> HashMap<String, f64> {
        measures_to_dict(&self.inner.base.measures)
    }
}

#[pyclass(name = "NoiseAnalysis")]
struct PyNoiseAnalysis {
    inner: crate::result::NoiseAnalysis,
}

#[pymethods]
impl PyNoiseAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> {
        measures_to_dict(&self.inner.base.measures)
    }
}

#[pyclass(name = "TransferFunctionAnalysis")]
struct PyTransferFunctionAnalysis {
    inner: crate::result::TransferFunctionAnalysis,
}

#[pymethods]
impl PyTransferFunctionAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> {
        measures_to_dict(&self.inner.base.measures)
    }
}

#[pyclass(name = "SensitivityAnalysis")]
struct PySensitivityAnalysis {
    inner: crate::result::SensitivityAnalysis,
}

#[pymethods]
impl PySensitivityAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> {
        measures_to_dict(&self.inner.base.measures)
    }
}

#[pyclass(name = "PoleZeroAnalysis")]
struct PyPoleZeroAnalysis {
    inner: crate::result::PoleZeroAnalysis,
}

#[pymethods]
impl PyPoleZeroAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner
            .base
            .get(name)
            .map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }

    #[getter]
    fn measures(&self) -> HashMap<String, f64> {
        measures_to_dict(&self.inner.base.measures)
    }
}

#[pyclass(name = "DistortionAnalysis")]
struct PyDistortionAnalysis {
    inner: crate::result::DistortionAnalysis,
}

#[pymethods]
impl PyDistortionAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner
            .base
            .get(name)
            .map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }

    #[getter]
    fn frequency(&self) -> Vec<f64> {
        self.inner.frequency.clone()
    }

    #[getter]
    fn measures(&self) -> HashMap<String, f64> {
        measures_to_dict(&self.inner.base.measures)
    }
}

#[pyclass(name = "PssAnalysis")]
struct PyPssAnalysis {
    inner: crate::result::PssAnalysis,
}

#[pymethods]
impl PyPssAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }
    #[getter]
    fn time(&self) -> Vec<f64> { self.inner.time.clone() }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.base.measures) }
}

#[pyclass(name = "SParamAnalysis")]
struct PySParamAnalysis {
    inner: crate::result::SParamAnalysis,
}

#[pymethods]
impl PySParamAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }
    #[getter]
    fn frequency(&self) -> Vec<f64> { self.inner.frequency.clone() }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.base.measures) }
}

#[pyclass(name = "HarmonicBalanceAnalysis")]
struct PyHarmonicBalanceAnalysis {
    inner: crate::result::HarmonicBalanceAnalysis,
}

#[pymethods]
impl PyHarmonicBalanceAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }
    #[getter]
    fn frequency(&self) -> Vec<f64> { self.inner.frequency.clone() }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.base.measures) }
}

#[pyclass(name = "StabilityAnalysis")]
struct PyStabilityAnalysis {
    inner: crate::result::StabilityAnalysis,
}

#[pymethods]
impl PyStabilityAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }
    #[getter]
    fn frequency(&self) -> Vec<f64> { self.inner.frequency.clone() }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.base.measures) }
}

#[pyclass(name = "TransientNoiseAnalysis")]
struct PyTransientNoiseAnalysis {
    inner: crate::result::TransientNoiseAnalysis,
}

#[pymethods]
impl PyTransientNoiseAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }
    #[getter]
    fn time(&self) -> Vec<f64> { self.inner.time.clone() }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.base.measures) }
}

#[pyclass(name = "SamplingAnalysis")]
struct PySamplingAnalysis {
    inner: crate::result::SamplingAnalysis,
}

#[pymethods]
impl PySamplingAnalysis {
    fn __getitem__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("Node '{}' not found", name)))
    }
    fn __getattr__(&self, name: &str) -> PyResult<Vec<f64>> {
        self.inner.base.get(name).map(|wf| wf.data.clone())
            .ok_or_else(|| PyAttributeError::new_err(format!("No node '{}'", name)))
    }
    #[getter]
    fn measures(&self) -> HashMap<String, f64> { measures_to_dict(&self.inner.base.measures) }
}

// ── Module-level functions ──

/// Lint a SPICE netlist for common issues and backend-specific warnings.
///
/// Returns a dict with "warnings" and "errors" lists.
#[pyfunction]
#[pyo3(signature = (netlist, backend=None))]
fn lint(netlist: &str, backend: Option<&str>) -> HashMap<String, Vec<HashMap<String, pyo3::PyObject>>> {
    pyo3::Python::with_gil(|py| {
        let result = crate::lint::lint_netlist(netlist, backend);

        let warnings: Vec<HashMap<String, pyo3::PyObject>> = result.warnings.iter().map(|w| {
            let mut map = HashMap::new();
            map.insert("line".to_string(), w.line.into_pyobject(py).unwrap().into_any().unbind());
            map.insert("message".to_string(), w.message.clone().into_pyobject(py).unwrap().into_any().unbind());
            if let Some(ref s) = w.suggestion {
                map.insert("suggestion".to_string(), s.clone().into_pyobject(py).unwrap().into_any().unbind());
            }
            let backends: Vec<String> = w.backends_affected.clone();
            map.insert("backends_affected".to_string(), backends.into_pyobject(py).unwrap().into_any().unbind());
            map
        }).collect();

        let errors: Vec<HashMap<String, pyo3::PyObject>> = result.errors.iter().map(|e| {
            let mut map = HashMap::new();
            map.insert("line".to_string(), e.line.into_pyobject(py).unwrap().into_any().unbind());
            map.insert("message".to_string(), e.message.clone().into_pyobject(py).unwrap().into_any().unbind());
            map
        }).collect();

        let mut out = HashMap::new();
        out.insert("warnings".to_string(), warnings);
        out.insert("errors".to_string(), errors);
        out
    })
}

// ── Verilog-A compilation ──

/// Compile Verilog-A source to OSDI. Accepts a .va file path or inline source.
/// Returns the path to the compiled .osdi file.
fn compile_veriloga_impl(source_or_path: &str) -> Result<String, String> {
    crate::veriloga::compile_osdi(&crate::veriloga::resolve(source_or_path)?)
}

/// Compile a Verilog-A source file or inline source to OSDI.
///
/// Returns the path to the compiled .osdi file.
///
/// ```python
/// from spicerack import compile_veriloga
///
/// # From file:
/// osdi_path = compile_veriloga("models/comparator.va")
///
/// # From inline source:
/// osdi_path = compile_veriloga('''
/// `include "disciplines.vams"
/// module myres(a, b);
///     inout a, b; electrical a, b;
///     parameter real r = 1000.0;
///     analog V(a,b) <+ r * I(a,b);
/// endmodule
/// ''')
/// circuit.osdi(osdi_path)
/// ```
#[pyfunction]
fn compile_veriloga(source_or_path: &str) -> PyResult<String> {
    compile_veriloga_impl(source_or_path)
        .map_err(pyo3::exceptions::PyRuntimeError::new_err)
}

// ── Verilog compilation / synthesis helpers ──

/// Resolve a Verilog source string to a file path.
/// If `source_or_path` looks like a file path (.v extension, no newlines), return it directly.
/// Otherwise treat as inline source, write to a temp file, and return that path.
fn resolve_verilog_source(source_or_path: &str) -> Result<std::path::PathBuf, String> {
    let trimmed = source_or_path.trim();

    if trimmed.ends_with(".v") && !trimmed.contains('\n') {
        let p = std::path::Path::new(trimmed);
        if !p.exists() {
            return Err(format!("Verilog file not found: {}", trimmed));
        }
        Ok(std::path::PathBuf::from(trimmed))
    } else {
        let dir = std::env::temp_dir().join("spicerack_verilog");
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create temp dir: {}", e))?;

        let hash = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            trimmed.hash(&mut h);
            h.finish()
        };
        let v_file = dir.join(format!("inline_{:016x}.v", hash));
        std::fs::write(&v_file, trimmed)
            .map_err(|e| format!("Failed to write temp .v file: {}", e))?;
        Ok(v_file)
    }
}

/// Extract the top-level module name from Verilog source.
fn extract_verilog_module_name(source: &str) -> Result<String, String> {
    // Match `module <name>` possibly followed by `(`, `;`, `#`, or whitespace
    let re = regex::Regex::new(r"(?m)^\s*module\s+(\w+)")
        .map_err(|e| format!("Regex error: {}", e))?;
    re.captures(source)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| "Could not extract module name from Verilog source. \
            Ensure source contains `module <name>`.".to_string())
}

/// Compile Verilog source to VVP using Icarus Verilog (iverilog).
/// Returns the path to the compiled .vvp file.
fn compile_verilog_iverilog(source_or_path: &str) -> Result<String, String> {
    use std::process::Command;

    let v_path = resolve_verilog_source(source_or_path)?;
    let vvp_path = v_path.with_extension("vvp");

    // Skip compilation if .vvp is newer than .v
    if vvp_path.exists()
        && let (Ok(v_meta), Ok(vvp_meta)) = (v_path.metadata(), vvp_path.metadata())
            && let (Ok(v_time), Ok(vvp_time)) = (v_meta.modified(), vvp_meta.modified())
                && vvp_time > v_time {
                    return Ok(vvp_path.to_string_lossy().to_string());
                }

    let output = Command::new("iverilog")
        .arg("-o")
        .arg(&vvp_path)
        .arg(&v_path)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "iverilog not found on $PATH. Install Icarus Verilog for co-simulation.\n\
                 Install: sudo apt install iverilog (Debian/Ubuntu) or brew install icarus-verilog (macOS)".to_string()
            } else {
                format!("Failed to run iverilog: {}", e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "iverilog compilation failed (exit {}):\n{}\n{}",
            output.status, stderr, stdout
        ));
    }

    Ok(vvp_path.to_string_lossy().to_string())
}

/// Resolve PDK liberty and SPICE model paths from $PDK_ROOT.
/// Returns (liberty_path, spice_models_path).
fn resolve_pdk_paths(pdk: &str) -> Result<(String, String), String> {
    let pdk_root = std::env::var("PDK_ROOT")
        .map_err(|_| "PDK_ROOT environment variable not set. Set it to your PDK installation root, \
            or provide explicit liberty= and spice_models= paths.".to_string())?;

    // Try with 'A' suffix first (e.g., sky130_fd_sc_hdA), then without
    let suffixes = ["A", ""];
    let mut liberty_path = None;
    let mut spice_path = None;

    for suffix in &suffixes {
        let base = format!("{}/{}{}/libs.ref/{}", pdk_root, pdk, suffix, pdk);

        // Look for liberty file (prefer tt corner)
        let lib_dir = format!("{}/lib", base);
        if let Ok(entries) = std::fs::read_dir(&lib_dir) {
            // Prefer tt corner, then any .lib file
            let mut found_lib = None;
            let mut tt_lib = None;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".lib") {
                    if name.contains("tt") && tt_lib.is_none() {
                        tt_lib = Some(entry.path().to_string_lossy().to_string());
                    }
                    if found_lib.is_none() {
                        found_lib = Some(entry.path().to_string_lossy().to_string());
                    }
                }
            }
            if liberty_path.is_none() {
                liberty_path = tt_lib.or(found_lib);
            }
        }

        // Look for SPICE models
        let spice_dir = format!("{}/spice", base);
        if let Ok(entries) = std::fs::read_dir(&spice_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".spice") {
                    if spice_path.is_none() {
                        spice_path = Some(entry.path().to_string_lossy().to_string());
                    }
                    break;
                }
            }
        }

        if liberty_path.is_some() && spice_path.is_some() {
            break;
        }
    }

    let liberty = liberty_path.ok_or_else(|| format!(
        "Could not find liberty (.lib) file for PDK '{}' under $PDK_ROOT={}. \
         Searched: {}/{pdk}A/libs.ref/{pdk}/lib/ and {}/{pdk}/libs.ref/{pdk}/lib/",
        pdk, pdk_root, pdk_root, pdk_root, pdk = pdk
    ))?;

    let spice_models = spice_path.ok_or_else(|| format!(
        "Could not find SPICE model (.spice) file for PDK '{}' under $PDK_ROOT={}. \
         Searched: {}/{pdk}A/libs.ref/{pdk}/spice/ and {}/{pdk}/libs.ref/{pdk}/spice/",
        pdk, pdk_root, pdk_root, pdk_root, pdk = pdk
    ))?;

    Ok((liberty, spice_models))
}

/// Synthesize Verilog to gate-level netlist using Yosys.
/// Returns the path to the synthesized Verilog netlist file.
fn synthesize_verilog_yosys(
    source_or_path: &str,
    liberty: &str,
    module_name: &str,
) -> Result<String, String> {
    use std::process::Command;

    let v_path = resolve_verilog_source(source_or_path)?;
    let synth_path = v_path.with_extension("synth.v");

    // Skip synthesis if output is newer than source
    if synth_path.exists()
        && let (Ok(v_meta), Ok(s_meta)) = (v_path.metadata(), synth_path.metadata())
            && let (Ok(v_time), Ok(s_time)) = (v_meta.modified(), s_meta.modified())
                && s_time > v_time {
                    return Ok(synth_path.to_string_lossy().to_string());
                }

    let script = format!(
        "read_verilog {v}; synth -top {top}; dfflibmap -liberty {lib}; abc -liberty {lib}; write_verilog {out}",
        v = v_path.display(),
        top = module_name,
        lib = liberty,
        out = synth_path.display(),
    );

    let output = Command::new("yosys")
        .arg("-p")
        .arg(&script)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "yosys not found on $PATH. Install Yosys for Verilog synthesis.\n\
                 Install: sudo apt install yosys (Debian/Ubuntu) or brew install yosys (macOS)".to_string()
            } else {
                format!("Failed to run yosys: {}", e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "yosys synthesis failed (exit {}):\n{}\n{}",
            output.status, stderr, stdout
        ));
    }

    Ok(synth_path.to_string_lossy().to_string())
}

/// One synthesized cell: (instance_name, cell_name, port connections).
type CellInstance = (String, String, Vec<(String, String)>);

/// Parse a Yosys-synthesized Verilog netlist into cell instances.
/// Returns vec of (instance_name, cell_name, port_connections as ".<port>(<net>)" pairs).
fn parse_synthesized_netlist(netlist_path: &str) -> Result<Vec<CellInstance>, String> {
    let content = std::fs::read_to_string(netlist_path)
        .map_err(|e| format!("Failed to read synthesized netlist: {}", e))?;

    let mut instances = Vec::new();

    // Match Yosys output patterns like: <cell> <instance> ( .<port>(<net>), ... );
    // Yosys writes: `  sky130_fd_sc_hd__and2_1 _0_ ( .A(\a ), .B(\b ), .X(\out ) );`
    let inst_re = regex::Regex::new(r"(?m)^\s+(\w+)\s+(\w+)\s*\(([^;]+)\);")
        .map_err(|e| format!("Regex error: {}", e))?;
    let port_re = regex::Regex::new(r"\.\s*(\w+)\s*\(\s*\\?([^\s)]+)\s*\)")
        .map_err(|e| format!("Regex error: {}", e))?;

    for cap in inst_re.captures_iter(&content) {
        let cell_name = cap[1].to_string();
        let inst_name = cap[2].to_string();

        // Skip `wire`, `assign`, `input`, `output`, `module`, `endmodule`
        if matches!(cell_name.as_str(), "wire" | "assign" | "input" | "output" | "module" | "endmodule" | "reg") {
            continue;
        }

        let port_str = &cap[3];
        let mut ports = Vec::new();
        for pcap in port_re.captures_iter(port_str) {
            ports.push((pcap[1].to_string(), pcap[2].to_string()));
        }

        instances.push((inst_name, cell_name, ports));
    }

    Ok(instances)
}

/// Extract connection values from a PyObject (either a string or list of strings).
fn extract_connection(py: pyo3::Python<'_>, obj: &pyo3::PyObject) -> PyResult<Vec<String>> {
    // Try extracting as a single string first
    if let Ok(s) = obj.extract::<String>(py) {
        return Ok(vec![s]);
    }
    // Try extracting as a list of strings
    if let Ok(v) = obj.extract::<Vec<String>>(py) {
        return Ok(v);
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "Connection value must be a string or list of strings"
    ))
}

/// Expand each Verilog block of `sub` into what simulates it: a `d_cosim`
/// XSPICE instance (simulate) or the synthesized standard-cell instances
/// (synthesize).
fn lower_verilog(sub: &mut ir::Subcircuit) -> Result<(), String> {
    use crate::ir::{VerilogConnection, VerilogMode};
    fn nets(c: &VerilogConnection) -> Vec<String> {
        match c {
            VerilogConnection::Single(net) => vec![net.clone()],
            VerilogConnection::Bus(nets) => nets.clone(),
        }
    }
    for vb in std::mem::take(&mut sub.verilog_blocks) {
        match vb.mode {
            VerilogMode::Simulate => {
                let vvp_path = compile_verilog_iverilog(&vb.source)?;
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
                sub.raw_spice.push(format!(".model {model} d_cosim(delay=1n simulation_id=\"{vvp_path}\")"));
                sub.components.push(Component::Xspice {
                    name: vb.instance_name.clone(),
                    connections: vb.connections.values().map(|c| format!("[{}]", nets(c).join(" "))).collect(),
                    model,
                });
            }
            VerilogMode::Synthesize => {
                let (lib_path, spice_path) = match (&vb.liberty, &vb.pdk) {
                    (Some(lib), Some(pdk)) => {
                        let sp = match &vb.spice_models {
                            Some(s) => s.clone(),
                            None => resolve_pdk_paths(pdk)?.1,
                        };
                        (lib.clone(), sp)
                    }
                    (Some(lib), None) => {
                        let sp = vb.spice_models.as_ref()
                            .ok_or("mode='synthesize' requires either pdk or both liberty and spice_models")?;
                        (lib.clone(), sp.clone())
                    }
                    (None, Some(pdk)) => {
                        let (lib, sp) = resolve_pdk_paths(pdk)?;
                        (lib, vb.spice_models.clone().unwrap_or(sp))
                    }
                    (None, None) => return Err("mode='synthesize' requires either pdk or liberty".to_string()),
                };
                let source_text = if vb.source.trim().ends_with(".v") && !vb.source.contains('\n') {
                    std::fs::read_to_string(vb.source.trim())
                        .map_err(|e| format!("Failed to read {}: {}", vb.source, e))?
                } else {
                    vb.source.clone()
                };
                let module_name = extract_verilog_module_name(&source_text)?;
                let synth_path = synthesize_verilog_yosys(&vb.source, &lib_path, &module_name)?;
                sub.includes.push(spice_path);
                let conn_map: HashMap<&String, Vec<String>> =
                    vb.connections.iter().map(|(port, c)| (port, nets(c))).collect();
                for (gate_inst, cell_name, ports) in parse_synthesized_netlist(&synth_path)? {
                    let port_mapping = ports.iter().map(|(_port, net)| {
                        conn_map.get(net).and_then(|v| v.first()).cloned()
                            .unwrap_or_else(|| format!("{}_{}", vb.instance_name, net))
                    }).collect();
                    sub.instances.push(ir::Instance {
                        name: format!("{}_{}", vb.instance_name, gate_inst),
                        subcircuit: cell_name,
                        port_mapping,
                        parameters: vec![],
                    });
                }
            }
        }
    }
    Ok(())
}

// ── Subcircuit: the element builder (Circuit extends it) ──

#[pyclass(name = "Subcircuit", subclass)]
#[derive(Clone)]
struct PySubcircuit {
    inner: ir::Subcircuit,
    /// `gnd` names the global ground `0`: true only for the top-level `Circuit`.
    ground_alias: bool,
}

impl PySubcircuit {
    fn net(&self, n: &str) -> String {
        if self.ground_alias && n.eq_ignore_ascii_case("gnd") { "0".into() } else { n.into() }
    }

    fn push(&mut self, c: Component) {
        self.inner.components.push(c);
    }

    fn src(&mut self, current: bool, name: &str, p: &str, n: &str, value: IrValue,
           ac: Option<f64>, ac_phase: Option<f64>, wf: Option<IrWaveform>) {
        let c = source(current, name, self.net(p), self.net(n), value, ac, ac_phase, wf);
        self.push(c);
    }
}

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

    // ── Elements ──

    #[pyo3(signature = (*, name, positive, negative, value, raw_spice=None))]
    fn R(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg, raw_spice: Option<&str>) {
        let value = match raw_spice {
            Some(text) => IrValue::Raw { text: text.to_string() },
            None => value.ir(),
        };
        self.push(Component::Resistor { name: name.into(), n1: self.net(positive), n2: self.net(negative), value, params: vec![] });
    }

    #[pyo3(signature = (*, name, positive, negative, value))]
    fn C(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg) {
        self.push(Component::Capacitor { name: name.into(), n1: self.net(positive), n2: self.net(negative), value: value.ir(), params: vec![] });
    }

    #[pyo3(signature = (*, name, positive, negative, value))]
    fn L(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg) {
        self.push(Component::Inductor { name: name.into(), n1: self.net(positive), n2: self.net(negative), value: value.ir(), params: vec![] });
    }

    #[pyo3(signature = (*, name, inductor1, inductor2, coupling))]
    fn K(&mut self, name: &str, inductor1: &str, inductor2: &str, coupling: f64) {
        self.push(Component::MutualInductor { name: name.into(), inductor1: inductor1.into(), inductor2: inductor2.into(), coupling });
    }

    #[pyo3(signature = (*, name, positive, negative, value, ac=None, ac_phase=None))]
    fn V(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg, ac: Option<f64>, ac_phase: Option<f64>) {
        self.src(false, name, positive, negative, value.ir(), ac, ac_phase, None);
    }

    #[pyo3(signature = (*, name, positive, negative, value, ac=None, ac_phase=None))]
    fn I(&mut self, name: &str, positive: &str, negative: &str, value: PyValueArg, ac: Option<f64>, ac_phase: Option<f64>) {
        self.src(true, name, positive, negative, value.ir(), ac, ac_phase, None);
    }

    #[pyo3(signature = (*, name, positive, negative, expression))]
    fn BV(&mut self, name: &str, positive: &str, negative: &str, expression: &str) {
        self.push(Component::BehavioralVoltage { name: name.into(), np: self.net(positive), nm: self.net(negative), expression: expression.into() });
    }

    #[pyo3(signature = (*, name, positive, negative, expression))]
    fn BI(&mut self, name: &str, positive: &str, negative: &str, expression: &str) {
        self.push(Component::BehavioralCurrent { name: name.into(), np: self.net(positive), nm: self.net(negative), expression: expression.into() });
    }

    #[pyo3(signature = (*, name, positive, negative, control_positive, control_negative, voltage_gain))]
    fn E(&mut self, name: &str, positive: &str, negative: &str, control_positive: &str, control_negative: &str, voltage_gain: f64) {
        self.push(Component::Vcvs {
            name: name.into(), np: self.net(positive), nm: self.net(negative),
            ncp: self.net(control_positive), ncm: self.net(control_negative), gain: voltage_gain,
        });
    }

    #[pyo3(signature = (*, name, positive, negative, control_positive, control_negative, transconductance))]
    fn G(&mut self, name: &str, positive: &str, negative: &str, control_positive: &str, control_negative: &str, transconductance: f64) {
        self.push(Component::Vccs {
            name: name.into(), np: self.net(positive), nm: self.net(negative),
            ncp: self.net(control_positive), ncm: self.net(control_negative), transconductance,
        });
    }

    #[pyo3(signature = (*, name, positive, negative, vsense, current_gain))]
    fn F(&mut self, name: &str, positive: &str, negative: &str, vsense: &str, current_gain: f64) {
        self.push(Component::Cccs { name: name.into(), np: self.net(positive), nm: self.net(negative), vsense: vsense.into(), gain: current_gain });
    }

    #[pyo3(signature = (*, name, positive, negative, vsense, transresistance))]
    fn H(&mut self, name: &str, positive: &str, negative: &str, vsense: &str, transresistance: f64) {
        self.push(Component::Ccvs { name: name.into(), np: self.net(positive), nm: self.net(negative), vsense: vsense.into(), transresistance });
    }

    #[pyo3(signature = (*, name, anode, cathode, model))]
    fn D(&mut self, name: &str, anode: &str, cathode: &str, model: &str) {
        self.push(Component::Diode { name: name.into(), np: self.net(anode), nm: self.net(cathode), model: model.into(), params: vec![] });
    }

    #[pyo3(signature = (*, name, collector, base, emitter, model))]
    fn Q(&mut self, name: &str, collector: &str, base: &str, emitter: &str, model: &str) {
        self.push(Component::Bjt {
            name: name.into(), nc: self.net(collector), nb: self.net(base), ne: self.net(emitter), model: model.into(), params: vec![],
        });
    }

    #[pyo3(signature = (*, name, collector, base, emitter, model))]
    fn BJT(&mut self, name: &str, collector: &str, base: &str, emitter: &str, model: &str) {
        self.Q(name, collector, base, emitter, model);
    }

    #[pyo3(signature = (*, name, drain, gate, source, bulk, model, **kwargs))]
    fn M(&mut self, name: &str, drain: &str, gate: &str, source: &str, bulk: &str, model: &str, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.push(Component::Mosfet {
            name: name.into(), nd: self.net(drain), ng: self.net(gate), ns: self.net(source), nb: self.net(bulk),
            model: model.into(), params: kw_params(kwargs)?,
        });
        Ok(())
    }

    #[pyo3(signature = (*, name, drain, gate, source, bulk, model, **kwargs))]
    fn MOSFET(&mut self, name: &str, drain: &str, gate: &str, source: &str, bulk: &str, model: &str, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.M(name, drain, gate, source, bulk, model, kwargs)
    }

    #[pyo3(signature = (*, name, drain, gate, source, model))]
    fn J(&mut self, name: &str, drain: &str, gate: &str, source: &str, model: &str) {
        self.push(Component::Jfet { name: name.into(), nd: self.net(drain), ng: self.net(gate), ns: self.net(source), model: model.into(), params: vec![] });
    }

    #[pyo3(signature = (*, name, drain, gate, source, model))]
    fn Z(&mut self, name: &str, drain: &str, gate: &str, source: &str, model: &str) {
        self.push(Component::Mesfet { name: name.into(), nd: self.net(drain), ng: self.net(gate), ns: self.net(source), model: model.into(), params: vec![] });
    }

    #[pyo3(signature = (*, name, positive, negative, control_positive, control_negative, model))]
    fn S(&mut self, name: &str, positive: &str, negative: &str, control_positive: &str, control_negative: &str, model: &str) {
        self.push(Component::VSwitch {
            name: name.into(), np: self.net(positive), nm: self.net(negative),
            ncp: self.net(control_positive), ncm: self.net(control_negative), model: model.into(),
        });
    }

    #[pyo3(signature = (*, name, positive, negative, vcontrol, model))]
    fn W(&mut self, name: &str, positive: &str, negative: &str, vcontrol: &str, model: &str) {
        self.push(Component::ISwitch { name: name.into(), np: self.net(positive), nm: self.net(negative), vcontrol: vcontrol.into(), model: model.into() });
    }

    #[pyo3(signature = (*, name, input_positive, input_negative, output_positive, output_negative, Z0, TD))]
    fn T(&mut self, name: &str, input_positive: &str, input_negative: &str, output_positive: &str, output_negative: &str, Z0: f64, TD: f64) {
        self.push(Component::TLine {
            name: name.into(), inp: self.net(input_positive), inm: self.net(input_negative),
            outp: self.net(output_positive), outm: self.net(output_negative), z0: Z0, td: TD,
        });
    }

    /// XSPICE code model instance (A-element).
    ///
    /// ```python
    /// circuit.A("adc1", connections=["[vin]", "[dout]"], model="adc_buf")
    /// circuit.A("dff1", connections=["[d clk $d_hi $d_hi]", "[q qbar]"], model="dff_model")
    /// ```
    #[pyo3(signature = (*, name, connections, model))]
    fn A(&mut self, name: &str, connections: Vec<String>, model: &str) {
        self.push(Component::Xspice { name: name.into(), connections, model: model.into() });
    }

    // ── High-level waveform sources ──

    #[pyo3(signature = (*, name, positive, negative, dc_offset=0.0, offset=0.0, amplitude=1.0, frequency=1000.0, ac=None, ac_phase=None))]
    fn SinusoidalVoltageSource(
        &mut self, name: &str, positive: &str, negative: &str,
        dc_offset: f64, offset: f64, amplitude: f64, frequency: f64, ac: Option<f64>, ac_phase: Option<f64>,
    ) {
        self.src(false, name, positive, negative, num(dc_offset), ac, ac_phase, sin(offset, amplitude, frequency));
    }

    #[pyo3(signature = (*, name, positive, negative, initial_value=0.0, pulsed_value=1.0, pulse_width=50e-9, period=100e-9, rise_time=1e-9, fall_time=1e-9))]
    fn PulseVoltageSource(
        &mut self, name: &str, positive: &str, negative: &str,
        initial_value: f64, pulsed_value: f64, pulse_width: f64, period: f64, rise_time: f64, fall_time: f64,
    ) {
        let wf = pulse(initial_value, pulsed_value, pulse_width, period, rise_time, fall_time);
        self.src(false, name, positive, negative, num(0.0), None, None, wf);
    }

    #[pyo3(signature = (*, name, positive, negative, values))]
    fn PieceWiseLinearVoltageSource(&mut self, name: &str, positive: &str, negative: &str, values: Vec<(f64, f64)>) {
        self.src(false, name, positive, negative, num(0.0), None, None, Some(IrWaveform::Pwl { values }));
    }

    #[pyo3(signature = (*, name, positive, negative, dc_offset=0.0, offset=0.0, amplitude=1.0, frequency=1000.0))]
    fn SinusoidalCurrentSource(
        &mut self, name: &str, positive: &str, negative: &str,
        dc_offset: f64, offset: f64, amplitude: f64, frequency: f64,
    ) {
        self.src(true, name, positive, negative, num(dc_offset), None, None, sin(offset, amplitude, frequency));
    }

    #[pyo3(signature = (*, name, positive, negative, initial_value=0.0, pulsed_value=1.0, pulse_width=50e-9, period=100e-9, rise_time=1e-9, fall_time=1e-9))]
    fn PulseCurrentSource(
        &mut self, name: &str, positive: &str, negative: &str,
        initial_value: f64, pulsed_value: f64, pulse_width: f64, period: f64, rise_time: f64, fall_time: f64,
    ) {
        let wf = pulse(initial_value, pulsed_value, pulse_width, period, rise_time, fall_time);
        self.src(true, name, positive, negative, num(0.0), None, None, wf);
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
            name: name.into(), subcircuit: subcircuit_name.into(), port_mapping, parameters: kw_params(params)?,
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
        let mode = match mode {
            "simulate" => ir::VerilogMode::Simulate,
            "synthesize" => ir::VerilogMode::Synthesize,
            _ => return Err(PyValueError::new_err(
                format!("Invalid mode '{}'. Must be 'simulate' or 'synthesize'.", mode)
            )),
        };
        let mut ir_connections = HashMap::new();
        for (port, obj) in &connections {
            let mut nets = extract_connection(py, obj)?;
            let conn = if nets.len() == 1 {
                ir::VerilogConnection::Single(nets.remove(0))
            } else {
                ir::VerilogConnection::Bus(nets)
            };
            ir_connections.insert(port.clone(), conn);
        }
        self.inner.verilog_blocks.push(ir::VerilogBlock {
            source: source.to_string(),
            mode,
            instance_name: instance_name.to_string(),
            connections: ir_connections,
            pdk: pdk.map(str::to_string),
            liberty: liberty.map(str::to_string),
            spice_models: spice_models.map(str::to_string),
        });
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

// ── Testbench: a simulator around a DUT, with stimulus and multi-analysis decks ──

#[pyclass(name = "Testbench", extends = PySimulator)]
struct PyTestbench {}

#[pymethods]
impl PyTestbench {
    #[new]
    fn new(dut: &PySubcircuit) -> (Self, PySimulator) {
        let tb = ir::Testbench { dut: dut.inner.name.clone(), ..Default::default() };
        let ir = CircuitIR { top: dut.inner.clone(), testbench: Some(tb), ..Default::default() };
        (Self {}, PySimulator { ir, backend: None, save_currents: false })
    }

    /// Register a child definition (its include/lib/osdi are hoisted to the top level).
    fn add_subcircuit(mut slf: PyRefMut<'_, Self>, subckt: &PySubcircuit) {
        let ir = &mut slf.as_super().ir;
        hoist_directives(&mut ir.top, &subckt.inner);
        ir.subcircuit_defs.push(subckt.inner.clone());
    }

    /// Attach a PDK model library. Codegen resolves the per-backend path and corner.
    fn use_pdk(mut slf: PyRefMut<'_, Self>, pdk: &PyModelLibrary) {
        slf.as_super().ir.model_libraries.push(pdk.inner.clone());
    }

    // ── Stimulus ──

    #[pyo3(signature = (*, name, positive, negative, value, ac=None, ac_phase=None))]
    fn V(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, value: PyValueArg, ac: Option<f64>, ac_phase: Option<f64>) {
        slf.as_super().stim(source(false, name, positive.into(), negative.into(), value.ir(), ac, ac_phase, None));
    }

    #[pyo3(signature = (*, name, positive, negative, value, ac=None, ac_phase=None))]
    fn I(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, value: PyValueArg, ac: Option<f64>, ac_phase: Option<f64>) {
        slf.as_super().stim(source(true, name, positive.into(), negative.into(), value.ir(), ac, ac_phase, None));
    }

    #[pyo3(signature = (*, name, positive, negative, dc_offset=0.0, offset=0.0, amplitude=1.0, frequency=1000.0, ac=None, ac_phase=None))]
    fn SinusoidalVoltageSource(
        mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
        dc_offset: f64, offset: f64, amplitude: f64, frequency: f64, ac: Option<f64>, ac_phase: Option<f64>,
    ) {
        let wf = sin(offset, amplitude, frequency);
        slf.as_super().stim(source(false, name, positive.into(), negative.into(), num(dc_offset), ac, ac_phase, wf));
    }

    #[pyo3(signature = (*, name, positive, negative, initial_value=0.0, pulsed_value=1.0, pulse_width=50e-9, period=100e-9, rise_time=1e-9, fall_time=1e-9))]
    fn PulseVoltageSource(
        mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
        initial_value: f64, pulsed_value: f64, pulse_width: f64, period: f64, rise_time: f64, fall_time: f64,
    ) {
        let wf = pulse(initial_value, pulsed_value, pulse_width, period, rise_time, fall_time);
        slf.as_super().stim(source(false, name, positive.into(), negative.into(), num(0.0), None, None, wf));
    }

    #[pyo3(signature = (*, name, positive, negative, values))]
    fn PieceWiseLinearVoltageSource(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, values: Vec<(f64, f64)>) {
        let wf = Some(IrWaveform::Pwl { values });
        slf.as_super().stim(source(false, name, positive.into(), negative.into(), num(0.0), None, None, wf));
    }

    #[pyo3(signature = (*, name, positive, negative, dc_offset=0.0, offset=0.0, amplitude=1.0, frequency=1000.0, ac=None, ac_phase=None))]
    fn SinusoidalCurrentSource(
        mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
        dc_offset: f64, offset: f64, amplitude: f64, frequency: f64, ac: Option<f64>, ac_phase: Option<f64>,
    ) {
        let wf = sin(offset, amplitude, frequency);
        slf.as_super().stim(source(true, name, positive.into(), negative.into(), num(dc_offset), ac, ac_phase, wf));
    }

    #[pyo3(signature = (*, name, positive, negative, initial_value=0.0, pulsed_value=1.0, pulse_width=50e-9, period=100e-9, rise_time=1e-9, fall_time=1e-9))]
    fn PulseCurrentSource(
        mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
        initial_value: f64, pulsed_value: f64, pulse_width: f64, period: f64, rise_time: f64, fall_time: f64,
    ) {
        let wf = pulse(initial_value, pulsed_value, pulse_width, period, rise_time, fall_time);
        slf.as_super().stim(source(true, name, positive.into(), negative.into(), num(0.0), None, None, wf));
    }

    #[pyo3(signature = (*, name, positive, negative, values))]
    fn PieceWiseLinearCurrentSource(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, values: Vec<(f64, f64)>) {
        let wf = Some(IrWaveform::Pwl { values });
        slf.as_super().stim(source(true, name, positive.into(), negative.into(), num(0.0), None, None, wf));
    }

    #[pyo3(signature = (*, name, positive, negative, value))]
    fn R(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, value: PyValueArg) {
        slf.as_super().stim(Component::Resistor { name: name.into(), n1: positive.into(), n2: negative.into(), value: value.ir(), params: vec![] });
    }

    #[pyo3(signature = (*, name, positive, negative, value))]
    fn C(mut slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str, value: PyValueArg) {
        slf.as_super().stim(Component::Capacitor { name: name.into(), n1: positive.into(), n2: negative.into(), value: value.ir(), params: vec![] });
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
    fn add_ac(mut slf: PyRefMut<'_, Self>, variation: &str, number_of_points: u32, start_frequency: f64, stop_frequency: f64) {
        let a = Analysis::Ac { variation: variation.into(), points: number_of_points, start: start_frequency, stop: stop_frequency };
        slf.as_super().tb().analyses.push(a);
    }

    #[pyo3(signature = (step_time, end_time, start_time=None, max_time=None, use_initial_condition=false))]
    fn add_transient(mut slf: PyRefMut<'_, Self>, step_time: f64, end_time: f64, start_time: Option<f64>, max_time: Option<f64>, use_initial_condition: bool) {
        let a = Analysis::Transient { step: step_time, stop: end_time, start: start_time, max_step: max_time, uic: use_initial_condition };
        slf.as_super().tb().analyses.push(a);
    }

    #[pyo3(signature = (output_node, ref_node, src, variation="dec", points=10, start_frequency=1e3, stop_frequency=1e8, points_per_summary=None))]
    fn add_noise(
        mut slf: PyRefMut<'_, Self>, output_node: &str, ref_node: &str, src: &str, variation: &str, points: u32,
        start_frequency: f64, stop_frequency: f64, points_per_summary: Option<u32>,
    ) {
        let a = Analysis::Noise {
            output: output_node.into(), reference: ref_node.into(), source: src.into(),
            variation: variation.into(), points, start: start_frequency, stop: stop_frequency, points_per_summary,
        };
        slf.as_super().tb().analyses.push(a);
    }

    #[pyo3(signature = (fundamental_frequency, outputs, num_harmonics=None))]
    fn add_fourier(mut slf: PyRefMut<'_, Self>, fundamental_frequency: f64, outputs: Vec<String>, num_harmonics: Option<u32>) {
        slf.as_super().tb().analyses.push(Analysis::Fourier { fundamental: fundamental_frequency, outputs, num_harmonics });
    }

    #[pyo3(signature = (param, start, stop, step, inner_analysis, inner_type))]
    fn add_spectre_sweep(mut slf: PyRefMut<'_, Self>, param: &str, start: f64, stop: f64, step: f64, inner_analysis: &str, inner_type: &str) {
        let a = Analysis::SpectreSweep {
            param: param.into(), start, stop, step, inner: inner_analysis.into(), inner_type: inner_type.into(),
        };
        slf.as_super().tb().analyses.push(a);
    }

    #[pyo3(signature = (num_iterations, inner_analysis, inner_type, seed=None))]
    fn add_spectre_monte_carlo(mut slf: PyRefMut<'_, Self>, num_iterations: u32, inner_analysis: &str, inner_type: &str, seed: Option<u64>) {
        let a = Analysis::SpectreMonteCarlo {
            iterations: num_iterations, inner: inner_analysis.into(), inner_type: inner_type.into(), seed,
        };
        slf.as_super().tb().analyses.push(a);
    }

    #[pyo3(signature = (num_iterations, inner_analysis, inner_type, seed=None))]
    fn add_spectre_montecarlo(slf: PyRefMut<'_, Self>, num_iterations: u32, inner_analysis: &str, inner_type: &str, seed: Option<u64>) {
        Self::add_spectre_monte_carlo(slf, num_iterations, inner_analysis, inner_type, seed);
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
        let mut bp = HashMap::new();
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

    macro_rules! add_unit {
        ($name:ident, $val:expr) => {
            unit_mod.add(stringify!($name), PyUnit { inner: $val })?;
        };
    }

    // Volts
    add_unit!(u_V, u::U_V);
    add_unit!(u_mV, u::U_MV);
    add_unit!(u_uV, u::U_UV);
    // Amperes
    add_unit!(u_A, u::U_A);
    add_unit!(u_mA, u::U_MA);
    add_unit!(u_uA, u::U_UA);
    add_unit!(u_nA, u::U_NA);
    // Ohms
    add_unit!(u_Ohm, u::U_OHM);
    add_unit!(u_kOhm, u::U_KOHM);
    add_unit!(u_MOhm, u::U_MOHM);
    // Farads
    add_unit!(u_F, u::U_F);
    add_unit!(u_mF, u::U_MF);
    add_unit!(u_uF, u::U_UF);
    add_unit!(u_nF, u::U_NF);
    add_unit!(u_pF, u::U_PF);
    add_unit!(u_fF, u::U_FF);
    // Henrys
    add_unit!(u_H, u::U_H);
    add_unit!(u_mH, u::U_MH);
    add_unit!(u_uH, u::U_UH);
    add_unit!(u_nH, u::U_NH);
    // Hertz
    add_unit!(u_Hz, u::U_HZ);
    add_unit!(u_kHz, u::U_KHZ);
    add_unit!(u_MHz, u::U_MHZ);
    add_unit!(u_GHz, u::U_GHZ);
    // Seconds
    add_unit!(u_s, u::U_S);
    add_unit!(u_ms, u::U_MS);
    add_unit!(u_us, u::U_US);
    add_unit!(u_ns, u::U_NS);
    add_unit!(u_ps, u::U_PS);
    // Watts
    add_unit!(u_W, u::U_W);
    add_unit!(u_mW, u::U_MW);
    add_unit!(u_uW, u::U_UW);
    // Degrees
    add_unit!(u_Degree, u::U_DEGREE);

    // Unicode aliases (PySpice compat)
    unit_mod.add("u_Ω", PyUnit { inner: u::U_OHM })?;
    unit_mod.add("u_kΩ", PyUnit { inner: u::U_KOHM })?;
    unit_mod.add("u_MΩ", PyUnit { inner: u::U_MOHM })?;
    unit_mod.add("u_µF", PyUnit { inner: u::U_UF })?;
    unit_mod.add("u_µH", PyUnit { inner: u::U_UH })?;
    unit_mod.add("u_µA", PyUnit { inner: u::U_UA })?;
    unit_mod.add("u_µV", PyUnit { inner: u::U_UV })?;
    unit_mod.add("u_µW", PyUnit { inner: u::U_UW })?;
    unit_mod.add("u_µs", PyUnit { inner: u::U_US })?;

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
    m.add_class::<PySamplingAnalysis>()?;

    // IR types
    m.add_class::<PySubcircuit>()?;
    m.add_class::<PyTestbench>()?;
    m.add_class::<PyModelLibrary>()?;

    // Module-level functions
    m.add_function(wrap_pyfunction!(lint, m)?)?;
    m.add_function(wrap_pyfunction!(compile_veriloga, m)?)?;

    create_unit_module(m)?;
    Ok(())
}
