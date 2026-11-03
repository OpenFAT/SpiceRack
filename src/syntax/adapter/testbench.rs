//! SPICE testbench cards (analyses, options, outputs, `.control`
//! commands) → `Testbench` rows, as `import/lower/analyses.rs` applies
//! them, each card first checked against its `grammar/spice.peg` rule
//! (`Cx::testbench_peg`).
//!
//! Cites: `import/lower/analyses.rs` (ported); spice.peg `Options`, `Temp`,
//! `Ic`, `Nodeset`, `Save`, `Out`, `Print`, `Meas`, `Op` … `Four`; ngspice.peg
//! `Sp`; hspice.peg `Tran`, `Probe`, `Out`; ltspice.peg `Tran`, `Step`;
//! xyce.peg `Options`, `Step`; Xyce Reference Guide p.96.
//!
//! Contents:
//! - `Cx::testbench_peg(&self, n: u32) -> bool`
//! - `Cx::testbench_card(&mut self, n: u32) -> bool`
//! - `Cx::head_kind(&self, n: u32) -> String`
//! - `Cx::numw(&self, k: u32) -> bool`
//! - `Cx::wordw(&self, k: u32) -> bool`

use super::Cx;
use super::outputs::print_kind;
use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

/// A testbench card's parts, as its grammar rule reads them.
enum Card {
    Op,
    /// `.dc`: the sweeps' `[source, start, stop, step]` items.
    Dc(Vec<[u32; 4]>),
    /// `.ac`, `.sp`: variation, points, start, stop.
    Freq(bool, [u32; 4]),
    /// `.tran`: its numbers, a `START=` pair, `uic`.
    Tran(Vec<u32>, Option<u32>, bool),
    /// `.noise v(out[,ref]) src var pts f1 f2 [pts_per_summary]`.
    Noise(Vec<String>, u32, [u32; 4], Option<u32>),
    /// `.tf out src`.
    Tf(String, String),
    /// `.sens out [ac var pts f1 f2]`.
    Sens(String, Option<[u32; 4]>),
    /// `.pz` six words.
    Pz([u32; 6]),
    /// `.four f outs`.
    Four(u32, Vec<String>),
    Options,
    Temp(u32),
    /// `.ic`/`.nodeset`: `(node item, value)`s.
    NodeVals(Vec<(u32, u32)>),
    Save(Vec<String>),
    Probe(Vec<String>),
    Print(String, Vec<String>),
    Meas,
    /// `.step [kind] [param] name a b c`.
    Step(Option<u32>, u32, [u32; 3]),
}

impl<'a> Cx<'a> {
    /// Whether the card's rule in the dialect's grammar reads it.
    pub(super) fn testbench_peg(&self, n: u32) -> bool {
        self.read_card(n).is_some()
    }

    /// The card applied to the testbench; `false` when the IR cannot hold
    /// it (lower/analyses.rs `testbench_card`).
    pub(super) fn testbench_card(&mut self, n: u32) -> bool {
        let Some(card) = self.read_card(n) else {
            return false;
        };
        if let Some(a) = self.analysis(&card) {
            self.tb.analyses.push(a);
            return true;
        }
        match card {
            Card::Options => self.options(n),
            Card::Probe(outs) => self.tb.saves.extend(outs),
            Card::Step(kind, name, nums) => {
                let sweep_type = kind.and_then(|s| self.text(s).parse().ok());
                let num = |k: u32| self.number(k).unwrap_or(f64::NAN);
                let step = StepParam {
                    param: self.text(name),
                    start: num(nums[0]),
                    stop: num(nums[1]),
                    step: num(nums[2]),
                    sweep_type,
                };
                if step.start.is_nan() || step.stop.is_nan() || step.step.is_nan() {
                    return false;
                }
                self.tb.step_params.push(step);
            }
            Card::Temp(v) => match self.number(v) {
                Some(t) => self.tb.temperature = Some(t),
                None => return false,
            },
            Card::NodeVals(vals) => {
                let v: Option<Vec<(String, f64)>> = vals
                    .iter()
                    .map(|&(k, v)| Some((self.name_of(k), self.number(v)?)))
                    .collect();
                let ic = self.head_kind(n) != "nodeset";
                match (v, ic) {
                    (Some(v), true) => self.tb.initial_conditions.extend(v),
                    (Some(v), false) => self.tb.node_sets.extend(v),
                    (None, _) => return false,
                }
            }
            Card::Save(outs) => {
                if !outs.iter().all(|x| x.eq_ignore_ascii_case("all")) {
                    self.tb.saves.extend(outs);
                }
            }
            Card::Print(analysis, outputs) => self.tb.prints.push(PrintCard { analysis, outputs }),
            Card::Meas => {
                let line = self.line(n);
                // A `.control` `meas` line was read as `.meas` (lower/analyses.rs:21).
                let line = if self.ttag(self.main(n)) == T::Ident {
                    format!(".{line}")
                } else {
                    line
                };
                self.tb.measures.push(line);
            }
            _ => return false,
        }
        true
    }

