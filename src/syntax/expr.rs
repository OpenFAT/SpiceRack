//! Tokens → expression nodes: one precedence climb (Pratt) for every
//! dialect, driven by the lexicon's [`Ops`] table: which tokens are binary
//! operators and how tight they bind, which group to the right, the prefix
//! operators and how far their operand reaches, the probe names. Blanks are
//! gaps between tokens and mean nothing here; where an expression ends is
//! the first token that is no operator or operand.
//!
//! The climb recurses once per nesting level and stops at
//! [`DEPTH_MAX`](crate::syntax::parse::DEPTH_MAX) with a `P0006`.
//!
//! Cites: ngspice manual §2.8.5 and numparam's levels (`grammar/ngspice.peg`
//! `x_ternary` … `x_atom`, `x_probe` §2.6); HSPICE UG08 pp.279-283; ADI
//! LTspice CIRCUIT-ELEMENTS-REFERENCE "Supported Operators"; Xyce RG Table
//! 2-32 (levels measured on 7.10); Spectre UG p.87 (§6.1, vectors §5);
//! VACASK `lib/dflparser.y` P216-P230, P740-P781.
//!
//! Contents:
//! - `Parser::expr_all(&mut self) -> Node`
//! - `Parser::expr_bp(&mut self, min_bp: u8) -> Result<Node, ()>`
//! - `Parser::continue_expr(&mut self, left: Node) -> Node`
//! - `Parser::quoted_expression(&mut self, open: u32) -> Node`

use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::diag::Code;
use crate::syntax::lexicon::{Ops, Quote};
use crate::syntax::parse::{DEPTH_MAX, Parser};
use crate::syntax::token::Tag as T;

impl Parser<'_, '_> {
    /// The tokens from the cursor to `self.end` as one expression; a `Bad`
    /// with a `P0002` when they are not one.
    pub(super) fn expr_all(&mut self) -> Node {
        let start = self.i;
        if start >= self.end {
            self.ast.diag(Code::P0002, start.saturating_sub(1));
            return Node(NONE);
        }
        let mark = self.ast.mark();
        let (depth, capped) = (self.depth, self.capped);
        match self.expr_bp(0) {
            Ok(e) if self.i == self.end => e,
            _ => {
                let deep = self.capped && !capped;
                self.ast.reset(mark);
                self.depth = depth;
                if deep {
                    self.ast.diag(Code::P0006, start);
                }
                self.ast.diag(Code::P0002, start);
                self.i = self.end;
                self.ast.push(Tag::Bad, start, self.end - 1, NONE)
            }
        }
    }

    /// One expression from the cursor, taking the binary operators that
    /// bind at least `min_bp`; `Err` when the tokens there start none (the
    /// cursor is then anywhere: the caller resets).
    pub(super) fn expr_bp(&mut self, min_bp: u8) -> Result<Node, ()> {
        self.depth += 1;
        let r = if self.depth > DEPTH_MAX {
            self.capped = true;
            Err(())
        } else {
            self.prefix().and_then(|left| self.infix(left, min_bp))
        };
        self.depth -= 1;
        r
    }

    /// `left` continued by the operators after it (ngspice `{a} + b`); `left`
    /// alone when what follows is no operand.
    pub(super) fn continue_expr(&mut self, left: Node) -> Node {
        let (mark, at) = (self.ast.mark(), self.i);
        match self.infix(left, 0) {
            Ok(e) => e,
            Err(()) => {
                self.ast.reset(mark);
                self.i = at;
                left
            }
        }
    }

    /// HSPICE `"expr"` (the `String` token `open`): its inside re-tokenized
    /// by the same lexicon and checked as one expression. A `Delimited`
    /// whose expression is not kept (its tokens are not the file's), or a
    /// `Bad` with a `P0002`.
    pub(super) fn quoted_expression(&mut self, open: u32) -> Node {
        let text = self.t.text(open);
        let inner = text
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .unwrap_or("");
        let lex = self.t.lexicon_at(open);
        let src = format!("\n{inner}\n");
        let sub = crate::syntax::tokenize(&src, lex);
        let tree = crate::syntax::parse::parse_expression(&sub, lex);
        if tree {
            self.ast.push(Tag::Delimited, open, NONE, open)
        } else {
            self.ast.diag(Code::P0002, open);
            self.ast.push(Tag::Bad, open, open, NONE)
        }
    }
}

