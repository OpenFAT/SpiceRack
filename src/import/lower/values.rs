//! Value rows -> IR values: numbers with their scale, expressions, `k=v`
//! parameter lists, vectors, and the identifiers Spectre and VACASK spell
//! with escapes or quotes.
//!
//! Cites: ngspice manual §2.1.3 (scale factors; ngspice's own arithmetic via
//! `unit::ngspice_value`); ADI LTspice help (`µ`, `6K34`); Kundert, Appendix
//! B.2 (Spectre names).
//!
//! Contents:
//! - `Cx::num(&self, i: usize) -> Option<f64>`, `Cx::number(&self, i: usize) -> Option<f64>`,
//!   `Cx::count(&self, i: usize) -> Option<u32>`
//! - `Cx::value(&self, i: usize) -> IrValue`, `Cx::irnum(&self, i: usize) -> Option<IrNum>`
//! - `Cx::params(&self, i: usize) -> Vec<(String, String)>`, `Cx::vtext(&self, v: usize) -> String`,
//!   `Cx::body(&self, i: usize) -> String`, `Cx::name(&self, i: usize) -> String`
//! - `Cx::pairs(&self, i: usize) -> Vec<(String, usize)>`,
//!   `Cx::get(&self, p: &[(String, usize)], k: &str) -> Option<usize>`
//! - `pub(super) fn unquoted(v: &str) -> Option<String>`
//! - `pub(super) fn one_group(v: &str) -> String`

use super::{Cx, trim};
use crate::codegen::Dialect;
use crate::import::Tag;
use crate::ir::*;

