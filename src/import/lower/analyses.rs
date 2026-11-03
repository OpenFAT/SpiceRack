//! SPICE testbench cards (analyses, options, outputs, `.control` analysis
//! lines) -> `Testbench` rows; `false` when the IR cannot hold the card.
//!
//! Cites: Xyce Reference Guide p.96 (`.options <pkg>`); HSPICE `.tran START=`;
//! LTspice `.tran Tstop`.
//!
//! Contents:
//! - `Cx::control(&mut self, cmd: usize)`
//! - `Cx::testbench_card(&mut self, card: usize) -> bool`

use std::collections::{HashMap, HashSet};

use super::Cx;
use crate::codegen::Dialect;
use crate::import::Report;
use crate::import::Tag;
use crate::ir::*;

impl Cx<'_> {
    /// A `.control` line ngspice runs as an analysis: read as its dot card.
    pub(super) fn control(&mut self, cmd: usize) {
        let line = format!(".{}\n", self.text(cmd));
        let folded = line.to_ascii_lowercase();
        let Some(one) = self.lang.g.rule("one") else {
            return;
        };
        let Ok(t) = self.lang.g.parse(one, &folded) else {
            return;
        };
        let Some(card) = t.children(None).next() else {
            return;
        };
        let mut cx = Cx {
            t: &t,
            src: &line,
            d: self.d,
            lang: self.lang,
            models: HashSet::new(),
            report: Report::default(),
            tb: std::mem::take(&mut self.tb),
            globals: vec![],
            functions: vec![],
            defs: vec![],
            sections: vec![],
            kinds: HashMap::new(),
            subckts: vec![],
        };
        cx.testbench_card(card);
        self.tb = cx.tb;
    }

    /// A testbench card (analysis, option, output), applied; `false` if the
    /// IR cannot hold it.
    pub(super) fn testbench_card(&mut self, card: usize) -> bool {
        let tag = self.tag(card);
        if let Some(a) = self.analysis(card, tag) {
            self.tb.analyses.push(a);
            return true;
        }
        match tag {
            // Xyce: `.options <pkg> k=v` (RG p.96), kept per package so writing
            // Xyce regroups them; DEVICE TEMP/TNOM are the temperatures.
            Tag::Options => match (self.d, self.kid(card, Tag::Pkg)) {
                (Dialect::Xyce, Some(pkg)) => {
                    let pkg = self.text(pkg).to_ascii_lowercase();
                    let p = self.params(card);
                    if p.is_empty() {
                        self.tb
                            .options
                            .backend_specific
                            .entry("xyce".into())
                            .or_default()
                            .push((format!("{pkg}."), String::new()));
                    }
                    let nums: Vec<Option<f64>> = self
                        .kids(card)
                        .into_iter()
                        .filter(|&k| matches!(self.tag(k), Tag::Pair | Tag::Flag))
                        .map(|k| match self.tag(k) {
                            Tag::Pair => self.number(self.kids(k)[1]),
                            _ => None,
                        })
                        .collect();
                    for ((k, v), x) in p.into_iter().zip(nums) {
                        match (pkg.as_str(), k.to_ascii_lowercase().as_str(), x) {
                            ("device", "temp", Some(x)) => self.tb.temperature = Some(x),
                            ("device", "tnom", Some(x)) => self.tb.nominal_temperature = Some(x),
                            _ => self
                                .tb
                                .options
                                .backend_specific
                                .entry("xyce".into())
                                .or_default()
                                .push((format!("{pkg}.{}", k.to_ascii_lowercase()), v)),
                        }
                    }
                }
                (Dialect::Xyce, None) => {
                    let p = self.params(card);
                    self.tb
                        .options
                        .backend_specific
                        .entry("xyce".into())
                        .or_default()
                        .extend(p);
                }
                _ => {
                    let p = self.params(card);
                    self.tb.options.portable.extend(p);
                }
            },
            Tag::Probe => {
                let outs = self.texts(card, Tag::Out);
                self.tb.saves.extend(outs);
            }
            Tag::Step => {
                let k = self.all(card, Tag::Num);
                let sweep_type = self
                    .kid(card, Tag::StepKind)
                    .and_then(|s| self.text(s).parse().ok());
                let step = StepParam {
                    param: self.text(self.kid(card, Tag::Name).unwrap()).to_string(),
                    start: self.num(k[0]).unwrap_or(f64::NAN),
                    stop: self.num(k[1]).unwrap_or(f64::NAN),
                    step: self.num(k[2]).unwrap_or(f64::NAN),
                    sweep_type,
                };
                if step.start.is_nan() || step.stop.is_nan() || step.step.is_nan() {
                    return false;
                }
                self.tb.step_params.push(step);
            }
            Tag::Temp => match self.kid(card, Tag::Val).and_then(|v| self.number(v)) {
                Some(t) => self.tb.temperature = Some(t),
                None => return false,
            },
            Tag::Ic | Tag::Nodeset => {
                let v: Option<Vec<(String, f64)>> = self
                    .all(card, Tag::NodeVal)
                    .into_iter()
                    .map(|nv| {
                        let k = self.kids(nv);
                        Some((self.name(k[0]), self.number(k[1])?))
                    })
                    .collect();
                match (v, tag) {
                    (Some(v), Tag::Ic) => self.tb.initial_conditions.extend(v),
                    (Some(v), _) => self.tb.node_sets.extend(v),
                    (None, _) => return false,
                }
            }
            Tag::Save => {
                let outs = self.texts(card, Tag::Out);
                // A lone `.save all` is what no save list means.
                if !outs.iter().all(|x| x.eq_ignore_ascii_case("all")) {
                    self.tb.saves.extend(outs);
                }
            }
            Tag::Print => {
                let analysis = self
                    .kid(card, Tag::PrintKind)
                    .map_or(String::new(), |k| self.text(k).to_ascii_lowercase());
                self.tb.prints.push(PrintCard {
                    analysis,
                    outputs: self.texts(card, Tag::Out),
                });
            }
            Tag::Meas => self.tb.measures.push(self.text(card).to_string()),
            _ => return false,
        }
        true
    }
}

