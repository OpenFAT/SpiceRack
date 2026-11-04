//! Spectre analysis, `options`, `save`, `ic`/`nodeset` statements and
//! VACASK `control` blocks → [`BenchCard`] views, what each adds to the
//! testbench as the old lowering applied them; `None` when the IR could not
//! hold one; and [`Rd::bench_cards`], every testbench item's views.
//!
//! Cites: the old lowering's named analyses; Spectre Circuit Simulator
//! Reference `[REF]` analysis statements; VACASK `share/doc/vacask/demo`
//! (`analysis`, `sweep`, `options`).
//!
//! Contents:
//! - `Rd::bench_cards(&self, b: Bench) -> Vec<BenchCard>`
//! - `Rd::spectre_analysis(&self, n: u32) -> Option<Ana>`
//! - `Rd::node_vals(&self, n: u32) -> Option<Vec<(String, f64)>>`

use super::named_elements::unquoted;
use super::{Ana, Bench, BenchCard, Opt, Rd, Sweep};
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

impl<'a> Rd<'a> {
    /// What testbench item `b` adds, in order; empty when the IR held
    /// nothing of it.
    pub(in crate::syntax::render) fn bench_cards(&self, b: Bench) -> Vec<BenchCard> {
        match b {
            Bench::Card(n) => self.bench_card(n).into_iter().collect(),
            Bench::Meas(k) => vec![BenchCard::Meas(self.line(k))],
            Bench::Save(n) => {
                let outs = self.fields(n).iter().map(|&o| {
                    let s = self.name_of(o);
                    // quirk: `dev:p` is `i(dev)` with `dev` as written, not as
                    // the name map spells its declaration (`V1`, `Vin1`); any
                    // other terminal (`main:d`) is kept as written, which no
                    // SPICE output names. Phase 7: both through the map.
                    match s.split_once(':') {
                        Some((dev, "p")) => format!("i({dev})"),
                        _ => s,
                    }
                });
                vec![BenchCard::Saves(outs.collect())]
            }
            Bench::Analysis(n) => self
                .spectre_analysis(n)
                .map(BenchCard::Ana)
                .into_iter()
                .collect(),
            Bench::Options(n) => vec![BenchCard::Options(self.spectre_options(n))],
            Bench::NodeVals(n) => self
                .node_vals(n)
                .map(|v| BenchCard::NodeVals(self.tag(n) == Tag::Ic, v))
                .into_iter()
                .collect(),
            Bench::Control(n) => self.vacask_control(n),
        }
    }

    /// `name [(p n)] kind k=v`: the analyses the old IR held (the old
    /// lowering's `spectre_analysis`).
    pub(in crate::syntax::render) fn spectre_analysis(&self, n: u32) -> Option<Ana> {
        let h = self.ast.header(Node(n));
        let kind = self.text(h.master);
        let ns: Vec<String> = h.nodes.iter().map(|&k| self.name_of(k)).collect();
        let p = self.pairs(n);
        let num = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let s = |k: &str| self.get(&p, k).map(|v| self.text(v));
        let sweep = || -> Option<Sweep> {
            let (variation, points) = ["dec", "lin", "log"]
                .into_iter()
                .find_map(|v| num(v).map(|x| (v, x as u32)))?;
            Some(Sweep {
                variation,
                points,
                start: num("start")?,
                stop: num("stop")?,
            })
        };
        Some(match (kind.as_str(), ns.as_slice()) {
            ("dc", []) if p.is_empty() => Ana::Op,
            ("dc", []) => {
                // quirk: `dev=` and `iprobe=` (below) name a source as
                // written, not as the name map spells its declaration
                // (`in1` is `Vin1`); phase 7 spells them through the map.
                let source = match (s("dev"), s("param")) {
                    (Some(dev), Some(param)) if param == "dc" => dev,
                    (None, Some(param)) => param,
                    _ => return None,
                };
                Ana::Dc(vec![(source, num("start")?, num("stop")?, num("step")?)])
            }
            ("ac", []) => Ana::Ac(sweep()?),
            ("tran", []) => Ana::Tran {
                step: num("step")?,
                stop: num("stop")?,
                start: num("start"),
                max_step: num("maxstep"),
                uic: s("skipdc").as_deref() == Some("yes"),
            },
            ("noise", [out, reference]) => Ana::Noise {
                output: out.clone(),
                reference: reference.clone(),
                source: s("iprobe")?,
                sweep: sweep()?,
                pps: None,
            },
            _ => return None,
        })
    }

