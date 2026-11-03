//! One element card as data: every SPICE component kind the IR models, and
//! the per-kind views passes run over them (names, parameters, expressions).
//!
//! Cites: ngspice manual §4.1.11 (RF ports); SPICE 2G6 `POLY` term order.
//!
//! Contents:
//! - `pub enum Component`, `pub struct RfPort`
//! - `Component::names_mut(&mut self) -> Vec<&mut String>`
//! - `Component::params(&self) -> &[(String, String)]`
//! - `Component::exprs(&self) -> Vec<&str>`, `Component::exprs_mut(&mut self) -> Vec<&mut String>`
//! - `Component::name(&self) -> Option<&str>`

use serde::{Deserialize, Serialize};

use super::*;

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

/// A voltage source's RF port (ngspice manual §4.1.11): its number among
/// the deck's ports, which count from 1 with no repeat, and its reference
/// impedance in ohms (ngspice's default is 50).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RfPort {
    pub num: u32,
    pub z0: f64,
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
