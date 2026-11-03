//! POLY sources, `B` sources, semiconductors and `X` cards → [`Card`]
//! views (as the old lowering typed them), each checked against its
//! old SPICE grammar rule: `Poly`, `B` and its `Body` (the text after
//! `key =` to the card's end), `nodes`/`inst`, `X`.
//!
//! Cites: the old lowering (as `syntax::adapter::devices` ports it); the old SPICE grammar's `Poly`, `B`,
//! `Body`, `D` … `Z`, `nodes`, `inst`, `X`.
//!
//! Contents:
//! - `Rd::poly(&self, n: u32, name: String) -> Option<Option<Card>>`
//! - `Rd::behavioural(&self, n: u32, name: String) -> Option<Card>`
//! - `Rd::body_to_end(&self, kv: u32) -> Option<String>`
//! - `Rd::semiconductor(&self, n: u32, e: Element, name: String) -> Option<Card>`
//! - `Rd::instance(&self, n: u32) -> Option<Card>`

use super::{Card, Rd};
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::device::Primitive as Element;
use crate::syntax::token::Tag as T;

impl<'a> Rd<'a> {
    /// `E o 0 POLY(n) controls... coeffs...` (and G, F, H); `None` when the
    /// card is no POLY card, `Some(None)` when it is one the IR cannot hold.
    pub(in crate::syntax::render) fn poly(&self, n: u32, name: String) -> Option<Option<Card>> {
        let items = &self.items(n)[..];
        let [a, b, p, rest @ ..] = items else {
            return None;
        };
        if !self.is_word(*a) || !self.is_word(*b) || self.tag(*p) != Tag::Group {
            return None;
        }
        let gname = self.ast.lhs(Node(*p));
        if gname == NONE || !self.word_is(gname, "poly") || !self.group_closed(*p) {
            return None;
        }
        let inside = self.group_items(*p);
        let [dims] = inside[..] else { return None };
        if !self.is_word(dims)
            || self.number(dims).is_none()
            || rest.is_empty()
            || !rest.iter().all(|&w| self.is_word(w))
        {
            return None;
        }
        let card = (|| {
            let letter = self
                .t
                .text(self.main(n))
                .chars()
                .next()?
                .to_ascii_uppercase();
            let dims = self.count(dims)? as usize;
            let n_controls = if matches!(letter, 'E' | 'G') {
                2 * dims
            } else {
                dims
            };
            let controls = rest
                .get(..n_controls)?
                .iter()
                .map(|&w| self.text(w))
                .collect();
            let coeffs = rest[n_controls..]
                .iter()
                .map(|&w| self.number(w))
                .collect::<Option<_>>()?;
            Some(Card::Poly {
                name,
                letter,
                np: self.text(*a),
                nm: self.text(*b),
                controls,
                coeffs,
            })
        })();
        Some(card)
    }

    /// `B1 n p V=expr`: the expression to the card's end (the old SPICE grammar's `B`,
    /// `Body`).
    pub(in crate::syntax::render) fn behavioural(&self, n: u32, name: String) -> Option<Card> {
        let items = &self.items(n)[..];
        let [a, b, kv, ..] = items else { return None };
        if !self.is_word(*a)
            || !self.is_word(*b)
            || self.tag(*kv) != Tag::Assign
            || !self.is_word(self.ast.lhs(Node(*kv)))
        {
            return None;
        }
        let expression = self.body_to_end(*kv)?;
        let (np, nm) = (self.text(*a), self.text(*b));
        let current = match self.key(*kv).to_ascii_lowercase().as_str() {
            "v" => false,
            "i" => true,
            _ => return None,
        };
        Some(Card::Behavioural {
            current,
            name,
            np,
            nm,
            expr: expression,
            value: self.ast.rhs(Node(*kv)),
        })
    }

