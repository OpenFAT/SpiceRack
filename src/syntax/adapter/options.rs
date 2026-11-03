//! SPICE `.options` → testbench options (`import/lower/analyses.rs`): the
//! portable `k=v` list, Xyce's per-package form (`.options <pkg> k=v`, `k v`,
//! comma lists), and `DEVICE TEMP`/`TNOM`.
//!
//! Cites: `import/lower/analyses.rs:581-629` (ported); spice.peg `Options`,
//! `oitem`; xyce.peg `Options`, `Pkg`, `xopt`, `Pair_x`, `MVal_x`,
//! `Flag_x`; Xyce Reference Guide p.96.
//!
//! Contents:
//! - `Cx::options_ok(&self, n: u32) -> bool`
//! - `Cx::options(&mut self, n: u32)`

use super::Cx;
use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

impl<'a> Cx<'a> {
    /// `.options` items (spice.peg `oitem`, xyce.peg `Options`).
    pub(super) fn options_ok(&self, n: u32) -> bool {
        let items = self.fields(n);
        if self.d == Dialect::Xyce && self.xyce_pkg(n).is_some() {
            return self.only_commas_between(n, items) && self.xyce_pairs(&items[1..]).is_some();
        }
        self.only_commas_between(n, items)
            && items.iter().all(|&k| {
                self.wordw(k)
                    || self.tag(k) == Tag::Assign
                        && self.wordw(self.ast.lhs(Node(k)))
                        && self.val_ok(self.ast.rhs(Node(k)))
            })
    }

    /// `.options` applied (lower/analyses.rs:581-629).
    pub(super) fn options(&mut self, n: u32) {
        let items = self.fields(n);
        match (self.d, self.xyce_pkg(n)) {
            (Dialect::Xyce, Some(pkg)) => {
                let pkg = self.text(pkg).to_ascii_lowercase();
                let pairs = self.xyce_pairs(&items[1..]).expect("options_ok read it");
                let specific = |tb: &mut Testbench, k: String, v: String| {
                    tb.options
                        .backend_specific
                        .entry("xyce".into())
                        .or_default()
                        .push((k, v));
                };
                if pairs.is_empty() {
                    specific(&mut self.tb, format!("{pkg}."), String::new());
                }
                for (k, v, x) in pairs {
                    match (pkg.as_str(), k.to_ascii_lowercase().as_str(), x) {
                        ("device", "temp", Some(x)) => self.tb.temperature = Some(x),
                        ("device", "tnom", Some(x)) => self.tb.nominal_temperature = Some(x),
                        _ => specific(&mut self.tb, format!("{pkg}.{}", k.to_ascii_lowercase()), v),
                    }
                }
            }
            (Dialect::Xyce, None) => {
                let p = self.params_of(items, false);
                self.tb
                    .options
                    .backend_specific
                    .entry("xyce".into())
                    .or_default()
                    .extend(p);
            }
            _ => {
                let p = self.params_of(items, false);
                self.tb.options.portable.extend(p);
            }
        }
    }
}

// ---- private ----

impl<'a> Cx<'a> {
    /// quirk: whether the only tokens between the card's items (and after them)
    /// are commas: spice.peg `oitem` reads no stray `( ) =`.
    fn only_commas_between(&self, n: u32, items: &[u32]) -> bool {
        let mut prev = self.main(n);
        for &k in items.iter() {
            if !(prev + 1..self.first(k)).all(|x| self.ttag(x) == T::Comma) {
                return false;
            }
            prev = self.last(k);
        }
        (prev + 1..=self.line_last(self.main(n))).all(|x| self.ttag(x) == T::Comma)
    }

    /// quirk: xyce.peg `xopt`: `Pair_x` (`k=v` or `k v`, the value run on
    /// by comma-separated words or blank-separated numbers, `MVal_x`) or
    /// `Flag_x` (a word or a brace): `(key, value text, its number)`.
    fn xyce_pairs(&self, rest: &[u32]) -> Option<Vec<(String, String, Option<f64>)>> {
        let mut out = Vec::new();
        let mut i = 0;
        let numberish = |k: u32| {
            self.wordw(k)
                && self
                    .text(k)
                    .starts_with(|c: char| c.is_ascii_digit() || "+-.".contains(c))
        };
        while i < rest.len() {
            let k = rest[i];
            let (key, v) = match self.tag(k) {
                Tag::Assign if self.wordw(self.ast.lhs(Node(k))) => {
                    (self.key(k), self.ast.rhs(Node(k)))
                }
                Tag::Word
                    if rest
                        .get(i + 1)
                        .is_some_and(|&x| numberish(x) && !self.comma_between(k, x))
                        && self
                            .text(k)
                            .starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') =>
                {
                    i += 1;
                    (self.text(k), rest[i])
                }
                Tag::Word | Tag::Delimited => {
                    out.push((self.text(k), String::new(), None));
                    i += 1;
                    continue;
                }
                _ => return None,
            };
            if v == NONE
                || !matches!(self.tag(v), Tag::Word | Tag::Str | Tag::Delimited)
                    && self.nests_delimiter(v)
            {
                return None;
            }
            let mut text = self.text(v).trim().to_string();
            let mut prev = v;
            let mut more = false;
            while let Some(&x) = rest.get(i + 1)
                && self.wordw(x)
                && (self.comma_between(prev, x) || numberish(x))
            {
                text.push_str(if self.comma_between(prev, x) {
                    ","
                } else {
                    " "
                });
                text.push_str(&self.text(x));
                (prev, more) = (x, true);
                i += 1;
            }
            let num = if more { None } else { self.number(v) };
            out.push((key, text, num));
            i += 1;
        }
        Some(out)
    }

    /// Whether a comma stands between items `a` and `b`.
    fn comma_between(&self, a: u32, b: u32) -> bool {
        (self.last(a) + 1..self.first(b)).any(|k| self.ttag(k) == T::Comma)
    }

    /// xyce.peg `Pkg`: a first word that is no `key=`.
    fn xyce_pkg(&self, n: u32) -> Option<u32> {
        self.fields(n).first().copied().filter(|&k| self.wordw(k))
    }
}
