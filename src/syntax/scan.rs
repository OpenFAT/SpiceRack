//! Bytes at an offset → one token: its [`Tag`] and its end, under a
//! [`Lexicon`]. The tokenizer (`tokenize.rs`) calls it for every token and
//! [`crate::syntax::token::Tokens::end`] re-lexes with it, so a token's end
//! depends only on its first byte's position, never on the line it is in.
//! Also the byte classes both share: blanks and backslash breaks.
//!
//! Cites: the word, number and string rules of `grammar/*.peg` (`l_word`,
//! `l_quoted`, `l_string`, `l_dstring`, `l_longstr`, `l_sqname`, `Name`,
//! `Num`); Zig `Ast.zig:279-295` (re-lexing a token from its start).
//!
//! Contents:
//! - `pub(super) fn token_at(src: &[u8], p: usize, lex: &Lexicon) -> (Tag, usize)`
//! - `pub(super) fn line_end(src: &[u8], p: usize) -> usize`
//! - `pub(super) fn backslash_break(src: &[u8], p: usize, lex: &Lexicon) -> Option<usize>`
//! - `pub(super) fn is_blank(b: u8) -> bool`, `pub(super) fn blanks(src: &[u8], p: usize) -> usize`

use crate::syntax::lexicon::{Backslash, Lexicon, Quote, Tail};
use crate::syntax::token::Tag;

/// The token at `p` (never a blank, comment or end of line) and its end:
/// what [`Tokens::end`](crate::syntax::token::Tokens::end) re-lexes.
/// Keywords come back as the word they are.
pub(super) fn token_at(src: &[u8], p: usize, lex: &Lexicon) -> (Tag, usize) {
    let c = src[p];
    let next = src.get(p + 1).copied().unwrap_or(0);
    if c.is_ascii_digit() || (c == b'.' && next.is_ascii_digit()) {
        return (Tag::Number, number_end(src, p, lex));
    }
    if c == b'"' {
        return (Tag::String, string_end(src, p, lex));
    }
    if lex.long_strings
        && let Some(end) = long_string_end(src, p)
    {
        return (Tag::String, end);
    }
    if Some(c) == lex.directive && ident_start(src, p + 1, lex) {
        return (Tag::Directive, word_end(src, p + 1, lex));
    }
    if ident_start(src, p, lex) {
        return (Tag::Ident, word_end(src, p, lex));
    }
    if let Some(&(s, tag)) = lex
        .operators
        .iter()
        .find(|(s, _)| src[p..].starts_with(s.as_bytes()))
    {
        return (tag, p + s.len());
    }
    let tag = match c {
        b'(' => Tag::LParen,
        b')' => Tag::RParen,
        b'[' => Tag::LBracket,
        b']' => Tag::RBracket,
        b'{' => Tag::LBrace,
        b'}' => Tag::RBrace,
        b',' => Tag::Comma,
        b'=' => Tag::Equal,
        b'\'' => Tag::Quote,
        b':' => Tag::Colon,
        b';' => Tag::Semicolon,
        b'?' => Tag::Question,
        b'+' => Tag::Plus,
        b'-' => Tag::Minus,
        b'*' => Tag::Star,
        b'/' => Tag::Slash,
        b'\\' => Tag::Backslash,
        b'%' => Tag::Percent,
        b'^' => Tag::Caret,
        b'!' => Tag::Bang,
        b'<' => Tag::Less,
        b'>' => Tag::Greater,
        b'&' => Tag::Amp,
        b'|' => Tag::Pipe,
        b'~' => Tag::Tilde,
        b'.' => Tag::Dot,
        b'@' => Tag::At,
        b'#' => Tag::Hash,
        b'`' => Tag::Backtick,
        // else: a control byte (letters, digits, `_`, `$`, `"` and non-ASCII
        // were read above; blanks and newlines never reach here).
        _ => Tag::Invalid,
    };
    (tag, p + 1)
}

/// The index of the `\n` ending the line `p` is in, or the text's end.
pub(super) fn line_end(src: &[u8], p: usize) -> usize {
    src[p..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(src.len(), |k| p + k)
}

/// Where the text continues when a backslash break starts at `p`: the next
/// line's first byte, or the text's end.
pub(super) fn backslash_break(src: &[u8], p: usize, lex: &Lexicon) -> Option<usize> {
    let to_eol = |q: usize| {
        let q = q + blanks(src, q);
        match src.get(q) {
            None => Some(src.len()),
            Some(b'\n') => Some(q + 1),
            Some(_) => None,
        }
    };
    let rest = &src[p..];
    match lex.backslash {
        Backslash::None => None,
        Backslash::Glue => rest.starts_with(b"\\\\").then(|| to_eol(p + 2)).flatten(),
        Backslash::Hspice if rest.starts_with(b"\\\\") => to_eol(p + 2),
        Backslash::Hspice => (rest.first() == Some(&b'\\') && p > 0 && is_blank(src[p - 1]))
            .then(|| to_eol(p + 1))
            .flatten(),
        Backslash::Join => (rest.first() == Some(&b'\\'))
            .then(|| to_eol(p + 1))
            .flatten(),
        Backslash::Tight => match rest {
            [b'\\', b'\n', ..] => Some(p + 2),
            [b'\\', b'\r', b'\n', ..] => Some(p + 3),
            _ => None,
        },
    }
}

/// Blanks: space, tab, carriage return, form feed.
pub(super) fn is_blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | 0x0c)
}

