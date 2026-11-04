//! The deck-wide refusals the named writer makes before it writes: text
//! kept from another dialect (`ForeignText`; a deck's own text passes), a
//! flag (a parameter with no value: Spectre takes `name=value` only), a
//! `^` the target reads with the other meaning (`CaretReread`).
//!
//! Cites: Spectre Reference 19.1 p.506 (`name=value`); ngspice manual
//! §2.8.5; LTspice help, Xyce RG §2.1.18 (`^` XOR).
//!
//! Contents:
//! - `W::checks(&mut self)`, `fn calls(e: &str) -> impl Iterator<Item = &str>`

use super::W;
use crate::syntax::Dialect;
use crate::syntax::ast::NONE;
use crate::syntax::diag::Code;
use crate::syntax::render::capability::Construct;
use crate::syntax::render::read::{Block, Card, Comp, Func, Num, Raw, Val};

impl W<'_, '_> {
    /// Records the deck-wide refusals.
    pub(super) fn checks(&mut self) {
        let blocks = self.blocks();
        // Text kept from another dialect reaches only its own, which
        // copies it unchecked.
        let raws: Vec<u32> = blocks.iter().flat_map(|b| self.raws(b)).collect();
        match self.rd.d != self.d {
            true => {
                if let Some(&tok) = raws.first() {
                    self.ask(Construct::ForeignText, tok);
                }
            }
            false => raws.iter().for_each(|&tok| self.note(Code::W0027, tok)),
        }
        // A waveform's values past its form's count are not written.
        for b in &blocks {
            for &c in &b.comps {
                if let Comp::Card(n) = c
                    && let Card::Source { wave: Some(w), .. } = self.rd.card_view(n)
                    && w.cut
                {
                    self.note(Code::W0026, self.rd.main(n));
                }
            }
        }
        if let Some(tok) = blocks.iter().find_map(|b| self.first_flag(b)) {
            self.refuse(Code::W0101, tok);
        }
        if caret_means_power(self.rd.d) != caret_means_power(self.d)
            && let Some((_, tok)) = self
                .expressions(&blocks)
                .into_iter()
                .find(|(e, _)| e.contains('^'))
        {
            self.ask(Construct::CaretReread, tok);
        }
    }
}

/// The names `e` calls: each identifier followed by `(`.
// ponytail: a scan over the kept text, as the old writer's `calls`; the
// expression tree's `Call` nodes when the views keep nodes, not text.
pub(super) fn calls(e: &str) -> impl Iterator<Item = &str> {
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
        let mut work: Vec<&Block> = std::iter::once(&deck.top)
            .chain(&deck.defs)
            .chain(sections)
            .collect();
        work.reverse();
        let mut out = Vec::new();
        while let Some(b) = work.pop() {
            out.push(b);
            let branches = b.conds.iter().flat_map(|(_, c)| &c.branches);
            let mut inner: Vec<&Block> = branches.map(|(_, body)| body).collect();
            inner.reverse();
            work.extend(inner);
        }
        out
    }

    /// A function's first token.
    fn func_token(&self, f: Func) -> u32 {
        let (Func::Param(n) | Func::Card(n) | Func::Real(n)) = f;
        self.rd.main(n)
    }

    /// The first `statistics` statement's first token, where the draws it
    /// lowers to anchor; 0 when the blocks hold none.
    fn statistics_token(&self, blocks: &[&Block]) -> u32 {
        let st = blocks.iter().find_map(|b| b.statistics.first());
        st.map_or(0, |&n| self.rd.main(n))
    }

    /// The first token of each of block `b`'s statements kept as text: its
    /// lines, then its elements.
    fn raws(&self, b: &Block) -> Vec<u32> {
        let lines = b.raw.iter().map(|&r| match r {
            Raw::Line(n) | Raw::Lines(n) | Raw::Joined(n) => n,
        });
        let comps = b.comps.iter().filter_map(|c| match c {
            Comp::Raw(n) => Some(*n),
            Comp::Card(_) => None,
        });
        lines.chain(comps).map(|n| self.rd.main(n)).collect()
    }

    /// The first token of block `b`'s first model or element with a flag.
    fn first_flag(&self, b: &Block) -> Option<u32> {
        for &(m, bin) in &b.models {
            if self
                .rd
                .model_view(m, bin)
                .params
                .iter()
                .any(|(_, v)| v.is_empty())
            {
                return Some(self.rd.main(m));
            }
        }
        for &c in &b.comps {
            let Comp::Card(n) = c else { continue };
            let params = match self.rd.card_view(n) {
                Card::Rcl { params, .. }
                | Card::Charge { params, .. }
                | Card::Semi { params, .. } => params,
                _ => continue,
            };
            if params.iter().any(|(_, v)| v.is_empty()) {
                return Some(self.rd.main(n));
            }
        }
        None
    }

    /// Every expression the deck's statements carry as text: the
    /// functions, parameters, model, instance and element values (the old
    /// IR's expressions). Text kept as lines is not: it reaches its own
    /// dialect only.
    fn expressions(&self, blocks: &[&Block]) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = self
            .deck
            .funcs
            .iter()
            .filter(|&&f| self.lifted.writes(f))
            .map(|&f| (self.rd.func_view(f).2, self.func_token(f)))
            .collect();
        let at = self.statistics_token(blocks);
        out.extend(self.draws.texts().map(|t| (t.to_string(), at)));
        for b in blocks {
            if b.node != NONE {
                let head = self
                    .rd
                    .subckt_head(b.node)
                    .expect("the reader read the head");
                let tok = self.rd.main(b.node);
                out.extend(head.params.into_iter().map(|(_, v)| (v, tok)));
            }
            for &p in b.params.iter().chain(&b.locals) {
                out.push((self.rd.param_view(p).1, self.rd.main(p)));
            }
            for &(m, bin) in &b.models {
                let tok = self.rd.main(m);
                let params = self.rd.model_view(m, bin).params;
                out.extend(params.into_iter().map(|(_, v)| (v, tok)));
            }
            for &(_, i) in &b.insts {
                if let Card::Inst { params, .. } = self.rd.card_view(i) {
                    let tok = self.rd.main(i);
                    out.extend(params.into_iter().map(|(_, v)| (v, tok)));
                }
            }
            for &c in &b.comps {
                if let Comp::Card(n) = c {
                    card_exprs(self.rd.card_view(n), self.rd.main(n), &mut out);
                }
            }
        }
        out
    }
}

/// The expressions one element carries (the old IR's `Component::exprs`).
fn card_exprs(c: Card, tok: u32, out: &mut Vec<(String, u32)>) {
    let expr = |v: Val| match v {
        Val::Expr(e) => Some((e, tok)),
        Val::Num(_) | Val::Raw(_) => None,
    };
    let at = |(_, v): (String, String)| (v, tok);
    match c {
        Card::Rcl { value, params, .. } => {
            out.extend(params.into_iter().map(at));
            out.extend(expr(value));
        }
        Card::Source { value, wave, .. } => {
            if let Some(w) = wave {
                let nums = w.args.into_iter().chain(w.repeat).chain(w.delay);
                out.extend(nums.filter_map(|n| match n {
                    Num::Expr(e) => Some((e, tok)),
                    Num::Num(_) => None,
                }));
            }
            out.extend(expr(value));
        }
        Card::Behavioural { expr, .. } => out.push((expr, tok)),
        Card::Charge { charge, params, .. } => {
            out.push((charge, tok));
            out.extend(params.into_iter().map(at));
        }
        Card::Semi { params, .. } => out.extend(params.into_iter().map(at)),
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
