//! Testbench: a DUT plus stimulus, PDK libraries and queued analyses →
//! one multi-analysis deck, and its run → one typed result per analysis.
//! `ModelLibrary` is the PDK input it takes.
//!
//! Contents:
//! - `pub(super) struct PyTestbench` (Python `Testbench`, extends `CircuitSimulator`)
//! - `pub(super) struct PyModelLibrary` (Python `ModelLibrary`)

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

// `either`, `IrNum`, `IrWaveform`: names `pymethods_with_waves!` resolves here.
use super::args::PyValueArg;
use super::card::{self, dc_sweeps};
use super::circuit::hoisted;
use super::deck::{check_backend_name, emit_ir_netlist, lowered, runtime_err};
use super::results::typed_result;
use super::simulator::PySimulator;
use super::source::{either, source, wave_source};
use super::subcircuit::{PySubcircuit, top_net};
use crate::ir::{self, Analysis, CircuitIR, Component, IrNum, IrWaveform};

#[pyclass(name = "Testbench", extends = PySimulator)]
pub(super) struct PyTestbench {}

#[pyclass(name = "ModelLibrary")]
pub(super) struct PyModelLibrary {
    inner: ir::ModelLibrary,
}

// ---- private ----

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

    /// Runs every queued analysis (`add_*`) as one deck and returns one
    /// result per analysis, in queue order, each the type its single-analysis
    /// twin returns (`op, tran = tb.run()`); `None` for `add_fourier`, whose
    /// table goes to stdout. `ValueError` with nothing queued.
    fn run(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let sim = slf.as_super();
        let ir = sim.deck_as_queued();
        let analyses = ir.testbench.as_ref().map(|tb| tb.analyses.clone()).unwrap_or_default();
        if analyses.is_empty() {
            return Err(PyValueError::new_err("no analyses queued: add one with add_operating_point, add_transient, ..."));
        }
        let plots = crate::simulation::run_each(&lowered(ir)?, sim.backend.as_deref()).map_err(runtime_err)?;
        assert_eq!(plots.len(), analyses.len(), "run_each returns one entry per analysis");
        analyses.iter().zip(plots).map(|(a, raw)| typed_result(py, a, raw)).collect()
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

impl PyTestbench {
    /// Adds one waveform source to the stimulus; the generated `*Source`
    /// methods call it.
    fn place_wave(
        mut slf: PyRefMut<'_, Self>,
        current: bool,
        name: &str,
        p: &str,
        n: &str,
        wf: IrWaveform,
        dc: Option<f64>,
        ac: Option<f64>,
        ac_phase: Option<f64>,
    ) {
        slf.as_super().stim(wave_source(
            current,
            name,
            top_net(p),
            top_net(n),
            wf,
            dc,
            ac,
            ac_phase,
        ));
    }
}

#[pymethods]
impl PyModelLibrary {
    #[new]
    #[pyo3(signature = (path, corner=None, setup_includes=None, **backend_paths))]
    fn new(
        path: &str,
        corner: Option<&str>,
        setup_includes: Option<Vec<String>>,
        backend_paths: Option<Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
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
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[getter]
    fn path(&self) -> String {
        self.inner.path.clone()
    }

    #[getter]
    fn corner(&self) -> Option<String> {
        self.inner.corner.clone()
    }

    fn __str__(&self) -> String {
        format!(
            "ModelLibrary('{}', corner={:?})",
            self.inner.path, self.inner.corner
        )
    }

    fn __repr__(&self) -> String {
        self.__str__()
    }
}
