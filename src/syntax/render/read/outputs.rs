//! SPICE output lists (`.save`, `.print`, `.four`, `.tf`, `.sens`, HSPICE
//! `.probe`) → their outputs' texts, as the old SPICE grammar's `osep`/`Out`
//! cut them: a group (`v(a)`), braces, a quoted run, `=`, or name bytes, cut
//! at blanks and commas (HSPICE: any blank-free run).
//!
//! Cites: the old SPICE grammar's `Save`, `osep`, `Out`, `group`, `gfield`, `Print`,
//! `PrintKind`; the old HSPICE grammar's `Out`, `Probe`.
//!
//! Contents:
//! - `fn print_kind(rest: &str) -> (&str, &str)`
//! - `Rd::rest_text(&self, n: u32) -> String`
//! - `Rd::outs_after(&self, n: u32, skip: usize) -> Option<Vec<String>>`
//! - `Rd::outs_text(&self, rest: &str) -> Option<Vec<String>>`
//! - `Rd::one_out<'r>(&self, rest: &'r str) -> Option<(String, &'r str)>`
//! - `Rd::probe_outs(&self, n: u32) -> Option<Vec<String>>`

use super::Rd;
use crate::syntax::Dialect;

/// the old SPICE grammar's `(s PrintKind)?`: the analysis word after `.print`, and the
/// text after it.
pub(in crate::syntax::render) fn print_kind(rest: &str) -> (&str, &str) {
    const KINDS: &[&str] = &[
        "noise", "disto", "tran", "sens", "four", "op", "dc", "ac", "tf", "pz",
    ];
    let blanks = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    if blanks == 0 {
        return ("", rest);
    }
    let w = &rest[blanks..];
    for k in KINDS {
        if w.len() >= k.len() && w[..k.len()].eq_ignore_ascii_case(k) {
            let after = &w[k.len()..];
            let namechar = after
                .bytes()
                .next()
                .is_some_and(|c| !b" \t\n,=()'\"{}".contains(&c));
            if !namechar {
                return (&w[..k.len()], after);
            }
        }
    }
    ("", rest)
}

impl<'a> Rd<'a> {
    /// The card's text after its head token, as the line stage joined it
    /// (a leading blank kept when there was one).
    pub(in crate::syntax::render) fn rest_text(&self, n: u32) -> String {
        let first = self.main(n);
        let last = self.line_last(first);
        if last == first {
            return String::new();
        }
        let s = self.span(first, last);
        s[self.t.text(first).len()..].to_string()
    }

    /// The outputs after the head and `skip` items: the old SPICE grammar's `osep* s?`.
    pub(in crate::syntax::render) fn outs_after(&self, n: u32, skip: usize) -> Option<Vec<String>> {
        debug_assert_eq!(skip, 0);
        self.outs_text(&self.rest_text(n))
    }

    /// quirk: the old SPICE grammar's `osep* s?` over `rest`: outputs after
    /// a blank or a comma, each an `Out` of the old SPICE grammar (of the
    /// old HSPICE grammar: any blank-free run), and nothing else.
    pub(in crate::syntax::render) fn outs_text(&self, rest: &str) -> Option<Vec<String>> {
        let b = rest.as_bytes();
        let mut i = 0;
        let mut outs = Vec::new();
        loop {
            let blanks = b[i..]
                .iter()
                .take_while(|c| matches!(c, b' ' | b'\t'))
                .count();
            let mut j = i + blanks;
            let comma = b.get(j) == Some(&b',');
            if comma {
                j += 1;
                j += b[j..]
                    .iter()
                    .take_while(|c| matches!(c, b' ' | b'\t'))
                    .count();
            } else if blanks == 0 {
                break;
            }
            match self.out_end(b, j) {
                Some(e) => {
                    outs.push(rest[j..e].to_string());
                    i = e;
                }
                None => {
                    i = if comma { usize::MAX } else { i + blanks };
                    break;
                }
            }
        }
        (i != usize::MAX
            && rest[i.min(rest.len())..]
                .trim_matches([' ', '\t'])
                .is_empty())
        .then_some(outs)
    }

