//! [`Tokens`] → [`Ast`]: one parser for every dialect. It never fails: what
//! it cannot read becomes a `Verbatim` statement or a `Bad` expression with
//! a `P` diagnostic, and it carries on at the next statement (the tokenizer
//! has already cut the statements, so resynchronising is free).
//!
//! The parser dispatches on a statement's head: a SPICE dot card or element
//! letter (`spice.rs`), or a Spectre/VACASK statement led by a keyword or a
//! name (`named.rs`). Everything after the head is shared: the field
//! reader (`fields.rs`), the expression parser (`expr.rs`) and the block
//! structure here, which nests `.subckt`, sections, libraries, `.if`
//! chains, `.control`, `.data`, Spectre `{ }` blocks and `simulator
//! lang=spice` regions on an explicit stack, so a deck nested 10⁴ deep
//! parses without recursion.
//!
//! The tokenizer's open points are settled here
//! (`docs/research/transpiler-architecture.md` §5, phase 2): gapless tokens
//! in a name or value position are one `Word` (an HSPICE `\\` glue counts
//! as no gap); an HSPICE `"expr"` is re-tokenized and checked as an
//! expression; keywords are looked up only at heads; an end of statement
//! inside open brackets is whitespace (the VACASK lexicon already joins
//! such lines, and Spectre `if (a &&⏎ b) {` is joined here); `.end` is a
//! node; a NaN number is a `P0005`.
//!
//! Cites: `transpiler-architecture.md` §2.2, §2.3; Zig `Parse.zig`
//! (`compiler-architecture-reference.md` §2.1-§2.6: never fails, scratch
//! lists, recovery at the next member); the card structure of
//! `grammar/spice.peg`, `spectre.peg`, `vacask.peg` (ngspice manual §2.5
//! `.subckt`, §2.7 `.lib`, §2.15 `.if`; Spectre Reference 19.1 p.491 `if`,
//! p.498 `library`; VACASK `lib/dflparser.y` P324-P509).
//!
//! Contents:
//! - `pub fn parse<'s>(t: &Tokens<'s>, lex: &'static Lexicon) -> Ast<'s>`
//! - `pub(super) struct Parser<'t, 's>`, `pub(super) enum Head`, `pub(super) struct Frame`,
//!   `pub(super) enum Wait`, `Frame::open(node: Node) -> Frame`
//! - `pub(super) fn is_glue(gap: &str) -> bool`
//! - `Parser::apply(&mut self, head: Head, stack: &mut Vec<Frame>)`,
//!   `Parser::finish(&mut self, f: Frame, closer: u32, stack: &mut [Frame])`,
//!   `Parser::condition(&mut self) -> u32`, `Parser::touches(&self, a: u32, b: u32) -> bool`,
//!   `Parser::span_text(&self, a: u32, b: u32) -> Cow<'s, str>`,
//!   `Parser::push_word(&mut self, a: u32, b: u32) -> Node`, `Parser::verbatim(&mut self, first: u32) -> Node`
//! - `pub const DEPTH_MAX: u32`
//! - `pub(super) fn parse_expression(t: &Tokens<'_>, lex: &'static Lexicon) -> bool`

use crate::syntax::ast::{Ast, NONE, Node, Tag};
use crate::syntax::diag::Code;
use crate::syntax::lexicon::{Backslash, Lexicon};
use crate::syntax::token::{Tag as T, Tokens};
use std::borrow::Cow;

/// The deepest expression, group or delimiter nesting the parser reads;
/// deeper input is a `P0006` and kept as tokens. Block nesting has no cap
/// (it is a heap stack).
pub const DEPTH_MAX: u32 = 200;

/// The parser's state over one file.
pub(super) struct Parser<'t, 's> {
    pub(super) t: &'t Tokens<'s>,
    /// The file's lexicon.
    pub(super) lex: &'static Lexicon,
    pub(super) ast: Ast<'s>,
    /// The cursor: the next token to read.
    pub(super) i: u32,
    /// The current statement's `EndOfStatement`, or a delimiter's closer
    /// while its inside is read.
    pub(super) end: u32,
    /// The current expression or group nesting.
    pub(super) depth: u32,
    /// The current statement's first token.
    pub(super) first: u32,
    /// Whether an expression hit [`DEPTH_MAX`] (so a failed parse reports
    /// `P0006` as well as `P0002`).
    pub(super) capped: bool,
}