// ---- private ----

impl Parser<'_, '_> {
    fn ops(&self) -> &'static Ops {
        &self.t.lexicon_at(self.i.min(self.end)).ops
    }

    fn tag_at(&self, k: u32) -> Option<T> {
        (k < self.end).then(|| self.t.tags()[k as usize])
    }

    /// The tag at the cursor, past the ends of statement a Spectre line
    /// join leaves inside open parentheses.
    fn peek(&mut self) -> Option<T> {
        while self.tag_at(self.i) == Some(T::EndOfStatement) {
            self.i += 1;
        }
        self.tag_at(self.i)
    }

    /// An operand: a prefix operator and its operand, or an atom with its
    /// postfix indexing.
    fn prefix(&mut self) -> Result<Node, ()> {
        let tag = self.peek().ok_or(())?;
        let ops = self.ops();
        if ops.prefix.contains(&tag) {
            let op = self.i;
            self.i += 1;
            let x = self.expr_bp(ops.prefix_bp + 1)?;
            return Ok(self.ast.push(Tag::Unary, op, x.0, NONE));
        }
        let mut x = self.atom(tag)?;
        while ops.index && self.peek() == Some(T::LBracket) {
            let open = self.i;
            self.i += 1;
            let k = self.expr_bp(0)?;
            if self.peek() != Some(T::RBracket) {
                return Err(());
            }
            let at = self.ast.push_extra(&[k.0, self.i]);
            self.i += 1;
            x = self.ast.push(Tag::Index, open, x.0, at);
        }
        Ok(x)
    }

    fn atom(&mut self, tag: T) -> Result<Node, ()> {
        let lex = self.t.lexicon_at(self.i);
        let at = self.i;
        match tag {
            T::Number => {
                self.i += 1;
                Ok(self.ast.push(Tag::Num, at, NONE, NONE))
            }
            T::String if !lex.string_expr => {
                self.i += 1;
                Ok(self.ast.push(Tag::Str, at, NONE, NONE))
            }
            T::Ident => self.name_or_call(),
            T::LParen => {
                self.i += 1;
                let x = self.expr_bp(0)?;
                if self.peek() != Some(T::RParen) {
                    return Err(());
                }
                self.i += 1;
                Ok(self.ast.push(Tag::Paren, at, x.0, self.i - 1))
            }
            T::LBrace if lex.expr_braces => Ok(self.delimited()),
            T::Quote if matches!(lex.quote, Quote::Expr(_)) => Ok(self.delimited()),
            T::String => Ok(self.delimited()),
            T::LBracket => self.vector(Tag::Vector, T::RBracket),
            T::LBrace => self.vector(Tag::List, T::RBrace),
            _ => Err(()),
        }
    }

    /// A name (Spectre `a.b` joined), a call `f(args)`, or a probe `v(a,b)`.
    fn name_or_call(&mut self) -> Result<Node, ()> {
        let a = self.i;
        let mut b = a;
        let named = self.t.lexicon_at(a).letters.is_empty();
        while named
            && self.tag_at(b + 1) == Some(T::Dot)
            && self.tag_at(b + 2) == Some(T::Ident)
            && self.touches(b, b + 1)
            && self.touches(b + 1, b + 2)
        {
            b += 2;
        }
        self.i = b + 1;
        let text = self.span_text(a, b);
        let name = self.ast.intern(text);
        if self.peek() != Some(T::LParen) {
            return Ok(self.ast.push(Tag::Ident, a, name.0, b));
        }
        let lower = self.ast.name(name).to_ascii_lowercase();
        if self.ops().probes.contains(&lower.as_str())
            && let Some(p) = self.probe(a, name.0)
        {
            return Ok(p);
        }
        self.i += 1;
        let mut args = Vec::new();
        let close = loop {
            if self.peek() == Some(T::RParen) {
                break self.i;
            }
            args.push(self.expr_bp(0)?.0);
            match self.peek() {
                Some(T::Comma) => self.i += 1,
                Some(T::RParen) => break self.i,
                _ => return Err(()),
            }
        };
        self.i = close + 1;
        let mut words = vec![close, args.len() as u32];
        words.extend(args);
        let at = self.ast.push_extra(&words);
        Ok(self.ast.push(Tag::Call, a, name.0, at))
    }

    /// `v(a)`, `v(a,b)`, `i(v1)` at the cursor's `(`: each argument a run of
    /// touching tokens; `None` (cursor unmoved) when the inside is not that.
    fn probe(&mut self, a: u32, name: u32) -> Option<Node> {
        let open = self.i;
        let mut k = open + 1;
        let mut args = Vec::new();
        let stop = |t: T| matches!(t, T::Comma | T::LParen | T::RParen | T::EndOfStatement);
        loop {
            let first = k;
            if self.tag_at(first).is_none_or(stop) {
                return None;
            }
            while self.tag_at(k + 1).is_some_and(|t| !stop(t)) && self.touches(k, k + 1) {
                k += 1;
            }
            args.push((first, k));
            k += 1;
            match self.tag_at(k) {
                Some(T::Comma) if args.len() == 1 => k += 1,
                Some(T::RParen) => break,
                _ => return None,
            }
        }
        let words: Vec<u32> = args.iter().map(|&(f, l)| self.push_word(f, l).0).collect();
        let mut extra = vec![k, words.len() as u32];
        extra.extend(words);
        let at = self.ast.push_extra(&extra);
        self.i = k + 1;
        Some(self.ast.push(Tag::Probe, a, name, at))
    }

    /// Spectre `[a b c]` (signed atoms, blank-separated), VACASK `[a, b]`
    /// and `{a, b}` (expressions, `,` or `;` between).
    fn vector(&mut self, tag: Tag, close: T) -> Result<Node, ()> {
        let open = self.i;
        let whole = self.ops().index;
        self.i += 1;
        let mut items = Vec::new();
        let end = loop {
            match self.peek().ok_or(())? {
                t if t == close => break self.i,
                T::Comma | T::Semicolon => self.i += 1,
                _ if whole => items.push(self.expr_bp(0)?.0),
                _ => items.push(self.prefix()?.0),
            }
        };
        self.i = end + 1;
        let mut words = vec![end, items.len() as u32];
        words.extend(items);
        let at = self.ast.push_extra(&words);
        Ok(self.ast.push(tag, open, NONE, at))
    }

    /// The binary operators after `left` that bind at least `min_bp`.
    fn infix(&mut self, mut left: Node, min_bp: u8) -> Result<Node, ()> {
        while let Some(tag) = self.peek() {
            let ops = self.ops();
            if tag == T::Question && min_bp <= Ops::TERNARY {
                let q = self.i;
                self.i += 1;
                let a = self.expr_bp(0)?;
                if self.peek() != Some(T::Colon) {
                    return Err(());
                }
                self.i += 1;
                let b = self.expr_bp(Ops::TERNARY)?;
                let at = self.ast.push_extra(&[a.0, b.0]);
                left = self.ast.push(Tag::Ternary, q, left.0, at);
                continue;
            }
            let Some(&(_, bp)) = ops.infix.iter().find(|(t, _)| *t == tag) else {
                break;
            };
            // An operator before `key =` separates two assignments (sky130's
            // `a = '-1*x' + b = '0 - 2.5'`).
            if bp < min_bp
                || (!self.t.lexicon_at(self.i).letters.is_empty() && self.pairstart(self.i + 1))
            {
                break;
            }
            let op = self.i;
            self.i += 1;
            let next = if ops.right.contains(&tag) { bp } else { bp + 1 };
            let r = self.expr_bp(next)?;
            left = self.ast.push(Tag::Binary, op, left.0, r.0);
        }
        Ok(left)
    }
}
