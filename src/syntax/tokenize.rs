//! Text → [`Tokens`] under a [`Lexicon`]: one engine for every dialect.
//!
//! Each physical line is a comment, a blank, a stop, the `.end` card, or the
//! start of a logical statement. A statement runs on through `+` lines and
//! backslash breaks (and, in VACASK, through newlines inside brackets) and
//! ends with an [`Tag::EndOfStatement`]. Comments, blank lines and the
//! continuation marks are skipped, never tokens. A number is decoded once,
//! here (`number::value`). The tokenizer never fails: a byte no rule reads
//! is an [`Tag::Invalid`] token.
//!
//! The two hooks of `transpiler-architecture.md` §2.1 are here and nowhere
//! else: HSPICE's `$` ([`Dollar::Hspice`], in `comment_at`), and Spectre's
//! `simulator lang=spice` switch (`switch_at`), which swaps the lexicon for
//! the statements up to `simulator lang=spectre`.
//!
//! Cites: the line stages of `grammar/{ngspice,hspice,ltspice,xyce,spectre,
//! vacask}.peg` (today's reader, which this reproduces; each lexicon file
//! cites the manual clauses); ESPice `src/frontend/lines_hspice.zig:88-106`
//! (the `$` rule); Zig `tokenizer.zig:389` (never fails).
//!
//! Contents:
//! - `pub fn tokenize<'s>(source: &'s str, lexicon: &'static Lexicon) -> Tokens<'s>`

use crate::syntax::lexicon::{Backslash, Dollar, Lexicon, Quote};
use crate::syntax::number;
use crate::syntax::scan::{backslash_break, blanks, is_blank, line_end, token_at};
use crate::syntax::token::{Tag, Tokens};

/// What a physical line is, read at its first byte.
enum Line {
    Skip,
    Stop,
    End,
    Statement,
}

/// The tokenizer's cursor and the rows it appends.
struct Lexer<'s> {
    src: &'s [u8],
    pos: usize,
    /// The lexicon in force.
    lex: &'static Lexicon,
    /// The file's own lexicon while a `simulator lang=spice` region is open.
    home: Option<&'static Lexicon>,
    out: Tokens<'s>,
}

/// Tokenizes all of `source`, line 1 a [`Tag::Title`] when the lexicon has
/// one. Allocates the token rows (about `source.len() / 8` up front).
pub fn tokenize<'s>(source: &'s str, lexicon: &'static Lexicon) -> Tokens<'s> {
    let src = source.as_bytes();
    let mut line_starts = vec![0];
    line_starts.extend(
        src.iter()
            .enumerate()
            .filter(|&(_, &b)| b == b'\n')
            .map(|(i, _)| i as u32 + 1),
    );
    let mut l = Lexer {
        src,
        pos: 0,
        lex: lexicon,
        home: None,
        out: Tokens {
            source,
            lexicon,
            tags: Vec::with_capacity(src.len() / 8),
            starts: Vec::with_capacity(src.len() / 8),
            line_starts,
            number_tokens: Vec::new(),
            number_values: Vec::new(),
            switches: Vec::new(),
        },
    };
    if lexicon.title {
        l.push(Tag::Title, 0);
        l.pos = line_end(src, 0);
        l.push(Tag::EndOfStatement, l.pos);
        l.next_line();
    }
    while l.pos < src.len() {
        match l.line() {
            Line::Skip => l.next_line(),
            Line::Stop => break,
            Line::End => {
                // The card alone: the rest of its line is a comment.
                let at = l.pos + blanks(src, l.pos);
                let (tag, end) = token_at(src, at, l.lex);
                let tag = l.keyword(tag, at, end, true);
                l.push(tag, at);
                l.push(Tag::EndOfStatement, end);
                l.pos = line_end(src, at);
                if l.lex.end.is_some_and(|e| e.stops) {
                    break;
                }
                l.next_line();
            }
            Line::Statement => {
                l.statement();
                l.next_line();
            }
        }
    }
    l.out
}

// ---- private ----

