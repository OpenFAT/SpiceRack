//! Value items → IR values, as `import/lower/values.rs` reads value rows:
//! numbers by the dialect's `Num` rule over a word's whole text,
//! expressions without their delimiters, `k=v` lists, a model's or an
//! element's number sequence, names unescaped.
//!
//! Cites: `import/lower/values.rs` (ported); each dialect's `Num` rule in
//! `grammar/*.peg`; ngspice manual §2.1.3; ADI LTspice help (`µ`).
//!
//! Contents:
//! - `Cx::number(&self, n: u32) -> Option<f64>`, `Cx::count(&self, n: u32) -> Option<u32>`
//! - `Cx::value(&self, n: u32) -> IrValue`, `Cx::irnum(&self, n: u32) -> Option<IrNum>`
//! - `Cx::vtext(&self, n: u32) -> String`
//! - `Cx::body_text(&self, n: u32) -> String`, `Cx::name_of(&self, n: u32) -> String`
//! - `Cx::expr_ok(&self, n: u32) -> bool`, `Cx::inner(&self, n: u32) -> String`
//! - `Cx::key(&self, n: u32) -> String`, `Cx::word_text(&self, n: u32) -> Option<String>`
//! - `Cx::mtext(&self, n: u32) -> String`, `Cx::params_of(&self, items: &[u32], model: bool) -> Vec<(String, String)>`,
//!   `Cx::nests_delimiter(&self, e: u32) -> bool`, `Cx::respell(&self, s: &str) -> String`

use super::Cx;
use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