/// What a statement head does to the block stack.
pub(super) enum Head {
    /// A statement with no body.
    Flat(Node),
    /// A block opener: its node, whose body follows.
    Open(Node),
    /// A block closer: the tag of the block it closes, and the statement
    /// (a `Card`).
    Close(Tag, Node),
    /// `.elseif`, `} else if (c) {` (an `If`), `.else`, `} else {` (an
    /// `Else`): closes a branch, opens the next.
    Branch(Node),
}

/// An open block: its node and the statements read into it so far.
pub(super) struct Frame {
    pub(super) node: Node,
    pub(super) items: Vec<u32>,
    /// Earlier branches of an `If` chain: their bodies' offsets, whose
    /// closer is patched when the chain ends.
    chain: Vec<u32>,
    /// The chain's first branch (`node` itself for any other block).
    head: Node,
    /// What a Spectre block waits for before its body.
    pub(super) wait: Wait,
    /// A Spectre branch's `}`, once read (the chain may go on with `else`).
    pub(super) closer: u32,
}

/// What a Spectre block on the stack waits for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Wait {
    /// Nothing: its body is being read.
    No,
    /// Its `{` (a `real f()` or `if (c)` whose brace is on the next line),
    /// or for a branch its one unbraced statement.
    Brace,
    /// A branch that is complete: an `else` may continue the chain.
    Else,
}

/// Parses all of `t` (read by `lex`, the file's lexicon). Allocates the node
/// rows (about one per two tokens).
pub fn parse<'s>(t: &Tokens<'s>, lex: &'static Lexicon) -> Ast<'s> {
    let mut p = Parser {
        t,
        lex,
        ast: Ast::new(t.len()),
        i: 0,
        end: 0,
        depth: 0,
        first: 0,
        capped: false,
    };
    let mut stack: Vec<Frame> = vec![Frame::root()];
    // A Spectre statement may close its blocks' waits (`}` then no `else`).
    let mut s = 0u32;
    while let Some((first, eos)) = p.next_statement(&mut s) {
        p.i = first;
        p.first = first;
        p.end = eos;
        p.check_tokens(first, eos);
        p.statement(&mut stack);
    }
    while stack.len() > 1 {
        let f = stack.pop().expect("the stack holds a block");
        if f.wait == Wait::Else {
            let closer = f.closer;
            p.finish(f, closer, &mut stack);
            continue;
        }
        let head = p.ast.main_token(f.node);
        p.ast.diag(Code::P0003, head);
        p.finish(f, NONE, &mut stack);
    }
    let root = stack.pop().expect("the root frame stays");
    let list = p.ast.push_list(&root.items);
    p.ast.set_root(list);
    p.ast
}

/// Whether the first statement of `t` (after its title) is one expression
/// and nothing else: HSPICE's `"expr"` check.
pub(super) fn parse_expression(t: &Tokens<'_>, lex: &'static Lexicon) -> bool {
    let mut p = Parser {
        t,
        lex,
        ast: Ast::new(t.len()),
        i: 0,
        end: 0,
        depth: 0,
        first: 0,
        capped: false,
    };
    let mut s = 0u32;
    let mut stmt = p.next_statement(&mut s);
    if stmt.is_some_and(|(f, _)| t.tags()[f as usize] == T::Title) {
        stmt = p.next_statement(&mut s);
    }
    let Some((first, eos)) = stmt else {
        return false;
    };
    (p.i, p.end) = (first, eos);
    let e = p.expr_all();
    e.0 != NONE && p.ast.tag(e) != Tag::Bad && p.next_statement(&mut s).is_none()
}

/// hspice.peg `l_glue_break`: blanks, `\\`, blanks, the newline, blanks.
pub(super) fn is_glue(gap: &str) -> bool {
    gap.trim_matches([' ', '\t'])
        .strip_prefix("\\\\")
        .is_some_and(|r| r.trim_start_matches([' ', '\t', '\r']) == "\n")
}

impl Frame {
    fn root() -> Frame {
        Frame::open(Node(0))
    }

    pub(super) fn open(node: Node) -> Frame {
        Frame {
            node,
            items: Vec::new(),
            chain: Vec::new(),
            head: node,
            wait: Wait::No,
            closer: NONE,
        }
    }
}