// ---- private ----

impl Cx<'_> {
    fn analysis(&self, card: usize, tag: Tag) -> Option<Analysis> {
        let k = self.kids(card);
        let n = |i: usize| k.get(i).and_then(|&r| self.number(r));
        let word = |i: usize| k.get(i).map(|&r| self.text(r).to_string());
        Some(match tag {
            Tag::Op => Analysis::Op,
            Tag::Dc => Analysis::Dc {
                sweeps: k
                    .iter()
                    .map(|&s| {
                        let w = self.kids(s);
                        Some(DcSweep {
                            source: self.text(w[0]).to_string(),
                            start: self.number(w[1])?,
                            stop: self.number(w[2])?,
                            step: self.number(w[3])?,
                        })
                    })
                    .collect::<Option<_>>()?,
            },
            Tag::Ac => Analysis::Ac {
                sweep: FreqSweep {
                    variation: word(0)?.parse().ok()?,
                    points: self.count(k[1])?,
                    start: n(2)?,
                    stop: n(3)?,
                },
            },
            Tag::Sp => Analysis::SPar {
                sweep: FreqSweep {
                    variation: word(0)?.parse().ok()?,
                    points: self.count(k[1])?,
                    start: n(2)?,
                    stop: n(3)?,
                },
            },
            Tag::Tran => {
                let uic = self.kid(card, Tag::Uic).is_some();
                let p: Vec<f64> = self
                    .all(card, Tag::Num)
                    .into_iter()
                    .map(|r| self.num(r))
                    .collect::<Option<_>>()?;
                // HSPICE `START=` names the start.
                let start = match self.kid(card, Tag::Pair) {
                    Some(pr) if self.text(self.kids(pr)[0]).eq_ignore_ascii_case("start") => {
                        Some(self.number(self.kids(pr)[1])?)
                    }
                    Some(_) => return None,
                    None => None,
                };
                match p.as_slice() {
                    // LTspice `.tran Tstop` (SCR .TRAN): the step is LTspice's own.
                    [stop] => Analysis::Transient {
                        step: 0.0,
                        stop: *stop,
                        start: None,
                        max_step: None,
                        uic,
                    },
                    [step, stop] if start.is_some() => Analysis::Transient {
                        step: *step,
                        stop: *stop,
                        start,
                        max_step: None,
                        uic,
                    },
                    [step, stop, rest @ ..] if rest.len() <= 2 => Analysis::Transient {
                        step: *step,
                        stop: *stop,
                        start: rest.first().copied(),
                        max_step: rest.get(1).copied(),
                        uic,
                    },
                    _ => return None,
                }
            }
            Tag::Noise => {
                let nodes = self.texts(card, Tag::Node);
                let nums = self.all(card, Tag::Num);
                Analysis::Noise {
                    output: nodes.first()?.clone(),
                    reference: nodes.get(1).cloned().unwrap_or_else(|| "0".into()),
                    source: self.text(self.kid(card, Tag::Name)?).to_string(),
                    sweep: FreqSweep {
                        variation: self.text(self.kid(card, Tag::Word)?).parse().ok()?,
                        points: self.count(nums[0])?,
                        start: self.num(nums[1])?,
                        stop: self.num(nums[2])?,
                    },
                    points_per_summary: nums.get(3).and_then(|&r| self.text(r).parse().ok()),
                }
            }
            Tag::Tf => Analysis::Tf {
                output: word(0)?,
                source: word(1)?,
            },
            Tag::Sens => match k.as_slice() {
                [out] => Analysis::Sensitivity {
                    output: self.text(*out).to_string(),
                    ac: None,
                },
                [out, var, pts, a, b] => Analysis::Sensitivity {
                    output: self.text(*out).to_string(),
                    ac: Some(FreqSweep {
                        variation: self.text(*var).parse().ok()?,
                        points: self.count(*pts)?,
                        start: self.number(*a)?,
                        stop: self.number(*b)?,
                    }),
                },
                _ => return None,
            },
            Tag::Pz => Analysis::PoleZero {
                node1: word(0)?,
                node2: word(1)?,
                node3: word(2)?,
                node4: word(3)?,
                tf_type: word(4)?.parse().ok()?,
                pz_type: word(5)?.parse().ok()?,
            },
            Tag::Four => Analysis::Fourier {
                fundamental: n(0)?,
                outputs: self.texts(card, Tag::Out),
                num_harmonics: None,
            },
            _ => return None,
        })
    }
}