impl Cx<'_> {
    /// The number value item `n` is when its whole text is one `Num` of the
    /// dialect (sign included in SPICE); `None` for anything else.
    pub(super) fn number(&self, n: u32) -> Option<f64> {
        if n == NONE {
            return None;
        }
        match self.tag(n) {
            Tag::Delimited | Tag::Str | Tag::Group | Tag::Seq | Tag::Bad | Tag::Assign => None,
            _ => self.num_text(&self.text(n), self.t.lexicon_at(self.main(n))),
        }
    }

    pub(super) fn count(&self, n: u32) -> Option<u32> {
        self.number(n)
            .filter(|v| *v >= 0.0 && v.fract() == 0.0)
            .map(|v| v as u32)
    }

    /// A value field (lower/values.rs `value`): a number, an expression, or
    /// text kept raw.
    pub(super) fn value(&self, n: u32) -> IrValue {
        if let Some(value) = self.number(n) {
            return IrValue::Numeric { value };
        }
        let text = self.text(n);
        let text = text.trim();
        if self.named() {
            // Spectre's and VACASK's `Val` holds one `Expr` row (a vector
            // holds its elements).
            return match self.tag(n) {
                Tag::Vector | Tag::List
                    if !text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') =>
                {
                    IrValue::Raw {
                        text: text.to_string(),
                    }
                }
                _ => IrValue::Expression {
                    expr: text.to_string(),
                },
            };
        }
        match self.tag(n) {
            Tag::Delimited => IrValue::Expression {
                expr: self.respell(self.inner(n).trim()),
            },
            Tag::Str => IrValue::Expression {
                expr: text[1..text.len() - 1].trim().to_string(),
            },
            _ if text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') => {
                IrValue::Expression {
                    expr: text.to_string(),
                }
            }
            _ => IrValue::Raw {
                text: text.to_string(),
            },
        }
    }

    pub(super) fn irnum(&self, n: u32) -> Option<IrNum> {
        match self.value(n) {
            IrValue::Numeric { value } => Some(IrNum::Num(value)),
            IrValue::Expression { expr } => Some(IrNum::Expr(expr)),
            IrValue::Raw { .. } => None,
        }
    }

    /// A value's text (lower/values.rs `vtext`); a `Seq`'s numbers joined to
    /// it by a comma where one stood, else by `blank`'s choice.
    pub(super) fn vtext(&self, n: u32) -> String {
        if self.tag(n) == Tag::Seq {
            return self.seq_text(n, ",");
        }
        self.single_vtext(n)
    }

    /// As [`Cx::vtext`], a `Seq` joined by one blank (a model's matrix).
    pub(super) fn mtext(&self, n: u32) -> String {
        if self.tag(n) == Tag::Seq {
            return self.seq_text(n, " ");
        }
        self.single_vtext(n)
    }

    /// `k=v` items and bare flags (`(word, "")`), in card order; `model`
    /// joins a `Seq` as a model's list does.
    pub(super) fn params_of(&self, items: &[u32], model: bool) -> Vec<(String, String)> {
        items
            .iter()
            .filter_map(|&k| match self.tag(k) {
                Tag::Assign => {
                    let v = self.ast.rhs(Node(k));
                    let text = if model { self.mtext(v) } else { self.vtext(v) };
                    Some((self.key(k), text))
                }
                Tag::Word => Some((self.text(k), String::new())),
                _ => None,
            })
            .collect()
    }

    /// An `Assign`'s key text.
    pub(super) fn key(&self, n: u32) -> String {
        self.text(self.ast.lhs(Node(n)))
    }

    /// A body kept whole (`B`, `.func`): its expression without delimiters.
    pub(super) fn body_text(&self, n: u32) -> String {
        match self.tag(n) {
            Tag::Delimited => self.inner(n).trim().to_string(),
            _ => self.text(n).trim().to_string(),
        }
    }

    /// A delimited expression's inside, as the grammar's `Expr` row: between
    /// the delimiters, and inside the quotes of ngspice's `{"…"}`.
    pub(super) fn inner(&self, n: u32) -> String {
        let a = self.ast;
        let (open, close) = (a.main_token(Node(n)), a.rhs(Node(n)));
        if self.ttag(open) == T::String {
            let t = self.t.text(open);
            return t[1..t.len() - 1].trim().to_string();
        }
        let e = a.lhs(Node(n));
        if e != NONE && self.tag(e) == Tag::Str && self.ttag(open) == T::LBrace {
            let t = self.t.text(self.main(e));
            return t[1..t.len() - 1].trim().to_string();
        }
        if close <= open + 1 {
            return String::new();
        }
        self.span(open + 1, close - 1)
    }

    /// Whether item `n` is a delimited expression the grammar's `Expr`
    /// reads (a `Bad` inside is not).
    pub(super) fn expr_ok(&self, n: u32) -> bool {
        self.tag(n) == Tag::Delimited && {
            let e = self.ast.lhs(Node(n));
            let hspice = self.ttag(self.main(n)) == T::String;
            hspice || (e != NONE && self.tag(e) != Tag::Bad && !self.nests_delimiter(e))
        }
    }

    /// quirk: the grammars' `x_atom` has no `{…}` or `'…'`: an expression
    /// holding one is no `Expr` (`(t<1 ? {a} : {b})`).
    pub(super) fn nests_delimiter(&self, e: u32) -> bool {
        let a = self.ast;
        let mut work = vec![e];
        while let Some(n) = work.pop() {
            let node = Node(n);
            match a.tag(node) {
                Tag::Delimited | Tag::Bad => return true,
                Tag::Unary | Tag::Paren => work.push(a.lhs(node)),
                Tag::Binary => work.extend([a.lhs(node), a.rhs(node)]),
                Tag::Ternary => {
                    let at = a.rhs(node);
                    work.extend([a.lhs(node), a.extra(at), a.extra(at + 1)]);
                }
                Tag::Call | Tag::Vector | Tag::List => work.extend(a.list(a.rhs(node) + 1)),
                Tag::Index => work.extend([a.lhs(node), a.extra(a.rhs(node))]),
                // else: leaves (and a probe's words) hold no expression.
                _ => {}
            }
        }
        false
    }

    /// A name item's identifier: Spectre's `\x` escapes and VACASK's `'…'`
    /// parts are the bytes they spell (lower/values.rs `name`).
    pub(super) fn name_of(&self, n: u32) -> String {
        let t = self.text(n);
        match self.d {
            Dialect::Spectre => {
                let mut out = String::with_capacity(t.len());
                let mut it = t.chars();
                while let Some(c) = it.next() {
                    match c {
                        '\\' => out.extend(it.next()),
                        _ => out.push(c),
                    }
                }
                out
            }
            Dialect::Vacask => {
                let mut out = String::new();
                let mut it = t.chars().peekable();
                let mut quoted = false;
                while let Some(c) = it.next() {
                    match c {
                        '\'' if quoted && it.peek() == Some(&'\'') => {
                            it.next();
                            out.push('\'');
                        }
                        '\'' => quoted = !quoted,
                        _ => out.push(c),
                    }
                }
                out
            }
            _ => t,
        }
    }

    /// A `Word` item's text; `None` for any other item.
    pub(super) fn word_text(&self, n: u32) -> Option<String> {
        (n != NONE && self.tag(n) == Tag::Word).then(|| self.text(n))
    }

    /// LTspice's `µ` is `u` (XVII help) in the text a SPICE reader gets.
    pub(super) fn respell(&self, s: &str) -> String {
        if self.d == Dialect::Ltspice {
            s.replace('µ', "u")
        } else {
            s.to_string()
        }
    }
}