    /// The card's word: its dot keyword (or `.control` command) lowercased,
    /// without the dot.
    pub(super) fn head_kind(&self, n: u32) -> String {
        let w = self.t.text(self.main(n)).to_ascii_lowercase();
        w.strip_prefix('.').unwrap_or(&w).to_string()
    }

    pub(super) fn numw(&self, k: u32) -> bool {
        self.tag(k) == Tag::Word && self.number(k).is_some()
    }

    pub(super) fn wordw(&self, k: u32) -> bool {
        self.tag(k) == Tag::Word
    }
}

// ---- private ----

impl<'a> Cx<'a> {
    /// Whether every item follows the one before it (or the head) after
    /// blanks only: the grammar's `s` between fields.
    fn spaced(&self, n: u32, items: &[u32]) -> bool {
        let mut prev = self.main(n);
        for &k in items {
            let first = self.main(k);
            if first != prev + 1 || self.t.end(prev) == self.t.start(first) {
                return false;
            }
            prev = self.last(k);
        }
        true
    }

    /// The card's parts by its grammar rule; `None` when the rule does not
    /// read it.
    fn read_card(&self, n: u32) -> Option<Card> {
        let items = self.fields(n);
        let kind = self.head_kind(n);
        let hspice = self.d == Dialect::Hspice;
        let lt = self.d == Dialect::Ltspice;
        match self.tag(n) {
            Tag::Options => self.options_ok(n).then_some(Card::Options),
            Tag::Temp => match items {
                [v] if self.val_ok(*v) && self.tag(*v) != Tag::Seq => Some(Card::Temp(*v)),
                _ => None,
            },
            Tag::Ic | Tag::Nodeset => items
                .iter()
                .map(|&a| {
                    let g = self.ast.lhs(Node(a));
                    let v = self.ast.rhs(Node(a));
                    let ok = self.tag(a) == Tag::Assign
                        && self.tag(g) == Tag::Group
                        && self.group_closed(g)
                        && self.ast.lhs(Node(g)) != NONE
                        && self.text(self.ast.lhs(Node(g))).eq_ignore_ascii_case("v")
                        && self.val_ok(v);
                    let inside = if ok { self.group_items(g) } else { &[] };
                    match inside {
                        [node] if self.wordw(*node) => Some((*node, v)),
                        _ => None,
                    }
                })
                .collect::<Option<Vec<_>>>()
                .map(Card::NodeVals),
            Tag::Save if kind == "probe" => self.probe_outs(n).map(Card::Probe),
            Tag::Save => self.outs_after(n, 0).map(Card::Save),
            Tag::Print => {
                let rest = self.rest_text(n);
                let (pk, after) = print_kind(&rest);
                let outs = self.outs_text(after)?;
                Some(Card::Print(pk.to_ascii_lowercase(), outs))
            }
            Tag::Measure => Some(Card::Meas),
            Tag::Step => self.step(n, items),
            Tag::Analysis => match kind.as_str() {
                "op" => Some(Card::Op),
                "dc" => {
                    if items.is_empty() || !items.len().is_multiple_of(4) || !self.spaced(n, items)
                    {
                        return None;
                    }
                    let sweeps: Vec<[u32; 4]> =
                        items.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
                    sweeps
                        .iter()
                        .all(|s| self.wordw(s[0]) && s[1..].iter().all(|&k| self.numw(k)))
                        .then_some(Card::Dc(sweeps))
                }
                "ac" | "sp" => match items {
                    [v, a, b, c]
                        if self.spaced(n, items)
                            && self.wordw(*v)
                            && [a, b, c].iter().all(|&&k| self.numw(k)) =>
                    {
                        Some(Card::Freq(kind == "sp", [*v, *a, *b, *c]))
                    }
                    _ => None,
                },
                "tran" => self.tran(n, items, hspice, lt),
                "noise" => self.noise(n, items),
                "tf" => {
                    let rest = self.rest_text(n);
                    let (out, after) = self.one_out(&rest)?;
                    let src = after
                        .strip_prefix([' ', '\t'])?
                        .trim_start_matches([' ', '\t']);
                    let src = src.trim_end_matches([' ', '\t']);
                    (!src.is_empty() && !src.contains([' ', '\t', ',', '=', '(', ')', '\'', '"']))
                        .then(|| Card::Tf(out, src.to_string()))
                }
                "sens" => self.sens(n),
                "pz" => match items {
                    [a, b, c, d, e, f]
                        if self.spaced(n, items) && items.iter().all(|&k| self.wordw(k)) =>
                    {
                        Some(Card::Pz([*a, *b, *c, *d, *e, *f]))
                    }
                    _ => None,
                },
                "four" | "fourier" => {
                    let (&f, _) = items.split_first()?;
                    if !self.numw(f) || !self.spaced(n, &[f]) {
                        return None;
                    }
                    let rest = self.span(self.last(f) + 1, self.line_last(self.main(n)));
                    let outs = self.outs_text(&format!(" {rest}"))?;
                    (!outs.is_empty()).then_some(Card::Four(f, outs))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn tran(&self, n: u32, items: &[u32], hspice: bool, lt: bool) -> Option<Card> {
        let (body, uic) = match items.split_last() {
            Some((&u, rest)) if self.wordw(u) && self.text(u).eq_ignore_ascii_case("uic") => {
                (rest, true)
            }
            _ => (items, false),
        };
        if !self.spaced(n, items) {
            return None;
        }
        let (nums, start) = match body.split_last() {
            Some((&p, rest)) if hspice && self.tag(p) == Tag::Assign => (rest, Some(p)),
            _ => (body, None),
        };
        let lo = if lt { 1 } else { 2 };
        let hi = if hspice { 2 } else { 4 };
        if nums.len() < lo || nums.len() > hi || !nums.iter().all(|&k| self.numw(k)) {
            return None;
        }
        if start.is_some_and(|p| {
            !self.wordw(self.ast.lhs(Node(p))) || !self.val_ok(self.ast.rhs(Node(p)))
        }) {
            return None;
        }
        Some(Card::Tran(nums.to_vec(), start, uic))
    }

    fn noise(&self, n: u32, items: &[u32]) -> Option<Card> {
        let [g, src, var, a, b, c, rest @ ..] = items else {
            return None;
        };
        if rest.len() > 1
            || self.tag(*g) != Tag::Group
            || !self.group_closed(*g)
            || !self.spaced(n, items)
        {
            return None;
        }
        let gn = self.ast.lhs(Node(*g));
        if gn == NONE || !self.text(gn).eq_ignore_ascii_case("v") {
            return None;
        }
        let nodes = self.group_items(*g);
        if nodes.is_empty() || nodes.len() > 2 || !nodes.iter().all(|&k| self.wordw(k)) {
            return None;
        }
        if !self.wordw(*src)
            || !self.wordw(*var)
            || ![a, b, c].iter().all(|&&k| self.numw(k))
            || rest.iter().any(|&k| !self.numw(k))
        {
            return None;
        }
        Some(Card::Noise(
            nodes.iter().map(|&k| self.text(k)).collect(),
            *src,
            [*var, *a, *b, *c],
            rest.first().copied(),
        ))
    }

    fn sens(&self, n: u32) -> Option<Card> {
        let rest = self.rest_text(n);
        let (out, after) = self.one_out(&rest)?;
        let words: Vec<&str> = after.split([' ', '\t']).filter(|w| !w.is_empty()).collect();
        if !after.is_empty() && !after.starts_with([' ', '\t']) {
            return None;
        }
        match words.as_slice() {
            [] => Some(Card::Sens(out, None)),
            [dc] if dc.eq_ignore_ascii_case("dc") => Some(Card::Sens(out, None)),
            [ac, ..] if ac.eq_ignore_ascii_case("ac") => {
                let items = self.fields(n);
                let tail = &items[items.len().checked_sub(4)?..];
                (tail.iter().skip(1).all(|&k| self.numw(k))
                    && self.wordw(tail[0])
                    && words.len() == 5)
                    .then(|| Card::Sens(out, Some([tail[0], tail[1], tail[2], tail[3]])))
            }
            _ => None,
        }
    }

    fn step(&self, n: u32, items: &[u32]) -> Option<Card> {
        let mut it = items;
        let kind = match it.first() {
            Some(&k)
                if ["lin", "dec", "oct"]
                    .iter()
                    .any(|w| self.wordw(k) && self.text(k).eq_ignore_ascii_case(w)) =>
            {
                it = &it[1..];
                Some(k)
            }
            _ => None,
        };
        if self.d == Dialect::Ltspice {
            let (&p, rest) = it.split_first()?;
            if !(self.wordw(p) && self.text(p).eq_ignore_ascii_case("param")) {
                return None;
            }
            it = rest;
        }
        match it {
            [name, a, b, c]
                if self.spaced(n, items)
                    && self.wordw(*name)
                    && [a, b, c].iter().all(|&&k| self.numw(k)) =>
            {
                Some(Card::Step(kind, *name, [*a, *b, *c]))
            }
            _ => None,
        }
    }

    /// An analysis card's IR form (lower/analyses.rs `analysis`).
    fn analysis(&self, card: &Card) -> Option<Analysis> {
        let num = |k: u32| self.number(k);
        let sweep = |f: &[u32; 4]| -> Option<FreqSweep> {
            Some(FreqSweep {
                variation: self.text(f[0]).parse().ok()?,
                points: self.count(f[1])?,
                start: num(f[2])?,
                stop: num(f[3])?,
            })
        };
        Some(match card {
            Card::Op => Analysis::Op,
            Card::Dc(sweeps) => Analysis::Dc {
                sweeps: sweeps
                    .iter()
                    .map(|s| {
                        Some(DcSweep {
                            source: self.text(s[0]),
                            start: num(s[1])?,
                            stop: num(s[2])?,
                            step: num(s[3])?,
                        })
                    })
                    .collect::<Option<_>>()?,
            },
            Card::Freq(false, f) => Analysis::Ac { sweep: sweep(f)? },
            Card::Freq(true, f) => Analysis::SPar { sweep: sweep(f)? },
            Card::Tran(nums, pair, uic) => {
                let p: Vec<f64> = nums.iter().map(|&k| num(k)).collect::<Option<_>>()?;
                let start = match pair {
                    Some(pr) if self.key(*pr).eq_ignore_ascii_case("start") => {
                        Some(num(self.ast.rhs(Node(*pr)))?)
                    }
                    Some(_) => return None,
                    None => None,
                };
                let uic = *uic;
                match p.as_slice() {
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
            Card::Noise(nodes, src, f, pps) => Analysis::Noise {
                output: nodes.first()?.clone(),
                reference: nodes.get(1).cloned().unwrap_or_else(|| "0".into()),
                source: self.text(*src),
                sweep: FreqSweep {
                    variation: self.text(f[0]).parse().ok()?,
                    points: self.count(f[1])?,
                    start: num(f[2])?,
                    stop: num(f[3])?,
                },
                points_per_summary: pps.and_then(|k| self.text(k).parse().ok()),
            },
            Card::Tf(out, src) => Analysis::Tf {
                output: out.clone(),
                source: src.clone(),
            },
            Card::Sens(out, ac) => Analysis::Sensitivity {
                output: out.clone(),
                ac: match ac {
                    Some(f) => Some(sweep(f)?),
                    None => None,
                },
            },
            Card::Pz(w) => Analysis::PoleZero {
                node1: self.text(w[0]),
                node2: self.text(w[1]),
                node3: self.text(w[2]),
                node4: self.text(w[3]),
                tf_type: self.text(w[4]).parse().ok()?,
                pz_type: self.text(w[5]).parse().ok()?,
            },
            Card::Four(f, outs) => Analysis::Fourier {
                fundamental: num(*f)?,
                outputs: outs.clone(),
                num_harmonics: None,
            },
            _ => return None,
        })
    }
}