    /// quirk: the old SPICE grammar's `Body`: the text after `key =` to the
    /// card's end, unless a ` key=` follows; braces (or a readable
    /// expression) around all of it are dropped.
    pub(in crate::syntax::render) fn body_to_end(&self, kv: u32) -> Option<String> {
        let eq = self.last(self.ast.lhs(Node(kv))) + 1;
        let first = eq + 1;
        let last = self.line_last(self.main(kv));
        if first > last {
            return None;
        }
        for k in first + 1..=last {
            let gap = self.t.end(k - 1) != self.t.start(k);
            let key = self.ttag(k) == T::Ident
                && self
                    .t
                    .text(k)
                    .starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
            if gap && key && k < last && self.ttag(k + 1) == T::Equal {
                return None;
            }
        }
        let v = self.ast.rhs(Node(kv));
        let whole = v != NONE && self.first(v) == first && self.last(v) == last;
        if whole
            && self.tag(v) == Tag::Delimited
            && (self.expr_ok(v) || self.ttag(first) == T::LBrace)
        {
            return Some(self.inner(v).trim().to_string());
        }
        if self.ttag(first) == T::LBrace && self.brace_close(first) == Some(last) {
            return Some(self.span(first + 1, last - 1).trim().to_string());
        }
        Some(self.span(first, last).trim().to_string())
    }

    /// `D/Q/M/J/Z name nodes... model [flags] [k=v...]` (the old SPICE grammar's `nodes`,
    /// `inst`).
    pub(in crate::syntax::render) fn semiconductor(
        &self,
        n: u32,
        e: Element,
        name: String,
    ) -> Option<Card> {
        let items = &self.items(n)[..];
        let iflag = |k: u32| ["thermal", "off", "on"].iter().any(|f| self.word_is(k, f));
        // Nodes while another word (no flag, no pair) follows; then the model.
        // In a kit, the split is where the kit's model is.
        let mut m = 0;
        while m < items.len() && self.is_word(items[m]) {
            let next = items.get(m + 1);
            if next.is_some_and(|&x| self.is_word(x) && !iflag(x)) {
                m += 1;
            } else {
                break;
            }
        }
        if let Some(s) = self.kit_split(n) {
            m = s.nodes as usize;
            assert_eq!(
                items.get(m),
                Some(&s.master.0),
                "the split's master is its item"
            );
        }
        if m >= items.len() || !self.is_word(items[m]) || !self.inst_ok(&items[m + 1..]) {
            return None;
        }
        let nodes: Vec<String> = items[..m].iter().map(|&k| self.text(k)).collect();
        let (model, params) = (self.text(items[m]), self.params_of(&items[m + 1..], false));
        let fits = match e {
            Element::Diode => nodes.len() == 2,
            Element::Bjt => (3..=5).contains(&nodes.len()),
            Element::Mosfet => (3..=7).contains(&nodes.len()),
            Element::Jfet | Element::Mesfet => nodes.len() == 3,
            _ => false,
        };
        fits.then(|| Card::Semi {
            letter: self.letter(n),
            name,
            nodes,
            model,
            params,
        })
    }

    /// An `X` card (the old SPICE grammar's `X`, the old lowering's `instance`).
    pub(in crate::syntax::render) fn instance(&self, n: u32) -> Option<Card> {
        let h = self.ast.header(Node(n));
        let (master, nodes) = (h.master, h.nodes);
        let items = &self.items(n)[..];
        if !self.spaced_items(self.name_last(n), items, false) {
            return None;
        }
        let pairs: Vec<u32> = items
            .iter()
            .copied()
            .filter(|&k| self.tag(k) == Tag::Assign)
            .collect();
        if !pairs.iter().all(|&p| self.pair_ok(p)) {
            return None;
        }
        debug_assert!(master != NONE, "an X card with a master");
        Some(Card::Inst {
            name: self.elem_name(n),
            ports: nodes.iter().map(|&k| self.text(k)).collect(),
            master: self.master_name(n),
            params: self.params_of(&pairs, false),
        })
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// The `}` matching the `{` at `open`, on its statement.
    fn brace_close(&self, open: u32) -> Option<u32> {
        let last = self.line_last(open);
        let mut depth = 0i32;
        for k in open..=last {
            match self.ttag(k) {
                T::LBrace => depth += 1,
                T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(k);
                    }
                }
                _ => {}
            }
        }
        None
    }
}
