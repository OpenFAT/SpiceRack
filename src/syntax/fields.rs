//! A statement's items after its head → field nodes: `Word`s (gapless
//! tokens), `Group`s (`word(items)`), `Delimited` expressions (`{…}`,
//! `'…'`, HSPICE `"…"`), strings and `key = value` `Assign`s. Every head
//! shares this reader; a [`Mode`] says what the dialect's card shape
//! makes of brackets and of a value's extent.
//!
//! A value after `=` is one expression: a gapless word, or an expression
//! that runs on across blanks while an operator follows (ngspice `r = a +
//! b`, `{1+x} + x*AGAUSS(0,1,1)`), but an operator followed by `key =` is a
//! separator (sky130's `a = '-1*x' + b = '0 - 2.5'`).
//!
//! Cites: the old SPICE grammar's §1.8 fields (`field`, `group`, `Pair`,
//! `value`), `MVal` (a model's blank-separated matrix), `Val_i` (SPICE's
//! comma vector), `Path`; ngspice manual §2.1 (`{expr}`, `'expr'`),
//! §2.4.1; HSPICE UG08 T2 (`'…'`, `"…"`).
//!
//! Contents:
//! - `pub(super) enum Mode`
//! - `Parser::fields(&mut self, mode: Mode) -> u32`, `Parser::items(&mut self, mode: Mode) -> Vec<u32>`
//! - `Parser::word(&mut self) -> Node`, `Parser::word_end(&self, i: u32) -> u32`
//! - `Parser::delimited(&mut self) -> Node`, `Parser::starts_delimited(&self) -> bool`
//! - `Parser::pairstart(&self, i: u32) -> bool`, `Parser::separates(&self, i: u32) -> bool`

use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::diag::Code;
use crate::syntax::lexicon::Quote;
use crate::syntax::parse::{DEPTH_MAX, Parser};
use crate::syntax::token::Tag as T;

/// How a card shape reads its items.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    /// `word(…)` is a group; a value is one expression.
    Fields,
    /// As `Fields`, and a value runs on through further numbers after a
    /// blank or a comma (an element's `ic=0,-5`).
    Element,
    /// Parentheses and commas only separate (`.subckt s (a b) w=1`).
    Plain,
    /// As `Plain`, and a value is one word or delimited expression, run on
    /// through blank-separated numbers (`.model`'s `c=1e-10 -6e-11`).
    Model,
    /// Blank-separated chunks, each a `Word` of any tokens or a string (a
    /// path: `.include a/b(1).sp`).
    Chunks,
}

impl Parser<'_, '_> {
    /// Reads the items from the cursor to the statement's end; returns the
    /// fields list.
    pub(super) fn fields(&mut self, mode: Mode) -> u32 {
        let items = self.items(mode);
        self.ast.push_list(&items)
    }

    /// The items from the cursor to the statement's end, not yet a list.
    pub(super) fn items(&mut self, mode: Mode) -> Vec<u32> {
        let mut items = Vec::new();
        while self.i < self.end {
            if let Some(n) = self.item(mode) {
                items.push(n.0);
            }
        }
        items
    }

    /// The last token of the word starting at `i`: tokens that touch, up to
    /// a separator.
    pub(super) fn word_end(&self, i: u32) -> u32 {
        let mut j = i;
        while j + 1 < self.end && !self.separates(j + 1) && self.touches(j, j + 1) {
            j += 1;
        }
        j
    }

    /// A `Word` from the cursor (which must not sit on a separator).
    pub(super) fn word(&mut self) -> Node {
        debug_assert!(!self.separates(self.i), "a word starts at a word token");
        let (a, b) = (self.i, self.word_end(self.i));
        self.i = b + 1;
        self.push_word(a, b)
    }

    /// Whether the cursor opens a delimited expression.
    pub(super) fn starts_delimited(&self) -> bool {
        match self.t.tags()[self.i as usize] {
            T::LBrace => self.t.lexicon_at(self.i).expr_braces,
            T::Quote => matches!(self.t.lexicon_at(self.i).quote, Quote::Expr(_)),
            T::DoubleQuote => true,
            _ => false,
        }
    }

