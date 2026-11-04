//! Value items → [`Val`] views, as the old lowering read value rows:
//! numbers by the dialect's `Num` rule over a word's whole text,
//! expressions without their delimiters, `k=v` lists, a model's or an
//! element's number sequence, names unescaped.
//!
//! Cites: the old lowering (ported); each dialect's `Num` rule in its
//! old PEG grammar; ngspice manual §2.1.3; ADI LTspice help (`µ`).
//!
//! Contents:
//! - `Rd::number(&self, n: u32) -> Option<f64>`, `Rd::count(&self, n: u32) -> Option<u32>`
//! - `enum Val`
//! - `Rd::value(&self, n: u32) -> Val`, `Rd::irnum(&self, n: u32) -> Option<Num>`
//! - `Rd::vtext(&self, n: u32) -> String`
//! - `Rd::body_text(&self, n: u32) -> String`, `Rd::name_of(&self, n: u32) -> String`
//! - `Rd::expr_ok(&self, n: u32) -> bool`, `Rd::inner(&self, n: u32) -> String`
//! - `Rd::key(&self, n: u32) -> String`, `Rd::word_text(&self, n: u32) -> Option<String>`
//! - `Rd::mtext(&self, n: u32) -> String`, `Rd::params_of(&self, items: &[u32], model: bool) -> Vec<(String, String)>`,
//!   `Rd::nests_delimiter(&self, e: u32) -> bool`, `Rd::respell(&self, s: &str) -> String`

use super::Num;
use super::Rd;
use crate::syntax::Dialect;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::lexicon::Arithmetic;
use crate::syntax::token::Tag as T;

/// A value field as the old reader read it: a number, an expression
/// (without its delimiters), or text kept as written.
#[derive(Clone, PartialEq, Debug)]
pub(in crate::syntax::render) enum Val {
    Num(f64),
    Expr(String),
    Raw(String),
}

