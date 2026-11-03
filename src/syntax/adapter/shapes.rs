//! SPICE element items → whether `grammar/spice.peg`'s field rules read
//! them: a `field`, a `Val`, a `Pair`, `Pair_i`'s run-on value, the `s`
//! between fields, a closed group; the helpers every element rule shares.
//!
//! Cites: spice.peg `element_any`, `field`, `Pair`, `Pair_i`, `Val_i`,
//! `inst`, `value`, `group`; xyce.peg `blob`.
//!
//! Contents:
//! - `Cx::items(&self, n: u32) -> Vec<u32>`
//! - `Cx::fields_ok(&self, n: u32) -> bool`
//! - `Cx::spaced_items(&self, prev: u32, items: &[u32], commas: bool) -> bool`
//! - `Cx::name_last(&self, n: u32) -> u32`
//! - `Cx::elem_name(&self, n: u32) -> String`
//! - `Cx::is_word(&self, n: u32) -> bool`
//! - `Cx::word_is(&self, n: u32, w: &str) -> bool`
//! - `Cx::val_ok(&self, v: u32) -> bool`
//! - `Cx::pair_ok(&self, a: u32) -> bool`
//! - `Cx::pair_i_ok(&self, a: u32) -> bool`
//! - `Cx::inst_ok(&self, rest: &[u32]) -> bool`
//! - `Cx::group_closed(&self, g: u32) -> bool`
//! - `Cx::group_items(&self, g: u32) -> &[u32]`

use super::Cx;
use crate::codegen::Dialect;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

impl<'a> Cx<'a> {
    /// An element card's items after its name, in source order: its
    /// nodes, master and arguments merged back.
    pub(super) fn items(&self, n: u32) -> Vec<u32> {
        let h = self.ast.header(Node(n));
        let mut items: Vec<u32> = h.nodes.iter().chain(self.fields(n)).copied().collect();
        if h.master != NONE {
            items.push(h.master);
        }
        items.sort_unstable_by_key(|&k| self.main(k));
        items
    }

    /// quirk: spice.peg `element_any <- &[a-z] Name (s? field)+`, the card's
    /// end: every item after the name a `field` (a word, group, string,
    /// readable expression, `k=v`, or one of `, ( ) =`).
    pub(super) fn fields_ok(&self, n: u32) -> bool {
        let items = &self.items(n)[..];
        let name = self.ast.header(Node(n)).name;
        let last = self.line_last(self.main(n));
        let mut covered = self.last(name) + 1;
        let mut any = false;
        let stray = |k: u32| {
            matches!(
                self.ttag(k),
                T::Comma
                    | T::LParen
                    | T::RParen
                    | T::Equal
                    | T::EqualEqual
                    | T::LessEqual
                    | T::GreaterEqual
                    | T::BangEqual
            )
        };
        for &it in items {
            if !(covered..self.first(it)).all(stray) {
                return false;
            }
            any = true;
            let ok = match self.tag(it) {
                Tag::Word | Tag::Str | Tag::Group => true,
                Tag::Delimited => self.expr_ok(it),
                Tag::Assign => {
                    let v = self.ast.rhs(Node(it));
                    v == NONE
                        || self.val_ok(v)
                        || self.tag(v) == Tag::Seq
                        || !matches!(self.tag(v), Tag::Bad | Tag::Delimited)
                        // A value that is one stray `(` read as a field.
                        || self.tag(v) == Tag::Bad && stray(self.main(v)) && self.ast.lhs(Node(v)) == self.main(v)
                }
                _ => false,
            };
            if !ok {
                return false;
            }
            covered = self.last(it) + 1;
        }
        any = any || covered <= last;
        any && (covered..=last).all(stray)
    }

    /// spice.peg's `s` before each item (`commas`: or `s? ','`s as `inst`
    /// allows, a blank still before the item).
    pub(super) fn spaced_items(&self, prev: u32, items: &[u32], commas: bool) -> bool {
        let mut prev = prev;
        for &k in items {
            let first = self.first(k);
            if !(prev + 1..first).all(|x| commas && self.ttag(x) == T::Comma) {
                return false;
            }
            if self.t.end(first - 1) == self.t.start(first) {
                return false;
            }
            prev = self.last(k);
        }
        true
    }

    /// The element's name word's last token.
    pub(super) fn name_last(&self, n: u32) -> u32 {
        self.last(self.ast.header(Node(n)).name)
    }

    /// The element's name after its letter (spice.peg `Name_e`).
    pub(super) fn elem_name(&self, n: u32) -> String {
        let w = self.ast.header(Node(n)).name;
        let t = self.text(w);
        t[1..].to_string()
    }

    pub(super) fn is_word(&self, n: u32) -> bool {
        self.tag(n) == Tag::Word
    }

    pub(super) fn word_is(&self, n: u32, w: &str) -> bool {
        self.is_word(n) && self.text(n).eq_ignore_ascii_case(w)
    }

    /// A value the grammar's `Val` reads: a word, a string, a readable
    /// delimited expression, or an expression that does not start with a
    /// delimiter.
    pub(super) fn val_ok(&self, v: u32) -> bool {
        if v == NONE {
            return false;
        }
        match self.tag(v) {
            Tag::Word | Tag::Str => true,
            Tag::Delimited => {
                self.expr_ok(v) || (self.d == Dialect::Xyce && self.ttag(self.main(v)) == T::LBrace)
            }
            Tag::Seq | Tag::Bad | Tag::Group | Tag::Assign => false,
            _ => !self.nests_delimiter(v),
        }
    }

    /// An `Assign` the grammar's `Pair` reads (`Key = Val`).
    pub(super) fn pair_ok(&self, a: u32) -> bool {
        self.tag(a) == Tag::Assign
            && self.is_word(self.ast.lhs(Node(a)))
            && self.val_ok(self.ast.rhs(Node(a)))
    }

    /// As [`Cx::pair_ok`], a value that may run on through numbers
    /// (`Pair_i`).
    pub(super) fn pair_i_ok(&self, a: u32) -> bool {
        if self.tag(a) != Tag::Assign || !self.is_word(self.ast.lhs(Node(a))) {
            return false;
        }
        let v = self.ast.rhs(Node(a));
        match v {
            NONE => false,
            _ if self.tag(v) == Tag::Seq => {
                let parts = self.ast.list(self.ast.lhs(Node(v)));
                self.val_ok(parts[0]) && parts[1..].iter().all(|&p| self.is_word(p))
            }
            _ => self.val_ok(v),
        }
    }

    /// spice.peg `inst`: flags and `Pair_i`s; `None` when another item
    /// stands among them.
    pub(super) fn inst_ok(&self, rest: &[u32]) -> bool {
        rest.iter().all(|&k| self.is_word(k) || self.pair_i_ok(k))
    }

    /// quirk: whether group `g` ends at its `)` (spice.peg `group`; an unclosed one runs to the
    /// card's end).
    pub(super) fn group_closed(&self, g: u32) -> bool {
        self.ttag(self.ast.extra(self.ast.rhs(Node(g)))) == T::RParen
    }

    /// A group's items.
    pub(super) fn group_items(&self, g: u32) -> &[u32] {
        let at = self.ast.rhs(Node(g));
        self.ast.list(at + 1)
    }
}

// ---- private ----