    /// `{expr}`, `'expr'` or HSPICE `"expr"` at the cursor.
    pub(super) fn delimited(&mut self) -> Node {
        let open = self.i;
        let Some(close) = self.closer_of(open) else {
            self.ast.diag(Code::P0008, open);
            let last = self.end - 1;
            self.i = self.end;
            return self.ast.push(Tag::Bad, open, last, NONE);
        };
        let saved = self.end;
        self.end = close;
        self.i = open + 1;
        let mark = self.ast.mark();
        let inner = if self.i == close {
            self.ast.diag(Code::P0002, open);
            NONE
        } else {
            self.expr_all().0
        };
        self.end = saved;
        self.i = close + 1;
        let tag = self.t.tags()[open as usize];
        if tag == T::DoubleQuote && (inner == NONE || self.ast.tag(Node(inner)) == Tag::Bad) {
            // HSPICE reads a `"…"` that holds no expression as one bad
            // value, not as a delimiter around one.
            self.ast.reset(mark);
            self.ast.diag(Code::P0002, open);
            return self.ast.push(Tag::Bad, open, close, NONE);
        }
        self.ast.push(Tag::Delimited, open, inner, close)
    }

    /// Whether token `i` separates words (the old SPICE grammar's `namechar`): blanks are
    /// gaps, and `,` `=` `(` `)` `'` `"` are never name bytes; nor are
    /// braces where they delimit expressions.
    pub(super) fn separates(&self, i: u32) -> bool {
        // Spectre and VACASK: a name is letters, digits, escapes and quoted
        // parts (one token), joined by `.` and `:` (`X1.n3`, `x1:p`).
        if self.t.lexicon_at(i).named {
            return !matches!(
                self.t.tags()[i as usize],
                T::Ident | T::Number | T::Dot | T::Colon
            );
        }
        match self.t.tags()[i as usize] {
            T::Comma
            | T::Equal
            | T::EqualEqual
            | T::BangEqual
            | T::LessEqual
            | T::GreaterEqual
            | T::LParen
            | T::RParen
            | T::Quote
            | T::DoubleQuote
            | T::String
            | T::EndOfStatement
            | T::Title
            | T::LangSpice
            | T::LangSpectre => true,
            T::LBrace | T::RBrace => self.t.lexicon_at(i).expr_braces,
            _ => false,
        }
    }

    /// Whether `word =` starts at `i`: a key and its `=` (not `==`).
    pub(super) fn pairstart(&self, i: u32) -> bool {
        if i >= self.end || self.separates(i) {
            return false;
        }
        let w = self.word_end(i);
        w + 1 < self.end && self.t.tags()[w as usize + 1] == T::Equal && self.key_shaped(i)
    }
}

// ---- private ----

impl Parser<'_, '_> {
    /// One item at the cursor, or `None` for a separator it skipped.
    fn item(&mut self, mode: Mode) -> Option<Node> {
        let tag = self.t.tags()[self.i as usize];
        if mode == Mode::Chunks {
            return Some(self.chunk());
        }
        match tag {
            // the old SPICE grammar's `field`: a stray `, ( ) =` only separates.
            T::Comma
            | T::Equal
            | T::RParen
            | T::LParen
            | T::EqualEqual
            | T::BangEqual
            | T::LessEqual
            | T::GreaterEqual => {
                self.i += 1;
                None
            }
            _ if self.starts_delimited() => Some(self.delimited()),
            T::String => {
                self.i += 1;
                Some(self.ast.push(Tag::Str, self.i - 1, NONE, NONE))
            }
            _ if self.separates(self.i) => {
                // A separator no item starts with (`}` with no `{`).
                self.i += 1;
                None
            }
            _ => {
                let w = self.word();
                // Spectre's and VACASK's parentheses only bracket a node or
                // parameter list (`(r=1)`, `sp_diode ( is=1 )`).
                let named = self.t.lexicon_at(self.i - 1).named;
                let grouped =
                    matches!(mode, Mode::Fields | Mode::Element) && self.at(T::LParen) && !named;
                let key = if grouped { self.group(w.0) } else { w };
                if self.at(T::Equal) && (grouped || self.key_shaped(self.ast.main_token(w))) {
                    self.i += 1;
                    let v = self.value(mode);
                    return Some(
                        self.ast
                            .push(Tag::Assign, self.ast.main_token(w), key.0, v.0),
                    );
                }
                Some(key)
            }
        }
    }

