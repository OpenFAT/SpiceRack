//! Spectre and VACASK instance statements → IR components (by built-in
//! master or model kind) or subcircuit `Instance`s, with their source
//! waveforms, as `import/lower/named_elements.rs` builds them.
//!
//! Cites: `import/lower/named_elements.rs`, `named_analyses.rs` (ported);
//! Spectre Circuit Simulator Reference 5.0 `[CMP]` (`vsource` #4, #17, #18,
//! p.686); `codegen::spectre`, `codegen::vacask` (the key names, inverted).
//!
//! Contents:
//! - `fn unquoted(v: &str) -> Option<String>`
//! - `Cx::inst(&self, n: u32, kinds: &HashMap<String, String>, subckts: &[String],) -> Option<Result<Component, Instance>>`

use std::collections::HashMap;

use super::Cx;
use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, Tag};

/// The keys one dialect's built-in masters use (lower/named_elements.rs).
struct Keys {
    source: &'static [&'static str],
    damp: &'static str,
    gm: &'static str,
    rm: &'static str,
    probe: &'static str,
    mutual: &'static str,
    coupling: &'static str,
}

const SPECTRE: Keys = Keys {
    source: &[
        "dc",
        "mag",
        "phase",
        "type",
        "val0",
        "val1",
        "delay",
        "rise",
        "fall",
        "width",
        "period",
        "sinedc",
        "ampl",
        "freq",
        "damp",
        "sinephase",
        "fmmodindex",
        "fmmodfreq",
        "td1",
        "tau1",
        "td2",
        "tau2",
        "wave",
        "pwlperiod",
    ],
    damp: "damp",
    gm: "gm",
    rm: "rm",
    probe: "probe",
    mutual: "mutual_inductor",
    coupling: "coupling",
};

const VACASK: Keys = Keys {
    source: &[
        "dc", "mag", "phase", "type", "val0", "val1", "delay", "rise", "fall", "width", "period",
        "sinedc", "ampl", "freq", "theta", "modindex", "modfreq", "tau1", "td2", "tau2", "wave",
    ],
    damp: "theta",
    gm: "gain",
    rm: "gain",
    probe: "ctlinst",
    mutual: "mutual",
    coupling: "k",
};

/// `"text"` without its quotes.
pub(super) fn unquoted(v: &str) -> Option<String> {
    Some(v.strip_prefix('"')?.strip_suffix('"')?.to_string())
}

