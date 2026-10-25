use std::collections::BTreeMap;
use serde::{Serialize, Deserialize};

// ── Core IR types ──

/// The netlist language a simulator reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
    Ngspice,
    Ltspice,
    Spectre,
    Vacask,
    /// ngspice text with Verilog-A loaded as `.hdl` instead of `pre_osdi`.
    Espice,
    /// HSPICE: `'expr'` quoting, `E`/`G` `vol=`/`cur=` behavioural sources,
    /// `.hdl` Verilog-A, and the card forms in ESPice's
    /// `tests/frontend/grammar/hspice.peg` and `tests/conformance/tests/netlist/hspice`.
    Hspice,
    /// Xyce: the xschem layout only (`codegen::xschem`).
    Xyce,
    /// CDL for LVS: the xschem layout only (`codegen::xschem`).
    Cdl,
    /// Structural Verilog: the xschem layout only (`codegen::xschem`).
    Verilog,
}

impl Dialect {
    pub const ALL: [Dialect; 9] = [Dialect::Ngspice, Dialect::Ltspice, Dialect::Spectre, Dialect::Vacask, Dialect::Espice, Dialect::Hspice, Dialect::Xyce, Dialect::Cdl, Dialect::Verilog];

    /// The dialect's name (`ngspice`, ...), as `FromStr` reads it.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Ngspice => "ngspice",
            Dialect::Ltspice => "ltspice",
            Dialect::Spectre => "spectre",
            Dialect::Vacask => "vacask",
            Dialect::Espice => "espice",
            Dialect::Hspice => "hspice",
            Dialect::Xyce => "xyce",
            Dialect::Cdl => "cdl",
            Dialect::Verilog => "verilog",
        }
    }

    /// The dialect whose text this one reads: ESPice reads ngspice decks.
    pub fn family(self) -> Dialect {
        if self == Dialect::Espice { Dialect::Ngspice } else { self }
    }
}

impl std::str::FromStr for Dialect {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Dialect::ALL.into_iter().find(|d| d.name().eq_ignore_ascii_case(s)).ok_or_else(|| format!("unknown dialect '{s}'"))
    }
}