impl Rd<'_> {
    /// The number value item `n` is when its whole text is one `Num` of the
    /// dialect (sign included in SPICE); `None` for anything else.
    pub(in crate::syntax::render) fn number(&self, n: u32) -> Option<f64> {
        if n == NONE {
            return None;
        }
        match self.tag(n) {
            Tag::Delimited | Tag::Str | Tag::Group | Tag::Seq | Tag::Bad | Tag::Assign => None,
            _ => self.num_text(&self.text(n), self.t.lexicon_at(self.main(n))),
        }
    }

    pub(in crate::syntax::render) fn count(&self, n: u32) -> Option<u32> {
        self.number(n)
            .filter(|v| *v >= 0.0 && v.fract() == 0.0)
            .map(|v| v as u32)
    }

    /// A value field (the old lowering's `value`): a number, an expression, or
    /// text kept raw.
    pub(in crate::syntax::render) fn value(&self, n: u32) -> Val {
        if let Some(value) = self.number(n) {
            return Val::Num(value);
        }
        let text = self.etext(n);
        let text = text.trim();
        if self.named() {
            // Spectre's and VACASK's `Val` holds one `Expr` row (a vector
            // holds its elements).
            return match self.tag(n) {
                Tag::Vector | Tag::List
                    if !text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') =>
                {
                    Val::Raw(text.to_string())
                }
                _ => Val::Expr(text.to_string()),
            };
        }
        match self.tag(n) {
            Tag::Delimited => Val::Expr(self.respell(self.inner(n).trim())),
            Tag::Str => Val::Expr(text[1..text.len() - 1].trim().to_string()),
            _ if text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') => {
                Val::Expr(text.to_string())
            }
            _ => Val::Raw(text.to_string()),
        }
    }

    pub(in crate::syntax::render) fn irnum(&self, n: u32) -> Option<Num> {
        match self.value(n) {
            Val::Num(value) => Some(Num::Num(value)),
            Val::Expr(expr) => Some(Num::Expr(expr)),
            Val::Raw(_) => None,
        }
    }

    /// A value's text (the old lowering's `vtext`); a `Seq`'s numbers joined to
    /// it by a comma where one stood, else by `blank`'s choice.
    pub(in crate::syntax::render) fn vtext(&self, n: u32) -> String {
        if self.tag(n) == Tag::Seq {
            return self.seq_text(n, ",");
        }
        self.single_vtext(n)
    }

    /// As [`Rd::vtext`], a `Seq` joined by one blank (a model's matrix).
    pub(in crate::syntax::render) fn mtext(&self, n: u32) -> String {
        if self.tag(n) == Tag::Seq {
            return self.seq_text(n, " ");
        }
        self.single_vtext(n)
    }

    /// `k=v` items and bare flags (`(word, "")`), in card order; `model`
    /// joins a `Seq` as a model's list does.
    pub(in crate::syntax::render) fn params_of(
        &self,
        items: &[u32],
        model: bool,
    ) -> Vec<(String, String)> {
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
    pub(in crate::syntax::render) fn key(&self, n: u32) -> String {
        self.text(self.ast.lhs(Node(n)))
    }

    /// A body kept whole (`B`, `.func`): its expression without delimiters.
    pub(in crate::syntax::render) fn body_text(&self, n: u32) -> String {
        match self.tag(n) {
            Tag::Delimited => self.inner(n).trim().to_string(),
            _ => self.etext(n).trim().to_string(),
        }
    }

    /// A delimited expression's inside, as the grammar's `Expr` row: between
    /// the delimiters, and inside the quotes of ngspice's `{"…"}`.
    pub(in crate::syntax::render) fn inner(&self, n: u32) -> String {
        let a = self.ast;
        let (open, close) = (a.main_token(Node(n)), a.rhs(Node(n)));
        let e = a.lhs(Node(n));
        if e != NONE && self.tag(e) == Tag::Str && self.ttag(open) == T::LBrace {
            let t = self.t.text(self.main(e));
            return t[1..t.len() - 1].trim().to_string();
        }
        if close <= open + 1 {
            return String::new();
        }
        self.espan(open + 1, close - 1)
    }

    /// Whether item `n` is a delimited expression the grammar's `Expr`
    /// reads (a `Bad` inside is not).
    pub(in crate::syntax::render) fn expr_ok(&self, n: u32) -> bool {
        self.tag(n) == Tag::Delimited && {
            let e = self.ast.lhs(Node(n));
            let hspice = self.ttag(self.main(n)) == T::DoubleQuote;
            hspice || (e != NONE && self.tag(e) != Tag::Bad && !self.nests_delimiter(e))
        }
    }

    /// quirk: the old grammars' expression atom (`x_atom`) has no `{…}` or `'…'`: an expression
    /// holding one is no `Expr` (`(t<1 ? {a} : {b})`).
    pub(in crate::syntax::render) fn nests_delimiter(&self, e: u32) -> bool {
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
    /// parts are the bytes they spell (the old lowering's `name`).
    pub(in crate::syntax::render) fn name_of(&self, n: u32) -> String {
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
    pub(in crate::syntax::render) fn word_text(&self, n: u32) -> Option<String> {
        (n != NONE && self.tag(n) == Tag::Word).then(|| self.text(n))
    }

    /// LTspice's `µ` is `u` (XVII help) in the text a SPICE reader gets.
    pub(in crate::syntax::render) fn respell(&self, s: &str) -> String {
        if self.d == Dialect::Ltspice {
            s.replace('µ', "u")
        } else {
            s.to_string()
        }
    }
}

// ---- private ----

impl Rd<'_> {
    fn single_vtext(&self, n: u32) -> String {
        let text = self.etext(n);
        let text = text.trim();
        if matches!(self.d, Dialect::Ngspice) {
            return one_group(text);
        }
        if self.named()
            && let Some(x) = self.number(n)
        {
            return crate::unit::format_spice_number(x);
        }
        self.respell(text)
    }

    /// quirk: the old lowering's `vtext` with `More` rows: the first value's
    /// text, then each further number after `,` where the gap held a comma,
    /// else after `blank`.
    fn seq_text(&self, n: u32, blank: &str) -> String {
        let parts = self.ast.list(self.ast.lhs(Node(n)));
        let mut out = self.etext(parts[0]).trim().to_string();
        for w in parts.windows(2) {
            let (prev, m) = (self.last(w[0]), self.main(w[1]));
            let gap = self.span(prev, m);
            let gap = &gap[self.t.text(prev).len()..gap.len() - self.t.text(m).len()];
            out.push_str(if gap.contains(',') { "," } else { blank });
            out.push_str(&self.etext(w[1]));
        }
        out
    }

    /// quirk: the dialect's `Num` rule over all of `s` (SPICE's sign
    /// included), with the old lowering's arithmetic (`num_value`).
    fn num_text(&self, s: &str, lex: &crate::syntax::lexicon::Lexicon) -> Option<f64> {
        if lex.named {
            // A signed literal where the sign is an operator (the old lowering
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
            if !ngspice_num(s) {
                return None;
            }
            // `lexicon::NGSPICE_GIVEN` reads its numbers correctly rounded.
            if lex.numbers.arithmetic == Arithmetic::Decimal {
                let (neg, body) = (s.starts_with('-'), s.trim_start_matches(['-', '+']));
                let v = crate::syntax::number::value(body, lex);
                return (!v.is_nan()).then_some(if neg { -v } else { v });
            }
            return crate::unit::ngspice_value(s);
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

/// the old ngspice grammar's `Num <- Mag scale? [a-z0-9_.%\\]*` over all of `s`, case
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

/// An ngspice value as one expression (the old lowering's `one_group`).
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
