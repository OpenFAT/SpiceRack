//! Tokens → the text `import::logical` gave them: the source inside one
//! physical piece, one blank between pieces, nothing across an HSPICE `\\`
//! glue, VACASK's brackets glued to what precedes them; and the token
//! extent of every statement, item and expression (a Spectre statement runs
//! on while its parentheses are open).
//!
//! Cites: `import/mod.rs` `logical` and the line stage of each
//! `grammar/*.peg` (`Line`, `Piece`, `Glue`).
//!
//! Contents:
//! - `Cx::line_last(&self, first: u32) -> u32`
//! - `Cx::line(&self, n: u32) -> String`
//! - `Cx::first(&self, n: u32) -> u32`
//! - `Cx::last(&self, n: u32) -> u32`
//! - `Cx::text(&self, n: u32) -> String`
//! - `Cx::span(&self, a: u32, b: u32) -> String`
//! - `Cx::lines(&self, n: u32) -> Vec<String>`

use super::Cx;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

impl<'a> Cx<'a> {
    /// The last token of the statement starting at token `first` (a
    /// Spectre statement runs on while parentheses are open, as the parser
    /// joins it).
    pub(super) fn line_last(&self, first: u32) -> u32 {
        let tags = self.t.tags();
        let named = self.t.lexicon_at(first).named;
        let mut depth = 0i32;
        let mut k = first as usize;
        loop {
            match tags[k] {
                T::LParen if named => depth += 1,
                T::RParen if named => depth -= 1,
                T::EndOfStatement => {
                    let more = tags[k + 1..].contains(&T::EndOfStatement);
                    if depth <= 0 || !more || matches!(tags[k + 1], T::LangSpice | T::LangSpectre) {
                        return k as u32 - 1;
                    }
                }
                _ => {}
            }
            k += 1;
        }
    }

    /// A flat statement's (or a block head's) logical-line text.
    pub(super) fn line(&self, n: u32) -> String {
        let first = self.main(n);
        self.span(first, self.line_last(first))
    }

    /// The first token of item or expression `n` (a binary operator's
    /// anchor is the operator).
    pub(super) fn first(&self, n: u32) -> u32 {
        let a = self.ast;
        let mut n = Node(n);
        while matches!(a.tag(n), Tag::Binary | Tag::Ternary | Tag::Index) {
            n = Node(a.lhs(n));
        }
        a.main_token(n)
    }

    /// The last token of item or expression `n`.
    pub(super) fn last(&self, n: u32) -> u32 {
        let a = self.ast;
        let mut n = Node(n);
        loop {
            match a.tag(n) {
                Tag::Word | Tag::Bad | Tag::Verbatim | Tag::Line => return a.lhs(n),
                Tag::Assign | Tag::Binary => {
                    if a.rhs(n) == NONE {
                        return self.last_of_key(n);
                    }
                    n = Node(a.rhs(n));
                }
                Tag::Unary => n = Node(a.lhs(n)),
                Tag::Ternary => n = Node(a.extra(a.rhs(n) + 1)),
                Tag::Group | Tag::Call | Tag::Probe | Tag::Vector | Tag::List => {
                    return a.extra(a.rhs(n));
                }
                Tag::Index => return a.extra(a.rhs(n) + 1),
                Tag::Paren | Tag::Delimited => return a.rhs(n),
                Tag::Ident => return a.rhs(n),
                Tag::Seq => {
                    let parts = a.list(a.lhs(n));
                    n = Node(*parts.last().expect("a Seq has parts"));
                }
                // else: leaves and statements anchor at their main token.
                _ => return a.main_token(n),
            }
        }
    }

    /// The text of item `n` as the line stage joined it.
    pub(super) fn text(&self, n: u32) -> String {
        self.span(self.first(n), self.last(n))
    }

    /// Tokens `a..=b` as `import::logical` joined them: the source inside
    /// one physical piece, one blank between pieces (a continuation, a
    /// comment), nothing across an HSPICE `\\` glue; VACASK glues a bracket
    /// to what precedes it and puts a blank after it.
    pub(super) fn span(&self, a: u32, b: u32) -> String {
        let mut out = String::new();
        for k in a..=b {
            if k > a {
                out.push_str(&self.gap(k - 1, k));
            }
            out.push_str(self.t.text(k));
        }
        out
    }

    /// Every line of statement `n`, a block's head, body and closer
    /// included: a `Block` kept whole.
    pub(super) fn lines(&self, n: u32) -> Vec<String> {
        let mut out = Vec::new();
        let mut work = vec![(n, false)];
        while let Some((s, closed)) = work.pop() {
            if closed {
                if let Some(c) = self.ast.closer(Node(s)) {
                    out.push(self.line(c.0));
                }
                continue;
            }
            out.push(self.line(s));
            if self.ast.rhs(Node(s)) == NONE {
                continue;
            }
            // Push in reverse: the closer, the chained branch, the body.
            match self.ast.next(Node(s)) {
                Some(nx) => work.push((nx.0, false)),
                None => work.push((s, true)),
            }
            for &b in self.body(s).iter().rev() {
                work.push((b, false));
            }
        }
        out
    }
}

// ---- private ----

impl<'a> Cx<'a> {
    fn last_of_key(&self, n: Node) -> u32 {
        // An `Assign` with no value ends at its `=`, the token after its key.
        self.last(self.ast.lhs(n)) + 1
    }

    /// What the line stage put between tokens `p` and `k`.
    fn gap(&self, p: u32, k: u32) -> std::borrow::Cow<'a, str> {
        let src = self.t.source();
        let g = &src[self.t.end(p) as usize..self.t.start(k) as usize];
        let lex = self.t.lexicon_at(k);
        if lex.brackets_join {
            // quirk: vacask.peg `Glue`, a bracket, is joined with nothing;
            // the `Piece` after one gets a blank (import/mod.rs `logical`).
            let bracket = |t: T| {
                matches!(
                    t,
                    T::LParen | T::RParen | T::LBracket | T::RBracket | T::LBrace | T::RBrace
                )
            };
            if bracket(self.ttag(k)) {
                return "".into();
            }
            if bracket(self.ttag(p)) {
                return " ".into();
            }
            return if g.bytes().all(|c| matches!(c, b' ' | b'\t' | b'\r')) {
                g.into()
            } else {
                " ".into()
            };
        }
        if lex.backslash == crate::syntax::lexicon::Backslash::Hspice
            && crate::syntax::parse::is_glue(g)
        {
            return "".into();
        }
        if g.contains('\n') || g.contains("/*") {
            " ".into()
        } else {
            g.into()
        }
    }
}
