use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    pub const ALL: [Dialect; 9] = [
        Dialect::Ngspice,
        Dialect::Ltspice,
        Dialect::Spectre,
        Dialect::Vacask,
        Dialect::Espice,
        Dialect::Hspice,
        Dialect::Xyce,
        Dialect::Cdl,
        Dialect::Verilog,
    ];

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
        if self == Dialect::Espice {
            Dialect::Ngspice
        } else {
            self
        }
    }

    /// Backend names kept as aliases of a dialect's own name, so scripts
    /// that name the old shared-library backends still run.
    pub const ALIASES: [(&'static str, Dialect); 3] = [
        ("ngspice-subprocess", Dialect::Ngspice),
        ("ngspice-shared", Dialect::Ngspice),
        ("vacask-shared", Dialect::Vacask),
    ];

    /// The dialect a backend name selects: its [`Self::name`] or one of
    /// [`Self::ALIASES`], spelled exactly (unlike `FromStr`, case matters).
    pub fn from_backend_name(s: &str) -> Option<Dialect> {
        let alias = Self::ALIASES.iter().find(|a| a.0 == s).map(|a| a.1);
        alias.or_else(|| Self::ALL.into_iter().find(|d| d.name() == s))
    }
}

impl std::str::FromStr for Dialect {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Dialect::ALL
            .into_iter()
            .find(|d| d.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown dialect '{s}'"))
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
    Resistor {
        name: String,
        n1: String,
        n2: String,
        value: IrValue,
        #[serde(default)]
        model: Option<String>,
        params: Vec<(String, String)>,
    },
    Capacitor {
        name: String,
        n1: String,
        n2: String,
        value: IrValue,
        #[serde(default)]
        model: Option<String>,
        params: Vec<(String, String)>,
    },
    Inductor {
        name: String,
        n1: String,
        n2: String,
        value: IrValue,
        #[serde(default)]
        model: Option<String>,
        params: Vec<(String, String)>,
    },
    MutualInductor {
        name: String,
        inductor1: String,
        inductor2: String,
        coupling: f64,
    },
    /// `port`: an RF port for `.sp` (ngspice `portnum n z0 x`, manual
    /// §4.1.11), which puts `z0` in series with the source in every analysis.
    VoltageSource {
        name: String,
        np: String,
        nm: String,
        value: IrValue,
        #[serde(default)]
        ac_magnitude: Option<f64>,
        #[serde(default)]
        ac_phase: Option<f64>,
        waveform: Option<IrWaveform>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<RfPort>,
    },
    CurrentSource {
        name: String,
        np: String,
        nm: String,
        value: IrValue,
        #[serde(default)]
        ac_magnitude: Option<f64>,
        #[serde(default)]
        ac_phase: Option<f64>,
        waveform: Option<IrWaveform>,
    },
    BehavioralVoltage {
        name: String,
        np: String,
        nm: String,
        expression: String,
    },
    BehavioralCurrent {
        name: String,
        np: String,
        nm: String,
        expression: String,
    },
    Vcvs {
        name: String,
        np: String,
        nm: String,
        ncp: String,
        ncm: String,
        gain: f64,
    },
    Vccs {
        name: String,
        np: String,
        nm: String,
        ncp: String,
        ncm: String,
        transconductance: f64,
    },
    Cccs {
        name: String,
        np: String,
        nm: String,
        vsense: String,
        gain: f64,
    },
    Ccvs {
        name: String,
        np: String,
        nm: String,
        vsense: String,
        transresistance: f64,
    },
    Diode {
        name: String,
        np: String,
        nm: String,
        model: String,
        params: Vec<(String, String)>,
    },
    /// `extra_nodes`: the substrate node, then the thermal node (VBIC, HICUM),
    /// written after the emitter in every dialect.
    Bjt {
        name: String,
        nc: String,
        nb: String,
        ne: String,
        #[serde(default)]
        extra_nodes: Vec<String>,
        model: String,
        params: Vec<(String, String)>,
    },
    /// `nb`: the bulk node, absent on a 3-terminal VDMOS; `extra_nodes`
    /// follow it (VDMOS thermal nodes, SOI body/back gate), in card order.
    Mosfet {
        name: String,
        nd: String,
        ng: String,
        ns: String,
        nb: Option<String>,
        #[serde(default)]
        extra_nodes: Vec<String>,
        model: String,
        params: Vec<(String, String)>,
    },
    Jfet {
        name: String,
        nd: String,
        ng: String,
        ns: String,
        model: String,
        params: Vec<(String, String)>,
    },
    Mesfet {
        name: String,
        nd: String,
        ng: String,
        ns: String,
        model: String,
        params: Vec<(String, String)>,
    },
    VSwitch {
        name: String,
        np: String,
        nm: String,
        ncp: String,
        ncm: String,
        model: String,
    },
    ISwitch {
        name: String,
        np: String,
        nm: String,
        vcontrol: String,
        model: String,
    },
    TLine {
        name: String,
        inp: String,
        inm: String,
        outp: String,
        outm: String,
        z0: f64,
        td: f64,
    },
    Xspice {
        name: String,
        connections: Vec<String>,
        model: String,
    },
    /// A polynomial controlled source, SPICE `POLY(n)`: `letter` is E (VCVS),
    /// G (VCCS), F (CCCS) or H (CCVS); `controls` are node pairs (E/G) or
    /// sensing V sources (F/H); `coeffs` in SPICE 2G6 term order.
    Poly {
        name: String,
        letter: char,
        np: String,
        nm: String,
        controls: Vec<String>,
        coeffs: Vec<f64>,
    },
    RawSpice {
        line: String,
    },
}

impl Component {
    /// Every name the component carries, for renaming: its own name, its
    /// nodes, sensing sources and model. XSPICE connections and raw lines
    /// are text and are not included.
    pub fn names_mut(&mut self) -> Vec<&mut String> {
        use Component::*;
        match self {
            Resistor { name, n1, n2, .. }
            | Capacitor { name, n1, n2, .. }
            | Inductor { name, n1, n2, .. } => vec![name, n1, n2],
            MutualInductor {
                name,
                inductor1,
                inductor2,
                ..
            } => vec![name, inductor1, inductor2],
            VoltageSource { name, np, nm, .. }
            | CurrentSource { name, np, nm, .. }
            | BehavioralVoltage { name, np, nm, .. }
            | BehavioralCurrent { name, np, nm, .. } => vec![name, np, nm],
            Vcvs {
                name,
                np,
                nm,
                ncp,
                ncm,
                ..
            }
            | Vccs {
                name,
                np,
                nm,
                ncp,
                ncm,
                ..
            } => vec![name, np, nm, ncp, ncm],
            Cccs {
                name,
                np,
                nm,
                vsense,
                ..
            }
            | Ccvs {
                name,
                np,
                nm,
                vsense,
                ..
            } => vec![name, np, nm, vsense],
            Diode {
                name,
                np,
                nm,
                model,
                ..
            } => vec![name, np, nm, model],
            Bjt {
                name,
                nc,
                nb,
                ne,
                extra_nodes,
                model,
                ..
            } => [name, nc, nb, ne, model]
                .into_iter()
                .chain(extra_nodes.iter_mut())
                .collect(),
            Mosfet {
                name,
                nd,
                ng,
                ns,
                nb,
                extra_nodes,
                model,
                ..
            } => [name, nd, ng, ns, model]
                .into_iter()
                .chain(nb.iter_mut())
                .chain(extra_nodes.iter_mut())
                .collect(),
            Jfet {
                name,
                nd,
                ng,
                ns,
                model,
                ..
            }
            | Mesfet {
                name,
                nd,
                ng,
                ns,
                model,
                ..
            } => vec![name, nd, ng, ns, model],
            VSwitch {
                name,
                np,
                nm,
                ncp,
                ncm,
                model,
            } => vec![name, np, nm, ncp, ncm, model],
            ISwitch {
                name,
                np,
                nm,
                vcontrol,
                model,
            } => vec![name, np, nm, vcontrol, model],
            TLine {
                name,
                inp,
                inm,
                outp,
                outm,
                ..
            } => vec![name, inp, inm, outp, outm],
            Xspice { name, .. } => vec![name],
            Poly {
                name,
                np,
                nm,
                controls,
                ..
            } => [name, np, nm]
                .into_iter()
                .chain(controls.iter_mut())
                .collect(),
            RawSpice { .. } => vec![],
        }
    }

    /// The `k=v` parameters of a device card (R, C, L, D, Q, M, J, Z); empty
    /// for every other component.
    pub fn params(&self) -> &[(String, String)] {
        use Component::*;
        match self {
            Resistor { params, .. }
            | Capacitor { params, .. }
            | Inductor { params, .. }
            | Diode { params, .. }
            | Bjt { params, .. }
            | Mosfet { params, .. }
            | Jfet { params, .. }
            | Mesfet { params, .. } => params,
            _ => &[],
        }
    }

