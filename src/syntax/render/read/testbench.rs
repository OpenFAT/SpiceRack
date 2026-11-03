//! SPICE testbench cards (analyses, options, outputs, `.control`
//! commands) → [`BenchCard`] views, what each adds to the testbench as the
//! old lowering applied them, each card first checked against its rule in
//! the old SPICE grammar (`Rd::testbench_peg`).
//!
//! Cites: the old lowering (as `syntax::adapter::testbench` ports it); the old SPICE grammar's `Options`, `Temp`,
//! `Ic`, `Nodeset`, `Save`, `Out`, `Print`, `Meas`, `Op` … `Four`; the old ngspice grammar
//! `Sp`; the old HSPICE grammar's `Tran`, `Probe`, `Out`; the old LTspice grammar's `Tran`, `Step`;
//! the old Xyce grammar's `Options`, `Step`; Xyce Reference Guide p.96.
//!
//! Contents:
//! - `pub(in crate::syntax::render) enum BenchCard`, `enum Ana`, `struct Sweep`
//! - `Rd::testbench_peg(&self, n: u32) -> bool`, `Rd::testbench_ok(&self, n: u32) -> bool`
//! - `Rd::bench_card(&self, n: u32) -> Option<BenchCard>`
//! - `Rd::head_kind(&self, n: u32) -> String`
//! - `Rd::numw(&self, k: u32) -> bool`
//! - `Rd::wordw(&self, k: u32) -> bool`

use super::Rd;
use super::options::Opt;
use super::outputs::print_kind;
use crate::codegen::Dialect;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

/// What one testbench card adds to the deck's testbench (the old
/// lowering's `testbench_card`).
#[derive(Clone, Debug)]
pub(in crate::syntax::render) enum BenchCard {
    Ana(Ana),
    Options(Vec<Opt>),
    /// Outputs to save.
    Saves(Vec<String>),
    /// `.step [lin|dec|oct] name start stop step`.
    Step {
        sweep: Option<&'static str>,
        param: String,
        nums: [f64; 3],
    },
    Temp(f64),
    /// `.ic` (`true`) or `.nodeset` values.
    NodeVals(bool, Vec<(String, f64)>),
    /// `.print [kind] outputs`.
    Print(String, Vec<String>),
    /// A `.meas` card's line.
    Meas(String),
}

/// An analysis as the old reader kept it.
#[derive(Clone, Debug)]
pub(in crate::syntax::render) enum Ana {
    Op,
    /// Each sweep's source, start, stop, step.
    Dc(Vec<(String, f64, f64, f64)>),
    Ac(Sweep),
    Sp(Sweep),
    Tran {
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
        sweep: Sweep,
        pps: Option<u32>,
    },
    Tf(String, String),
    Sens(String, Option<Sweep>),
    /// Four nodes, the transfer and pole-zero keywords.
    Pz([String; 4], &'static str, &'static str),
    Four(f64, Vec<String>),
}

/// A frequency sweep: its variation keyword (lowercase), points, start,
/// stop.
#[derive(Clone, Debug)]
pub(in crate::syntax::render) struct Sweep {
    pub(in crate::syntax::render) variation: &'static str,
    pub(in crate::syntax::render) points: u32,
    pub(in crate::syntax::render) start: f64,
    pub(in crate::syntax::render) stop: f64,
}

/// The sweep keywords (`Sweep` in the IR).
const SWEEPS: &[&str] = &["dec", "oct", "lin", "log"];

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

impl<'a> Rd<'a> {
    /// Whether the card's rule in the dialect's grammar reads it.
    pub(in crate::syntax::render) fn testbench_peg(&self, n: u32) -> bool {
        self.read_card(n).is_some()
    }

    /// Whether the IR the old reader built could hold the card (the old
    /// lowering's `testbench_card` returned `true`).
    pub(in crate::syntax::render) fn testbench_ok(&self, n: u32) -> bool {
        self.bench_card(n).is_some()
    }

