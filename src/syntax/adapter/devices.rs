//! POLY sources, `B` sources, semiconductors and `X` cards → `Component`s
//! and `Instance`s (the old lowering), each checked against its
//! the old SPICE grammar rule: `Poly`, `B` and its `Body` (the text after
//! `key =` to the card's end), `nodes`/`inst`, `X`.
//!
//! Cites: the old lowering (ported); the old SPICE grammar's `Poly`, `B`,
//! `Body`, `D` … `Z`, `nodes`, `inst`, `X`.
//!
//! Contents:
//! - `Cx::poly(&self, n: u32, name: String) -> Option<Option<Component>>`
//! - `Cx::behavioural(&self, n: u32, name: String) -> Option<Component>`
//! - `Cx::body_to_end(&self, kv: u32) -> Option<String>`
//! - `Cx::semiconductor(&self, n: u32, e: Element, name: String) -> Option<Component>`
//! - `Cx::instance(&self, n: u32) -> Option<Instance>`

use super::Cx;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::device::Primitive as Element;
use crate::syntax::token::Tag as T;

impl<'a> Cx<'a> {
    /// `E o 0 POLY(n) controls... coeffs...` (and G, F, H); `None` when the
    /// card is no POLY card, `Some(None)` when it is one the IR cannot hold.
    pub(super) fn poly(&self, n: u32, name: String) -> Option<Option<Component>> {
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
            Some(Component::Poly {
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
    pub(super) fn behavioural(&self, n: u32, name: String) -> Option<Component> {
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
        match self.key(*kv).to_ascii_lowercase().as_str() {
            "v" => Some(Component::BehavioralVoltage {
                name,
                np,
                nm,
                expression,
            }),
            "i" => Some(Component::BehavioralCurrent {
                name,
                np,
                nm,
                expression,
            }),
            _ => None,
        }
    }

    /// quirk: the old SPICE grammar's `Body`: the text after `key =` to the
    /// card's end, unless a ` key=` follows; braces (or a readable
    /// expression) around all of it are dropped.
    pub(super) fn body_to_end(&self, kv: u32) -> Option<String> {
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
    pub(super) fn semiconductor(&self, n: u32, e: Element, name: String) -> Option<Component> {
        let items = &self.items(n)[..];
        let iflag = |k: u32| ["thermal", "off", "on"].iter().any(|f| self.word_is(k, f));
        // Nodes while another word (no flag, no pair) follows; then the model.
        let mut m = 0;
        while m < items.len() && self.is_word(items[m]) {
            let next = items.get(m + 1);
            if next.is_some_and(|&x| self.is_word(x) && !iflag(x)) {
                m += 1;
            } else {
                break;
            }
        }
        if m >= items.len() || !self.is_word(items[m]) || !self.inst_ok(&items[m + 1..]) {
            return None;
        }
        let nodes: Vec<String> = items[..m].iter().map(|&k| self.text(k)).collect();
        let (model, params) = (self.text(items[m]), self.params_of(&items[m + 1..], false));
        Some(match (e, nodes.as_slice()) {
            (Element::Diode, [np, nm]) => Component::Diode {
                name,
                np: np.clone(),
                nm: nm.clone(),
                model,
                params,
            },
            (Element::Bjt, [nc, nb, ne, extra @ ..]) if extra.len() <= 2 => Component::Bjt {
                name,
                nc: nc.clone(),
                nb: nb.clone(),
                ne: ne.clone(),
                extra_nodes: extra.to_vec(),
                model,
                params,
            },
            (Element::Mosfet, [nd, ng, ns, rest @ ..]) if rest.len() <= 4 => Component::Mosfet {
                name,
                nd: nd.clone(),
                ng: ng.clone(),
                ns: ns.clone(),
                nb: rest.first().cloned(),
                extra_nodes: rest.iter().skip(1).cloned().collect(),
                model,
                params,
            },
            (Element::Jfet, [nd, ng, ns]) => Component::Jfet {
                name,
                nd: nd.clone(),
                ng: ng.clone(),
                ns: ns.clone(),
                model,
                params,
            },
            (Element::Mesfet, [nd, ng, ns]) => Component::Mesfet {
                name,
                nd: nd.clone(),
                ng: ng.clone(),
                ns: ns.clone(),
                model,
                params,
            },
            _ => return None,
        })
    }

    /// An `X` card (the old SPICE grammar's `X`, the old lowering's `instance`).
    pub(super) fn instance(&self, n: u32) -> Option<Instance> {
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
        Some(Instance {
            name: self.elem_name(n),
            subcircuit: self.text(master),
            port_mapping: nodes.iter().map(|&k| self.text(k)).collect(),
            parameters: self.params_of(&pairs, false),
            at: None,
        })
    }
}

// ---- private ----

impl<'a> Cx<'a> {
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