impl<'s> Parser<'_, 's> {
    /// Puts one head's node where it belongs on the block stack.
    pub(super) fn apply(&mut self, head: Head, stack: &mut Vec<Frame>) {
        let at = self.first;
        let open = self.ast.tag(stack.last().expect("root").node);
        match head {
            Head::Flat(n) => stack.last_mut().expect("root").items.push(n.0),
            Head::Open(n) => stack.push(Frame::open(n)),
            Head::Close(tag, card) => {
                let closes = open == tag || (tag == Tag::If && open == Tag::Else);
                if stack.len() > 1 && closes {
                    let f = stack.pop().expect("a block is open");
                    self.finish(f, card.0, stack);
                } else {
                    self.ast.diag(Code::P0004, at);
                    stack.last_mut().expect("root").items.push(card.0);
                }
            }
            Head::Branch(branch) if stack.len() > 1 && open == Tag::If => {
                let mut f = stack.pop().expect("an If is open");
                let body = self.ast.push_body(branch.0, f.closer, &f.items);
                self.ast.set_rhs(f.node, body);
                f.chain.push(body);
                stack.push(Frame {
                    node: branch,
                    items: Vec::new(),
                    chain: f.chain,
                    head: f.head,
                    wait: Wait::No,
                    closer: NONE,
                });
            }
            Head::Branch(branch) => {
                // A branch with no `.if` open: kept as a statement.
                self.ast.diag(Code::P0004, at);
                stack.last_mut().expect("root").items.push(branch.0);
            }
        }
    }

    /// Closes frame `f` with `closer` and adds it to its parent; an `If`
    /// chain's earlier branches get the same closer, and the chain is one
    /// statement of its parent, its first branch.
    pub(super) fn finish(&mut self, f: Frame, closer: u32, stack: &mut [Frame]) {
        let body = self.ast.push_body(NONE, closer, &f.items);
        self.ast.set_rhs(f.node, body);
        for &b in &f.chain {
            self.ast.set_extra(b + 1, closer);
        }
        stack.last_mut().expect("root").items.push(f.head.0);
    }

    /// A conditional's condition, the rest of its statement as one
    /// expression; the fields list holding it.
    pub(super) fn condition(&mut self) -> u32 {
        let e = self.expr_all();
        self.ast.push_list(&[e.0])
    }

    /// Whether tokens `a` and `b` touch: nothing between them, or only an
    /// HSPICE `\\` glue break.
    pub(super) fn touches(&self, a: u32, b: u32) -> bool {
        let (e, s) = (self.t.end(a) as usize, self.t.start(b) as usize);
        if e == s {
            return true;
        }
        self.t.lexicon_at(b).backslash == Backslash::Hspice && is_glue(&self.t.source()[e..s])
    }

    /// The text of tokens `a..=b` as one name: the source span, or with an
    /// HSPICE glue cut out.
    pub(super) fn span_text(&self, a: u32, b: u32) -> Cow<'s, str> {
        let src = self.t.source();
        let (s, e) = (self.t.start(a) as usize, self.t.end(b) as usize);
        if (a..b).all(|k| self.t.end(k) == self.t.start(k + 1)) {
            return Cow::Borrowed(&src[s..e]);
        }
        let mut out = String::new();
        for k in a..=b {
            out.push_str(self.t.text(k));
        }
        Cow::Owned(out)
    }

    /// Pushes a `Word` over tokens `a..=b`.
    pub(super) fn push_word(&mut self, a: u32, b: u32) -> Node {
        let text = self.span_text(a, b);
        let name = self.ast.intern(text);
        self.ast.push(Tag::Word, a, b, name.0)
    }

    /// Keeps the rest of the statement from `first` as a `Verbatim` with a
    /// `P0001`.
    pub(super) fn verbatim(&mut self, first: u32) -> Node {
        self.ast.diag(Code::P0001, first);
        let d = self.t.lexicon_at(first).dialect;
        let k = crate::codegen::Dialect::ALL
            .iter()
            .position(|&x| x == d)
            .expect("every lexicon's dialect is in Dialect::ALL");
        self.i = self.end;
        self.ast.push(Tag::Verbatim, first, self.end - 1, k as u32)
    }
}

// ---- private ----