/// The number of blanks at `p`.
pub(super) fn blanks(src: &[u8], p: usize) -> usize {
    src[p.min(src.len())..]
        .iter()
        .take_while(|&&b| is_blank(b))
        .count()
}

// ---- private ----

fn ident_start(src: &[u8], p: usize, lex: &Lexicon) -> bool {
    let Some(&c) = src.get(p) else {
        return false;
    };
    c.is_ascii_alphabetic()
        || c == b'_'
        || c >= 0x80
        || lex.ident_start.contains(&c)
        || escape_len(src, p, lex) > 0
        || name_quote_len(src, p, lex) > 0
}

/// The end of the name starting at `p`.
fn word_end(src: &[u8], p: usize, lex: &Lexicon) -> usize {
    let mut q = p;
    while let Some(&c) = src.get(q) {
        let step = if c.is_ascii_alphanumeric()
            || c == b'_'
            || c >= 0x80
            || lex.ident_continue.contains(&c)
            || (q == p && lex.ident_start.contains(&c))
        {
            1
        } else {
            escape_len(src, q, lex).max(name_quote_len(src, q, lex))
        };
        if step == 0 {
            break;
        }
        q += step;
    }
    q
}

/// Spectre §2.2: `\` and a printable byte inside a name.
fn escape_len(src: &[u8], p: usize, lex: &Lexicon) -> usize {
    let hit = lex.name_escape
        && src.get(p) == Some(&b'\\')
        && src.get(p + 1).is_some_and(|&b| (0x21..=0x7e).contains(&b));
    if hit { 2 } else { 0 }
}

/// VACASK `'…'` name part: non-blank bytes, `''` a quote; 0 if unclosed.
fn name_quote_len(src: &[u8], p: usize, lex: &Lexicon) -> usize {
    if lex.quote != Quote::Name || src.get(p) != Some(&b'\'') {
        return 0;
    }
    let mut q = p + 1;
    loop {
        match src.get(q) {
            Some(b'\'') if src.get(q + 1) == Some(&b'\'') => q += 2,
            Some(b'\'') => return q + 1 - p,
            Some(&b) if !matches!(b, b' ' | b'\t' | b'\r' | b'\n') => q += 1,
            _ => return 0,
        }
    }
}

/// The end of the number at `p`: mantissa, exponent, a punctuation scale
/// (Spectre `%`), then the unit tail.
fn number_end(src: &[u8], p: usize, lex: &Lexicon) -> usize {
    let digits = |q: usize| src[q..].iter().take_while(|b| b.is_ascii_digit()).count();
    let mut q = p + digits(p);
    if src.get(q) == Some(&b'.') {
        q += 1 + digits(q + 1);
    }
    if src.get(q).is_some_and(|b| lex.numbers.exponent.contains(b)) {
        let sign = usize::from(matches!(src.get(q + 1), Some(b'+' | b'-')));
        if src.get(q + 1 + sign).is_some_and(u8::is_ascii_digit) {
            q += 1 + sign + digits(q + 1 + sign);
        }
    }
    if let Some((s, _)) = lex.numbers.scales.iter().find(|(s, _)| {
        !s.as_bytes()[0].is_ascii_alphanumeric() && src[q..].starts_with(s.as_bytes())
    }) {
        q += s.len();
    }
    let dot = matches!(lex.numbers.tail, Tail::Spice { dot: true } | Tail::Ltspice);
    q + src[q..]
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80 || (dot && b == b'.'))
        .count()
}

/// The end of the `"…"` string at `p`: its closing quote, else its line's
/// end or a backslash break (which a string that continues across breaks
/// reads through), or a comment starter it does not protect.
fn string_end(src: &[u8], p: usize, lex: &Lexicon) -> usize {
    let q_rule = lex.string;
    let mut q = p + 1;
    while q < src.len() {
        match src[q] {
            b'"' => return q + 1,
            b'\n' => return q,
            b'\r' if src.get(q + 1) == Some(&b'\n') => return q,
            b'\\' if q_rule.escapes && !matches!(src.get(q + 1), None | Some(b'\n')) => {
                if backslash_break(src, q, lex).is_some() && !q_rule.across_breaks {
                    return q;
                }
                q += 2;
                continue;
            }
            _ => {}
        }
        if let Some(next) = backslash_break(src, q, lex) {
            if !q_rule.across_breaks {
                return q;
            }
            q = next;
            continue;
        }
        if !q_rule.protects
            && lex
                .comment_tails
                .iter()
                .any(|s| src[q..].starts_with(s.as_bytes()))
        {
            return q;
        }
        q += 1;
    }
    q
}

/// VACASK L695: `<<<MARK`, a newline, then up to the first `>>>word`.
fn long_string_end(src: &[u8], p: usize) -> Option<usize> {
    let rest = src[p..].strip_prefix(b"<<<")?;
    let mark = rest
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
        .count();
    let after = &rest[mark..];
    let pad = after
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t' | b'\r'))
        .count();
    if mark == 0 || after.get(pad) != Some(&b'\n') {
        return None;
    }
    let body = p + 3 + mark + pad + 1;
    let close = src[body..]
        .windows(4)
        .position(|w| w[..3] == *b">>>" && (w[3].is_ascii_alphanumeric() || w[3] == b'_'))?;
    let word = body + close + 3;
    Some(
        word + src[word..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
            .count(),
    )
}