impl Lexer<'_> {
    fn push(&mut self, tag: Tag, at: usize) {
        self.out.tags.push(tag);
        self.out.starts.push(at as u32);
    }

    /// Moves past the `\n` ending the current line.
    fn next_line(&mut self) {
        self.pos = (line_end(self.src, self.pos) + 1).min(self.src.len());
    }

    /// What the line at `self.pos` is, by the lexicon's line rules.
    fn line(&self) -> Line {
        let (src, lex, at) = (self.src, self.lex, self.pos);
        let lead = blanks(src, at);
        let first = at + lead;
        let rest = &src[first..line_end(src, at)];
        if rest.iter().all(|&b| is_blank(b)) {
            return Line::Skip;
        }
        if lex.stop == Some(rest[0]) {
            return Line::Stop;
        }
        // A SPICE region of a Spectre file has no `.end` (spectre.peg's
        // `Line_spice`).
        if let Some(end) = lex.end
            && self.home.is_none()
            && (!end.col1 || lead == 0)
            && rest.len() >= 4
            && rest[..4].eq_ignore_ascii_case(b".end")
        {
            let next = rest.get(4).copied().unwrap_or(b' ');
            let ok = if end.then_blank {
                is_blank(next)
            } else {
                !(next.is_ascii_alphanumeric() || next == b'_')
            };
            if ok {
                return Line::End;
            }
        }
        if self.comment_line(at, false) {
            return Line::Skip;
        }
        if lex.plus && rest[0] == b'+' {
            return Line::Skip;
        }
        Line::Statement
    }

    /// Whether the line at `at` is a comment line: for a `+` continuation to
    /// read past (`cont`), or at the top.
    fn comment_line(&self, at: usize, cont: bool) -> bool {
        let (src, lex) = (self.src, self.lex);
        let lead = blanks(src, at);
        let rest = &src[at + lead..line_end(src, at)];
        let starters = if cont {
            lex.cont_skip
        } else {
            lex.comment_lines
        };
        if src.get(at).is_some_and(|b| lex.comment_col1.contains(b)) {
            return true;
        }
        if lex.blank_led_comment && lead > 0 && rest.first() != Some(&b'+') {
            return true;
        }
        let Some(s) = starters.iter().find(|s| rest.starts_with(s.as_bytes())) else {
            return false;
        };
        // ngspice: a `//` line whose comment ends at a `\\` break is a
        // statement, joined to the next line (ngspice.peg `l_skip`).
        !(*s == "//" && lex.backslash == Backslash::Glue && glue_at_end(rest))
    }

    /// Lexes one logical statement from `self.pos`, leaving `self.pos` at its
    /// last line's end. Appends [`Tag::EndOfStatement`] when it holds a token.
    fn statement(&mut self) {
        let switch = self.switch_at(self.pos);
        let first = self.out.tags.len();
        self.pos += blanks(self.src, self.pos);
        let mut piece = self.pos;
        let mut quote = false;
        let mut dquote = false;
        let mut depth = 0u32;
        let mut head = true;
        loop {
            while self.pos < self.src.len() && is_blank(self.src[self.pos]) {
                self.pos += 1;
            }
            let p = self.pos;
            if p >= self.src.len() || self.src[p] == b'\n' {
                if p < self.src.len() && depth > 0 {
                    self.pos += 1;
                    continue;
                }
                (quote, dquote) = (false, false);
                if let Some(plus) = self.plus_line(p) {
                    self.pos = plus + 1;
                    piece = self.pos;
                    continue;
                }
                break;
            }
            let protected = match self.lex.quote {
                Quote::Expr(q) => quote && q.protects,
                Quote::Plain | Quote::Name => false,
            } || (dquote && self.lex.string.protects);
            if !protected && let Some(next) = self.comment_at(p, piece) {
                self.pos = next;
                continue;
            }
            if let Some(next) = backslash_break(self.src, p, self.lex) {
                self.pos = next;
                piece = next;
                if !matches!(self.lex.quote, Quote::Expr(q) if q.across_breaks) {
                    quote = false;
                }
                dquote &= self.lex.string.across_breaks;
                continue;
            }
            let (tag, end) = token_at(self.src, p, self.lex);
            let tag = self.keyword(tag, p, end, head);
            self.push(tag, p);
            if tag == Tag::Number {
                let text = std::str::from_utf8(&self.src[p..end]).unwrap_or("");
                self.out.number_tokens.push(self.out.tags.len() as u32 - 1);
                self.out.number_values.push(number::value(text, self.lex));
            }
            if tag == Tag::Quote && matches!(self.lex.quote, Quote::Expr(_)) {
                quote = !quote;
            }
            dquote ^= tag == Tag::DoubleQuote;
            if self.lex.brackets_join {
                match tag {
                    Tag::LParen | Tag::LBracket | Tag::LBrace => depth += 1,
                    Tag::RParen | Tag::RBracket | Tag::RBrace => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            head = self.lex.heads_after.contains(&tag);
            self.pos = end;
        }
        if self.out.tags.len() > first {
            self.push(Tag::EndOfStatement, self.pos);
        }
        if let Some(tag) = switch {
            self.out.switches.push(self.out.tags.len() as u32);
            self.push(tag, self.pos);
            (self.lex, self.home) = match self.home {
                Some(home) => (home, None),
                None => (self.lex.lang_spice.unwrap_or(self.lex), Some(self.lex)),
            };
        }
    }

    /// Spectre's hook: the switch the statement at `at` makes, read off the
    /// raw line as `spectre.peg` does (`l_to_spice` in lowercase only,
    /// `l_to_spectre` in any case).
    fn switch_at(&self, at: usize) -> Option<Tag> {
        let (to, tag) = match self.home {
            Some(_) => ("spectre", Tag::LangSpectre),
            None if self.lex.lang_spice.is_some() => ("spice", Tag::LangSpice),
            None => return None,
        };
        let fold = self.home.is_some();
        let word = |p: &mut usize, w: &str| {
            let hit = self.src.get(*p..*p + w.len()).is_some_and(|s| {
                if fold {
                    s.eq_ignore_ascii_case(w.as_bytes())
                } else {
                    s == w.as_bytes()
                }
            });
            *p += w.len();
            hit
        };
        let mut p = at + blanks(self.src, at);
        let simulator = word(&mut p, "simulator");
        let gap = blanks(self.src, p) > 0;
        p += blanks(self.src, p);
        if !(simulator && gap && word(&mut p, "lang")) {
            return None;
        }
        p += blanks(self.src, p);
        if !word(&mut p, "=") {
            return None;
        }
        p += blanks(self.src, p);
        let after = self.src.get(p + to.len()).copied().unwrap_or(b' ');
        let wchar = after.is_ascii_alphanumeric() || after == b'_' || after == b'!';
        (word(&mut p, to) && !wchar).then_some(tag)
    }

    /// The `+` line continuing the statement whose line ends at `nl`: past
    /// blank lines and the lexicon's continuation comment lines.
    fn plus_line(&self, nl: usize) -> Option<usize> {
        if !self.lex.plus {
            return None;
        }
        let mut at = nl + 1;
        while nl < self.src.len() && at < self.src.len() {
            let end = line_end(self.src, at);
            let first = at + blanks(self.src, at);
            if first < end && self.src[first] == b'+' {
                return Some(first);
            }
            if end == self.src.len() || !(first == end || self.comment_line(at, true)) {
                return None;
            }
            at = end + 1;
        }
        None
    }

    /// Where a comment starting at `p` ends (its line's end, a `\\` break
    /// in ngspice, a `*/`); `None` when none starts here.
    fn comment_at(&self, p: usize, piece: usize) -> Option<usize> {
        let (src, lex) = (self.src, self.lex);
        let rest = &src[p..];
        let to_eol = || {
            let end = line_end(src, p);
            if lex.backslash == Backslash::Glue {
                // ngspice.peg `l_comment`: a `\\` break still continues.
                (p..end)
                    .find(|&q| backslash_break(src, q, lex).is_some())
                    .unwrap_or(end)
            } else {
                end
            }
        };
        if lex
            .comment_tails
            .iter()
            .any(|s| rest.starts_with(s.as_bytes()))
        {
            return Some(to_eol());
        }
        if lex.block_comment && rest.starts_with(b"/*") {
            let close = rest[2..].windows(2).position(|w| w == b"*/")?;
            return Some(p + 2 + close + 2);
        }
        if rest[0] != b'$' {
            return None;
        }
        let after_sep = |q: usize, seps: &[u8]| q == piece || seps.contains(&src[q - 1]);
        let comment = match lex.dollar {
            Dollar::Text => false,
            Dollar::AfterBlank => after_sep(p, b" \t\r,"),
            Dollar::Hspice => {
                const SEPS: &[u8] = b" \t\r,=()'\"";
                // hspice.peg `l_numtok`: a signed number with letters only
                // after its digits, a chunk of its own.
                let number = self.out.tags.last() == Some(&Tag::Number)
                    && self.out.starts.last().is_some_and(|&s| {
                        let s = s as usize;
                        let digits = src[s..p]
                            .iter()
                            .rposition(|b| !b.is_ascii_alphabetic())
                            .map_or(0, |k| k + 1);
                        token_at(src, s, lex).1 == p
                            && src[s..s + digits]
                                .iter()
                                .all(|b| b.is_ascii_digit() || b".eEdD+-".contains(b))
                            && s >= piece
                            && (after_sep(s, SEPS)
                                || (s > piece
                                    && matches!(src[s - 1], b'+' | b'-')
                                    && after_sep(s - 1, SEPS)))
                    });
                after_sep(p, SEPS) || number
            }
        };
        comment.then(to_eol)
    }

    /// A word's keyword tag, by the lexicon's tables, or `tag` itself.
    fn keyword(&self, tag: Tag, start: usize, end: usize, head: bool) -> Tag {
        let looked_up = match tag {
            Tag::Directive => true,
            Tag::Ident => head && self.lex.head_words,
            _ => false,
        };
        let word = std::str::from_utf8(&self.src[start..end]).unwrap_or("");
        looked_up
            .then(|| self.lex.keyword(word))
            .flatten()
            .unwrap_or(tag)
    }
}

/// Whether `rest` (one line, no `\n`) ends with `\\` and blanks.
fn glue_at_end(rest: &[u8]) -> bool {
    let body = rest.trim_ascii_end();
    body.ends_with(b"\\\\")
}