// ---- private ----

impl Cx<'_> {
    fn single_vtext(&self, n: u32) -> String {
        let text = self.text(n);
        let text = text.trim();
        if matches!(self.d, Dialect::Ngspice) {
            return one_group(text);
        }
        if self.named()
            && let Some(x) = self.number(n)
        {
            return crate::codegen::format_spice_number(x);
        }
        self.respell(text)
    }

    /// quirk: lower/values.rs `vtext` with `More` rows: the first value's
    /// text, then each further number after `,` where the gap held a comma,
    /// else after `blank`.
    fn seq_text(&self, n: u32, blank: &str) -> String {
        let parts = self.ast.list(self.ast.lhs(Node(n)));
        let mut out = self.text(parts[0]).trim().to_string();
        for w in parts.windows(2) {
            let (prev, m) = (self.last(w[0]), self.main(w[1]));
            let gap = self.span(prev, m);
            let gap = &gap[self.t.text(prev).len()..gap.len() - self.t.text(m).len()];
            out.push_str(if gap.contains(',') { "," } else { blank });
            out.push_str(&self.text(w[1]));
        }
        out
    }

    /// quirk: the dialect's `Num` rule over all of `s` (SPICE's sign
    /// included), with today's arithmetic (lower/values.rs `num_value`).
    fn num_text(&self, s: &str, lex: &crate::syntax::lexicon::Lexicon) -> Option<f64> {
        if lex.letters.is_empty() {
            // A signed literal where the sign is an operator (lower/values.rs
            // `number`): `-1.5`, `- 1.5`.
            let (neg, body) = match s.as_bytes().first() {
                Some(b'-') => (true, s[1..].trim_start()),
                Some(b'+') => (false, s[1..].trim_start()),
                _ => (false, s),
            };
            let v = crate::syntax::number::value(body, lex);
            let ok = !v.is_nan() && body.starts_with(|c: char| c.is_ascii_digit() || c == '.');
            return ok.then_some(if neg { -v } else { v });
        }
        if matches!(lex.dialect, Dialect::Ngspice) {
            return ngspice_num(s)
                .then(|| crate::unit::ngspice_value(s))
                .flatten();
        }
        let (neg, body) = match s.as_bytes().first() {
            Some(b'-') => (true, &s[1..]),
            Some(b'+') => (false, &s[1..]),
            _ => (false, s),
        };
        if !body.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
            return None;
        }
        let v = crate::syntax::number::value(body, lex);
        (!v.is_nan()).then_some(if neg { -v } else { v })
    }
}

/// ngspice.peg `Num <- Mag scale? [a-z0-9_.%\\]*` over all of `s`, case
/// folded.
fn ngspice_num(s: &str) -> bool {
    let b = s.to_ascii_lowercase().into_bytes();
    let mut q = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let digits = |q: usize| b[q..].iter().take_while(|c| c.is_ascii_digit()).count();
    let int = digits(q);
    q += int;
    if b.get(q) == Some(&b'.') {
        let frac = digits(q + 1);
        if int == 0 && frac == 0 {
            return false;
        }
        q += 1 + frac;
    } else if int == 0 {
        return false;
    }
    if matches!(b.get(q), Some(b'e' | b'd')) {
        let sign = usize::from(matches!(b.get(q + 1), Some(b'+' | b'-')));
        let e = digits(q + 1 + sign);
        if e > 0 {
            q += 1 + sign + e;
        }
    }
    b[q..]
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.%\\".contains(c))
}

/// An ngspice value as one expression (lower/values.rs `one_group`).
fn one_group(v: &str) -> String {
    if let Some(inner) = v
        .strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .map(str::trim)
        && let Some(e) = inner.strip_prefix('"').and_then(|r| r.strip_suffix('"'))
    {
        return format!("{{{}}}", e.trim());
    }
    let single = v.starts_with('{') && {
        let mut depth = 0i32;
        let closes = v.char_indices().find(|&(_, c)| {
            depth += i32::from(c == '{') - i32::from(c == '}');
            depth == 0
        });
        closes.is_some_and(|(i, _)| i + 1 == v.len())
    };
    if single || !v.contains('{') {
        return v.to_string();
    }
    format!("{{{}}}", v.replace('{', "(").replace('}', ")"))
}
