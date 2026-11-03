//! Subcircuit: element, instance and directive calls → one
//! `ir::Subcircuit` (Circuit extends it). Nets are named here: `gnd` is
//! ground `0` only at the top level.
//!
//! Contents:
//! - `pub(super) struct PySubcircuit` (Python `Subcircuit`)
//! - `pub(super) fn top_net(n: &str) -> String`

use std::collections::HashMap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

// `either`, `IrNum`, `IrWaveform`: names `pymethods_with_waves!` resolves here.
use super::args::{PyValueArg, kw_params};
use super::deck::runtime_err;
use super::source::{either, source, wave_source};
use super::verilog::verilog_block;
use crate::ir::{self, Component, IrNum, IrValue, IrWaveform};

#[pyclass(name = "Subcircuit", subclass)]
#[derive(Clone)]
pub(super) struct PySubcircuit {
    pub(super) inner: ir::Subcircuit,
    /// `gnd` names the global ground `0`: true only for the top-level `Circuit`.
    pub(super) ground_alias: bool,
}

/// A node name at the top level: `gnd` (any case) is the global ground `0`.
pub(super) fn top_net(n: &str) -> String {
    if n.eq_ignore_ascii_case("gnd") {
        "0".into()
    } else {
        n.into()
    }
}

// ---- private ----

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
        if matches!(letter, 'E' | 'G') && !controls.len().is_multiple_of(2) {
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

impl PySubcircuit {
    fn net(&self, n: &str) -> String {
        if self.ground_alias {
            top_net(n)
        } else {
            n.into()
        }
    }

    fn push(&mut self, c: Component) {
        self.inner.components.push(c);
    }

    /// Adds one waveform source; the generated `*Source` methods call it.
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
        let c = wave_source(current, name, slf.net(p), slf.net(n), wf, dc, ac, ac_phase);
        slf.push(c);
    }

    fn src(
        &mut self,
        current: bool,
        name: &str,
        p: &str,
        n: &str,
        value: IrValue,
        ac: Option<f64>,
        ac_phase: Option<f64>,
        wf: Option<IrWaveform>,
    ) {
        let c = source(
            current,
            name,
            self.net(p),
            self.net(n),
            value,
            ac,
            ac_phase,
            wf,
        );
        self.push(c);
    }
}