    fn at(&self, tag: T) -> bool {
        self.i < self.end && self.t.tags()[self.i as usize] == tag
    }

    /// A key starts with a letter or `_` (the old SPICE grammar's `Key`).
    /// Spectre's and VACASK's keys are names (`$mfactor`, `'a b'`, `\x`).
    fn key_shaped(&self, i: u32) -> bool {
        let named = self.t.lexicon_at(i).named;
        self.t.text(i).bytes().next().is_some_and(|b| {
            b.is_ascii_alphabetic() || b == b'_' || named && matches!(b, b'$' | b'\'' | b'\\')
        })
    }

    /// A blank-separated chunk: a string, a quoted run, or touching tokens
    /// of any kind.
    fn chunk(&mut self) -> Node {
        let a = self.i;
        let tags = self.t.tags();
        if tags[a as usize] == T::String {
            self.i += 1;
            return self.ast.push(Tag::Str, a, NONE, NONE);
        }
        let q = tags[a as usize];
        if q == T::DoubleQuote
            && let Some(c) = (a + 1..self.end).find(|&k| tags[k as usize] == q)
        {
            // HSPICE's `"path"`: a string, its tokens inside.
            self.i = c + 1;
            return self.ast.push(Tag::Str, a, c, NONE);
        }
        let mut b = a;
        if q == T::Quote
            && let Some(c) = (a + 1..self.end).find(|&k| tags[k as usize] == q)
        {
            b = c;
        }
        while b + 1 < self.end && self.touches(b, b + 1) {
            b += 1;
        }
        self.i = b + 1;
        self.push_word(a, b)
    }

