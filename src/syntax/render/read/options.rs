//! SPICE `.options` → [`Opt`] views (as the old lowering applied them): the
//! portable `k=v` list, Xyce's per-package form (`.options <pkg> k=v`, `k v`,
//! comma lists), and `DEVICE TEMP`/`TNOM`.
//!
//! Cites: the old lowering; the old SPICE grammar's `Options`,
//! `oitem`; the old Xyce grammar's `Options`, `Pkg`, `xopt`, `Pair_x`, `MVal_x`,
//! `Flag_x`; Xyce Reference Guide p.96.
//!
//! Contents:
//! - `pub(in crate::syntax::render) enum Opt`
//! - `Rd::options_ok(&self, n: u32) -> bool`
//! - `Rd::options_view(&self, n: u32) -> Vec<Opt>`

use super::Rd;
use crate::syntax::Dialect;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

/// One option a `.options` card sets.
#[derive(Clone, Debug)]
pub(in crate::syntax::render) enum Opt {
    /// A `k=v` (a flag: `v` empty) every SPICE dialect spells alike.
    Portable(String, String),
    /// Xyce's own: `pkg.key` (`pkg.` alone for a bare package; a key with
    /// no package when the card names none) and its value.
    Xyce(String, String),
    /// Xyce `.options device temp=`.
    Temp(f64),
    /// Xyce `.options device tnom=`.
    Tnom(f64),
    /// VACASK's own `options k=v`, its value as written.
    Vacask(String, String),
    /// `ngbehavior=<mode>`: ngspice's compatibility mode, which ngspice
    /// reads only before the deck (its `-D ngbehavior=`), so no card of
    /// any dialect sets it (`Construct::NgBehavior`).
    NgBehavior(String),
}

impl<'a> Rd<'a> {
    /// `.options` items (the old SPICE grammar's `oitem`, the old Xyce grammar's `Options`).
    pub(in crate::syntax::render) fn options_ok(&self, n: u32) -> bool {
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

    /// What `.options` card `n` sets, in order (the old lowering).
    pub(in crate::syntax::render) fn options_view(&self, n: u32) -> Vec<Opt> {
        let items = self.fields(n);
        match (self.d, self.xyce_pkg(n)) {
            (Dialect::Xyce, Some(pkg)) => {
                let pkg = self.text(pkg).to_ascii_lowercase();
                let pairs = self.xyce_pairs(&items[1..]).expect("options_ok read it");
                let mut out = Vec::new();
                if pairs.is_empty() {
                    out.push(Opt::Xyce(format!("{pkg}."), String::new()));
                }
                for (k, v, x) in pairs {
                    out.push(match (pkg.as_str(), k.to_ascii_lowercase().as_str(), x) {
                        ("device", "temp", Some(x)) => Opt::Temp(x),
                        ("device", "tnom", Some(x)) => Opt::Tnom(x),
                        _ => Opt::Xyce(format!("{pkg}.{}", k.to_ascii_lowercase()), v),
                    });
                }
                out
            }
            (Dialect::Xyce, None) => self
                .params_of(items, false)
                .into_iter()
                .map(|(k, v)| Opt::Xyce(k, v))
                .collect(),
            _ => self
                .params_of(items, false)
                .into_iter()
                .map(|(k, v)| match k.eq_ignore_ascii_case("ngbehavior") {
                    true => Opt::NgBehavior(v),
                    false => Opt::Portable(k, v),
                })
                .collect(),
        }
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// quirk: whether the only tokens between the card's items (and after them)
    /// are commas: the old SPICE grammar's `oitem` reads no stray `( ) =`.
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

    /// quirk: the old Xyce grammar's `xopt`: `Pair_x` (`k=v` or `k v`, the
    /// value run on
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

    /// the old Xyce grammar's `Pkg`: a first word that is no `key=`.
    fn xyce_pkg(&self, n: u32) -> Option<u32> {
        self.fields(n).first().copied().filter(|&k| self.wordw(k))
    }
}