    /// What the card adds to the testbench; `None` when the IR cannot hold
    /// it (the old lowering's `testbench_card`).
    pub(in crate::syntax::render) fn bench_card(&self, n: u32) -> Option<BenchCard> {
        let card = self.read_card(n)?;
        if let Some(a) = self.analysis(&card) {
            return Some(BenchCard::Ana(a));
        }
        Some(match card {
            Card::Options => BenchCard::Options(self.options_view(n)),
            Card::Probe(outs) => BenchCard::Saves(outs),
            Card::Step(kind, name, nums) => {
                let sweep = kind.and_then(|s| keyword(&self.text(s), SWEEPS));
                let num = |k: u32| self.number(k).unwrap_or(f64::NAN);
                let nums = [num(nums[0]), num(nums[1]), num(nums[2])];
                if nums.iter().any(|x| x.is_nan()) {
                    return None;
                }
                BenchCard::Step {
                    sweep,
                    param: self.text(name),
                    nums,
                }
            }
            Card::Temp(v) => BenchCard::Temp(self.number(v)?),
            Card::NodeVals(vals) => {
                let v: Vec<(String, f64)> = vals
                    .iter()
                    .map(|&(k, v)| Some((self.name_of(k), self.number(v)?)))
                    .collect::<Option<_>>()?;
                BenchCard::NodeVals(self.head_kind(n) != "nodeset", v)
            }
            Card::Save(outs) if outs.iter().all(|x| x.eq_ignore_ascii_case("all")) => {
                BenchCard::Saves(vec![])
            }
            Card::Save(outs) => BenchCard::Saves(outs),
            Card::Print(analysis, outputs) => BenchCard::Print(analysis, outputs),
            Card::Meas => {
                let line = self.line(n);
                // A `.control` `meas` line was read as `.meas` (the old lowering).
                BenchCard::Meas(if self.ttag(self.main(n)) == T::Ident {
                    format!(".{line}")
                } else {
                    line
                })
            }
            _ => return None,
        })
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

impl<'a> Rd<'a> {
    /// quirk: whether every item follows the one before it (or the head) after
    /// blanks only: the old grammars' `s` between fields.
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

    /// An analysis card's view (the old lowering's `analysis`).
    fn analysis(&self, card: &Card) -> Option<Ana> {
        let num = |k: u32| self.number(k);
        let sweep = |f: &[u32; 4]| -> Option<Sweep> {
            Some(Sweep {
                variation: keyword(&self.text(f[0]), SWEEPS)?,
                points: self.count(f[1])?,
                start: num(f[2])?,
                stop: num(f[3])?,
            })
        };
        Some(match card {
            Card::Op => Ana::Op,
            Card::Dc(sweeps) => Ana::Dc(
                sweeps
                    .iter()
                    .map(|s| Some((self.text(s[0]), num(s[1])?, num(s[2])?, num(s[3])?)))
                    .collect::<Option<_>>()?,
            ),
            Card::Freq(false, f) => Ana::Ac(sweep(f)?),
            Card::Freq(true, f) => Ana::Sp(sweep(f)?),
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
                    [stop] => Ana::Tran {
                        step: 0.0,
                        stop: *stop,
                        start: None,
                        max_step: None,
                        uic,
                    },
                    [step, stop] if start.is_some() => Ana::Tran {
                        step: *step,
                        stop: *stop,
                        start,
                        max_step: None,
                        uic,
                    },
                    [step, stop, rest @ ..] if rest.len() <= 2 => Ana::Tran {
                        step: *step,
                        stop: *stop,
                        start: rest.first().copied(),
                        max_step: rest.get(1).copied(),
                        uic,
                    },
                    _ => return None,
                }
            }
            Card::Noise(nodes, src, f, pps) => Ana::Noise {
                output: nodes.first()?.clone(),
                reference: nodes.get(1).cloned().unwrap_or_else(|| "0".into()),
                source: self.text(*src),
                sweep: sweep(f)?,
                pps: pps.and_then(|k| self.text(k).parse().ok()),
            },
            Card::Tf(out, src) => Ana::Tf(out.clone(), src.clone()),
            Card::Sens(out, ac) => Ana::Sens(
                out.clone(),
                match ac {
                    Some(f) => Some(sweep(f)?),
                    None => None,
                },
            ),
            Card::Pz(w) => Ana::Pz(
                [
                    self.text(w[0]),
                    self.text(w[1]),
                    self.text(w[2]),
                    self.text(w[3]),
                ],
                keyword(&self.text(w[4]), &["vol", "cur"])?,
                keyword(&self.text(w[5]), &["pol", "zer", "pz"])?,
            ),
            Card::Four(f, outs) => Ana::Four(num(*f)?, outs.clone()),
            _ => return None,
        })
    }
}

/// `w` as the keyword of `set` it names in any case.
fn keyword(w: &str, set: &[&'static str]) -> Option<&'static str> {
    set.iter().copied().find(|k| k.eq_ignore_ascii_case(w))
}
