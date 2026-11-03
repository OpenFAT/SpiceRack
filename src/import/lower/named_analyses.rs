use super::{Cx, values::unquoted};
use crate::import::Tag;
use crate::ir::*;

impl Cx<'_> {
    /// `name [(p n)] kind k=v`: the analyses `codegen::spectre` writes.
    pub(super) fn spectre_analysis(&self, card: usize) -> Option<Analysis> {
        let kind = self.text(self.kid(card, Tag::Word)?);
        let ns: Vec<String> = self
            .all(card, Tag::Node)
            .into_iter()
            .map(|n| self.name(n))
            .collect();
        let p = self.pairs(card);
        let n = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let s = |k: &str| self.get(&p, k).map(|v| self.text(v).to_string());
        let sweep = || {
            [Sweep::Dec, Sweep::Lin, Sweep::Log]
                .into_iter()
                .find_map(|v| n(v.name()).map(|x| (v, x as u32)))
        };
        Some(match (kind, ns.as_slice()) {
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
                        start: n("start")?,
                        stop: n("stop")?,
                        step: n("step")?,
                    }],
                }
            }
            ("ac", []) => {
                let (variation, points) = sweep()?;
                Analysis::Ac {
                    sweep: FreqSweep {
                        variation,
                        points,
                        start: n("start")?,
                        stop: n("stop")?,
                    },
                }
            }
            ("tran", []) => Analysis::Transient {
                step: n("step")?,
                stop: n("stop")?,
                start: n("start"),
                max_step: n("maxstep"),
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
                        start: n("start")?,
                        stop: n("stop")?,
                    },
                    points_per_summary: None,
                }
            }
            _ => return None,
        })
    }

    /// `name options k=v`: options, `temp`/`tnom`, and the inverse of
    /// `codegen::spectre::map_option_name`.
    pub(super) fn spectre_options(&mut self, card: usize) {
        for (k, v) in self.pairs(card) {
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

    /// `ic=["n"; v; ...]` (or `{"n", v, ...}`) -> node values.
    fn node_list(&self, v: usize) -> Option<Vec<(String, f64)>> {
        let mut items = self.kids(v);
        // The `Val` holds one `Expr`, whose rows are the list's elements.
        while let [one] = items.as_slice() {
            items = self.kids(*one);
        }
        items
            .chunks(2)
            .map(|c| Some((unquoted(self.text(*c.first()?))?, self.number(*c.get(1)?)?)))
            .collect()
    }

    /// One VACASK `control` block statement the IR models; `false` if not.
    pub(super) fn vacask_control(&mut self, st: usize) -> bool {
        let p = self.pairs(st);
        let n = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let s = |k: &str| self.get(&p, k).and_then(|v| unquoted(self.text(v)));
        match self.tag(st) {
            Tag::Ctl
                if self
                    .kid(st, Tag::Word)
                    .is_some_and(|w| self.text(w) == "options") =>
            {
                for (k, v) in &p {
                    match (k.as_str(), self.number(*v)) {
                        ("rawfile", _) => {}
                        ("temp", Some(x)) => self.tb.temperature = Some(x),
                        ("tnom", Some(x)) => self.tb.nominal_temperature = Some(x),
                        // VACASK's own option names stay VACASK's, written back
                        // as they read (the portable names are SPICE's).
                        _ => {
                            let v = self.text(*v).to_string();
                            self.tb
                                .options
                                .backend_specific
                                .entry("vacask".into())
                                .or_default()
                                .push((k.clone(), v));
                        }
                    }
                }
                true
            }
            Tag::Ana => {
                // `sweep NAME instance="v1" parameter="dc" from= to= mode="lin" points=n`
                let sweeps: Option<Vec<DcSweep>> = self
                    .all(st, Tag::Sweep)
                    .into_iter()
                    .rev()
                    .map(|sw| {
                        let p = self.pairs(sw);
                        let n = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
                        let s = |k: &str| self.get(&p, k).and_then(|v| unquoted(self.text(v)));
                        let (src, from, to, pts) =
                            (s("instance")?, n("from")?, n("to")?, n("points")?);
                        (s("parameter").as_deref() == Some("dc")).then(|| DcSweep {
                            source: src,
                            start: from,
                            stop: to,
                            step: (to - from) / pts,
                        })
                    })
                    .collect();
                let Some(sweeps) = sweeps else { return false };
                let kind = self
                    .kid(st, Tag::Word)
                    .map(|w| self.text(w).to_string())
                    .unwrap_or_default();
                let freq = || -> Option<FreqSweep> {
                    let mode: Sweep = s("mode")?.parse().ok()?;
                    let points = n("points")? as u32 + u32::from(mode == Sweep::Lin);
                    Some(FreqSweep {
                        variation: mode,
                        points,
                        start: n("from")?,
                        stop: n("to")?,
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
                            step: n("step")?,
                            stop: n("stop")?,
                            start: None,
                            max_step: n("maxstep"),
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
            _ => false,
        }
    }
}