impl<'s> Parser<'_, 's> {
    /// The statement after the one ending at token `*s`: its first token and
    /// its `EndOfStatement`; moves `*s` past it.
    fn next_statement(&self, s: &mut u32) -> Option<(u32, u32)> {
        let tags = self.t.tags();
        let mut first = *s as usize;
        while first < tags.len() && matches!(tags[first], T::LangSpice | T::LangSpectre) {
            first += 1;
        }
        if first >= tags.len() {
            return None;
        }
        let mut eos = (first..tags.len())
            .find(|&k| tags[k] == T::EndOfStatement)
            .expect("every statement ends with an EndOfStatement");
        // Spectre: an end of statement inside open parentheses is a blank
        // (`if (a &&` ⏎ `b) {`); VACASK's lexicon already joins such lines.
        if self.t.lexicon_at(first as u32).named {
            let mut depth = 0i32;
            let mut k = first;
            loop {
                match tags[k] {
                    T::LParen => depth += 1,
                    T::RParen => depth -= 1,
                    T::EndOfStatement if k == eos => {
                        let more = tags[k + 1..].iter().position(|&t| t == T::EndOfStatement);
                        match more {
                            Some(m)
                                if depth > 0
                                    && !matches!(tags[k + 1], T::LangSpice | T::LangSpectre) =>
                            {
                                eos = k + 1 + m
                            }
                            _ => break,
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
        }
        *s = eos as u32 + 1;
        Some((first as u32, eos as u32))
    }

    /// `P0007` for an `Invalid` token and `P0005` for a NaN number.
    fn check_tokens(&mut self, first: u32, eos: u32) {
        for k in first..eos {
            match self.t.tags()[k as usize] {
                T::Invalid => self.ast.diag(Code::P0007, k),
                T::Number if self.t.value(k).is_nan() => self.ast.diag(Code::P0005, k),
                _ => {}
            }
        }
    }

    /// Reads the statement at `self.i` into the top frame, opening or
    /// closing blocks.
    fn statement(&mut self, stack: &mut Vec<Frame>) {
        let top = stack.last().expect("the root frame stays");
        let in_tag = self.ast.tag(top.node);
        let first = self.i;
        if in_tag == Tag::Region {
            // spectre.peg `SpiceRegion`: a region ends at the switch back, or
            // where its section or library ends.
            let word = self.t.text(first);
            if word == "endsection" || word == "endlibrary" {
                let f = stack.pop().expect("a region is open");
                self.finish(f, NONE, stack);
                return self.named_statement(stack);
            }
            if self.t.tags().get(self.end as usize + 1) == Some(&T::LangSpectre) {
                self.i = self.end;
                let empty = self.ast.push_list(&[]);
                let card = self.ast.push(Tag::Card, first, empty, NONE);
                return self.apply(Head::Close(Tag::Region, card), stack);
            }
        }
        let named = self.t.lexicon_at(first).named;
        let head = match in_tag {
            Tag::Data => self.script_line(top.node),
            Tag::Control if !named => self.script_line(top.node),
            _ if named => return self.named_statement(stack),
            _ => self.spice(in_tag),
        };
        self.apply(head, stack);
    }

    /// A `.control` or `.data` line: the closer, a `.control` analysis
    /// command read as its dot card, or a `Line`.
    fn script_line(&mut self, block: Node) -> Head {
        let first = self.i;
        let closer = match self.ast.tag(block) {
            Tag::Control => &[T::KwEndc][..],
            _ => &[T::KwEnddata, T::KwEndVariation][..],
        };
        let tag = self.t.tags()[first as usize];
        if closer.contains(&tag) {
            let empty = self.ast.push_list(&[]);
            let card = self.ast.push(Tag::Card, first, empty, NONE);
            return Head::Close(self.ast.tag(block), card);
        }
        if self.ast.tag(block) == Tag::Control
            && let Some(kw) = self.control_command(first)
        {
            return Head::Flat(self.dot_card(kw));
        }
        Head::Flat(self.ast.push(Tag::Line, first, self.end - 1, NONE))
    }

    /// A `.control` line ngspice runs as an analysis (`tran 1n 10n`): the
    /// keyword of its dot card.
    fn control_command(&self, first: u32) -> Option<T> {
        const RUNS: &[&str] = &[
            "noise", "disto", "tran", "sens", "four", "meas", "op", "dc", "ac", "tf", "pz",
        ];
        let t = self.t;
        if t.tags()[first as usize] != T::Ident {
            return None;
        }
        let word = t.text(first).to_ascii_lowercase();
        if !RUNS.contains(&word.as_str()) {
            return None;
        }
        Some(
            self.lex
                .keyword(&format!(".{word}"))
                .unwrap_or(T::Directive),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::lexicon::{HSPICE, NGSPICE};
    use crate::syntax::tokenize;

    fn tags(src: &str) -> Vec<Tag> {
        let t = tokenize(src, &NGSPICE);
        let a = parse(&t, &NGSPICE);
        a.fields(a.root()).iter().map(|&n| a.tag(Node(n))).collect()
    }

    #[test]
    fn subckt_nested_ten_thousand_deep_parses_without_recursion() {
        // ngspice manual §2.5: definitions nest; the stack is on the heap.
        let n = 10_000;
        let mut src = String::from("t\n");
        for k in 0..n {
            src.push_str(&format!(".subckt s{k} a\n"));
        }
        for _ in 0..n {
            src.push_str(".ends\n");
        }
        let t = tokenize(&src, &NGSPICE);
        let a = parse(&t, &NGSPICE);
        assert!(a.diags().is_empty());
        let mut s = Node(a.fields(a.root())[1]);
        for _ in 1..n {
            assert_eq!(a.tag(s), Tag::Subckt);
            s = Node(a.body(s)[0]);
        }
        assert!(a.body(s).is_empty());
    }

    #[test]
    fn parens_nested_ten_thousand_deep_are_capped_with_a_code() {
        // No clause: the parser's own cap, DEPTH_MAX.
        let n = 10_000;
        let src = format!("t\n.param a={}1{}\n", "(".repeat(n), ")".repeat(n));
        let t = tokenize(&src, &NGSPICE);
        let a = parse(&t, &NGSPICE);
        assert!(a.diags().iter().any(|d| d.code == Code::P0006));
        assert_eq!(tags(&src), [Tag::Title, Tag::Param]);
    }

    #[test]
    fn a_stray_closer_is_a_card_with_p0004() {
        // ngspice manual §2.5: `.ends` closes a `.subckt`.
        let t = tokenize("t\n.ends\n", &NGSPICE);
        let a = parse(&t, &NGSPICE);
        assert_eq!(a.diags()[0].code, Code::P0004);
        assert_eq!(a.tag(Node(a.fields(a.root())[1])), Tag::Card);
    }

    #[test]
    fn an_unclosed_block_is_p0003_and_holds_the_rest() {
        let t = tokenize("t\n.subckt s a\nR1 a 0 1\n", &NGSPICE);
        let a = parse(&t, &NGSPICE);
        assert_eq!(a.diags()[0].code, Code::P0003);
        let s = Node(a.fields(a.root())[1]);
        assert_eq!(a.body(s).len(), 1);
        assert_eq!(a.closer(s), None);
    }

    #[test]
    fn if_chains_share_their_endif() {
        // ngspice manual §2.15: `.if` … `.elseif` … `.else` … `.endif`.
        let src = "t\n.if (a)\nR1 a 0 1\n.elseif (b)\nR2 a 0 2\n.else\nR3 a 0 3\n.endif\n";
        let t = tokenize(src, &NGSPICE);
        let a = parse(&t, &NGSPICE);
        assert!(a.diags().is_empty());
        let top = a.fields(a.root());
        assert_eq!(top.len(), 2);
        let first = Node(top[1]);
        let second = a.next(first).expect("an elseif");
        let third = a.next(second).expect("an else");
        assert_eq!(
            [a.tag(first), a.tag(second), a.tag(third)],
            [Tag::If, Tag::If, Tag::Else]
        );
        assert_eq!(a.closer(first), a.closer(third));
        assert!(a.closer(first).is_some());
    }

    #[test]
    fn hspice_glue_joins_a_word() {
        // HSPICE L6: `\\` joins the next line with nothing between.
        let t = tokenize("t\nR1 ab\\\\\ncd 0 1\n", &HSPICE);
        let a = parse(&t, &HSPICE);
        let inst = Node(a.fields(a.root())[1]);
        let w = Node(a.header(inst).nodes[0]);
        assert_eq!(a.name(crate::syntax::ast::Name(a.rhs(w))), "abcd");
    }
}