    /// The `)` (or `}` or `'`) closing the delimiter at `open`, on this
    /// statement.
    fn closer_of(&self, open: u32) -> Option<u32> {
        let tags = self.t.tags();
        let (o, c) = match tags[open as usize] {
            T::LBrace => (T::LBrace, T::RBrace),
            T::LParen => (T::LParen, T::RParen),
            q => return (open + 1..self.end).find(|&k| tags[k as usize] == q),
        };
        let mut depth = 0u32;
        for k in open..self.end {
            match tags[k as usize] {
                t if t == o => depth += 1,
                t if t == c => {
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

    /// `word(items)`, the cursor on `(`; `name` the word or `NONE` for a
    /// bare `(`.
    fn group(&mut self, name: u32) -> Node {
        let open = self.i;
        let first = if name == NONE {
            open
        } else {
            self.ast.main_token(Node(name))
        };
        self.i += 1;
        self.depth += 1;
        let mut items = Vec::new();
        let close = loop {
            if self.i >= self.end {
                self.ast.diag(Code::P0008, open);
                break self.end - 1;
            }
            if self.depth > DEPTH_MAX {
                self.ast.diag(Code::P0006, self.i);
                self.i = self.end;
                break self.end - 1;
            }
            match self.t.tags()[self.i as usize] {
                T::RParen => {
                    self.i += 1;
                    break self.i - 1;
                }
                T::LParen => {
                    let g = self.group(NONE);
                    items.push(g.0);
                }
                T::Comma | T::Equal => self.i += 1,
                _ if self.starts_delimited() => {
                    let d = self.delimited();
                    items.push(d.0);
                }
                T::String => {
                    items.push(self.ast.push(Tag::Str, self.i, NONE, NONE).0);
                    self.i += 1;
                }
                _ if self.separates(self.i) => self.i += 1,
                _ => {
                    let w = self.word();
                    let item = if self.at(T::LParen) {
                        self.group(w.0)
                    } else {
                        w
                    };
                    items.push(item.0);
                }
            }
        };
        self.depth -= 1;
        let mut words = vec![close, items.len() as u32];
        words.extend(items);
        let at = self.ast.push_extra(&words);
        self.ast.push(Tag::Group, first, name, at)
    }

    /// The value after a key's `=`.
    fn value(&mut self, mode: Mode) -> Node {
        if self.i >= self.end {
            self.ast.diag(Code::P0002, self.i - 1);
            return Node(NONE);
        }
        let v = match mode {
            Mode::Model => self.atom_value(),
            _ => self.expr_value(),
        };
        let blank_only = match mode {
            Mode::Model => true,
            Mode::Element => false,
            Mode::Fields | Mode::Plain | Mode::Chunks => return v,
        };
        self.sequence(v, blank_only)
    }

    /// One word, string or delimited expression.
    fn atom_value(&mut self) -> Node {
        if self.starts_delimited() {
            return self.delimited();
        }
        if self.at(T::String) {
            self.i += 1;
            return self.ast.push(Tag::Str, self.i - 1, NONE, NONE);
        }
        if self.separates(self.i) {
            self.ast.diag(Code::P0002, self.i);
            return Node(NONE);
        }
        self.word()
    }

    /// A value read as an expression where one reaches at least as far as
    /// the gapless word at the cursor, else that word.
    fn expr_value(&mut self) -> Node {
        let start = self.i;
        if self.starts_delimited() {
            let d = self.delimited();
            return self.continue_expr(d);
        }
        let word_end = if self.separates(start) {
            start
        } else {
            self.word_end(start)
        };
        let mark = self.ast.mark();
        let (saved_depth, capped) = (self.depth, self.capped);
        match self.expr_bp(0) {
            Ok(e) if self.i > word_end => e,
            _ => {
                self.ast.reset(mark);
                self.depth = saved_depth;
                self.i = start;
                if self.capped && !capped {
                    self.ast.diag(Code::P0006, start);
                }
                if self.separates(start) {
                    self.ast.diag(Code::P0002, start);
                    self.i += 1;
                    return self.ast.push(Tag::Bad, start, start, NONE);
                }
                self.word()
            }
        }
    }

    /// A value continued by numbers: blank-separated (a model's matrix) or
    /// also comma-separated (an element's vector); a `Seq` when any follow.
    fn sequence(&mut self, first: Node, blank_only: bool) -> Node {
        let mut parts = vec![first.0];
        loop {
            let mut k = self.i;
            if k >= self.end || first.0 == NONE {
                break;
            }
            let comma = !blank_only && self.t.tags()[k as usize] == T::Comma;
            if comma {
                k += 1;
                while k < self.end && self.t.tags()[k as usize] == T::Comma {
                    k += 1;
                }
            } else if self.touches(k - 1, k) {
                break;
            }
            if k >= self.end || self.separates(k) || !self.numberish(k) || self.pairstart_loose(k) {
                break;
            }
            self.i = k;
            parts.push(self.word().0);
        }
        if parts.len() == 1 {
            return first;
        }
        let list = self.ast.push_list(&parts);
        self.ast
            .push(Tag::Seq, self.ast.main_token(first), list, NONE)
    }

    /// A word starting with `+ - .` or a digit (the old SPICE grammar's `mnum`).
    fn numberish(&self, k: u32) -> bool {
        self.t
            .text(k)
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_digit() || b"+-.".contains(&b))
    }

    /// A word followed by `=` (any key shape): `!(s? '=')` after a number.
    fn pairstart_loose(&self, k: u32) -> bool {
        let w = self.word_end(k);
        w + 1 < self.end && self.t.tags()[w as usize + 1] == T::Equal
    }
}