    /// Every expression the component carries, for passes over expression
    /// text (`^` checks, case folding): the parameter values, a waveform's
    /// expression parameters, an expression value, a behavioural source's
    /// expression. Numbers and names are not included.
    pub fn exprs(&self) -> Vec<&str> {
        use Component::*;
        let (value, waveform) = match self {
            Resistor { value, .. } | Capacitor { value, .. } | Inductor { value, .. } => {
                (Some(value), None)
            }
            VoltageSource {
                value, waveform, ..
            }
            | CurrentSource {
                value, waveform, ..
            } => (Some(value), waveform.as_ref()),
            BehavioralVoltage { expression, .. } | BehavioralCurrent { expression, .. } => {
                return vec![expression];
            }
            _ => (None, None),
        };
        let expr = value.and_then(|v| match v {
            IrValue::Expression { expr } => Some(expr.as_str()),
            _ => None,
        });
        self.params()
            .iter()
            .map(|(_, v)| v.as_str())
            .chain(waveform.into_iter().flat_map(IrWaveform::exprs_ref))
            .chain(expr)
            .collect()
    }

    /// [`Self::exprs`], mutable.
    pub fn exprs_mut(&mut self) -> Vec<&mut String> {
        use Component::*;
        let (value, params, waveform): (_, &mut [(String, String)], _) = match self {
            Resistor { value, params, .. }
            | Capacitor { value, params, .. }
            | Inductor { value, params, .. } => (Some(value), params, None),
            VoltageSource {
                value, waveform, ..
            }
            | CurrentSource {
                value, waveform, ..
            } => (Some(value), &mut [], waveform.as_mut()),
            BehavioralVoltage { expression, .. } | BehavioralCurrent { expression, .. } => {
                return vec![expression];
            }
            Diode { params, .. }
            | Bjt { params, .. }
            | Mosfet { params, .. }
            | Jfet { params, .. }
            | Mesfet { params, .. } => (None, params, None),
            _ => (None, &mut [], None),
        };
        let expr = value.and_then(|v| match v {
            IrValue::Expression { expr } => Some(expr),
            _ => None,
        });
        let nums = waveform
            .into_iter()
            .flat_map(IrWaveform::nums_mut)
            .filter_map(|n| match n {
                IrNum::Expr(e) => Some(e),
                IrNum::Num(_) => None,
            });
        params
            .iter_mut()
            .map(|(_, v)| v)
            .chain(nums)
            .chain(expr)
            .collect()
    }

    /// Instance name without its SPICE letter; `None` for a raw line.
    pub fn name(&self) -> Option<&str> {
        use Component::*;
        match self {
            Resistor { name, .. }
            | Capacitor { name, .. }
            | Inductor { name, .. }
            | MutualInductor { name, .. }
            | VoltageSource { name, .. }
            | CurrentSource { name, .. }
            | BehavioralVoltage { name, .. }
            | BehavioralCurrent { name, .. }
            | Vcvs { name, .. }
            | Vccs { name, .. }
            | Cccs { name, .. }
            | Ccvs { name, .. }
            | Diode { name, .. }
            | Bjt { name, .. }
            | Mosfet { name, .. }
            | Jfet { name, .. }
            | Mesfet { name, .. }
            | VSwitch { name, .. }
            | ISwitch { name, .. }
            | TLine { name, .. }
            | Xspice { name, .. }
            | Poly { name, .. } => Some(name),
            RawSpice { .. } => None,
        }
    }
}

/// A voltage source's RF port (ngspice manual §4.1.11): its number among
/// the deck's ports, which count from 1 with no repeat, and its reference
/// impedance in ohms (ngspice's default is 50).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RfPort {
    pub num: u32,
    pub z0: f64,
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
    Sin {
        offset: IrNum,
        amplitude: IrNum,
        frequency: IrNum,
        delay: IrNum,
        damping: IrNum,
        phase: IrNum,
    },
    Pulse {
        initial: IrNum,
        pulsed: IrNum,
        delay: IrNum,
        rise_time: IrNum,
        fall_time: IrNum,
        pulse_width: IrNum,
        period: IrNum,
    },
    /// `repeat` is ngspice's `r=`: the time point the sequence repeats from,
    /// forever (`0` repeats all of it; one of the times before the last).
    /// `delay` is `td=`: the whole sequence shifted later. `None` is the
    /// card without them (once, no delay), and JSON leaves it out.
    Pwl {
        values: Vec<(IrNum, IrNum)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repeat: Option<IrNum>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delay: Option<IrNum>,
    },
    Exp {
        initial: IrNum,
        pulsed: IrNum,
        rise_delay: IrNum,
        rise_tau: IrNum,
        fall_delay: IrNum,
        fall_tau: IrNum,
    },
    Sffm {
        offset: IrNum,
        amplitude: IrNum,
        carrier_freq: IrNum,
        modulation_index: IrNum,
        signal_freq: IrNum,
    },
    Am {
        amplitude: IrNum,
        offset: IrNum,
        modulating_freq: IrNum,
        carrier_freq: IrNum,
        delay: IrNum,
    },
    /// ngspice transient noise (manual §4.1.7): Gaussian white noise of rms
    /// `na`, a new sample every `nt` seconds; 1/f noise of exponent `nalpha`
    /// and rms `namp`; a random telegraph signal of amplitude `rtsam` with
    /// mean capture and emission times `rtscapt`, `rtsemt` (0 for none),
    /// added to the source's DC value. ngspice only: no other dialect has a source form.
    TrNoise {
        na: IrNum,
        nt: IrNum,
        nalpha: IrNum,
        namp: IrNum,
        rtsam: IrNum,
        rtscapt: IrNum,
        rtsemt: IrNum,
    },
    /// ngspice random source (manual §4.1.8): a `kind` draw held for `ts`
    /// seconds, starting after `td` (the output is `p2` until then), added
    /// to the source's DC value as TRNOISE is. `p1`,
    /// `p2`: range and offset (uniform, values within `p2 ± p1`), standard
    /// deviation and mean (Gaussian), mean and offset (exponential), lambda
    /// and offset (Poisson). ngspice only.
    TrRandom {
        kind: RandomKind,
        ts: IrNum,
        td: IrNum,
        p1: IrNum,
        p2: IrNum,
    },
}

/// TRRANDOM's distribution, its `TYPE` number in the deck and in JSON;
/// any other number fails to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
#[repr(u8)]
pub enum RandomKind {
    Uniform = 1,
    Gaussian = 2,
    Exponential = 3,
    Poisson = 4,
}

impl RandomKind {
    /// The four kinds, in `TYPE` order.
    pub const ALL: [RandomKind; 4] = [
        RandomKind::Uniform,
        RandomKind::Gaussian,
        RandomKind::Exponential,
        RandomKind::Poisson,
    ];

    /// The lowercase name (`uniform`, ...), as Python spells it.
    pub fn name(self) -> &'static str {
        match self {
            RandomKind::Uniform => "uniform",
            RandomKind::Gaussian => "gaussian",
            RandomKind::Exponential => "exponential",
            RandomKind::Poisson => "poisson",
        }
    }
}

impl TryFrom<u8> for RandomKind {
    type Error = String;

    fn try_from(t: u8) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|k| *k as u8 == t)
            .ok_or_else(|| format!("unknown TRRANDOM type {t} (expected 1 to 4)"))
    }
}

impl From<RandomKind> for u8 {
    fn from(k: RandomKind) -> u8 {
        k as u8
    }
}

impl std::str::FromStr for RandomKind {
    type Err = String;

    /// Reads a [`RandomKind::name`] in any case.
    fn from_str(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|k| k.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| {
                format!(
                    "unknown random type '{s}' (expected uniform, gaussian, exponential or poisson)"
                )
            })
    }
}

impl IrWaveform {
    /// Every parameter, in field order.
    pub fn nums(&self) -> Vec<&IrNum> {
        use IrWaveform::*;
        match self {
            Sin {
                offset,
                amplitude,
                frequency,
                delay,
                damping,
                phase,
            } => vec![offset, amplitude, frequency, delay, damping, phase],
            Pulse {
                initial,
                pulsed,
                delay,
                rise_time,
                fall_time,
                pulse_width,
                period,
            } => vec![
                initial,
                pulsed,
                delay,
                rise_time,
                fall_time,
                pulse_width,
                period,
            ],
            Pwl {
                values,
                repeat,
                delay,
            } => values
                .iter()
                .flat_map(|(t, v)| [t, v])
                .chain(repeat)
                .chain(delay)
                .collect(),
            Exp {
                initial,
                pulsed,
                rise_delay,
                rise_tau,
                fall_delay,
                fall_tau,
            } => vec![initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau],
            Sffm {
                offset,
                amplitude,
                carrier_freq,
                modulation_index,
                signal_freq,
            } => vec![
                offset,
                amplitude,
                carrier_freq,
                modulation_index,
                signal_freq,
            ],
            Am {
                amplitude,
                offset,
                modulating_freq,
                carrier_freq,
                delay,
            } => vec![amplitude, offset, modulating_freq, carrier_freq, delay],
            TrNoise {
                na,
                nt,
                nalpha,
                namp,
                rtsam,
                rtscapt,
                rtsemt,
            } => vec![na, nt, nalpha, namp, rtsam, rtscapt, rtsemt],
            TrRandom { ts, td, p1, p2, .. } => vec![ts, td, p1, p2],
        }
    }