/// Backend-neutral intermediate representation for a complete simulation
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CircuitIR {
    pub top: Subcircuit,
    pub testbench: Option<Testbench>,
    pub subcircuit_defs: Vec<Subcircuit>,
    pub model_libraries: Vec<ModelLibrary>,
    /// The dialect the `raw_spice` lines are written in, when known (the
    /// importer sets it: "ngspice", "hspice"). Such lines reach only that
    /// dialect; `None` means generic SPICE, which every SPICE dialect takes.
    #[serde(default)]
    pub raw_dialect: Option<Dialect>,
    /// `.global` nodes: one net under the same name in every subcircuit.
    #[serde(default)]
    pub globals: Vec<String>,
    /// Top-level `.func` definitions: `name(args) = body`, the body an
    /// expression over the arguments and parameters.
    #[serde(default)]
    pub functions: Vec<FuncDef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuncDef {
    pub name: String,
    pub args: Vec<String>,
    pub body: String,
}

/// Named, parameterized, composable circuit fragment -- the unit of reuse
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Subcircuit {
    pub name: String,
    pub ports: Vec<Port>,
    pub parameters: Vec<ParamDef>,
    pub components: Vec<Component>,
    pub instances: Vec<Instance>,
    pub models: Vec<ModelDef>,
    pub raw_spice: Vec<String>,
    pub includes: Vec<String>,
    pub libs: Vec<(String, String)>,
    pub osdi_loads: Vec<String>,
    #[serde(default)]
    pub verilog_blocks: Vec<VerilogBlock>,
}

/// Verilog co-simulation or synthesis block stored in the IR
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerilogBlock {
    pub source: String,
    pub mode: VerilogMode,
    pub instance_name: String,
    pub connections: BTreeMap<String, VerilogConnection>,
    pub pdk: Option<String>,
    pub liberty: Option<String>,
    pub spice_models: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VerilogMode {
    Simulate,
    Synthesize,
}

/// A Verilog port connection: single net or a bus (vector of nets)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VerilogConnection {
    Single(String),
    Bus(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Port {
    pub name: String,
    pub direction: PortDirection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PortDirection {
    InOut,
    Input,
    Output,
}

/// Parameter definition with optional default
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamDef {
    pub name: String,
    pub default: Option<String>,
}

/// Reference to a Subcircuit definition + port mapping + parameter overrides
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub subcircuit: String,
    pub port_mapping: Vec<String>,
    pub parameters: Vec<(String, String)>,
    /// Where the instance sat among its subcircuit's components, as a reader
    /// saw it: before `components[at]`; `None` after them all. A simulator's
    /// matrix order follows card order, so a deck written back keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u32>,
}

/// One element of a subcircuit, in card order ([`Subcircuit::elements`]).
pub enum Element<'a> {
    Component(&'a Component),
    Instance(&'a Instance),
}

impl Subcircuit {
    /// Components and instances merged in card order (each instance before
    /// the component it preceded).
    pub fn elements(&self) -> Vec<Element<'_>> {
        let n = self.components.len();
        let pos = |i: &Instance| i.at.map_or(n, |a| (a as usize).min(n));
        let mut insts: Vec<&Instance> = self.instances.iter().collect();
        insts.sort_by_key(|i| pos(i)); // stable: same-place instances keep their order
        let mut insts = insts.into_iter().peekable();
        let mut out = Vec::with_capacity(n + self.instances.len());
        for (k, c) in self.components.iter().enumerate() {
            while let Some(i) = insts.next_if(|i| pos(i) <= k) {
                out.push(Element::Instance(i));
            }
            out.push(Element::Component(c));
        }
        out.extend(insts.map(Element::Instance));
        out
    }
}

/// Model definition
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelDef {
    pub name: String,
    pub kind: String,
    pub parameters: Vec<(String, String)>,
}

// ── Component types ──

/// All SPICE component types as structured data
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Component {
    Resistor { name: String, n1: String, n2: String, value: IrValue, #[serde(default)] model: Option<String>, params: Vec<(String, String)> },
    Capacitor { name: String, n1: String, n2: String, value: IrValue, #[serde(default)] model: Option<String>, params: Vec<(String, String)> },
    Inductor { name: String, n1: String, n2: String, value: IrValue, #[serde(default)] model: Option<String>, params: Vec<(String, String)> },
    MutualInductor { name: String, inductor1: String, inductor2: String, coupling: f64 },
    VoltageSource { name: String, np: String, nm: String, value: IrValue, #[serde(default)] ac_magnitude: Option<f64>, #[serde(default)] ac_phase: Option<f64>, waveform: Option<IrWaveform> },
    CurrentSource { name: String, np: String, nm: String, value: IrValue, #[serde(default)] ac_magnitude: Option<f64>, #[serde(default)] ac_phase: Option<f64>, waveform: Option<IrWaveform> },
    BehavioralVoltage { name: String, np: String, nm: String, expression: String },
    BehavioralCurrent { name: String, np: String, nm: String, expression: String },
    Vcvs { name: String, np: String, nm: String, ncp: String, ncm: String, gain: f64 },
    Vccs { name: String, np: String, nm: String, ncp: String, ncm: String, transconductance: f64 },
    Cccs { name: String, np: String, nm: String, vsense: String, gain: f64 },
    Ccvs { name: String, np: String, nm: String, vsense: String, transresistance: f64 },
    Diode { name: String, np: String, nm: String, model: String, params: Vec<(String, String)> },
    /// `extra_nodes`: the substrate node, then the thermal node (VBIC, HICUM),
    /// written after the emitter in every dialect.
    Bjt { name: String, nc: String, nb: String, ne: String, #[serde(default)] extra_nodes: Vec<String>, model: String, params: Vec<(String, String)> },
    /// `nb`: the bulk node, absent on a 3-terminal VDMOS; `extra_nodes`
    /// follow it (VDMOS thermal nodes, SOI body/back gate), in card order.
    Mosfet { name: String, nd: String, ng: String, ns: String, nb: Option<String>, #[serde(default)] extra_nodes: Vec<String>, model: String, params: Vec<(String, String)> },
    Jfet { name: String, nd: String, ng: String, ns: String, model: String, params: Vec<(String, String)> },
    Mesfet { name: String, nd: String, ng: String, ns: String, model: String, params: Vec<(String, String)> },
    VSwitch { name: String, np: String, nm: String, ncp: String, ncm: String, model: String },
    ISwitch { name: String, np: String, nm: String, vcontrol: String, model: String },
    TLine { name: String, inp: String, inm: String, outp: String, outm: String, z0: f64, td: f64 },
    Xspice { name: String, connections: Vec<String>, model: String },
    /// A polynomial controlled source, SPICE `POLY(n)`: `letter` is E (VCVS),
    /// G (VCCS), F (CCCS) or H (CCVS); `controls` are node pairs (E/G) or
    /// sensing V sources (F/H); `coeffs` in SPICE 2G6 term order.
    Poly { name: String, letter: char, np: String, nm: String, controls: Vec<String>, coeffs: Vec<f64> },
    RawSpice { line: String },
}

impl Component {
    /// Every name the component carries, for renaming: its own name, its
    /// nodes, sensing sources and model. XSPICE connections and raw lines
    /// are text and are not included.
    pub fn names_mut(&mut self) -> Vec<&mut String> {
        use Component::*;
        match self {
            Resistor { name, n1, n2, .. } | Capacitor { name, n1, n2, .. } | Inductor { name, n1, n2, .. } => vec![name, n1, n2],
            MutualInductor { name, inductor1, inductor2, .. } => vec![name, inductor1, inductor2],
            VoltageSource { name, np, nm, .. } | CurrentSource { name, np, nm, .. } | BehavioralVoltage { name, np, nm, .. } | BehavioralCurrent { name, np, nm, .. } => vec![name, np, nm],
            Vcvs { name, np, nm, ncp, ncm, .. } | Vccs { name, np, nm, ncp, ncm, .. } => vec![name, np, nm, ncp, ncm],
            Cccs { name, np, nm, vsense, .. } | Ccvs { name, np, nm, vsense, .. } => vec![name, np, nm, vsense],
            Diode { name, np, nm, model, .. } => vec![name, np, nm, model],
            Bjt { name, nc, nb, ne, extra_nodes, model, .. } => [name, nc, nb, ne, model].into_iter().chain(extra_nodes.iter_mut()).collect(),
            Mosfet { name, nd, ng, ns, nb, extra_nodes, model, .. } => [name, nd, ng, ns, model].into_iter().chain(nb.iter_mut()).chain(extra_nodes.iter_mut()).collect(),
            Jfet { name, nd, ng, ns, model, .. } | Mesfet { name, nd, ng, ns, model, .. } => vec![name, nd, ng, ns, model],
            VSwitch { name, np, nm, ncp, ncm, model } => vec![name, np, nm, ncp, ncm, model],
            ISwitch { name, np, nm, vcontrol, model } => vec![name, np, nm, vcontrol, model],
            TLine { name, inp, inm, outp, outm, .. } => vec![name, inp, inm, outp, outm],
            Xspice { name, .. } => vec![name],
            Poly { name, np, nm, controls, .. } => [name, np, nm].into_iter().chain(controls.iter_mut()).collect(),
            RawSpice { .. } => vec![],
        }
    }

    /// Instance name without its SPICE letter; `None` for a raw line.
    pub fn name(&self) -> Option<&str> {
        use Component::*;
        match self {
            Resistor { name, .. } | Capacitor { name, .. } | Inductor { name, .. }
            | MutualInductor { name, .. } | VoltageSource { name, .. } | CurrentSource { name, .. }
            | BehavioralVoltage { name, .. } | BehavioralCurrent { name, .. } | Vcvs { name, .. }
            | Vccs { name, .. } | Cccs { name, .. } | Ccvs { name, .. } | Diode { name, .. }
            | Bjt { name, .. } | Mosfet { name, .. } | Jfet { name, .. } | Mesfet { name, .. }
            | VSwitch { name, .. } | ISwitch { name, .. } | TLine { name, .. } | Xspice { name, .. } | Poly { name, .. } => Some(name),
            RawSpice { .. } => None,
        }
    }
}

// ── Value and waveform types ──

/// Component value in the IR -- numeric, expression, or raw
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IrValue {
    Numeric { value: f64 },
    Expression { expr: String },
    Raw { text: String },
}

/// One waveform parameter: a number, or an expression over `.param`s
/// (`PULSE(0 {vdd} ...)`). JSON keeps a number a number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IrNum {
    Num(f64),
    Expr(String),
}

impl IrNum {
    pub fn num(&self) -> Option<f64> {
        match self {
            IrNum::Num(v) => Some(*v),
            IrNum::Expr(_) => None,
        }
    }
}

impl From<f64> for IrNum {
    fn from(v: f64) -> Self {
        IrNum::Num(v)
    }
}

impl From<&IrNum> for IrValue {
    fn from(n: &IrNum) -> Self {
        match n {
            IrNum::Num(value) => IrValue::Numeric { value: *value },
            IrNum::Expr(expr) => IrValue::Expression { expr: expr.clone() },
        }
    }
}

/// `x == 0.0` reads as before; an expression equals no number.
impl PartialEq<f64> for IrNum {
    fn eq(&self, other: &f64) -> bool {
        self.num() == Some(*other)
    }
}

/// Waveform types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IrWaveform {
    Sin { offset: IrNum, amplitude: IrNum, frequency: IrNum, delay: IrNum, damping: IrNum, phase: IrNum },
    Pulse { initial: IrNum, pulsed: IrNum, delay: IrNum, rise_time: IrNum, fall_time: IrNum, pulse_width: IrNum, period: IrNum },
    Pwl { values: Vec<(IrNum, IrNum)> },
    Exp { initial: IrNum, pulsed: IrNum, rise_delay: IrNum, rise_tau: IrNum, fall_delay: IrNum, fall_tau: IrNum },
    Sffm { offset: IrNum, amplitude: IrNum, carrier_freq: IrNum, modulation_index: IrNum, signal_freq: IrNum },
    Am { amplitude: IrNum, offset: IrNum, modulating_freq: IrNum, carrier_freq: IrNum, delay: IrNum },
}

impl IrWaveform {
    /// Every parameter, for passes over the expressions (`^` checks,
    /// renaming).
    pub fn nums_mut(&mut self) -> Vec<&mut IrNum> {
        use IrWaveform::*;
        match self {
            Sin { offset, amplitude, frequency, delay, damping, phase } => vec![offset, amplitude, frequency, delay, damping, phase],
            Pulse { initial, pulsed, delay, rise_time, fall_time, pulse_width, period } => vec![initial, pulsed, delay, rise_time, fall_time, pulse_width, period],
            Pwl { values } => values.iter_mut().flat_map(|(t, v)| [t, v]).collect(),
            Exp { initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau } => vec![initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau],
            Sffm { offset, amplitude, carrier_freq, modulation_index, signal_freq } => vec![offset, amplitude, carrier_freq, modulation_index, signal_freq],
            Am { amplitude, offset, modulating_freq, carrier_freq, delay } => vec![amplitude, offset, modulating_freq, carrier_freq, delay],
        }
    }

    /// The expression parameters.
    pub fn exprs_ref(&self) -> Vec<&str> {
        use IrWaveform::*;
        let all: Vec<&IrNum> = match self {
            Sin { offset, amplitude, frequency, delay, damping, phase } => vec![offset, amplitude, frequency, delay, damping, phase],
            Pulse { initial, pulsed, delay, rise_time, fall_time, pulse_width, period } => vec![initial, pulsed, delay, rise_time, fall_time, pulse_width, period],
            Pwl { values } => values.iter().flat_map(|(t, v)| [t, v]).collect(),
            Exp { initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau } => vec![initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau],
            Sffm { offset, amplitude, carrier_freq, modulation_index, signal_freq } => vec![offset, amplitude, carrier_freq, modulation_index, signal_freq],
            Am { amplitude, offset, modulating_freq, carrier_freq, delay } => vec![amplitude, offset, modulating_freq, carrier_freq, delay],
        };
        all.into_iter().filter_map(|n| match n {
            IrNum::Expr(e) => Some(e.as_str()),
            IrNum::Num(_) => None,
        }).collect()
    }

    /// The value at t=0, which SPICE takes as the DC value of a card that
    /// gives none (AM starts at 0).
    pub fn at_zero(&self) -> IrValue {
        use IrWaveform::*;
        match self {
            Sin { offset, .. } | Sffm { offset, .. } => offset.into(),
            Pulse { initial, .. } | Exp { initial, .. } => initial.into(),
            Pwl { values } => values.first().map_or(IrValue::Numeric { value: 0.0 }, |p| (&p.1).into()),
            Am { .. } => IrValue::Numeric { value: 0.0 },
        }
    }
}

// ── Testbench ──

/// Testbench -- wraps a DUT with stimulus + analysis
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Testbench {
    pub dut: String,
    pub stimulus: Vec<Component>,
    pub analyses: Vec<Analysis>,
    pub options: SimOptions,
    pub saves: Vec<String>,
    pub measures: Vec<String>,
    pub temperature: Option<f64>,
    pub nominal_temperature: Option<f64>,
    pub initial_conditions: Vec<(String, f64)>,
    pub node_sets: Vec<(String, f64)>,
    pub step_params: Vec<StepParam>,
    pub extra_lines: Vec<String>,
    /// `.print`/`.plot` cards: what a SPICE run prints to stdout. A rawfile
    /// run ignores them (ngspice manual p.353), so dialects without the card
    /// (LTspice, Spectre, VACASK) omit them without changing a result.
    #[serde(default)]
    pub prints: Vec<PrintCard>,
}

/// One `.print` card: the analysis (`tran`, `dc`, ...; empty if none named)
/// and its output expressions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrintCard {
    pub analysis: String,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepParam {
    pub param: String,
    pub start: f64,
    pub stop: f64,
    pub step: f64,
    pub sweep_type: Option<String>,
}

// ── Analysis types ──

/// Analysis types -- all supported analyses as a tagged union
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Analysis {
    Op,
    Dc { sweeps: Vec<DcSweep> },
    Ac { variation: String, points: u32, start: f64, stop: f64 },
    Transient { step: f64, stop: f64, start: Option<f64>, max_step: Option<f64>, uic: bool },
    Noise { output: String, reference: String, source: String, variation: String, points: u32, start: f64, stop: f64, points_per_summary: Option<u32> },
    Tf { output: String, source: String },
    Sensitivity { output: String, ac: Option<AcSweepParams> },
    PoleZero { node1: String, node2: String, node3: String, node4: String, tf_type: String, pz_type: String },
    Distortion { variation: String, points: u32, start: f64, stop: f64, f2overf1: Option<f64> },
    Pss { fundamental: f64, stabilization: f64, observe_node: String, points_per_period: u32, harmonics: u32 },
    HarmonicBalance { frequencies: Vec<f64>, harmonics: Vec<u32> },
    SPar { variation: String, points: u32, start: f64, stop: f64 },
    Stability { probe: String, variation: String, points: u32, start: f64, stop: f64 },
    TransientNoise { step: f64, stop: f64 },
    Fourier { fundamental: f64, outputs: Vec<String>, num_harmonics: Option<u32> },
    // Vendor-specific
    SpectreSweep { param: String, start: f64, stop: f64, step: f64, inner: String, inner_type: String },
    SpectreMonteCarlo { iterations: u32, inner: String, inner_type: String, seed: Option<u64> },
    SpectrePac { pss_fundamental: f64, pss_stabilization: f64, pss_harmonics: u32, variation: String, points: u32, start: f64, stop: f64, sweep_type: String },
    SpectrePnoise { pss_fundamental: f64, pss_stabilization: f64, pss_harmonics: u32, output: String, reference: String, variation: String, points: u32, start: f64, stop: f64 },
    SpectrePxf { pss_fundamental: f64, pss_stabilization: f64, pss_harmonics: u32, output: String, source: String, variation: String, points: u32, start: f64, stop: f64 },
    SpectrePstb { pss_fundamental: f64, pss_stabilization: f64, pss_harmonics: u32, probe: String, variation: String, points: u32, start: f64, stop: f64 },
}

impl Analysis {
    pub fn kind_str(&self) -> &str {
        match self {
            Analysis::Op => "op",
            Analysis::Dc { .. } => "dc",
            Analysis::Ac { .. } => "ac",
            Analysis::Transient { .. } => "tran",
            Analysis::Noise { .. } => "noise",
            Analysis::Tf { .. } => "tf",
            Analysis::Sensitivity { ac: Some(_), .. } => "sens_ac",
            Analysis::Sensitivity { .. } => "sens",
            Analysis::PoleZero { .. } => "pz",
            Analysis::Distortion { .. } => "disto",
            Analysis::Pss { .. } => "pss",
            Analysis::HarmonicBalance { .. } => "hb",
            Analysis::SPar { .. } => "sp",
            Analysis::Stability { .. } => "stb",
            Analysis::TransientNoise { .. } => "trannoise",
            Analysis::Fourier { .. } => "tran",
            Analysis::SpectreSweep { .. } => "tran",
            Analysis::SpectreMonteCarlo { .. } => "tran",
            Analysis::SpectrePac { .. } => "pac",
            Analysis::SpectrePnoise { .. } => "pnoise",
            Analysis::SpectrePxf { .. } => "pxf",
            Analysis::SpectrePstb { .. } => "pstb",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DcSweep {
    pub source: String,
    pub start: f64,
    pub stop: f64,
    pub step: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcSweepParams {
    pub variation: String,
    pub points: u32,
    pub start: f64,
    pub stop: f64,
}

// ── Simulation options ──

/// Simulation options -- portable + per-backend
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SimOptions {
    pub portable: Vec<(String, String)>,
    pub backend_specific: BTreeMap<String, Vec<(String, String)>>,
}

/// Opaque PDK/model library reference
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelLibrary {
    pub name: String,
    pub path: String,
    pub corner: Option<String>,
    pub backend_paths: BTreeMap<String, String>,
    #[serde(default)]
    pub setup_includes: Vec<String>,
}

// ── Feature flags ──

/// What a deck needs from its simulator, one bit per feature
/// (`backend::BackendKind::capabilities` is the same mask, what it can do).
pub mod feature {
    pub const XSPICE: u8 = 1 << 0;
    pub const OSDI: u8 = 1 << 1;
    pub const MEASURES: u8 = 1 << 2;
    pub const STEP_PARAMS: u8 = 1 << 3;
    pub const CONTROL_BLOCKS: u8 = 1 << 4;
    pub const LAPLACE: u8 = 1 << 5;
    pub const VERILOG_COSIM: u8 = 1 << 6;
}

/// Feature flags computed from IR
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeatureFlags {
    /// `feature::*` bits.
    pub needs: u8,
    pub element_count: usize,
}

impl FeatureFlags {
    pub fn has(&self, f: u8) -> bool {
        self.needs & f != 0
    }
}

/// Each feature a backend may lack, how bad that is, and what to say
/// (`{}` is the backend).
#[cfg(feature = "backend")]
const MISSING: &[(u8, IssueSeverity, &str)] = &[
    (feature::XSPICE, IssueSeverity::Error, "Backend '{}' does not support XSPICE A-elements (only ngspice does)"),
    (feature::OSDI, IssueSeverity::Error, "Backend '{}' does not support OSDI/Verilog-A model loading"),
    (feature::MEASURES, IssueSeverity::Error, "Backend '{}' does not support .meas directives"),
    (feature::STEP_PARAMS, IssueSeverity::Error, "Backend '{}' does not support .step param sweeps"),
    (feature::CONTROL_BLOCKS, IssueSeverity::Error, "Backend '{}' does not support .control blocks (ngspice only)"),
    (feature::LAPLACE, IssueSeverity::Warning, "Backend '{}' does not support Laplace-domain B-sources"),
    (feature::VERILOG_COSIM, IssueSeverity::Error, "Backend '{}' does not support Verilog co-simulation"),
];

// ── Compatibility checking ──

/// Issue found by check_backend
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: IssueSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IssueSeverity {
    Error,
    Warning,
}

// ── CircuitIR methods ──

impl CircuitIR {
    /// Compute feature flags by walking the IR
    pub fn compute_features(&self) -> FeatureFlags {
        let mut flags = FeatureFlags::default();

        self.scan_subcircuit(&self.top, &mut flags);
        for sub in &self.subcircuit_defs {
            self.scan_subcircuit(sub, &mut flags);
        }

        // Check testbench for measures, step params
        if let Some(ref tb) = self.testbench {
            if !tb.measures.is_empty() {
                flags.needs |= feature::MEASURES;
            }
            if !tb.step_params.is_empty() {
                flags.needs |= feature::STEP_PARAMS;
            }
        }

        flags
    }

    fn scan_subcircuit(&self, sub: &Subcircuit, flags: &mut FeatureFlags) {
        if !sub.osdi_loads.is_empty() {
            flags.needs |= feature::OSDI;
        }

        if sub.verilog_blocks.iter().any(|vb| matches!(vb.mode, VerilogMode::Simulate)) {
            flags.needs |= feature::VERILOG_COSIM;
        }

        for line in &sub.raw_spice {
            let trimmed = line.trim().to_lowercase();
            if trimmed == ".control" || trimmed.starts_with(".control ") {
                flags.needs |= feature::CONTROL_BLOCKS;
            }
        }

        for comp in &sub.components {
            flags.element_count += 1;
            match comp {
                Component::Xspice { .. } => {
                    flags.needs |= feature::XSPICE;
                }
                Component::BehavioralVoltage { expression, .. }
                | Component::BehavioralCurrent { expression, .. }
                    if crate::codegen::laplace::is_laplace(expression) => {
                        flags.needs |= feature::LAPLACE;
                    }
                _ => {}
            }
        }

        flags.element_count += sub.instances.len();
    }

    #[cfg(feature = "backend")]
    /// Check backend compatibility -- pure function, no I/O.
    ///
    /// Returns a list of issues (errors and warnings) for the given backend name.
    pub fn check_backend(&self, backend: &str) -> Vec<Issue> {
        let features = self.compute_features();
        let mut issues = Vec::new();

        let kind = match crate::backend::BackendKind::from_str(backend) {
            Some(k) => k,
            None => {
                issues.push(Issue {
                    severity: IssueSeverity::Error,
                    message: format!("Unknown backend '{}'", backend),
                });
                return issues;
            }
        };
        let missing = features.needs & !kind.capabilities();
        for &(f, severity, message) in MISSING {
            if missing & f != 0 {
                issues.push(Issue { severity, message: message.replace("{}", backend) });
            }
        }

        if features.element_count > 10_000
            && (backend == "ngspice" || backend == "ngspice-subprocess") {
                issues.push(Issue {
                    severity: IssueSeverity::Warning,
                    message: format!(
                        "Large circuit ({} elements) may be slow on '{}'",
                        features.element_count, backend
                    ),
                });
            }

        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_features_xspice() {
        let ir = CircuitIR { 
            top: Subcircuit {
                name: "xspice_test".into(),
                ports: Vec::new(),
                parameters: Vec::new(),
                components: vec![
                    Component::Xspice {
                        name: "1".into(),
                        connections: vec!["in".into(), "out".into()],
                        model: "d_and".into(),
                    },
                    Component::Resistor {
                        name: "1".into(),
                        n1: "out".into(),
                        n2: "0".into(),
                        value: IrValue::Numeric { value: 1000.0 },
                        model: None, params: Vec::new(),
                    },
                ],
                instances: Vec::new(),
                models: Vec::new(),
                raw_spice: Vec::new(),
                includes: Vec::new(),
                libs: Vec::new(),
                osdi_loads: Vec::new(), ..Default::default() },
            subcircuit_defs: Vec::new(),
            model_libraries: Vec::new(), ..Default::default() };

        let features = ir.compute_features();
        assert!(features.has(feature::XSPICE));
        assert!(!features.has(feature::OSDI));
        assert!(!features.has(feature::MEASURES));
        assert!(!features.has(feature::LAPLACE));
        assert_eq!(features.element_count, 2);
    }

    #[test]
    fn test_compute_features_osdi_and_laplace() {
        let ir = CircuitIR { 
            top: Subcircuit {
                name: "osdi_test".into(),
                ports: Vec::new(),
                parameters: Vec::new(),
                components: vec![
                    Component::BehavioralVoltage {
                        name: "1".into(),
                        np: "out".into(),
                        nm: "0".into(),
                        expression: "laplace(V(in), {1/(1+s*1e-6)})".into(),
                    },
                ],
                instances: Vec::new(),
                models: Vec::new(),
                raw_spice: Vec::new(),
                includes: Vec::new(),
                libs: Vec::new(),
                osdi_loads: vec!["model.osdi".into()], ..Default::default() },
            subcircuit_defs: Vec::new(),
            model_libraries: Vec::new(), ..Default::default() };

        let features = ir.compute_features();
        assert!(!features.has(feature::XSPICE));
        assert!(features.has(feature::OSDI));
        assert!(features.has(feature::LAPLACE));
        assert_eq!(features.element_count, 1);
    }

    #[test]
    fn test_compute_features_control_blocks() {
        let ir = CircuitIR { 
            top: Subcircuit {
                name: "ctrl_test".into(),
                ports: Vec::new(),
                parameters: Vec::new(),
                components: Vec::new(),
                instances: Vec::new(),
                models: Vec::new(),
                raw_spice: vec![".control".into(), "run".into(), ".endc".into()],
                includes: Vec::new(),
                libs: Vec::new(),
                osdi_loads: Vec::new(), ..Default::default() },
            subcircuit_defs: Vec::new(),
            model_libraries: Vec::new(), ..Default::default() };

        let features = ir.compute_features();
        assert!(features.has(feature::CONTROL_BLOCKS));
    }

    #[test]
    fn test_compute_features_measures_and_step() {
        let ir = CircuitIR { 
            top: Subcircuit {
                name: "meas_test".into(),
                ports: Vec::new(),
                parameters: Vec::new(),
                components: Vec::new(),
                instances: Vec::new(),
                models: Vec::new(),
                raw_spice: Vec::new(),
                includes: Vec::new(),
                libs: Vec::new(),
                osdi_loads: Vec::new(), ..Default::default() },
            testbench: Some(Testbench {
                dut: "meas_test".into(),
                stimulus: Vec::new(),
                analyses: Vec::new(),
                saves: Vec::new(),
                measures: vec!["tran v_max MAX V(out)".into()],
                initial_conditions: Vec::new(),
                node_sets: Vec::new(),
                step_params: vec![StepParam {
                    param: "R1".into(),
                    start: 100.0,
                    stop: 10000.0,
                    step: 100.0,
                    sweep_type: None,
                }],
                extra_lines: Vec::new(), ..Default::default() }),
            subcircuit_defs: Vec::new(),
            model_libraries: Vec::new(), ..Default::default() };

        let features = ir.compute_features();
        assert!(features.has(feature::MEASURES));
        assert!(features.has(feature::STEP_PARAMS));
    }

    #[cfg(feature = "backend")]
    #[test]
    fn test_check_backend_osdi_against_ltspice() {
        let ir = CircuitIR { 
            top: Subcircuit {
                name: "osdi_check".into(),
                ports: Vec::new(),
                parameters: Vec::new(),
                components: Vec::new(),
                instances: Vec::new(),
                models: Vec::new(),
                raw_spice: Vec::new(),
                includes: Vec::new(),
                libs: Vec::new(),
                osdi_loads: vec!["model.osdi".into()], ..Default::default() },
            subcircuit_defs: Vec::new(),
            model_libraries: Vec::new(), ..Default::default() };

        let issues = ir.check_backend("ltspice");
        assert!(issues.iter().any(|i| {
            i.severity == IssueSeverity::Error && i.message.contains("OSDI")
        }));

        // Spectre supports OSDI
        assert!(ir.check_backend("spectre").is_empty());
    }

    #[cfg(feature = "backend")]
    #[test]
    fn test_check_backend_no_issues_for_clean_circuit() {
        let ir = CircuitIR { 
            top: Subcircuit {
                name: "clean".into(),
                ports: Vec::new(),
                parameters: Vec::new(),
                components: vec![Component::Resistor {
                    name: "1".into(),
                    n1: "in".into(),
                    n2: "out".into(),
                    value: IrValue::Numeric { value: 1000.0 },
                    model: None, params: Vec::new(),
                }],
                instances: Vec::new(),
                models: Vec::new(),
                raw_spice: Vec::new(),
                includes: Vec::new(),
                libs: Vec::new(),
                osdi_loads: Vec::new(), ..Default::default() },
            subcircuit_defs: Vec::new(),
            model_libraries: Vec::new(), ..Default::default() };

        for backend in &["ngspice", "ltspice", "vacask", "spectre"] {
            assert!(ir.check_backend(backend).is_empty(), "Expected no issues for {}", backend);
        }
    }

    // ── JSON serialization tests ──

    #[test]
    fn test_load_example_voltage_divider() {
        let json = include_str!("../../schema/examples/voltage_divider.circuit.json");
        let ir: CircuitIR = serde_json::from_str(json).unwrap();
        assert_eq!(ir.top.name, "Voltage Divider");
        assert_eq!(ir.top.components.len(), 3);
        assert!(ir.testbench.is_some());
        let tb = ir.testbench.as_ref().unwrap();
        assert_eq!(tb.analyses.len(), 1);
        match &tb.analyses[0] {
            Analysis::Op => {}
            other => panic!("Expected Op analysis, got {:?}", other),
        }
    }

    #[test]
    fn test_all_component_variants_roundtrip() {
        // Build an IR containing every Component variant to ensure full roundtrip coverage
        let all_components = vec![
            Component::Resistor { name: "1".into(), n1: "a".into(), n2: "b".into(), value: IrValue::Numeric { value: 1e3 }, model: None, params: vec![("tc1".into(), "0.001".into())] },
            Component::Capacitor { name: "1".into(), n1: "a".into(), n2: "b".into(), value: IrValue::Expression { expr: "cap_val".into() }, model: None, params: vec![] },
            Component::Inductor { name: "1".into(), n1: "a".into(), n2: "b".into(), value: IrValue::Raw { text: "10u".into() }, model: None, params: vec![] },
            Component::MutualInductor { name: "1".into(), inductor1: "L1".into(), inductor2: "L2".into(), coupling: 0.95 },
            Component::VoltageSource { name: "1".into(), np: "a".into(), nm: "b".into(), value: IrValue::Numeric { value: 5.0 }, ac_magnitude: None, ac_phase: None, waveform: None },
            Component::CurrentSource { name: "1".into(), np: "a".into(), nm: "b".into(), value: IrValue::Numeric { value: 1e-3 }, ac_magnitude: None, ac_phase: None, waveform: Some(IrWaveform::Pulse { initial: 0.0.into(), pulsed: 1e-3.into(), delay: 0.0.into(), rise_time: 1e-9.into(), fall_time: 1e-9.into(), pulse_width: 5e-6.into(), period: 10e-6.into() }) },
            Component::BehavioralVoltage { name: "1".into(), np: "a".into(), nm: "b".into(), expression: "V(x)*2+1".into() },
            Component::BehavioralCurrent { name: "1".into(), np: "a".into(), nm: "b".into(), expression: "I(V1)*10".into() },
            Component::Vcvs { name: "1".into(), np: "a".into(), nm: "b".into(), ncp: "c".into(), ncm: "d".into(), gain: 100.0 },
            Component::Vccs { name: "1".into(), np: "a".into(), nm: "b".into(), ncp: "c".into(), ncm: "d".into(), transconductance: 0.01 },
            Component::Cccs { name: "1".into(), np: "a".into(), nm: "b".into(), vsense: "Vs".into(), gain: 50.0 },
            Component::Ccvs { name: "1".into(), np: "a".into(), nm: "b".into(), vsense: "Vs".into(), transresistance: 500.0 },
            Component::Diode { name: "1".into(), np: "a".into(), nm: "b".into(), model: "D1".into(), params: vec![] },
            Component::Bjt { name: "1".into(), nc: "c".into(), nb: "b".into(), ne: "e".into(), extra_nodes: vec![], model: "Q1".into(), params: vec![("area".into(), "2".into())] },
            Component::Mosfet { name: "1".into(), nd: "d".into(), ng: "g".into(), ns: "s".into(), nb: Some("b".into()), extra_nodes: vec![], model: "M1".into(), params: vec![("W".into(), "1u".into()), ("L".into(), "180n".into())] },
            Component::Jfet { name: "1".into(), nd: "d".into(), ng: "g".into(), ns: "s".into(), model: "J1".into(), params: vec![] },
            Component::Mesfet { name: "1".into(), nd: "d".into(), ng: "g".into(), ns: "s".into(), model: "Z1".into(), params: vec![] },
            Component::VSwitch { name: "1".into(), np: "a".into(), nm: "b".into(), ncp: "c".into(), ncm: "d".into(), model: "SW".into() },
            Component::ISwitch { name: "1".into(), np: "a".into(), nm: "b".into(), vcontrol: "Vs".into(), model: "CSW".into() },
            Component::TLine { name: "1".into(), inp: "i1".into(), inm: "i2".into(), outp: "o1".into(), outm: "o2".into(), z0: 50.0, td: 1e-9 },
            Component::Xspice { name: "1".into(), connections: vec!["[din]".into(), "[dout]".into()], model: "d_buf".into() },
            Component::RawSpice { line: ".probe V(out)".into() },
        ];

        // The JSON tags each variant by name.
        for c in &all_components {
            let name = format!("{c:?}");
            assert_eq!(serde_json::to_value(c).unwrap()["type"], name.split([' ', '{']).next().unwrap());
        }
        let ir = CircuitIR {
            top: Subcircuit {
                name: "all_components".into(),
                components: all_components, ..Default::default() }, ..Default::default() };

        let json = serde_json::to_string_pretty(&ir).unwrap();
        let ir2: CircuitIR = serde_json::from_str(&json).unwrap();
        assert_eq!(ir, ir2);
    }

    #[test]
    fn test_all_analysis_variants_roundtrip() {
        let analyses = vec![
            Analysis::Op,
            Analysis::Dc { sweeps: vec![DcSweep { source: "V1".into(), start: 0.0, stop: 5.0, step: 0.01 }] },
            Analysis::Ac { variation: "dec".into(), points: 100, start: 1.0, stop: 1e9 },
            Analysis::Transient { step: 1e-9, stop: 1e-3, start: Some(0.0), max_step: Some(1e-10), uic: true },
            Analysis::Noise { output: "V(out)".into(), reference: "0".into(), source: "V1".into(), variation: "dec".into(), points: 10, start: 1.0, stop: 1e6, points_per_summary: None },
            Analysis::Tf { output: "V(out)".into(), source: "Vin".into() },
            Analysis::Sensitivity { output: "V(out)".into(), ac: Some(AcSweepParams { variation: "dec".into(), points: 10, start: 1.0, stop: 1e6 }) },
            Analysis::PoleZero { node1: "in".into(), node2: "0".into(), node3: "out".into(), node4: "0".into(), tf_type: "vol".into(), pz_type: "pz".into() },
            Analysis::Distortion { variation: "dec".into(), points: 10, start: 1.0, stop: 1e6, f2overf1: Some(0.9) },
            Analysis::Pss { fundamental: 1e6, stabilization: 100e-6, observe_node: "out".into(), points_per_period: 100, harmonics: 10 },
            Analysis::HarmonicBalance { frequencies: vec![1e6, 2e6], harmonics: vec![5, 3] },
            Analysis::SPar { variation: "lin".into(), points: 201, start: 1e6, stop: 10e9 },
            Analysis::Stability { probe: "iprobe".into(), variation: "dec".into(), points: 100, start: 1.0, stop: 1e9 },
            Analysis::TransientNoise { step: 1e-9, stop: 1e-6 },
            Analysis::Fourier { fundamental: 1e3, outputs: vec!["V(out)".into()], num_harmonics: Some(10) },
            Analysis::SpectreSweep { param: "R1".into(), start: 1e3, stop: 10e3, step: 1e3, inner: "dc1".into(), inner_type: "dc".into() },
            Analysis::SpectreMonteCarlo { iterations: 100, inner: "tran1".into(), inner_type: "tran".into(), seed: Some(42) },
            Analysis::SpectrePac { pss_fundamental: 1e6, pss_stabilization: 100e-6, pss_harmonics: 10, variation: "dec".into(), points: 100, start: 1.0, stop: 1e9, sweep_type: "relative".into() },
            Analysis::SpectrePnoise { pss_fundamental: 1e6, pss_stabilization: 100e-6, pss_harmonics: 10, output: "out".into(), reference: "0".into(), variation: "dec".into(), points: 100, start: 1.0, stop: 1e9 },
            Analysis::SpectrePxf { pss_fundamental: 1e6, pss_stabilization: 100e-6, pss_harmonics: 10, output: "out".into(), source: "V1".into(), variation: "dec".into(), points: 100, start: 1.0, stop: 1e9 },
            Analysis::SpectrePstb { pss_fundamental: 1e6, pss_stabilization: 100e-6, pss_harmonics: 10, probe: "iprobe".into(), variation: "dec".into(), points: 100, start: 1.0, stop: 1e9 },
        ];

        for a in &analyses {
            let name = format!("{a:?}");
            assert_eq!(serde_json::to_value(a).unwrap()["type"], name.split([' ', '{']).next().unwrap());
        }
        let ir = CircuitIR {
            top: Subcircuit {
                name: "analysis_test".into(), ..Default::default() },
            testbench: Some(Testbench {
                dut: "analysis_test".into(),
                analyses, ..Default::default() }), ..Default::default() };

        let json = serde_json::to_string_pretty(&ir).unwrap();
        let ir2: CircuitIR = serde_json::from_str(&json).unwrap();
        assert_eq!(ir, ir2);
    }

    #[test]
    fn elements_keep_card_order() {
        let r = |n: &str| Component::RawSpice { line: n.into() };
        let x = |n: &str, at: Option<u32>| Instance { name: n.into(), at, ..Default::default() };
        let sc = Subcircuit { components: vec![r("a"), r("b")], instances: vec![x("late", None), x("first", Some(0)), x("mid", Some(1))], ..Default::default() };
        let names: Vec<&str> = sc.elements().into_iter().map(|e| match e { Element::Component(Component::RawSpice { line }) => line.as_str(), Element::Instance(i) => i.name.as_str(), _ => "" }).collect();
        assert_eq!(names, ["first", "a", "mid", "b", "late"]);
    }

    // ── Issue 02: Analysis::kind_str() ──

    #[test]
    fn test_analysis_kind_str() {
        assert_eq!(Analysis::Op.kind_str(), "op");
        assert_eq!(
            Analysis::Dc { sweeps: vec![] }.kind_str(),
            "dc"
        );
        assert_eq!(
            Analysis::Ac { variation: "dec".into(), points: 10, start: 1.0, stop: 1e6 }.kind_str(),
            "ac"
        );
        assert_eq!(
            Analysis::Transient { step: 1e-9, stop: 1e-6, start: None, max_step: None, uic: false }.kind_str(),
            "tran"
        );
        assert_eq!(
            Analysis::Noise { output: "out".into(), reference: "0".into(), source: "V1".into(), variation: "dec".into(), points: 10, start: 1.0, stop: 1e6, points_per_summary: None }.kind_str(),
            "noise"
        );
        assert_eq!(
            Analysis::Pss { fundamental: 1e6, stabilization: 10e-6, observe_node: "out".into(), points_per_period: 100, harmonics: 10 }.kind_str(),
            "pss"
        );
        assert_eq!(
            Analysis::HarmonicBalance { frequencies: vec![1e6], harmonics: vec![5] }.kind_str(),
            "hb"
        );
        assert_eq!(
            Analysis::Stability { probe: "p".into(), variation: "dec".into(), points: 10, start: 1.0, stop: 1e6 }.kind_str(),
            "stb"
        );
        assert_eq!(
            Analysis::TransientNoise { step: 1e-9, stop: 1e-6 }.kind_str(),
            "trannoise"
        );
        assert_eq!(
            Analysis::SpectrePac { pss_fundamental: 1e6, pss_stabilization: 10e-6, pss_harmonics: 10, variation: "dec".into(), points: 10, start: 1.0, stop: 1e6, sweep_type: "relative".into() }.kind_str(),
            "pac"
        );
    }

    #[test]
    fn test_ac_magnitude_serde_roundtrip() {
        let v_with_ac = Component::VoltageSource {
            name: "ac".into(), np: "in".into(), nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0), ac_phase: Some(45.0),
            waveform: None,
        };
        let json = serde_json::to_string(&v_with_ac).unwrap();
        assert!(json.contains("\"ac_magnitude\":1.0"), "json has ac_magnitude: {}", json);
        assert!(json.contains("\"ac_phase\":45.0"), "json has ac_phase: {}", json);

        let roundtripped: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(v_with_ac, roundtripped);

        // Without AC — fields should be absent (serde skip_serializing_none via default)
        let v_no_ac = Component::VoltageSource {
            name: "dc".into(), np: "vdd".into(), nm: "0".into(),
            value: IrValue::Numeric { value: 3.3 },
            ac_magnitude: None, ac_phase: None,
            waveform: None,
        };
        let json_no_ac = serde_json::to_string(&v_no_ac).unwrap();
        let rt_no_ac: Component = serde_json::from_str(&json_no_ac).unwrap();
        assert_eq!(v_no_ac, rt_no_ac);

        // Deserialize JSON without ac fields (backwards compat)
        let old_json = r#"{"type":"VoltageSource","name":"old","np":"a","nm":"0","value":{"type":"Numeric","value":5.0},"waveform":null}"#;
        let old_v: Component = serde_json::from_str(old_json).unwrap();
        match old_v {
            Component::VoltageSource { ac_magnitude, ac_phase, .. } => {
                assert_eq!(ac_magnitude, None);
                assert_eq!(ac_phase, None);
            }
            _ => panic!("Expected VoltageSource"),
        }
    }
}