    /// A Spectre `ic`/`nodeset`'s node values; `None` when one is no
    /// number (the old lowering's `named_testbench`).
    pub(in crate::syntax::render) fn node_vals(&self, n: u32) -> Option<Vec<(String, f64)>> {
        self.fields(n)
            .iter()
            .map(|&k| {
                (self.tag(k) == Tag::Assign).then_some(())?;
                Some((self.key(k), self.number(self.ast.rhs(Node(k)))?))
            })
            .collect()
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// `name options k=v` (the old lowering's `spectre_options`).
    fn spectre_options(&self, n: u32) -> Vec<Opt> {
        let mut out = Vec::new();
        for (k, v) in self.pairs(n) {
            out.push(match (k.as_str(), self.number(v)) {
                ("temp", Some(x)) => Opt::Temp(x),
                ("tnom", Some(x)) => Opt::Tnom(x),
                _ => {
                    let k = match k.as_str() {
                        "iabstol" => "abstol",
                        "vabstol" => "vntol",
                        "dcmaxiters" => "max_iterations",
                        other => other,
                    };
                    Opt::Portable(k.to_string(), self.vtext(v))
                }
            });
        }
        out
    }

    /// A VACASK `control` block: its options and analyses, the rest kept as
    /// lines (the old lowering's `ControlBlock`).
    fn vacask_control(&self, n: u32) -> Vec<BenchCard> {
        let mut out = Vec::new();
        let mut sweeps: Vec<u32> = Vec::new();
        let extra = |out: &mut Vec<BenchCard>, s: u32| out.push(BenchCard::Extra(self.line(s)));
        for &st in self.body(n) {
            let head = self.ttag(self.main(st));
            if head == T::KwSweep {
                sweeps.push(st);
                continue;
            }
            if head == T::KwAnalysis {
                match self.vacask_analysis(&sweeps, st) {
                    Some(cards) => out.extend(cards),
                    None => {
                        for &s in sweeps.iter().chain([&st]) {
                            extra(&mut out, s);
                        }
                    }
                }
                sweeps.clear();
                continue;
            }
            for &s in &sweeps {
                extra(&mut out, s);
            }
            sweeps.clear();
            match self.vacask_options(st) {
                Some(o) => out.push(BenchCard::Options(o)),
                None => extra(&mut out, st),
            }
        }
        for &s in &sweeps {
            extra(&mut out, s);
        }
        out
    }

    /// A VACASK `options` statement in a `control` block (the old
    /// lowering's `vacask_control`, `Ctl`).
    fn vacask_options(&self, st: u32) -> Option<Vec<Opt>> {
        if self.tag(st) != Tag::Options {
            return None;
        }
        let mut out = Vec::new();
        for (k, v) in self.pairs(st) {
            match (k.as_str(), self.number(v)) {
                ("rawfile", _) => {}
                ("temp", Some(x)) => out.push(Opt::Temp(x)),
                ("tnom", Some(x)) => out.push(Opt::Tnom(x)),
                _ => out.push(Opt::Vacask(k, self.text(v))),
            }
        }
        Some(out)
    }

    /// `sweep …` lines and the `analysis NAME KIND k=v` they belong to
    /// (the old lowering's `vacask_control`, `Ana`): the analysis and the
    /// initial conditions it sets; `None` when the IR had no such analysis.
    fn vacask_analysis(&self, sweeps: &[u32], st: u32) -> Option<Vec<BenchCard>> {
        let p = self.pairs(st);
        let num = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let s = |k: &str| self.get(&p, k).and_then(|v| unquoted(&self.text(v)));
        let sweeps: Vec<(String, f64, f64, f64)> = sweeps
            .iter()
            .rev()
            .map(|&sw| {
                let p = self.pairs(sw);
                let num = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
                let s = |k: &str| self.get(&p, k).and_then(|v| unquoted(&self.text(v)));
                let (src, from, to, pts) =
                    (s("instance")?, num("from")?, num("to")?, num("points")?);
                (s("parameter").as_deref() == Some("dc"))
                    .then(|| (src, from, to, (to - from) / pts))
            })
            .collect::<Option<_>>()?;
        let kind = match self.ast.header(Node(st)).master {
            NONE => String::new(),
            w => self.text(w),
        };
        let freq = || -> Option<Sweep> {
            let mode = ["dec", "oct", "lin", "log"]
                .into_iter()
                .find(|m| s("mode").is_some_and(|x| x.eq_ignore_ascii_case(m)))?;
            let points = num("points")? as u32 + u32::from(mode == "lin");
            Some(Sweep {
                variation: mode,
                points,
                start: num("from")?,
                stop: num("to")?,
            })
        };
        let a = match kind.as_str() {
            "op" if !sweeps.is_empty() => Ana::Dc(sweeps),
            _ if !sweeps.is_empty() => return None,
            "op" => Ana::Op,
            "ac" => Ana::Ac(freq()?),
            "tran" => {
                let ic = match self.get(&p, "ic") {
                    Some(v) => self.node_list(v)?,
                    None => vec![],
                };
                let ns = match self.get(&p, "nodeset") {
                    Some(v) => self.node_list(v)?,
                    None => vec![],
                };
                let a = Ana::Tran {
                    step: num("step")?,
                    stop: num("stop")?,
                    start: None,
                    max_step: num("maxstep"),
                    uic: s("icmode").as_deref() == Some("uic"),
                };
                return Some(vec![
                    BenchCard::NodeVals(true, ic),
                    BenchCard::NodeVals(false, ns),
                    BenchCard::Ana(a),
                ]);
            }
            "noise" => Ana::Noise {
                output: s("out")?,
                reference: "0".into(),
                source: s("in")?,
                sweep: freq()?,
                pps: None,
            },
            "dcxf" => Ana::Tf(s("out")?, String::new()),
            _ => return None,
        };
        Some(vec![BenchCard::Ana(a)])
    }

    /// `ic=["n"; v; ...]` (or `{"n", v, ...}`) → node values (the old
    /// lowering's `node_list`).
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