    /// [`Self::nums`], mutable: for passes over the expressions (renaming,
    /// case folding).
    pub fn nums_mut(&mut self) -> Vec<&mut IrNum> {
        use IrWaveform::*;
        match self {
            Sin {
                offset,
                amplitude,
                frequency,
                delay,
                damping,
                phase,
            } => vec![offset, amplitude, frequency, delay, damping, phase],
            Pulse {
                initial,
                pulsed,
                delay,
                rise_time,
                fall_time,
                pulse_width,
                period,
            } => vec![
                initial,
                pulsed,
                delay,
                rise_time,
                fall_time,
                pulse_width,
                period,
            ],
            Pwl {
                values,
                repeat,
                delay,
            } => values
                .iter_mut()
                .flat_map(|(t, v)| [t, v])
                .chain(repeat)
                .chain(delay)
                .collect(),
            Exp {
                initial,
                pulsed,
                rise_delay,
                rise_tau,
                fall_delay,
                fall_tau,
            } => vec![initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau],
            Sffm {
                offset,
                amplitude,
                carrier_freq,
                modulation_index,
                signal_freq,
            } => vec![
                offset,
                amplitude,
                carrier_freq,
                modulation_index,
                signal_freq,
            ],
            Am {
                amplitude,
                offset,
                modulating_freq,
                carrier_freq,
                delay,
            } => vec![amplitude, offset, modulating_freq, carrier_freq, delay],
            TrNoise {
                na,
                nt,
                nalpha,
                namp,
                rtsam,
                rtscapt,
                rtsemt,
            } => vec![na, nt, nalpha, namp, rtsam, rtscapt, rtsemt],
            TrRandom { ts, td, p1, p2, .. } => vec![ts, td, p1, p2],
        }
    }

    /// The expression parameters.
    pub fn exprs_ref(&self) -> Vec<&str> {
        self.nums()
            .into_iter()
            .filter_map(|n| match n {
                IrNum::Expr(e) => Some(e.as_str()),
                IrNum::Num(_) => None,
            })
            .collect()
    }

    /// The Spectre `pwlperiod` that repeats PWL `values` exactly as `repeat`
    /// 0 (ngspice `r=0`) does, when one does: numeric points from t = 0
    /// whose last value equals the first, the last segment at least the
    /// default `twidth` (period/1000) long. Spectre ramps over `twidth` from
    /// the value at period - twidth to the first value before each repeat
    /// (Spectre Reference 5.0, `vsource` #17, #18, p.686); under these
    /// conditions that ramp is the last segment itself. `None` otherwise.
    pub fn spectre_pwl_period(values: &[(IrNum, IrNum)]) -> Option<f64> {
        let ([(t0, v0), ..], [.., (tp, _), (tn, vn)]) = (values, values) else {
            return None;
        };
        let (t0, v0, tp, tn, vn) = (t0.num()?, v0.num()?, tp.num()?, tn.num()?, vn.num()?);
        (t0 == 0.0 && vn == v0 && tn > 0.0 && tn - tp >= tn / 1000.0).then_some(tn)
    }

    /// The DC value SPICE takes for a card that gives none: the value at
    /// t=0 (AM starts at 0), and 0 for TRNOISE and TRRANDOM, which add to it.
    pub fn at_zero(&self) -> IrValue {
        use IrWaveform::*;
        match self {
            Sin { offset, .. } | Sffm { offset, .. } => offset.into(),
            Pulse { initial, .. } | Exp { initial, .. } => initial.into(),
            Pwl { values, .. } => values
                .first()
                .map_or(IrValue::Numeric { value: 0.0 }, |p| (&p.1).into()),
            // TRNOISE and TRRANDOM add to the card's DC value instead of
            // replacing it (ngspice 45: `V1 a 0 3 TRRANDOM(1 1u 10u 2 3)`
            // holds 6 V before TD), so a card with none has DC 0.
            Am { .. } | TrNoise { .. } | TrRandom { .. } => IrValue::Numeric { value: 0.0 },
        }
    }
}

// ── Testbench ──

/// Testbench -- wraps a DUT with stimulus + analysis. The DUT is
/// [`CircuitIR::top`]; JSON written before 0.1.3 also carries a `dut` name,
/// which is ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Testbench {
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
    /// ngspice's compatibility mode (`hsa`, `ps`, `lt`, ...), which it reads
    /// only before parsing the deck. The ngspice backend passes it as
    /// `-D ngbehavior=<mode>`; every other dialect ignores it, since it says
    /// how ngspice reads text, not what the circuit is.
    #[serde(default)]
    pub ngbehavior: Option<String>,
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
    /// `lin`, `dec`, `oct` (or Spectre's `log`); `None` is linear.
    pub sweep_type: Option<Sweep>,
}

// ── Analysis types ──

