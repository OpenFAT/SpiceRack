//! The deck-wide refusals the SPICE-family writer makes before it writes: a
//! statement kept as another family's text (`ForeignText`), a `^` the
//! target reads with the other meaning (`CaretReread`), a statistical call
//! the target has no function for (`StatisticalCall`). Each reads every
//! block the deck writes (definitions, sections, branches) and every
//! expression the old reader kept as text.
//!
//! Cites: `docs/research/transpiler-architecture.md` §2.3 (`Verbatim`),
//! §3 (`^`); ngspice manual §2.8.5, §2.11.5; LTspice help (`^` XOR,
//! `gauss(x)`); Xyce RG §2.1.18.
//!
//! Contents:
//! - `W::checks(&mut self)`

use super::W;
use crate::codegen::Dialect;
use crate::syntax::render::capability::{Action, Construct};
use crate::syntax::render::read::{Block, Card, Comp, Func, Num, Val};

/// The statistical functions (ngspice §2.11.5).
const STATISTICAL: [&str; 5] = ["agauss", "gauss", "aunif", "unif", "limit"];

impl W<'_, '_> {
    /// Records the deck-wide refusals.
    pub(super) fn checks(&mut self) {
        let blocks = self.blocks();
        // Text kept from another family reaches only its own.
        if self.rd.d.family() != self.d.family()
            && let Some(tok) = blocks.iter().find_map(|b| self.first_raw(b))
        {
            self.ask(Construct::ForeignText, tok);
        }
        let caret = caret_means_power(self.rd.d) != caret_means_power(self.d);
        let stat = crate::syntax::render::capability::action(self.d, Construct::StatisticalCall)
            != Action::Native;
        if !caret && !stat {
            return;
        }
        let exprs = self.expressions(&blocks);
        if caret && exprs.iter().any(|e| e.contains('^')) {
            self.ask(Construct::CaretReread, 0);
        }
        let is_stat = |f: &str| STATISTICAL.iter().any(|s| s.eq_ignore_ascii_case(f));
        if stat && exprs.iter().any(|e| calls(e).any(is_stat)) {
            self.ask(Construct::StatisticalCall, 0);
        }
    }
}

// ---- private ----

impl<'r> W<'_, 'r> {
    /// Every block the deck writes from: the top, its definitions, each
    /// section's body and definitions, and the branches inside each.
    fn blocks(&self) -> Vec<&'r Block> {
        let deck = self.deck;
        let sections = deck
            .sections
            .iter()
            .flat_map(|s| std::iter::once(&s.body).chain(&s.defs));
        let mut out = Vec::new();
        let mut work: Vec<&Block> = std::iter::once(&deck.top)
            .chain(&deck.defs)
            .chain(sections)
            .collect();
        work.reverse();
        while let Some(b) = work.pop() {
            out.push(b);
            let branches = b.conds.iter().flat_map(|(_, c)| &c.branches);
            let mut inner: Vec<&Block> = branches.map(|(_, body)| body).collect();
            inner.reverse();
            work.extend(inner);
        }
        out
    }

    /// The first token of block `b`'s first statement kept as text.
    fn first_raw(&self, b: &Block) -> Option<u32> {
        let line = b.raw.first().map(|&r| match r {
            crate::syntax::render::read::Raw::Line(n)
            | crate::syntax::render::read::Raw::Lines(n)
            | crate::syntax::render::read::Raw::Joined(n) => n,
        });
        let comp = || {
            b.comps.iter().find_map(|c| match c {
                Comp::Raw(n) => Some(*n),
                Comp::Card(_) => None,
            })
        };
        line.or_else(comp).map(|n| self.rd.main(n))
    }

    /// Every expression the deck's cards carry as text: functions,
    /// parameters, model, instance and element values. Text kept as lines
    /// is not: it reaches its own dialect only.
    fn expressions(&mut self, blocks: &[&Block]) -> Vec<String> {
        let deck = self.deck;
        let funcs = deck
            .funcs
            .iter()
            .chain(deck.sections.iter().flat_map(|s| &s.funcs));
        let mut out: Vec<String> = funcs.map(|&f: &Func| self.rd.func_view(f).2).collect();
        for b in blocks {
            if b.node != crate::syntax::ast::NONE {
                let head = self
                    .rd
                    .subckt_head(b.node)
                    .expect("the reader read the head");
                out.extend(head.params.into_iter().map(|(_, v)| v));
            }
            for &p in b.params.iter().chain(&b.locals) {
                out.push(self.rd.param_view(p).1);
            }
            for &(m, bin) in &b.models {
                out.extend(
                    self.rd
                        .model_view(m, bin)
                        .params
                        .into_iter()
                        .map(|(_, v)| v),
                );
            }
            for &(_, i) in &b.insts {
                if let Card::Inst { params, .. } = self.rd.card_view(i) {
                    out.extend(params.into_iter().map(|(_, v)| v));
                }
            }
            for &c in &b.comps {
                if let Comp::Card(n) = c {
                    card_exprs(self.rd.card_view(n), &mut out);
                }
            }
            if !b.statistics.is_empty() {
                // ponytail: the statistics lowering (`Lift(StatisticsDraws)`)
                // is not ported: a deck holding a block is refused, never
                // written without its draws.
                if let Action::Lift(_) = self.ask(Construct::Statistics, 0) {
                    self.refuse(
                        crate::syntax::diag::Code::W0018,
                        self.rd.main(b.statistics[0]),
                    );
                }
            }
        }
        out
    }
}