impl<'a> Cx<'a> {
    /// One instance statement as a component or subcircuit instance; `None`
    /// when the IR has no form for it (lower/named_elements.rs `inst`).
    pub(super) fn inst(
        &self,
        n: u32,
        kinds: &HashMap<String, String>,
        subckts: &[String],
    ) -> Option<Result<Component, Instance>> {
        let keys = self.keys();
        let words = self.ast.rhs(Node(n));
        let name = self.name_of(self.ast.extra(words));
        let ns: Vec<String> = self
            .ast
            .list(words + 2)
            .iter()
            .map(|&k| self.name_of(k))
            .collect();
        let m = self.ast.extra(words + 1);
        if m == NONE {
            return None;
        }
        let mut master = self.name_of(m);
        if let Some(b) = kinds.get(&master).and_then(|k| k.strip_prefix('=')) {
            master = b.to_string();
        }
        let p = self.pairs(n);
        let kept = |used: &[&str]| -> Vec<(String, String)> {
            p.iter()
                .filter(|(k, _)| !used.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), self.vtext(*v)))
                .collect()
        };
        let two = || match ns.as_slice() {
            [a, b] => Some((a.clone(), b.clone())),
            _ => None,
        };
        let num = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let ident = |k: &str| {
            self.get(&p, k).map(|v| {
                let t = self.text(v);
                unquoted(&t).unwrap_or(t)
            })
        };
        Some(Ok(match master.as_str() {
            "resistor" | "capacitor" | "inductor" => {
                let (n1, n2) = two()?;
                let (letter, key) = match master.as_str() {
                    "resistor" => ('r', "r"),
                    "capacitor" => ('c', "c"),
                    _ => ('l', "l"),
                };
                let (name, value, params) = (
                    self.strip(&name, letter),
                    self.value(self.get(&p, key)?),
                    kept(&[key]),
                );
                match letter {
                    'r' => Component::Resistor {
                        name,
                        n1,
                        n2,
                        value,
                        model: None,
                        params,
                    },
                    'c' => Component::Capacitor {
                        name,
                        n1,
                        n2,
                        value,
                        model: None,
                        params,
                    },
                    _ => Component::Inductor {
                        name,
                        n1,
                        n2,
                        value,
                        model: None,
                        params,
                    },
                }
            }
            "vsource" | "isource" => {
                let (np, nm) = two()?;
                if p.iter().any(|(k, _)| !keys.source.contains(&k.as_str())) {
                    return None;
                }
                let waveform = self.named_waveform(&p)?;
                let value = self
                    .get(&p, "dc")
                    .map_or(IrValue::Numeric { value: 0.0 }, |v| self.value(v));
                let (ac_magnitude, ac_phase) = (num("mag"), num("phase"));
                if master == "vsource" {
                    Component::VoltageSource {
                        name: self.strip(&name, 'v'),
                        np,
                        nm,
                        value,
                        ac_magnitude,
                        ac_phase,
                        waveform,
                        port: None,
                    }
                } else {
                    Component::CurrentSource {
                        name: self.strip(&name, 'i'),
                        np,
                        nm,
                        value,
                        ac_magnitude,
                        ac_phase,
                        waveform,
                    }
                }
            }
            "vcvs" | "vccs" => {
                let [np, nm, ncp, ncm] = ns.as_slice() else {
                    return None;
                };
                let (np, nm, ncp, ncm) = (np.clone(), nm.clone(), ncp.clone(), ncm.clone());
                if master == "vcvs" {
                    Component::Vcvs {
                        name: self.strip(&name, 'e'),
                        np,
                        nm,
                        ncp,
                        ncm,
                        gain: num("gain")?,
                    }
                } else {
                    Component::Vccs {
                        name: self.strip(&name, 'g'),
                        np,
                        nm,
                        ncp,
                        ncm,
                        transconductance: num(keys.gm)?,
                    }
                }
            }
            "cccs" | "ccvs" => {
                let (np, nm) = two()?;
                let vsense = ident(keys.probe)?;
                if master == "cccs" {
                    Component::Cccs {
                        name: self.strip(&name, 'f'),
                        np,
                        nm,
                        vsense,
                        gain: num("gain")?,
                    }
                } else {
                    Component::Ccvs {
                        name: self.strip(&name, 'h'),
                        np,
                        nm,
                        vsense,
                        transresistance: num(keys.rm)?,
                    }
                }
            }
            m if m == keys.mutual && (self.d == Dialect::Spectre || ns.is_empty()) => {
                Component::MutualInductor {
                    name: self.strip(&name, 'k'),
                    inductor1: self.strip(&ident("ind1")?, 'l'),
                    inductor2: self.strip(&ident("ind2")?, 'l'),
                    coupling: num(keys.coupling)?,
                }
            }
            "pvcvs" | "pvccs" | "pcccs" | "pccvs" if self.d == Dialect::Spectre => {
                let letter = match master.as_str() {
                    "pvcvs" => 'E',
                    "pvccs" => 'G',
                    "pcccs" => 'F',
                    _ => 'H',
                };
                let list = |k: &str| self.get(&p, k).map(|v| self.velems(v));
                let coeffs: Vec<f64> = list("coeffs")?
                    .into_iter()
                    .map(|c| self.number(c))
                    .collect::<Option<_>>()?;
                let (np, nm) = (ns.first()?.clone(), ns.get(1)?.clone());
                let controls = if matches!(letter, 'E' | 'G') {
                    ns[2..].to_vec()
                } else {
                    list("probes")?
                        .into_iter()
                        .map(|c| self.name_of(c))
                        .collect()
                };
                Component::Poly {
                    name: self.strip(&name, letter.to_ascii_lowercase()),
                    letter,
                    np,
                    nm,
                    controls,
                    coeffs,
                }
            }
            m if subckts.iter().any(|s| s == m) => {
                return Some(Err(Instance {
                    name: self.strip(&name, 'x'),
                    subcircuit: m.to_string(),
                    port_mapping: ns,
                    parameters: kept(&[]),
                    at: None,
                }));
            }
            m => {
                let kind = kinds.get(m)?.as_str();
                let (model, params) = (m.to_string(), kept(&[]));
                match (kind, ns.as_slice()) {
                    ("d", [np, nm]) => Component::Diode {
                        name: self.strip(&name, 'd'),
                        np: np.clone(),
                        nm: nm.clone(),
                        model,
                        params,
                    },
                    ("npn" | "pnp", [nc, nb, ne, extra @ ..]) if extra.len() <= 2 => {
                        Component::Bjt {
                            name: self.strip(&name, 'q'),
                            nc: nc.clone(),
                            nb: nb.clone(),
                            ne: ne.clone(),
                            extra_nodes: extra.to_vec(),
                            model,
                            params,
                        }
                    }
                    ("nmos" | "pmos", [nd, ng, s, nb]) => Component::Mosfet {
                        name: self.strip(&name, 'm'),
                        nd: nd.clone(),
                        ng: ng.clone(),
                        ns: s.clone(),
                        nb: Some(nb.clone()),
                        extra_nodes: vec![],
                        model,
                        params,
                    },
                    ("njf" | "pjf", [nd, ng, s]) if self.d == Dialect::Vacask => Component::Jfet {
                        name: self.strip(&name, 'j'),
                        nd: nd.clone(),
                        ng: ng.clone(),
                        ns: s.clone(),
                        model,
                        params,
                    },
                    ("nmf" | "pmf", [nd, ng, s]) if self.d == Dialect::Vacask => {
                        Component::Mesfet {
                            name: self.strip(&name, 'z'),
                            nd: nd.clone(),
                            ng: ng.clone(),
                            ns: s.clone(),
                            model,
                            params,
                        }
                    }
                    _ => return None,
                }
            }
        }))
    }
}