/// Analysis types -- all supported analyses as a tagged union
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Analysis {
    Op,
    Dc {
        sweeps: Vec<DcSweep>,
    },
    Ac {
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    Transient {
        step: f64,
        stop: f64,
        start: Option<f64>,
        max_step: Option<f64>,
        uic: bool,
    },
    Noise {
        output: String,
        reference: String,
        source: String,
        #[serde(flatten)]
        sweep: FreqSweep,
        points_per_summary: Option<u32>,
    },
    Tf {
        output: String,
        source: String,
    },
    Sensitivity {
        output: String,
        ac: Option<FreqSweep>,
    },
    PoleZero {
        node1: String,
        node2: String,
        node3: String,
        node4: String,
        tf_type: PzTransfer,
        pz_type: PzKind,
    },
    Distortion {
        #[serde(flatten)]
        sweep: FreqSweep,
        f2overf1: Option<f64>,
    },
    /// `sc_iter` (shooting iterations, ngspice's default 50), `steady_coeff`
    /// (the convergence weight, default 1e-3) and `uic` are ngspice's
    /// `.pss … sciter steadycoeff <uic>` (manual §11.3.12); JSON leaves them
    /// out when unset.
    Pss {
        fundamental: f64,
        stabilization: f64,
        observe_node: String,
        points_per_period: u32,
        harmonics: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sc_iter: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steady_coeff: Option<f64>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        uic: bool,
    },
    HarmonicBalance {
        frequencies: Vec<f64>,
        harmonics: Vec<u32>,
    },
    SPar {
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    Stability {
        probe: String,
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    TransientNoise {
        step: f64,
        stop: f64,
    },
    Fourier {
        fundamental: f64,
        outputs: Vec<String>,
        num_harmonics: Option<u32>,
    },
    // Vendor-specific
    SpectreSweep {
        param: String,
        start: f64,
        stop: f64,
        step: f64,
        inner: String,
        inner_type: SpectreAnalysis,
    },
    SpectreMonteCarlo {
        iterations: u32,
        inner: String,
        inner_type: SpectreAnalysis,
        seed: Option<u64>,
    },
    SpectrePac {
        #[serde(flatten)]
        pss: PssBase,
        #[serde(flatten)]
        sweep: FreqSweep,
        sweep_type: String,
    },
    SpectrePnoise {
        #[serde(flatten)]
        pss: PssBase,
        output: String,
        reference: String,
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    SpectrePxf {
        #[serde(flatten)]
        pss: PssBase,
        output: String,
        source: String,
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    SpectrePstb {
        #[serde(flatten)]
        pss: PssBase,
        probe: String,
        #[serde(flatten)]
        sweep: FreqSweep,
    },
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

/// A frequency sweep. Flattened into each analysis that carries one, so the
/// JSON keeps the four fields `variation`, `points`, `start`, `stop` inline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FreqSweep {
    pub variation: Sweep,
    /// Per decade or octave, or in total for [`Sweep::Lin`].
    pub points: u32,
    pub start: f64,
    pub stop: f64,
}

/// A closed set of deck keywords: `name()` is the keyword, `FromStr` and
/// JSON read it in any case (anything else fails, naming the choices), and
/// JSON writes it lowercase, so the text is parsed once, where it enters.
macro_rules! keywords {
    ($(#[$m:meta])* $name:ident, $what:literal { $($(#[$vm:meta])* $v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "&'static str")]
        pub enum $name { $($(#[$vm])* $v),+ }

        impl $name {
            /// Every keyword, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$v),+];

            /// The keyword as a deck and JSON spell it.
            pub fn name(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            /// Reads a [`Self::name`] in any case.
            fn from_str(s: &str) -> Result<Self, String> {
                Self::ALL.iter().copied().find(|k| k.name().eq_ignore_ascii_case(s)).ok_or_else(|| {
                    let names: Vec<&str> = Self::ALL.iter().map(|k| k.name()).collect();
                    let (last, rest) = names.split_last().expect("a keyword set is never empty");
                    format!("unknown {} '{s}' (expected {} or {last})", $what, rest.join(", "))
                })
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(s: String) -> Result<Self, String> {
                s.parse()
            }
        }

        impl From<$name> for &'static str {
            fn from(k: $name) -> Self {
                k.name()
            }
        }
    };
}

keywords! {
    /// How a sweep spaces its points: a frequency sweep, or a `.step`.
    Sweep, "sweep" {
        Dec = "dec",
        Oct = "oct",
        Lin = "lin",
        /// Spectre's `log=N`: N points in total, log-spaced.
        Log = "log",
    }
}

keywords! {
    /// What a `.pz` transfer function relates (ngspice manual §11.3.6):
    /// output over input voltage, or output voltage over input current.
    PzTransfer, "transfer type" {
        Vol = "vol",
        Cur = "cur",
    }
}

keywords! {
    /// What a `.pz` finds: poles, zeros, or both.
    PzKind, "pole-zero type" {
        Pol = "pol",
        Zer = "zer",
        Pz = "pz",
    }
}

keywords! {
    /// The analysis a Spectre `sweep` or `montecarlo` runs inside it, as
    /// its statement keyword (`[REF]` analysis statements).
    SpectreAnalysis, "Spectre analysis" {
        Dc = "dc",
        Ac = "ac",
        Tran = "tran",
        Noise = "noise",
        Xf = "xf",
        Sp = "sp",
        Stb = "stb",
        Pss = "pss",
        Pac = "pac",
        Pnoise = "pnoise",
        Pxf = "pxf",
        Pstb = "pstb",
        Hb = "hb",
    }
}

/// The periodic steady state a SpectreRF small-signal analysis linearizes
/// around (`pac`, `pnoise`, `pxf`, `pstb`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PssBase {
    pub pss_fundamental: f64,
    pub pss_stabilization: f64,
    pub pss_harmonics: u32,
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
/// (`Dialect::capabilities`, in `backend`, is the same mask: what a simulator does).
pub mod feature {
    pub const XSPICE: u8 = 1 << 0;
    pub const OSDI: u8 = 1 << 1;
    pub const MEASURES: u8 = 1 << 2;
    pub const STEP_PARAMS: u8 = 1 << 3;
    pub const CONTROL_BLOCKS: u8 = 1 << 4;
    pub const LAPLACE: u8 = 1 << 5;
    pub const VERILOG_COSIM: u8 = 1 << 6;
    /// Every bit above.
    pub const ALL: u8 =
        XSPICE | OSDI | MEASURES | STEP_PARAMS | CONTROL_BLOCKS | LAPLACE | VERILOG_COSIM;
}

// Seven features, one distinct bit each, inside the `u8`.
const _: () = assert!(feature::ALL.count_ones() == 7);

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
    (
        feature::XSPICE,
        IssueSeverity::Error,
        "Backend '{}' does not support XSPICE A-elements (only ngspice does)",
    ),
    (
        feature::OSDI,
        IssueSeverity::Error,
        "Backend '{}' does not support OSDI/Verilog-A model loading",
    ),
    (
        feature::MEASURES,
        IssueSeverity::Error,
        "Backend '{}' does not support .meas directives",
    ),
    (
        feature::STEP_PARAMS,
        IssueSeverity::Error,
        "Backend '{}' does not support .step param sweeps",
    ),
    (
        feature::CONTROL_BLOCKS,
        IssueSeverity::Error,
        "Backend '{}' does not support .control blocks (ngspice only)",
    ),
    (
        feature::LAPLACE,
        IssueSeverity::Warning,
        "Backend '{}' does not support Laplace-domain B-sources",
    ),
    (
        feature::VERILOG_COSIM,
        IssueSeverity::Error,
        "Backend '{}' does not support Verilog co-simulation",
    ),
];

// Exactly one `MISSING` row per feature bit, so a backend `select` refuses
// for a feature always has the issue `check_backend` names for it.
#[cfg(feature = "backend")]
const _: () = {
    let (mut seen, mut i) = (0u8, 0);
    while i < MISSING.len() {
        assert!(seen & MISSING[i].0 == 0, "a feature has two MISSING rows");
        assert!(
            MISSING[i].0.count_ones() == 1,
            "a MISSING row names one feature"
        );
        seen |= MISSING[i].0;
        i += 1;
    }
    assert!(seen == feature::ALL, "a feature has no MISSING row");
};

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
    /// Calls `f` on every identifier, for renaming: definition names (the
    /// top's is a title and is skipped), ports, model names, instance names,
    /// the subcircuits they call and their port mappings, every
    /// [`Component::names_mut`] in the bodies and the stimulus, and the
    /// globals. Expressions, saves and analyses are not visited.
    pub fn names_mut(&mut self, mut f: impl FnMut(&mut String)) {
        self.subcircuit_defs
            .iter_mut()
            .for_each(|sc| f(&mut sc.name));
        for sc in std::iter::once(&mut self.top).chain(&mut self.subcircuit_defs) {
            sc.ports.iter_mut().for_each(|p| f(&mut p.name));
            sc.models.iter_mut().for_each(|m| f(&mut m.name));
            for i in &mut sc.instances {
                f(&mut i.name);
                f(&mut i.subcircuit);
                i.port_mapping.iter_mut().for_each(&mut f);
            }
            sc.components
                .iter_mut()
                .flat_map(Component::names_mut)
                .for_each(&mut f);
        }
        self.testbench
            .iter_mut()
            .flat_map(|tb| &mut tb.stimulus)
            .flat_map(Component::names_mut)
            .for_each(&mut f);
        self.globals.iter_mut().for_each(f);
    }

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

        if sub
            .verilog_blocks
            .iter()
            .any(|vb| matches!(vb.mode, VerilogMode::Simulate))
        {
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
                    if crate::codegen::laplace::is_laplace(expression) =>
                {
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

        let kind = match crate::backend::parse(backend) {
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
                issues.push(Issue {
                    severity,
                    message: message.replace("{}", backend),
                });
            }
        }

        if features.element_count > 10_000 && kind == Dialect::Ngspice {
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
mod schema_check;

#[cfg(test)]
mod tests {
    use super::*;

    /// A deck whose top is `top`, with no testbench.
    fn deck(top: Subcircuit) -> CircuitIR {
        CircuitIR {
            top,
            ..Default::default()
        }
    }

    fn resistor() -> Component {
        Component::Resistor {
            name: "1".into(),
            n1: "out".into(),
            n2: "0".into(),
            value: IrValue::Numeric { value: 1000.0 },
            model: None,
            params: vec![],
        }
    }

    /// The JSON `"type"` tag of `x` is its variant's name.
    fn assert_tagged<T: Serialize + std::fmt::Debug>(x: &T) {
        let name = format!("{x:?}");
        assert_eq!(
            serde_json::to_value(x).unwrap()["type"],
            name.split([' ', '{']).next().unwrap(),
            "{name}"
        );
    }

    #[test]
    fn test_compute_features_xspice() {
        let xspice = Component::Xspice {
            name: "1".into(),
            connections: vec!["in".into(), "out".into()],
            model: "d_and".into(),
        };
        let features = deck(Subcircuit {
            components: vec![xspice, resistor()],
            ..Default::default()
        })
        .compute_features();
        assert!(features.has(feature::XSPICE));
        assert!(!features.has(feature::OSDI));
        assert!(!features.has(feature::MEASURES));
        assert!(!features.has(feature::LAPLACE));
        assert_eq!(features.element_count, 2);
    }

    #[test]
    fn test_compute_features_osdi_and_laplace() {
        let b = Component::BehavioralVoltage {
            name: "1".into(),
            np: "out".into(),
            nm: "0".into(),
            expression: "laplace(V(in), {1/(1+s*1e-6)})".into(),
        };
        let features = deck(Subcircuit {
            components: vec![b],
            osdi_loads: vec!["model.osdi".into()],
            ..Default::default()
        })
        .compute_features();
        assert!(!features.has(feature::XSPICE));
        assert!(features.has(feature::OSDI));
        assert!(features.has(feature::LAPLACE));
        assert_eq!(features.element_count, 1);
    }

    #[test]
    fn test_compute_features_control_blocks() {
        let raw_spice = vec![".control".into(), "run".into(), ".endc".into()];
        assert!(
            deck(Subcircuit {
                raw_spice,
                ..Default::default()
            })
            .compute_features()
            .has(feature::CONTROL_BLOCKS)
        );
    }

    #[test]
    fn test_compute_features_measures_and_step() {
        let step = StepParam {
            param: "R1".into(),
            start: 100.0,
            stop: 10000.0,
            step: 100.0,
            sweep_type: None,
        };
        let tb = Testbench {
            measures: vec!["tran v_max MAX V(out)".into()],
            step_params: vec![step],
            ..Default::default()
        };
        let features = CircuitIR {
            testbench: Some(tb),
            ..Default::default()
        }
        .compute_features();
        assert!(features.has(feature::MEASURES));
        assert!(features.has(feature::STEP_PARAMS));
    }

    #[cfg(feature = "backend")]
    #[test]
    fn test_check_backend_osdi_against_ltspice() {
        let ir = deck(Subcircuit {
            osdi_loads: vec!["model.osdi".into()],
            ..Default::default()
        });
        let issues = ir.check_backend("ltspice");
        assert!(
            issues
                .iter()
                .any(|i| i.severity == IssueSeverity::Error && i.message.contains("OSDI"))
        );
        // Spectre supports OSDI
        assert!(ir.check_backend("spectre").is_empty());
    }

    /// For every feature bit, a deck that needs it: `check_backend` and the
    /// `select` predicate (`backend::runs`) agree on every runnable dialect.
    #[cfg(feature = "backend")]
    #[test]
    fn check_backend_and_select_agree_on_every_feature() {
        let bench = |tb: Testbench| CircuitIR {
            testbench: Some(tb),
            ..Default::default()
        };
        let laplace = Component::BehavioralVoltage {
            name: "1".into(),
            np: "out".into(),
            nm: "0".into(),
            expression: "laplace(V(in), {1/(1+s*1e-6)})".into(),
        };
        let cosim = VerilogBlock {
            source: "m.v".into(),
            mode: VerilogMode::Simulate,
            instance_name: "u1".into(),
            connections: BTreeMap::new(),
            pdk: None,
            liberty: None,
            spice_models: None,
        };
        let decks = [
            (
                feature::XSPICE,
                deck(Subcircuit {
                    components: vec![Component::Xspice {
                        name: "1".into(),
                        connections: vec!["a".into()],
                        model: "m".into(),
                    }],
                    ..Default::default()
                }),
            ),
            (
                feature::OSDI,
                deck(Subcircuit {
                    osdi_loads: vec!["m.osdi".into()],
                    ..Default::default()
                }),
            ),
            (
                feature::MEASURES,
                bench(Testbench {
                    measures: vec!["tran x max v(a)".into()],
                    ..Default::default()
                }),
            ),
            (
                feature::STEP_PARAMS,
                bench(Testbench {
                    step_params: vec![StepParam {
                        param: "r".into(),
                        start: 1.0,
                        stop: 2.0,
                        step: 1.0,
                        sweep_type: None,
                    }],
                    ..Default::default()
                }),
            ),
            (
                feature::CONTROL_BLOCKS,
                deck(Subcircuit {
                    raw_spice: vec![".control".into(), ".endc".into()],
                    ..Default::default()
                }),
            ),
            (
                feature::LAPLACE,
                deck(Subcircuit {
                    components: vec![laplace],
                    ..Default::default()
                }),
            ),
            (
                feature::VERILOG_COSIM,
                deck(Subcircuit {
                    verilog_blocks: vec![cosim],
                    ..Default::default()
                }),
            ),
        ];
        assert_eq!(
            decks.iter().fold(0, |m, (f, _)| m | f),
            feature::ALL,
            "one deck per feature"
        );
        for (f, ir) in &decks {
            assert_eq!(ir.compute_features().needs, *f, "deck for bit {f:#04b}");
            for d in crate::backend::RUNNABLE {
                let issues = ir.check_backend(d.name());
                assert_eq!(
                    issues.is_empty(),
                    crate::backend::runs(d, *f),
                    "{} on bit {f:#04b}: {issues:?}",
                    d.name()
                );
            }
        }
    }

    #[cfg(feature = "backend")]
    #[test]
    fn test_check_backend_no_issues_for_clean_circuit() {
        let ir = deck(Subcircuit {
            components: vec![resistor()],
            ..Default::default()
        });
        for backend in &["ngspice", "ltspice", "vacask", "spectre"] {
            assert!(
                ir.check_backend(backend).is_empty(),
                "Expected no issues for {}",
                backend
            );
        }
    }

    // ── JSON serialization tests ──

    /// `ir` as JSON breaks nothing in `schema/circuit-ir.schema.json`.
    fn assert_schema(ir: &CircuitIR) {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schema/circuit-ir.schema.json")).unwrap();
        let errs = super::schema_check::errors(&schema, &serde_json::to_value(ir).unwrap());
        assert!(
            errs.is_empty(),
            "{} schema errors:\n{}",
            errs.len(),
            errs.join("\n")
        );
    }

    /// Every field an IR can hold, set (a PWL's repeat/delay as numbers and
    /// expressions, every port direction and Verilog form), the shipped
    /// example, and what the importers make of a deck per dialect: the JSON
    /// the schema promises Schemify. Regression: the schema had no R/C/L
    /// `model`, no `verilog_blocks` and an `IrNum` that was only a number.
    #[test]
    fn every_ir_field_and_imported_deck_matches_the_schema() {
        let s = |x: &str| x.to_string();
        let kv = || vec![(s("k"), s("v"))];
        let x = |e: &str| IrNum::Expr(e.into());
        let pwl = |repeat, delay| IrWaveform::Pwl {
            values: vec![(0.0.into(), x("v0")), (x("t1"), 1.0.into())],
            repeat,
            delay,
        };
        let src = |name: &str, waveform| Component::VoltageSource {
            name: s(name),
            np: s("a"),
            nm: s("0"),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0),
            ac_phase: Some(90.0),
            waveform: Some(waveform),
            port: Some(RfPort { num: 1, z0: 50.0 }),
        };
        let rcl = |value: IrValue| {
            [
                Component::Resistor {
                    name: s("1"),
                    n1: s("a"),
                    n2: s("b"),
                    value: value.clone(),
                    model: Some(s("rm")),
                    params: kv(),
                },
                Component::Capacitor {
                    name: s("1"),
                    n1: s("a"),
                    n2: s("b"),
                    value: value.clone(),
                    model: Some(s("cm")),
                    params: kv(),
                },
                Component::Inductor {
                    name: s("1"),
                    n1: s("a"),
                    n2: s("b"),
                    value,
                    model: Some(s("lm")),
                    params: kv(),
                },
            ]
        };
        let connections = [
            (s("a"), VerilogConnection::Single(s("n1"))),
            (s("b"), VerilogConnection::Bus(vec![s("d0"), s("d1")])),
        ]
        .into();
        let verilog = |mode, opt: Option<String>| VerilogBlock {
            source: s("module m; endmodule"),
            mode,
            instance_name: s("u1"),
            connections: BTreeMap::clone(&connections),
            pdk: opt.clone(),
            liberty: opt.clone(),
            spice_models: opt,
        };
        let full = Subcircuit {
            name: s("full"),
            ports: [
                PortDirection::InOut,
                PortDirection::Input,
                PortDirection::Output,
            ]
            .into_iter()
            .map(|direction| Port {
                name: s("p"),
                direction,
            })
            .collect(),
            parameters: vec![
                ParamDef {
                    name: s("w"),
                    default: Some(s("1u")),
                },
                ParamDef {
                    name: s("l"),
                    default: None,
                },
            ],
            components: rcl(IrValue::Raw {
                text: String::new(),
            })
            .into_iter()
            .chain([
                src("1", pwl(Some(0.0.into()), Some(x("td")))),
                src("2", pwl(Some(x("r")), Some(1e-3.into()))),
            ])
            .collect(),
            instances: vec![Instance {
                name: s("x1"),
                subcircuit: s("full"),
                port_mapping: vec![s("a")],
                parameters: kv(),
                at: Some(0),
            }],
            models: vec![ModelDef {
                name: s("rm"),
                kind: s("r"),
                parameters: kv(),
            }],
            raw_spice: vec![s(".probe v(a)")],
            includes: vec![s("a.inc")],
            libs: vec![(s("a.lib"), s("tt"))],
            osdi_loads: vec![s("a.osdi")],
            verilog_blocks: vec![
                verilog(VerilogMode::Simulate, None),
                verilog(VerilogMode::Synthesize, Some(s("x"))),
            ],
        };
        let tb = Testbench {
            stimulus: rcl(IrValue::Expression { expr: s("r") }).into(),
            analyses: vec![Analysis::Op],
            options: SimOptions {
                portable: kv(),
                backend_specific: [(s("xyce"), kv())].into(),
            },
            saves: vec![s("v(a)")],
            measures: vec![s("tran m max v(a)")],
            temperature: Some(27.0),
            nominal_temperature: Some(25.0),
            initial_conditions: vec![(s("a"), 1.0)],
            node_sets: vec![(s("a"), 0.5)],
            step_params: vec![
                StepParam {
                    param: s("r"),
                    start: 1.0,
                    stop: 2.0,
                    step: 1.0,
                    sweep_type: Some(Sweep::Lin),
                },
                StepParam {
                    param: s("c"),
                    start: 1.0,
                    stop: 2.0,
                    step: 1.0,
                    sweep_type: None,
                },
            ],
            extra_lines: vec![s(".option x")],
            prints: vec![PrintCard {
                analysis: s("tran"),
                outputs: vec![s("v(a)")],
            }],
            ngbehavior: Some(s("hsa")),
        };
        let library = ModelLibrary {
            name: s("sky"),
            path: s("sky.lib"),
            corner: Some(s("tt")),
            backend_paths: [(s("vacask"), s("sky.inc"))].into(),
            setup_includes: vec![s("a.inc")],
        };
        let ir = CircuitIR {
            top: full.clone(),
            testbench: Some(tb),
            subcircuit_defs: vec![full, Subcircuit::default()],
            model_libraries: vec![library],
            raw_dialect: Some(Dialect::Ngspice),
            globals: vec![s("vdd")],
            functions: vec![FuncDef {
                name: s("f"),
                args: vec![s("x")],
                body: s("x*2"),
            }],
        };
        assert_schema(&ir);
        assert_schema(&CircuitIR {
            testbench: None,
            ..ir
        });
        assert_schema(
            &serde_json::from_str(include_str!(
                "../../schema/examples/voltage_divider.circuit.json"
            ))
            .unwrap(),
        );
        for (d, deck) in [
            (
                Dialect::Ngspice,
                "t\n.param w=1u\n.func f(x) {x*2}\n.global vdd\n.model rm r\nR1 a b rm l=1u\nV1 a 0 pwl(0 0 1m {w}) r=0 td=1m\n.probe v(a)\n.tran 1u 1m\n.end\n",
            ),
            (
                Dialect::Spectre,
                "// t\nsimulator lang=spectre\nv1 (a 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=2m\nr1 (a 0) resistor r=1k\ntr tran step=1u stop=1m\n",
            ),
            (
                Dialect::Vacask,
                "t\nload \"spice/resistor.osdi\"\nmodel r sp_resistor\nr1 (a 0) r r=1k\ncontrol\n  analysis op1 op\nendc\n",
            ),
        ] {
            assert_schema(&crate::import::parse(deck, "t", d).unwrap().0);
        }
    }

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

    /// The flat `variation`/`points`/`start`/`stop` and `pss_*` fields that
    /// `FreqSweep` and `PssBase` now hold. Every line is the JSON the string
    /// `variation` code wrote (0.1.2, taken from the test suite's own decks);
    /// it loads, and writes back byte for byte.
    #[test]
    fn sweep_json_from_0_1_2_round_trips_byte_for_byte() {
        let old = [
            r#"{"type":"Ac","variation":"dec","points":50,"start":1.0,"stop":10000000000.0}"#,
            r#"{"type":"Distortion","variation":"dec","points":10,"start":1.0,"stop":1000.0,"f2overf1":null}"#,
            r#"{"type":"Noise","output":"p2","reference":"0","source":"Vp1","variation":"dec","points":50,"start":1000000.0,"stop":10000000000.0,"points_per_summary":null}"#,
            r#"{"type":"SPar","variation":"dec","points":20,"start":1000000.0,"stop":1000000000.0}"#,
            r#"{"type":"Sensitivity","output":"out","ac":{"variation":"dec","points":10,"start":1.0,"stop":1000000.0}}"#,
            r#"{"type":"SpectrePac","pss_fundamental":1000000000.0,"pss_stabilization":1e-7,"pss_harmonics":10,"variation":"dec","points":100,"start":1.0,"stop":1000000000.0,"sweep_type":"relative"}"#,
            r#"{"type":"SpectrePnoise","pss_fundamental":1000000000.0,"pss_stabilization":1e-7,"pss_harmonics":10,"output":"out","reference":"0","variation":"dec","points":10,"start":1.0,"stop":1000000.0}"#,
            r#"{"type":"SpectrePstb","pss_fundamental":1000000000.0,"pss_stabilization":1e-7,"pss_harmonics":10,"probe":"IPRB0","variation":"dec","points":10,"start":1.0,"stop":1000000.0}"#,
            r#"{"type":"SpectrePxf","pss_fundamental":1000000000.0,"pss_stabilization":1e-7,"pss_harmonics":10,"output":"out","source":"Vin","variation":"dec","points":10,"start":1.0,"stop":1000000.0}"#,
            r#"{"type":"Stability","probe":"IPRB0","variation":"dec","points":20,"start":1.0,"stop":1000000000.0}"#,
            r#"{"type":"Noise","output":"o","reference":"0","source":"V1","variation":"oct","points":3,"start":1.0,"stop":8.0,"points_per_summary":2}"#,
            r#"{"type":"Ac","variation":"lin","points":2,"start":1000.0,"stop":2000.0}"#,
            r#"{"type":"Ac","variation":"log","points":7,"start":1.0,"stop":1000.0}"#,
        ];
        for s in old {
            let a: Analysis = serde_json::from_str(s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(serde_json::to_string(&a).unwrap(), s);
        }
    }

    /// A PWL written before `repeat`/`delay` existed loads with neither and
    /// writes back byte for byte; a set one is written after the values.
    #[test]
    fn pwl_json_without_repeat_or_delay_round_trips_byte_for_byte() {
        let old = r#"{"type":"Pwl","values":[[0.0,0.0],[0.001,"vdd"]]}"#;
        let w: IrWaveform = serde_json::from_str(old).unwrap();
        assert_eq!(
            w,
            IrWaveform::Pwl {
                values: vec![
                    (0.0.into(), 0.0.into()),
                    (0.001.into(), IrNum::Expr("vdd".into()))
                ],
                repeat: None,
                delay: None
            }
        );
        assert_eq!(serde_json::to_string(&w).unwrap(), old);
        let IrWaveform::Pwl { values, .. } = w else {
            unreachable!()
        };
        let set = IrWaveform::Pwl {
            values,
            repeat: Some(0.0.into()),
            delay: Some(IrNum::Expr("td".into())),
        };
        assert_eq!(
            serde_json::to_string(&set).unwrap(),
            r#"{"type":"Pwl","values":[[0.0,0.0],[0.001,"vdd"]],"repeat":0.0,"delay":"td"}"#
        );
    }

    /// The keyword fields that were strings (`.pz` types, a `.step` sweep,
    /// a Spectre child analysis) load from the JSON the string code wrote
    /// (0.1.2, the test suite's own values) and write it back byte for byte;
    /// a keyword no simulator has fails the load.
    #[test]
    fn keyword_json_from_0_1_2_round_trips_byte_for_byte() {
        let old = [
            r#"{"type":"PoleZero","node1":"in","node2":"0","node3":"out","node4":"0","tf_type":"vol","pz_type":"pz"}"#,
            r#"{"type":"PoleZero","node1":"in","node2":"0","node3":"out","node4":"0","tf_type":"cur","pz_type":"pol"}"#,
            r#"{"type":"PoleZero","node1":"in","node2":"0","node3":"out","node4":"0","tf_type":"vol","pz_type":"zer"}"#,
            r#"{"type":"SpectreSweep","param":"R1","start":1000.0,"stop":10000.0,"step":1000.0,"inner":"dc1","inner_type":"dc"}"#,
            r#"{"type":"SpectreSweep","param":"rload","start":1000.0,"stop":10000.0,"step":1000.0,"inner":"ac1","inner_type":"ac"}"#,
            r#"{"type":"SpectreMonteCarlo","iterations":100,"inner":"tran1","inner_type":"tran","seed":42}"#,
        ];
        for s in old {
            let a: Analysis = serde_json::from_str(s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(serde_json::to_string(&a).unwrap(), s);
        }
        let pz: Analysis = serde_json::from_str(old[1]).unwrap();
        assert!(matches!(
            pz,
            Analysis::PoleZero {
                tf_type: PzTransfer::Cur,
                pz_type: PzKind::Pol,
                ..
            }
        ));
        let mc: Analysis = serde_json::from_str(old[5]).unwrap();
        assert!(matches!(
            mc,
            Analysis::SpectreMonteCarlo {
                inner_type: SpectreAnalysis::Tran,
                ..
            }
        ));
        for s in [
            r#"{"param":"R1","start":100.0,"stop":10000.0,"step":100.0,"sweep_type":"dec"}"#,
            r#"{"param":"R1","start":100.0,"stop":10000.0,"step":100.0,"sweep_type":null}"#,
        ] {
            let sp: StepParam = serde_json::from_str(s).unwrap();
            assert_eq!(serde_json::to_string(&sp).unwrap(), s);
        }
        let err = serde_json::from_str::<Analysis>(&old[0].replace("\"vol\"", "\"volt\""))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("unknown transfer type 'volt' (expected vol or cur)"),
            "{err}"
        );
        let err = serde_json::from_str::<Analysis>(&old[5].replace("\"tran\"", "\"op\""))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown Spectre analysis 'op'"), "{err}");
    }

    /// A `.pss` and a V source written before the shooting controls and RF
    /// ports existed load with them unset and write back byte for byte.
    #[test]
    fn pss_and_source_json_without_the_new_fields_round_trip_byte_for_byte() {
        let pss = r#"{"type":"Pss","fundamental":1000000000.0,"stabilization":1e-7,"observe_node":"out","points_per_period":128,"harmonics":10}"#;
        let a: Analysis = serde_json::from_str(pss).unwrap();
        assert!(matches!(
            a,
            Analysis::Pss {
                sc_iter: None,
                steady_coeff: None,
                uic: false,
                ..
            }
        ));
        assert_eq!(serde_json::to_string(&a).unwrap(), pss);
        let v = r#"{"type":"VoltageSource","name":"1","np":"in","nm":"0","value":{"type":"Numeric","value":0.0},"ac_magnitude":1.0,"ac_phase":null,"waveform":null}"#;
        let c: Component = serde_json::from_str(v).unwrap();
        assert!(matches!(c, Component::VoltageSource { port: None, .. }));
        assert_eq!(serde_json::to_string(&c).unwrap(), v);
        let Component::VoltageSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform,
            ..
        } = c
        else {
            unreachable!()
        };
        let rf = Component::VoltageSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform,
            port: Some(RfPort { num: 1, z0: 50.0 }),
        };
        assert!(
            serde_json::to_string(&rf)
                .unwrap()
                .ends_with(r#""waveform":null,"port":{"num":1,"z0":50.0}}"#)
        );
    }

    /// TRRANDOM's type is its deck number in JSON; one ngspice does not
    /// define fails the load.
    #[test]
    fn trrandom_kind_is_its_type_number() {
        let j = r#"{"type":"TrRandom","kind":2,"ts":1e-6,"td":0.0,"p1":1.0,"p2":0.0}"#;
        let w: IrWaveform = serde_json::from_str(j).unwrap();
        assert!(matches!(
            w,
            IrWaveform::TrRandom {
                kind: RandomKind::Gaussian,
                ..
            }
        ));
        assert_eq!(serde_json::to_string(&w).unwrap(), j);
        let err = serde_json::from_str::<IrWaveform>(&j.replace(":2,", ":7,"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown TRRANDOM type 7"), "{err}");
    }

    /// The sweep name is parsed where JSON is read: any case loads, and a
    /// name no simulator has fails the load instead of reaching a deck.
    #[test]
    fn sweep_names_parse_once_at_load() {
        let ac = |v: &str| {
            serde_json::from_str::<Analysis>(&format!(
                r#"{{"type":"Ac","variation":"{v}","points":5,"start":1.0,"stop":10.0}}"#
            ))
        };
        assert_eq!(
            ac("DEC").unwrap(),
            Analysis::Ac {
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 5,
                    start: 1.0,
                    stop: 10.0
                }
            }
        );
        let err = ac("decade").unwrap_err().to_string();
        assert!(err.contains("unknown sweep 'decade'"), "{err}");
    }

    /// A testbench written before 0.1.3 names its DUT in `dut`; the field is
    /// gone, and such JSON still loads (no `deny_unknown_fields`).
    #[test]
    fn testbench_json_with_dut_still_loads() {
        let old = r#"{"dut":"top","stimulus":[],"analyses":[{"type":"Op"}],"options":{"portable":[],"backend_specific":{}},
            "saves":[],"measures":[],"temperature":null,"nominal_temperature":null,"initial_conditions":[],
            "node_sets":[],"step_params":[],"extra_lines":[]}"#;
        let tb: Testbench = serde_json::from_str(old).unwrap();
        assert_eq!(tb.analyses, vec![Analysis::Op]);
        assert!(!serde_json::to_string(&tb).unwrap().contains("dut"));
    }

    #[test]
    fn test_all_component_variants_roundtrip() {
        // Build an IR containing every Component variant to ensure full roundtrip coverage
        let all_components = vec![
            Component::Resistor {
                name: "1".into(),
                n1: "a".into(),
                n2: "b".into(),
                value: IrValue::Numeric { value: 1e3 },
                model: None,
                params: vec![("tc1".into(), "0.001".into())],
            },
            Component::Capacitor {
                name: "1".into(),
                n1: "a".into(),
                n2: "b".into(),
                value: IrValue::Expression {
                    expr: "cap_val".into(),
                },
                model: None,
                params: vec![],
            },
            Component::Inductor {
                name: "1".into(),
                n1: "a".into(),
                n2: "b".into(),
                value: IrValue::Raw { text: "10u".into() },
                model: None,
                params: vec![],
            },
            Component::MutualInductor {
                name: "1".into(),
                inductor1: "L1".into(),
                inductor2: "L2".into(),
                coupling: 0.95,
            },
            Component::VoltageSource {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                value: IrValue::Numeric { value: 5.0 },
                ac_magnitude: None,
                ac_phase: None,
                waveform: None,
                port: None,
            },
            Component::CurrentSource {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                value: IrValue::Numeric { value: 1e-3 },
                ac_magnitude: None,
                ac_phase: None,
                waveform: Some(IrWaveform::Pulse {
                    initial: 0.0.into(),
                    pulsed: 1e-3.into(),
                    delay: 0.0.into(),
                    rise_time: 1e-9.into(),
                    fall_time: 1e-9.into(),
                    pulse_width: 5e-6.into(),
                    period: 10e-6.into(),
                }),
            },
            Component::BehavioralVoltage {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                expression: "V(x)*2+1".into(),
            },
            Component::BehavioralCurrent {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                expression: "I(V1)*10".into(),
            },
            Component::Vcvs {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                ncp: "c".into(),
                ncm: "d".into(),
                gain: 100.0,
            },
            Component::Vccs {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                ncp: "c".into(),
                ncm: "d".into(),
                transconductance: 0.01,
            },
            Component::Cccs {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                vsense: "Vs".into(),
                gain: 50.0,
            },
            Component::Ccvs {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                vsense: "Vs".into(),
                transresistance: 500.0,
            },
            Component::Diode {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                model: "D1".into(),
                params: vec![],
            },
            Component::Bjt {
                name: "1".into(),
                nc: "c".into(),
                nb: "b".into(),
                ne: "e".into(),
                extra_nodes: vec![],
                model: "Q1".into(),
                params: vec![("area".into(), "2".into())],
            },
            Component::Mosfet {
                name: "1".into(),
                nd: "d".into(),
                ng: "g".into(),
                ns: "s".into(),
                nb: Some("b".into()),
                extra_nodes: vec![],
                model: "M1".into(),
                params: vec![("W".into(), "1u".into()), ("L".into(), "180n".into())],
            },
            Component::Jfet {
                name: "1".into(),
                nd: "d".into(),
                ng: "g".into(),
                ns: "s".into(),
                model: "J1".into(),
                params: vec![],
            },
            Component::Mesfet {
                name: "1".into(),
                nd: "d".into(),
                ng: "g".into(),
                ns: "s".into(),
                model: "Z1".into(),
                params: vec![],
            },
            Component::VSwitch {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                ncp: "c".into(),
                ncm: "d".into(),
                model: "SW".into(),
            },
            Component::ISwitch {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                vcontrol: "Vs".into(),
                model: "CSW".into(),
            },
            Component::TLine {
                name: "1".into(),
                inp: "i1".into(),
                inm: "i2".into(),
                outp: "o1".into(),
                outm: "o2".into(),
                z0: 50.0,
                td: 1e-9,
            },
            Component::Xspice {
                name: "1".into(),
                connections: vec!["[din]".into(), "[dout]".into()],
                model: "d_buf".into(),
            },
            Component::Poly {
                name: "1".into(),
                letter: 'E',
                np: "a".into(),
                nm: "b".into(),
                controls: vec!["c".into(), "d".into()],
                coeffs: vec![0.0, 2.0],
            },
            Component::RawSpice {
                line: ".probe V(out)".into(),
            },
        ];
        let x = || IrNum::Expr("vdd/2".into());
        let waveforms = [
            IrWaveform::Sin {
                offset: x(),
                amplitude: 1.0.into(),
                frequency: 1e3.into(),
                delay: 0.0.into(),
                damping: 0.0.into(),
                phase: 90.0.into(),
            },
            IrWaveform::Pulse {
                initial: 0.0.into(),
                pulsed: x(),
                delay: 0.0.into(),
                rise_time: 1e-9.into(),
                fall_time: 1e-9.into(),
                pulse_width: 5e-6.into(),
                period: 10e-6.into(),
            },
            IrWaveform::Pwl {
                values: vec![(0.0.into(), 0.0.into()), (1e-6.into(), x())],
                repeat: None,
                delay: None,
            },
            IrWaveform::Exp {
                initial: 0.0.into(),
                pulsed: 1.0.into(),
                rise_delay: x(),
                rise_tau: 1e-9.into(),
                fall_delay: 1e-6.into(),
                fall_tau: 1e-9.into(),
            },
            IrWaveform::Sffm {
                offset: 0.0.into(),
                amplitude: 1.0.into(),
                carrier_freq: 1e6.into(),
                modulation_index: x(),
                signal_freq: 1e3.into(),
            },
            IrWaveform::Am {
                amplitude: 1.0.into(),
                offset: 0.0.into(),
                modulating_freq: 1e3.into(),
                carrier_freq: 1e6.into(),
                delay: x(),
            },
            IrWaveform::TrNoise {
                na: x(),
                nt: 1e-9.into(),
                nalpha: 1.0.into(),
                namp: 1e-3.into(),
                rtsam: 0.0.into(),
                rtscapt: 0.0.into(),
                rtsemt: 0.0.into(),
            },
            IrWaveform::TrRandom {
                kind: RandomKind::Exponential,
                ts: 1e-6.into(),
                td: 0.0.into(),
                p1: x(),
                p2: 0.5.into(),
            },
        ];
        let sources = waveforms.into_iter().map(|w| Component::VoltageSource {
            name: "w".into(),
            np: "a".into(),
            nm: "0".into(),
            value: w.at_zero(),
            ac_magnitude: Some(1.0),
            ac_phase: None,
            waveform: Some(w),
            port: None,
        });
        let all_components: Vec<Component> = all_components.into_iter().chain(sources).collect();

        // The JSON tags each variant by name: every component, waveform and value.
        for c in &all_components {
            assert_tagged(c);
            match c {
                Component::VoltageSource {
                    value, waveform, ..
                }
                | Component::CurrentSource {
                    value, waveform, ..
                } => {
                    assert_tagged(value);
                    waveform.iter().for_each(assert_tagged);
                }
                Component::Resistor { value, .. }
                | Component::Capacitor { value, .. }
                | Component::Inductor { value, .. } => assert_tagged(value),
                _ => {}
            }
            // The two expression walkers visit the same fields.
            let mut m = c.clone();
            assert_eq!(
                c.exprs(),
                m.exprs_mut()
                    .into_iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>(),
                "{c:?}"
            );
        }
        let ir = CircuitIR {
            top: Subcircuit {
                name: "all_components".into(),
                components: all_components,
                ..Default::default()
            },
            ..Default::default()
        };

        let json = serde_json::to_string_pretty(&ir).unwrap();
        let ir2: CircuitIR = serde_json::from_str(&json).unwrap();
        assert_eq!(ir, ir2);
        assert_schema(&ir);
    }

    #[test]
    fn test_all_analysis_variants_roundtrip() {
        let analyses = vec![
            Analysis::Op,
            Analysis::Dc {
                sweeps: vec![DcSweep {
                    source: "V1".into(),
                    start: 0.0,
                    stop: 5.0,
                    step: 0.01,
                }],
            },
            Analysis::Ac {
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
            Analysis::Transient {
                step: 1e-9,
                stop: 1e-3,
                start: Some(0.0),
                max_step: Some(1e-10),
                uic: true,
            },
            Analysis::Noise {
                output: "V(out)".into(),
                reference: "0".into(),
                source: "V1".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6,
                },
                points_per_summary: None,
            },
            Analysis::Tf {
                output: "V(out)".into(),
                source: "Vin".into(),
            },
            Analysis::Sensitivity {
                output: "V(out)".into(),
                ac: Some(FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6,
                }),
            },
            Analysis::PoleZero {
                node1: "in".into(),
                node2: "0".into(),
                node3: "out".into(),
                node4: "0".into(),
                tf_type: PzTransfer::Vol,
                pz_type: PzKind::Pz,
            },
            Analysis::Distortion {
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6,
                },
                f2overf1: Some(0.9),
            },
            Analysis::Pss {
                fundamental: 1e6,
                stabilization: 100e-6,
                observe_node: "out".into(),
                points_per_period: 100,
                harmonics: 10,
                sc_iter: None,
                steady_coeff: None,
                uic: false,
            },
            Analysis::HarmonicBalance {
                frequencies: vec![1e6, 2e6],
                harmonics: vec![5, 3],
            },
            Analysis::SPar {
                sweep: FreqSweep {
                    variation: Sweep::Lin,
                    points: 201,
                    start: 1e6,
                    stop: 10e9,
                },
            },
            Analysis::Stability {
                probe: "iprobe".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
            Analysis::TransientNoise {
                step: 1e-9,
                stop: 1e-6,
            },
            Analysis::Fourier {
                fundamental: 1e3,
                outputs: vec!["V(out)".into()],
                num_harmonics: Some(10),
            },
            Analysis::SpectreSweep {
                param: "R1".into(),
                start: 1e3,
                stop: 10e3,
                step: 1e3,
                inner: "dc1".into(),
                inner_type: SpectreAnalysis::Dc,
            },
            Analysis::SpectreMonteCarlo {
                iterations: 100,
                inner: "tran1".into(),
                inner_type: SpectreAnalysis::Tran,
                seed: Some(42),
            },
            Analysis::SpectrePac {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 100e-6,
                    pss_harmonics: 10,
                },
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
                sweep_type: "relative".into(),
            },
            Analysis::SpectrePnoise {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 100e-6,
                    pss_harmonics: 10,
                },
                output: "out".into(),
                reference: "0".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
            Analysis::SpectrePxf {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 100e-6,
                    pss_harmonics: 10,
                },
                output: "out".into(),
                source: "V1".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
            Analysis::SpectrePstb {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 100e-6,
                    pss_harmonics: 10,
                },
                probe: "iprobe".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
        ];

        analyses.iter().for_each(assert_tagged);
        let ir = CircuitIR {
            top: Subcircuit {
                name: "analysis_test".into(),
                ..Default::default()
            },
            testbench: Some(Testbench {
                analyses,
                ..Default::default()
            }),
            ..Default::default()
        };

        let json = serde_json::to_string_pretty(&ir).unwrap();
        let ir2: CircuitIR = serde_json::from_str(&json).unwrap();
        assert_eq!(ir, ir2);
        assert_schema(&ir);
    }

    #[test]
    fn elements_keep_card_order() {
        let r = |n: &str| Component::RawSpice { line: n.into() };
        let x = |n: &str, at: Option<u32>| Instance {
            name: n.into(),
            at,
            ..Default::default()
        };
        let sc = Subcircuit {
            components: vec![r("a"), r("b")],
            instances: vec![x("late", None), x("first", Some(0)), x("mid", Some(1))],
            ..Default::default()
        };
        let names: Vec<&str> = sc
            .elements()
            .into_iter()
            .map(|e| match e {
                Element::Component(Component::RawSpice { line }) => line.as_str(),
                Element::Instance(i) => i.name.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(names, ["first", "a", "mid", "b", "late"]);
    }

    // ── Issue 02: Analysis::kind_str() ──

    #[test]
    fn test_analysis_kind_str() {
        assert_eq!(Analysis::Op.kind_str(), "op");
        assert_eq!(Analysis::Dc { sweeps: vec![] }.kind_str(), "dc");
        assert_eq!(
            Analysis::Ac {
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6
                }
            }
            .kind_str(),
            "ac"
        );
        assert_eq!(
            Analysis::Transient {
                step: 1e-9,
                stop: 1e-6,
                start: None,
                max_step: None,
                uic: false
            }
            .kind_str(),
            "tran"
        );
        assert_eq!(
            Analysis::Noise {
                output: "out".into(),
                reference: "0".into(),
                source: "V1".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6
                },
                points_per_summary: None
            }
            .kind_str(),
            "noise"
        );
        assert_eq!(
            Analysis::Pss {
                fundamental: 1e6,
                stabilization: 10e-6,
                observe_node: "out".into(),
                points_per_period: 100,
                harmonics: 10,
                sc_iter: None,
                steady_coeff: None,
                uic: false
            }
            .kind_str(),
            "pss"
        );
        assert_eq!(
            Analysis::HarmonicBalance {
                frequencies: vec![1e6],
                harmonics: vec![5]
            }
            .kind_str(),
            "hb"
        );
        assert_eq!(
            Analysis::Stability {
                probe: "p".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6
                }
            }
            .kind_str(),
            "stb"
        );
        assert_eq!(
            Analysis::TransientNoise {
                step: 1e-9,
                stop: 1e-6
            }
            .kind_str(),
            "trannoise"
        );
        assert_eq!(
            Analysis::SpectrePac {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 10e-6,
                    pss_harmonics: 10
                },
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6
                },
                sweep_type: "relative".into()
            }
            .kind_str(),
            "pac"
        );
    }

    #[test]
    fn test_ac_magnitude_serde_roundtrip() {
        let v_with_ac = Component::VoltageSource {
            name: "ac".into(),
            np: "in".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0),
            ac_phase: Some(45.0),
            waveform: None,
            port: None,
        };
        let json = serde_json::to_string(&v_with_ac).unwrap();
        assert!(
            json.contains("\"ac_magnitude\":1.0"),
            "json has ac_magnitude: {}",
            json
        );
        assert!(
            json.contains("\"ac_phase\":45.0"),
            "json has ac_phase: {}",
            json
        );

        let roundtripped: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(v_with_ac, roundtripped);

        // Without AC — fields should be absent (serde skip_serializing_none via default)
        let v_no_ac = Component::VoltageSource {
            name: "dc".into(),
            np: "vdd".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 3.3 },
            ac_magnitude: None,
            ac_phase: None,
            waveform: None,
            port: None,
        };
        let json_no_ac = serde_json::to_string(&v_no_ac).unwrap();
        let rt_no_ac: Component = serde_json::from_str(&json_no_ac).unwrap();
        assert_eq!(v_no_ac, rt_no_ac);

        // Deserialize JSON without ac fields (backwards compat)
        let old_json = r#"{"type":"VoltageSource","name":"old","np":"a","nm":"0","value":{"type":"Numeric","value":5.0},"waveform":null}"#;
        let old_v: Component = serde_json::from_str(old_json).unwrap();
        match old_v {
            Component::VoltageSource {
                ac_magnitude,
                ac_phase,
                ..
            } => {
                assert_eq!(ac_magnitude, None);
                assert_eq!(ac_phase, None);
            }
            _ => panic!("Expected VoltageSource"),
        }
    }
}