impl Cx<'_> {
    /// A `Num` row's value: its `Mag` (one decimal, `d` an exponent letter)
    /// scaled by its scale row, parsed once so `0.1n` is exactly 1e-10.
    pub(super) fn num(&self, i: usize) -> Option<f64> {
        // ngspice's own arithmetic, so a deck written back reads as the same
        // double (`0.5u` and `500n` differ in the last bit there).
        if matches!(self.d, Dialect::Ngspice | Dialect::Espice) {
            return crate::unit::ngspice_value(self.text(i));
        }
        let mag = self.kid(i, Tag::Mag)?;
        let mut m = self.text(mag).to_ascii_lowercase().replace('d', "e");
        // LTspice `6K34`: the digits after the scale are its fraction.
        if let Some(f) = self.kid(i, Tag::Frac) {
            m = format!("{m}.{}", self.text(f));
        }
        let scale = self
            .kids(i)
            .into_iter()
            .find(|&k| k != mag && self.tag(k) != Tag::Frac)
            .map(|k| self.tag(k));
        let (base, exp) = match m.split_once('e') {
            Some((b, e)) => (b.to_string(), e.parse::<i32>().ok()?),
            None => (m.clone(), 0),
        };
        let shift = match scale {
            Some(Tag::Mil) => return Some(m.parse::<f64>().ok()? * 25.4e-6),
            Some(Tag::Peta) => 15,
            Some(Tag::Tera) => 12,
            Some(Tag::Giga) => 9,
            Some(Tag::Meg) => 6,
            Some(Tag::Kilo) => 3,
            Some(Tag::Centi) => -2,
            Some(Tag::Milli) => -3,
            Some(Tag::Micro) => -6,
            Some(Tag::Nano) => -9,
            Some(Tag::Pico) => -12,
            Some(Tag::Femto) => -15,
            Some(Tag::Atto) => -18,
            _ => 0,
        };
        format!("{base}e{}", exp + shift).parse().ok()
    }

    /// The number a value row holds when it is one: a `Num`, or a `Word`/
    /// `Val` that is all one `Num`.
    pub(super) fn number(&self, i: usize) -> Option<f64> {
        match self.tag(i) {
            Tag::Num => self.num(i),
            Tag::Word | Tag::Val | Tag::MVal | Tag::Expr => {
                let k = self.kids(i);
                let t = self.text(i).trim();
                match k.as_slice() {
                    [n] if self.text(*n) == t => self.number(*n),
                    // A signed literal where the sign is an operator (Spectre,
                    // VACASK): `-1.5`.
                    [n] if t
                        .strip_prefix('-')
                        .is_some_and(|r| r.trim_start() == self.text(*n)) =>
                    {
                        self.number(*n).map(|x| -x)
                    }
                    [n] if t
                        .strip_prefix('+')
                        .is_some_and(|r| r.trim_start() == self.text(*n)) =>
                    {
                        self.number(*n)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(super) fn count(&self, i: usize) -> Option<u32> {
        self.number(i)
            .filter(|v| *v >= 0.0 && v.fract() == 0.0)
            .map(|v| v as u32)
    }

    /// A value field: a number, an expression (braced, quoted, or a bare
    /// word, which SPICE reads as a parameter), or text kept raw.
    pub(super) fn value(&self, i: usize) -> IrValue {
        if let Some(value) = self.number(i) {
            return IrValue::Numeric { value };
        }
        let text = self.text(i).trim();
        match self.kids(i).as_slice() {
            [e] if matches!(self.tag(*e), Tag::Expr) => IrValue::Expression {
                expr: self.respell(self.text(*e).trim()),
            },
            [s] if self.tag(*s) == Tag::Str => IrValue::Expression {
                expr: trim(&text[1..text.len() - 1]),
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

    pub(super) fn irnum(&self, i: usize) -> Option<IrNum> {
        match self.value(i) {
            IrValue::Numeric { value } => Some(IrNum::Num(value)),
            IrValue::Expression { expr } => Some(IrNum::Expr(expr)),
            IrValue::Raw { .. } => None,
        }
    }

    /// `k=v` rows (and bare flags, `(word, "")`) in card order.
    pub(super) fn params(&self, i: usize) -> Vec<(String, String)> {
        self.kids(i)
            .into_iter()
            .filter_map(|k| match self.tag(k) {
                Tag::Pair | Tag::MPair => {
                    let kv = self.kids(k);
                    Some((self.text(kv[0]).to_string(), self.vtext(kv[1])))
                }
                Tag::Flag => Some((self.text(k).to_string(), String::new())),
                _ => None,
            })
            .collect()
    }

    /// A value's text; a vector's further numbers (`More`) joined to it by
    /// one blank (`r=0.03  0` is `0.03 0`, `ic=0, -5` is `0,-5`).
    pub(super) fn vtext(&self, v: usize) -> String {
        let more = self.all(v, Tag::More);
        if more.is_empty() {
            if matches!(self.d, Dialect::Ngspice | Dialect::Espice) {
                return one_group(self.text(v).trim());
            }
            // A Spectre/VACASK number in the IR's SPICE spelling: `1M` must
            // not reach a SPICE reader as milli.
            if matches!(self.d, Dialect::Spectre | Dialect::Vacask)
                && let Some(x) = self.number(v)
            {
                return crate::codegen::format_spice_number(x);
            }
            return self.respell(self.text(v).trim());
        }
        // A comma stays a comma; a blank is one blank in a model's list and a
        // comma in an instance's (SPICE's comma vector).
        let blank = if self.tag(v) == Tag::Val { "," } else { " " };
        let mut out = String::new();
        let mut at = self.t.rows[v].start as usize;
        for m in more {
            let r = self.t.rows[m];
            let gap = &self.src[at..r.start as usize];
            let head = gap.trim_end();
            let sep = if head.ends_with(',') { "," } else { blank };
            out.push_str(head.trim_end_matches(',').trim_end());
            out.push_str(sep);
            out.push_str(self.text(m));
            at = r.end as usize;
        }
        out
    }

    /// A body kept whole (`B`, `.func`): its expression without delimiters.
    pub(super) fn body(&self, i: usize) -> String {
        match self.kid(i, Tag::Expr) {
            Some(e) => trim(self.text(e)),
            None => trim(self.text(i)),
        }
    }

    /// A name row's identifier: Spectre's `\x` escapes and VACASK's `'...'`
    /// parts are the bytes they spell.
    pub(super) fn name(&self, i: usize) -> String {
        let t = self.text(i);
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
            _ => t.to_string(),
        }
    }

    /// `k=v` rows of a statement, by key, values as written.
    pub(super) fn pairs(&self, i: usize) -> Vec<(String, usize)> {
        self.all(i, Tag::Pair)
            .into_iter()
            .map(|p| (self.text(self.kids(p)[0]).to_string(), self.kids(p)[1]))
            .collect()
    }

    pub(super) fn get(&self, p: &[(String, usize)], k: &str) -> Option<usize> {
        p.iter().find(|(n, _)| n == k).map(|(_, v)| *v)
    }
}

/// `"text"` without its quotes.
pub(super) fn unquoted(v: &str) -> Option<String> {
    Some(v.strip_prefix('"')?.strip_suffix('"')?.to_string())
}

/// An ngspice value as one expression, as ngspice evaluates it: `{"e"}` is
/// `{e}`, and braced groups with more after them (`{a} + b`) are one
/// braced expression, each group a parenthesis (`{(a) + b}`). Any other
/// value is itself.
pub(super) fn one_group(v: &str) -> String {
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

// ---- private ----

impl Cx<'_> {
    /// LTspice's `µ` is `u` (XVII help) in the text a SPICE reader gets.
    fn respell(&self, s: &str) -> String {
        if self.d == Dialect::Ltspice {
            s.replace('µ', "u")
        } else {
            s.to_string()
        }
    }
}