    /// One `Out` at the start of `rest` (after its blanks), and the text
    /// after it.
    pub(in crate::syntax::render) fn one_out<'r>(
        &self,
        rest: &'r str,
    ) -> Option<(String, &'r str)> {
        let b = rest.as_bytes();
        let blanks = b.iter().take_while(|c| matches!(c, b' ' | b'\t')).count();
        if blanks == 0 {
            return None;
        }
        let e = self.out_end(b, blanks)?;
        Some((rest[blanks..e].to_string(), &rest[e..]))
    }

    /// the old HSPICE grammar's `Probe`: analysis words and blank-free outputs.
    pub(in crate::syntax::render) fn probe_outs(&self, n: u32) -> Option<Vec<String>> {
        let rest = self.rest_text(n);
        let mut outs = Vec::new();
        for w in rest.split([' ', '\t']).filter(|w| !w.is_empty()) {
            if !["noise", "tran", "op", "dc", "ac"]
                .iter()
                .any(|k| w.eq_ignore_ascii_case(k))
            {
                outs.push(w.to_string());
            }
        }
        Some(outs)
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// The end of the `Out` at `i`, if one starts there.
    fn out_end(&self, b: &[u8], i: usize) -> Option<usize> {
        if self.d == Dialect::Hspice {
            let e = i + b[i..]
                .iter()
                .take_while(|c| !matches!(c, b' ' | b'\t' | b'\n'))
                .count();
            return (e > i).then_some(e);
        }
        let mut j = i;
        loop {
            if let Some(e) = group_end(b, j) {
                j = e;
                continue;
            }
            match b.get(j) {
                Some(b'{') => j = nested_end(b, j, b'{', b'}')?,
                Some(b'\'') => {
                    let k = b[j + 1..].iter().position(|&c| c == b'\'' || c == b'\n')?;
                    if b[j + 1 + k] != b'\'' {
                        return None;
                    }
                    j += k + 2;
                }
                Some(b'=') => {
                    j += 1;
                    j += b[j..]
                        .iter()
                        .take_while(|c| matches!(c, b' ' | b'\t'))
                        .count();
                }
                Some(c) if !b" \t\n,({'=".contains(c) => j += 1,
                _ => break,
            }
        }
        (j > i).then_some(j)
    }
}

/// the old SPICE grammar's `group <- word s? '(' s? (gfield (s? gfield)*)? s? ')'` at
/// `i`: its end. A `gfield` is a group, a braced or quoted expression, a
/// string, a word, `,` or `=`.
fn group_end(b: &[u8], i: usize) -> Option<usize> {
    let wordchar = |c: u8| !b" \t\n,=()'\"{}[]".contains(&c);
    let word_end = |mut j: usize| {
        while j < b.len() && (wordchar(b[j]) || b[j] == b'[' || b[j] == b']') {
            j += 1;
        }
        j
    };
    let blanks = |j: usize| {
        j + b[j..]
            .iter()
            .take_while(|c| matches!(c, b' ' | b'\t'))
            .count()
    };
    let mut j = word_end(i);
    if j == i {
        return None;
    }
    j = blanks(j);
    if b.get(j) != Some(&b'(') {
        return None;
    }
    j += 1;
    loop {
        j = blanks(j);
        match *b.get(j)? {
            b')' => return Some(j + 1),
            b',' | b'=' => j += 1,
            b'{' => j = nested_end(b, j, b'{', b'}')?,
            q @ (b'\'' | b'"') => {
                let k = b[j + 1..].iter().position(|&c| c == q || c == b'\n')?;
                if b[j + 1 + k] != q {
                    return None;
                }
                j += k + 2;
            }
            _ => {
                let e = word_end(j);
                if e == j {
                    return None;
                }
                j = group_end(b, j).unwrap_or(e);
            }
        }
    }
}

/// The end of the bracket run opened at `i` (one past its closer).
fn nested_end(b: &[u8], i: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0;
    for (k, &c) in b.iter().enumerate().skip(i) {
        if c == b'\n' {
            return None;
        }
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(k + 1);
            }
        }
    }
    None
}
