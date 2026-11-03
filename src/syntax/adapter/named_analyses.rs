//! Spectre analysis and `options` statements and VACASK `control`
//! statements → `Testbench` rows, as `import/lower/named_analyses.rs`
//! applies them; `false`/`None` when the IR cannot hold one.
//!
//! Cites: `import/lower/named_analyses.rs` (ported); Spectre Circuit
//! Simulator Reference `[REF]` analysis statements;
//! `codegen::spectre::map_option_name` (inverted).
//!
//! Contents:
//! - `Cx::spectre_analysis(&self, n: u32) -> Option<Analysis>`
//! - `Cx::spectre_options(&mut self, n: u32)`
//! - `Cx::vacask_options(&mut self, st: u32) -> bool`
//! - `Cx::vacask_analysis(&mut self, sweeps: &[u32], st: u32) -> bool`

use super::Cx;
use super::named_elements::unquoted;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, Tag};

impl<'a> Cx<'a> {
    /// `name [(p n)] kind k=v`: the analyses `codegen::spectre` writes
    /// (lower/named_analyses.rs `spectre_analysis`).
    pub(super) fn spectre_analysis(&self, n: u32) -> Option<Analysis> {
        let h = self.ast.header(Node(n));
        let kind = self.text(h.master);
        let ns: Vec<String> = h.nodes.iter().map(|&k| self.name_of(k)).collect();
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
        if self.tag(st) != Tag::Options {
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
        let kind = match self.ast.header(Node(st)).master {
            NONE => String::new(),
            w => self.text(w),
        };
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

impl<'a> Cx<'a> {
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