// ---- private ----

impl<'a> Cx<'a> {
    fn keys(&self) -> &'static Keys {
        if self.d == Dialect::Vacask {
            &VACASK
        } else {
            &SPECTRE
        }
    }

    /// `name`'s SPICE spelling: the letter `codegen` adds back is dropped
    /// (lower/named_elements.rs `strip`).
    fn strip(&self, name: &str, letter: char) -> String {
        let name = if self.d == Dialect::Vacask {
            name.to_lowercase()
        } else {
            name.to_string()
        };
        match name.chars().next() {
            Some(c) if c.eq_ignore_ascii_case(&letter) && name.len() > 1 => {
                name[c.len_utf8()..].to_string()
            }
            _ => name,
        }
    }

    /// lower/named_elements.rs `named_waveform`.
    fn named_waveform(&self, p: &[(String, u32)]) -> Option<Option<IrWaveform>> {
        let keys = self.keys();
        let f = |k: &str| {
            self.get(p, k)
                .map_or(Some(IrNum::Num(0.0)), |v| self.irnum(v))
        };
        let ty = self
            .get(p, "type")
            .map(|t| self.text(t).trim_matches('"').to_string());
        let fm = ty.as_deref() == Some("fm")
            || (ty.as_deref() == Some("sine") && self.get(p, "fmmodindex").is_some());
        let spectre_exp = self.d == Dialect::Spectre && ty.as_deref() == Some("exp");
        if (fm || spectre_exp)
            && self
                .get(p, "delay")
                .is_some_and(|v| self.number(v) != Some(0.0))
        {
            return None;
        }
        if ty.as_deref() != Some("pwl") && self.get(p, "pwlperiod").is_some() {
            return None;
        }
        Some(Some(match ty.as_deref() {
            None | Some("dc") => return Some(None),
            Some("pulse") => IrWaveform::Pulse {
                initial: f("val0")?,
                pulsed: f("val1")?,
                delay: f("delay")?,
                rise_time: f("rise")?,
                fall_time: f("fall")?,
                pulse_width: f("width")?,
                period: f("period")?,
            },
            Some("sine") if self.get(p, "fmmodindex").is_some() => IrWaveform::Sffm {
                offset: f("sinedc")?,
                amplitude: f("ampl")?,
                carrier_freq: f("freq")?,
                modulation_index: f("fmmodindex")?,
                signal_freq: f("fmmodfreq")?,
            },
            Some("sine") => IrWaveform::Sin {
                offset: f("sinedc")?,
                amplitude: f("ampl")?,
                frequency: f("freq")?,
                delay: f("delay")?,
                damping: f(keys.damp)?,
                phase: f("sinephase")?,
            },
            Some("exp") if self.d == Dialect::Vacask => {
                let n = |k: &str| self.get(p, k).map_or(Some(0.0), |v| self.number(v));
                IrWaveform::Exp {
                    initial: f("val0")?,
                    pulsed: f("val1")?,
                    rise_delay: n("delay")?.into(),
                    rise_tau: f("tau1")?,
                    fall_delay: (n("delay")? + n("td2")?).into(),
                    fall_tau: f("tau2")?,
                }
            }
            Some("exp") => IrWaveform::Exp {
                initial: f("val0")?,
                pulsed: f("val1")?,
                rise_delay: f("td1")?,
                rise_tau: f("tau1")?,
                fall_delay: f("td2")?,
                fall_tau: f("tau2")?,
            },
            Some("fm") => IrWaveform::Sffm {
                offset: f("sinedc")?,
                amplitude: f("ampl")?,
                carrier_freq: f("freq")?,
                modulation_index: f("modindex")?,
                signal_freq: f("modfreq")?,
            },
            Some("am") => IrWaveform::Am {
                offset: f("sinedc")?,
                amplitude: f("ampl")?,
                carrier_freq: f("freq")?,
                modulating_freq: f("modfreq")?,
                delay: f("delay")?,
            },
            Some("pwl") if self.d == Dialect::Spectre => {
                let w = self.get(p, "wave")?;
                let xs: Vec<IrNum> = self
                    .velems(w)
                    .into_iter()
                    .map(|x| self.irnum(x))
                    .collect::<Option<_>>()?;
                if !xs.len().is_multiple_of(2) {
                    return None;
                }
                let values: Vec<(IrNum, IrNum)> =
                    xs.chunks(2).map(|c| (c[0].clone(), c[1].clone())).collect();
                let repeat = match self.get(p, "pwlperiod") {
                    Some(v) if IrWaveform::spectre_pwl_period(&values)? == self.number(v)? => {
                        Some(IrNum::Num(0.0))
                    }
                    Some(_) => return None,
                    None => None,
                };
                let delay = match self.get(p, "delay") {
                    Some(v) => Some(self.irnum(v)?).filter(|d| *d != 0.0),
                    None => None,
                };
                IrWaveform::Pwl {
                    values,
                    repeat,
                    delay,
                }
            }
            Some(_) => return None,
        }))
    }

    /// quirk: lower/named_elements.rs `velems`: a vector's element rows;
    /// spectre.peg `s_unary2` captures no sign, so `-1` reads as `1`.
    fn velems(&self, v: u32) -> Vec<u32> {
        if self.tag(v) != Tag::Vector {
            return vec![v];
        }
        self.ast
            .list(self.ast.rhs(Node(v)) + 1)
            .iter()
            .map(|&k| match self.tag(k) {
                Tag::Unary => self.ast.lhs(Node(k)),
                _ => k,
            })
            .collect()
    }
}
