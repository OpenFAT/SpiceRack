//! Spectre and VACASK instance, analysis and control statements → IR
//! components, subcircuit instances and testbench rows, as
//! `import/lower/named_elements.rs` and `named_analyses.rs` build them.
//!
//! Cites: `import/lower/named_elements.rs`, `named_analyses.rs` (ported);
//! Spectre Circuit Simulator Reference 5.0 `[CMP]` (`vsource` #4, #17, #18,
//! p.686); `codegen::spectre`, `codegen::vacask` (the key names, inverted).
//!
//! Contents:
//! - `Cx::inst(&self, n: u32, kinds: &HashMap<String, String>, subckts: &[String])
//!   -> Option<Result<Component, Instance>>`
//! - `Cx::spectre_analysis(&self, n: u32) -> Option<Analysis>`, `Cx::spectre_options(&mut self, n: u32)`
//! - `Cx::vacask_analysis(&mut self, sweeps: &[u32], st: u32) -> bool`,
//!   `Cx::vacask_options(&mut self, st: u32) -> bool`

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

impl Cx<'_> {
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

    /// `name [(p n)] kind k=v`: the analyses `codegen::spectre` writes
    /// (lower/named_analyses.rs `spectre_analysis`).
    pub(super) fn spectre_analysis(&self, n: u32) -> Option<Analysis> {
        let words = self.ast.rhs(Node(n));
        let kind = self.text(self.ast.extra(words + 1));
        let ns: Vec<String> = self
            .ast
            .list(words + 2)
            .iter()
            .map(|&k| self.name_of(k))
            .collect();
        let p = self.pairs(n);
        let num = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let s = |k: &str| self.get(&p, k).map(|v| self.text(v));
        let sweep = || {
            [Sweep::Dec, Sweep::Lin, Sweep::Log]
                .into_iter()
                .find_map(|v| num(v.name()).map(|x| (v, x as u32)))
        };
        Some(match (kind.as_str(), ns.as_slice()) {
            ("dc", []) if p.is_empty() => Analysis::Op,
            ("dc", []) => {
                let source = match (s("dev"), s("param")) {
                    (Some(dev), Some(param)) if param == "dc" => dev,
                    (None, Some(param)) => param,
                    _ => return None,
                };
                Analysis::Dc {
                    sweeps: vec![DcSweep {
                        source,
                        start: num("start")?,
                        stop: num("stop")?,
                        step: num("step")?,
                    }],
                }
            }
            ("ac", []) => {
                let (variation, points) = sweep()?;
                Analysis::Ac {
                    sweep: FreqSweep {
                        variation,
                        points,
                        start: num("start")?,
                        stop: num("stop")?,
                    },
                }
            }
            ("tran", []) => Analysis::Transient {
                step: num("step")?,
                stop: num("stop")?,
                start: num("start"),
                max_step: num("maxstep"),
                uic: s("skipdc").as_deref() == Some("yes"),
            },
            ("noise", [out, reference]) => {
                let (variation, points) = sweep()?;
                Analysis::Noise {
                    output: out.clone(),
                    reference: reference.clone(),
                    source: s("iprobe")?,
                    sweep: FreqSweep {
                        variation,
                        points,
                        start: num("start")?,
                        stop: num("stop")?,
                    },
                    points_per_summary: None,
                }
            }
            _ => return None,
        })
    }

    /// `name options k=v` (lower/named_analyses.rs `spectre_options`).
    pub(super) fn spectre_options(&mut self, n: u32) {
        for (k, v) in self.pairs(n) {
            match (k.as_str(), self.number(v)) {
                ("temp", Some(x)) => self.tb.temperature = Some(x),
                ("tnom", Some(x)) => self.tb.nominal_temperature = Some(x),
                _ => {
                    let k = match k.as_str() {
                        "iabstol" => "abstol",
                        "vabstol" => "vntol",
                        "dcmaxiters" => "max_iterations",
                        other => other,
                    };
                    let v = self.vtext(v);
                    self.tb.options.portable.push((k.to_string(), v));
                }
            }
        }
    }

    /// A VACASK `options` statement in a `control` block
    /// (lower/named_analyses.rs `vacask_control`, `Ctl`).
    pub(super) fn vacask_options(&mut self, st: u32) -> bool {
        if self.tag(st) != Tag::Card || self.t.text(self.main(st)) != "options" {
            return false;
        }
        for (k, v) in self.pairs(st) {
            match (k.as_str(), self.number(v)) {
                ("rawfile", _) => {}
                ("temp", Some(x)) => self.tb.temperature = Some(x),
                ("tnom", Some(x)) => self.tb.nominal_temperature = Some(x),
                _ => {
                    let v = self.text(v);
                    self.tb
                        .options
                        .backend_specific
                        .entry("vacask".into())
                        .or_default()
                        .push((k, v));
                }
            }
        }
        true
    }

    /// `sweep …` lines and the `analysis NAME KIND k=v` they belong to
    /// (lower/named_analyses.rs `vacask_control`, `Ana`); `false` when the
    /// IR has no such analysis.
    pub(super) fn vacask_analysis(&mut self, sweeps: &[u32], st: u32) -> bool {
        let p = self.pairs(st);
        let num = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let s = |k: &str| self.get(&p, k).and_then(|v| unquoted(&self.text(v)));
        let parsed: Option<Vec<DcSweep>> = sweeps
            .iter()
            .rev()
            .map(|&sw| {
                let p = self.pairs(sw);
                let num = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
                let s = |k: &str| self.get(&p, k).and_then(|v| unquoted(&self.text(v)));
                let (src, from, to, pts) =
                    (s("instance")?, num("from")?, num("to")?, num("points")?);
                (s("parameter").as_deref() == Some("dc")).then(|| DcSweep {
                    source: src,
                    start: from,
                    stop: to,
                    step: (to - from) / pts,
                })
            })
            .collect();
        let Some(sweeps) = parsed else { return false };
        let words: Vec<u32> = self
            .fields(st)
            .iter()
            .copied()
            .filter(|&k| self.tag(k) == Tag::Word)
            .collect();
        let kind = words.get(1).map(|&w| self.text(w)).unwrap_or_default();
        let freq = || -> Option<FreqSweep> {
            let mode: Sweep = s("mode")?.parse().ok()?;
            let points = num("points")? as u32 + u32::from(mode == Sweep::Lin);
            Some(FreqSweep {
                variation: mode,
                points,
                start: num("from")?,
                stop: num("to")?,
            })
        };
        let a = match kind.as_str() {
            "op" if !sweeps.is_empty() => Some(Analysis::Dc { sweeps }),
            _ if !sweeps.is_empty() => None,
            "op" => Some(Analysis::Op),
            "ac" => freq().map(|sweep| Analysis::Ac { sweep }),
            "tran" => (|| {
                let ic = match self.get(&p, "ic") {
                    Some(v) => self.node_list(v)?,
                    None => vec![],
                };
                let ns = match self.get(&p, "nodeset") {
                    Some(v) => self.node_list(v)?,
                    None => vec![],
                };
                let a = Analysis::Transient {
                    step: num("step")?,
                    stop: num("stop")?,
                    start: None,
                    max_step: num("maxstep"),
                    uic: s("icmode").as_deref() == Some("uic"),
                };
                Some((a, ic, ns))
            })()
            .map(|(a, ic, ns)| {
                self.tb.initial_conditions.extend(ic);
                self.tb.node_sets.extend(ns);
                a
            }),
            "noise" => freq().and_then(|sweep| {
                Some(Analysis::Noise {
                    output: s("out")?,
                    reference: "0".into(),
                    source: s("in")?,
                    sweep,
                    points_per_summary: None,
                })
            }),
            "dcxf" => s("out").map(|o| Analysis::Tf {
                output: o,
                source: String::new(),
            }),
            _ => None,
        };
        match a {
            Some(a) => {
                self.tb.analyses.push(a);
                true
            }
            None => false,
        }
    }
}

// ---- private ----

impl Cx<'_> {
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

    /// `ic=["n"; v; ...]` (or `{"n", v, ...}`) → node values
    /// (lower/named_analyses.rs `node_list`).
    fn node_list(&self, v: u32) -> Option<Vec<(String, f64)>> {
        let items: Vec<u32> = match self.tag(v) {
            Tag::Vector | Tag::List => self.ast.list(self.ast.rhs(Node(v)) + 1).to_vec(),
            _ => vec![v],
        };
        items
            .chunks(2)
            .map(|c| Some((unquoted(&self.text(*c.first()?))?, self.number(*c.get(1)?)?)))
            .collect()
    }
}

/// `"text"` without its quotes.
fn unquoted(v: &str) -> Option<String> {
    Some(v.strip_prefix('"')?.strip_suffix('"')?.to_string())
}