/// The expressions one element card carries (the old IR's
/// `Component::exprs`): its parameters, its waveform's expressions, an
/// expression value, a behavioural source's or charge's expression.
fn card_exprs(c: Card, out: &mut Vec<String>) {
    let expr = |v: Val| match v {
        Val::Expr(e) => Some(e),
        Val::Num(_) | Val::Raw(_) => None,
    };
    match c {
        Card::Rcl { value, params, .. } => {
            out.extend(params.into_iter().map(|(_, v)| v));
            out.extend(expr(value));
        }
        Card::Source { value, wave, .. } => {
            if let Some(w) = wave {
                let nums = w.args.into_iter().chain(w.repeat).chain(w.delay);
                out.extend(nums.filter_map(|n| match n {
                    Num::Expr(e) => Some(e),
                    Num::Num(_) => None,
                }));
            }
            out.extend(expr(value));
        }
        Card::Behavioural { expr, .. } => out.push(expr),
        Card::Charge { charge, params, .. } => {
            out.push(charge);
            out.extend(params.into_iter().map(|(_, v)| v));
        }
        Card::Semi { params, .. } => out.extend(params.into_iter().map(|(_, v)| v)),
        Card::Coupling { .. }
        | Card::Linear { .. }
        | Card::Poly { .. }
        | Card::Switch { .. }
        | Card::Line { .. }
        | Card::Inst { .. } => {}
    }
}

/// `^` means power in ngspice, HSPICE, ESPice and Spectre, but XOR in
/// LTspice, Xyce and VACASK.
fn caret_means_power(d: Dialect) -> bool {
    !matches!(d, Dialect::Ltspice | Dialect::Xyce | Dialect::Vacask)
}

/// The names `e` calls: each identifier followed by `(`.
// ponytail: a scan over the kept text, as the old writer's `calls`; the
// expression tree's `Call` nodes when the reader keeps nodes, not text.
fn calls(e: &str) -> impl Iterator<Item = &str> {
    let b = e.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    (0..b.len()).filter_map(move |i| {
        let starts = b[i].is_ascii_alphabetic() && (i == 0 || !ident(b[i - 1]) && b[i - 1] != b'.');
        if !starts {
            return None;
        }
        let end = i + b[i..].iter().take_while(|&&c| ident(c)).count();
        let open = end
            + b[end..]
                .iter()
                .take_while(|c| c.is_ascii_whitespace())
                .count();
        (b.get(open) == Some(&b'(')).then(|| &e[i..end])
    })
}
