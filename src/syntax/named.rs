//! A Spectre or VACASK statement → its nodes on the block stack. A head is a
//! keyword (`model`, `parameters`, `subckt`, `section`, `if`, `statistics`,
//! VACASK `load`, `control`, `@if` …) or a name: `name [(nodes)] master
//! params`: an instance, or by its master word (`Lexicon::masters`) an
//! analysis, a control statement or options.
//!
//! Spectre's `{ }` blocks may open and close inside one line, so a statement
//! is read as units cut at its top-level braces: `} else if (c) {` is a
//! closer, a branch head and an opener. A block whose `{` is on the next line
//! (`real f(real a)` ⏎ `{`) or a branch with one unbraced statement (`} else`
//! ⏎ `r0 (a b) resistor r=1`) waits on the stack ([`Wait`]); a completed
//! branch waits for an `else`.
//!
//! Cites: `grammar/spectre.peg` (`card`, `If`, `Branch`, `bbody`, `Func`,
//! `Model_group`, `Statistics`, `Library`, `LibDef`, `Inst`, `SpiceRegion`)
//! and `grammar/vacask.peg` (`Inst`, `ControlBlock`, `If`); Spectre
//! Reference 19.1 p.491 (`if`), p.498 (`library`), pp.175-179
//! (`statistics`); Spectre UG p.92 (`real` functions), p.94 (`subckt`), p.66
//! (instances), p.108 (model groups); VACASK `lib/dflparser.y` P324-P509,
//! P856-P1065, P1267-P1296.
//!
//! Contents:
//! - `Parser::named_statement(&mut self, stack: &mut Vec<Frame>)`

use crate::syntax::ast::{NONE, Name, Node, Tag};
use crate::syntax::diag::Code;
use crate::syntax::fields::Mode;
use crate::syntax::parse::{Frame, Head, Parser, Wait};
use crate::syntax::statement::Statement;
use crate::syntax::token::Tag as T;

impl Parser<'_, '_> {
    /// Reads the Spectre or VACASK statement at the cursor (its units, for
    /// Spectre) onto the block stack.
    pub(super) fn named_statement(&mut self, stack: &mut Vec<Frame>) {
        let stmt_end = self.end;
        let braces = self.t.lexicon_at(self.i).heads_after.contains(&T::LBrace);
        while self.i < stmt_end {
            self.first = self.i;
            let top = stack.last().expect("the root frame stays");
            let tag = self.t.tags()[self.i as usize];
            if top.wait == Wait::Else {
                if tag == T::KwElse {
                    self.else_branch(stack, stmt_end);
                    continue;
                }
                let f = stack.pop().expect("a branch waits");
                let closer = f.closer;
                self.finish(f, closer, stack);
                continue;
            }
            if braces && tag == T::RBrace {
                self.close_brace(stack);
                continue;
            }
            let unit_end = if braces {
                self.unit_end(stmt_end)
            } else {
                stmt_end
            };
            let opens = unit_end < stmt_end && self.t.tags()[unit_end as usize] == T::LBrace;
            if top.wait == Wait::Brace {
                if unit_end == self.i && opens {
                    stack.last_mut().expect("a block waits").wait = Wait::No;
                    self.i += 1;
                    continue;
                }
                // An unbraced branch: this one statement is its body.
                self.end = unit_end;
                let in_tag = self.ast.tag(stack.last().expect("root").node);
                let (head, _) = self.named_head(in_tag, false, stmt_end);
                self.end = stmt_end;
                self.apply(head, stack);
                stack.last_mut().expect("a branch").wait = Wait::Else;
                continue;
            }
            let in_tag = self.ast.tag(top.node);
            self.end = unit_end;
            let (head, wait) = self.named_head(in_tag, opens, stmt_end);
            self.end = stmt_end;
            if opens && wait == Wait::No && self.i <= unit_end {
                self.i = unit_end + 1;
            }
            let pushed = matches!(head, Head::Open(_));
            self.apply(head, stack);
            if pushed {
                stack.last_mut().expect("just pushed").wait = wait;
            }
        }
        self.i = stmt_end;
    }
}

// ---- private ----

impl Parser<'_, '_> {
    /// The end of the unit at the cursor: the next top-level `{` or `}`, or
    /// `end`.
    fn unit_end(&self, end: u32) -> u32 {
        let mut depth = 0i32;
        for k in self.i..end {
            match self.t.tags()[k as usize] {
                T::LParen | T::LBracket => depth += 1,
                T::RParen | T::RBracket => depth -= 1,
                T::LBrace | T::RBrace if depth <= 0 => return k,
                _ => {}
            }
        }
        end
    }

    /// A `}`: closes a branch (which then waits for an `else`) or the
    /// innermost block.
    fn close_brace(&mut self, stack: &mut Vec<Frame>) {
        let at = self.i;
        self.i += 1;
        let empty = self.ast.push_list(&[]);
        let card = self.ast.push(Tag::Card, at, empty, NONE);
        let top = stack.last_mut().expect("root");
        if matches!(self.ast.tag(top.node), Tag::If | Tag::Else) && top.wait == Wait::No {
            top.wait = Wait::Else;
            top.closer = card.0;
            return;
        }
        if stack.len() > 1 {
            let f = stack.pop().expect("a block is open");
            self.finish(f, card.0, stack);
        } else {
            self.ast.diag(Code::P0004, at);
            stack.last_mut().expect("root").items.push(card.0);
        }
    }

    /// `else`, `else if (c)`: the next branch of the waiting chain, its `{`
    /// read if it follows.
    fn else_branch(&mut self, stack: &mut Vec<Frame>, stmt_end: u32) {
        let at = self.i;
        self.i += 1;
        let node = if self.i < stmt_end && self.t.tags()[self.i as usize] == T::KwIf {
            self.i += 1;
            let cond = self.paren_condition(stmt_end);
            self.ast.push(Tag::If, at, cond, NONE)
        } else {
            self.ast.push(Tag::Else, at, NONE, NONE)
        };
        // The chain's waiting branch takes this one as its next.
        stack.last_mut().expect("a branch waits").wait = Wait::No;
        self.apply(Head::Branch(node), stack);
        let braced = self.i < stmt_end && self.t.tags()[self.i as usize] == T::LBrace;
        if braced {
            self.i += 1;
        }
        let wait = if braced { Wait::No } else { Wait::Brace };
        stack.last_mut().expect("the branch").wait = wait;
    }

    /// A Spectre `(cond)` from the cursor: the fields list holding it.
    fn paren_condition(&mut self, stmt_end: u32) -> u32 {
        let saved = self.end;
        self.end = stmt_end;
        let start = self.i;
        let e = match self.expr_bp(0) {
            Ok(e) => e,
            Err(()) => {
                self.ast.diag(Code::P0002, start);
                self.i = start + 1;
                self.ast.push(Tag::Bad, start, start, NONE)
            }
        };
        self.end = saved;
        self.ast.push_list(&[e.0])
    }

    /// The unit at the cursor (inside `in_tag`, `opens` a `{`), and what its
    /// block waits for when it opens one.
    fn named_head(&mut self, in_tag: Tag, opens: bool, stmt_end: u32) -> (Head, Wait) {
        let first = self.i;
        let tag = self.t.tags()[first as usize];
        let flat = |n: Node| (Head::Flat(n), Wait::No);
        let block = |n: Node| (Head::Open(n), if opens { Wait::No } else { Wait::Brace });
        let at_switch = self.t.tags().get(stmt_end as usize + 1) == Some(&T::LangSpice);
        match tag {
            T::Title => {
                self.i = self.end;
                flat(self.ast.push(Tag::Title, first, NONE, NONE))
            }
            T::KwSimulator if at_switch => (
                Head::Open(self.head_fields(Tag::Region, Mode::Fields)),
                Wait::No,
            ),
            T::KwInclude => flat(self.head_fields(Tag::Include, Mode::Fields)),
            T::KwLibrary => (
                Head::Open(self.head_fields(Tag::Library, Mode::Fields)),
                Wait::No,
            ),
            T::KwEndlibrary => (
                Head::Close(Tag::Library, self.head_fields(Tag::Card, Mode::Fields)),
                Wait::No,
            ),
            T::KwSection if in_tag == Tag::Section => {
                self.ast.diag(Code::P0009, first);
                flat(self.verbatim(first))
            }
            T::KwSection => (
                Head::Open(self.head_fields(Tag::Section, Mode::Fields)),
                Wait::No,
            ),
            T::KwEndsection => (
                Head::Close(Tag::Section, self.head_fields(Tag::Card, Mode::Fields)),
                Wait::No,
            ),
            T::KwInline | T::KwSubckt => {
                self.i += 1;
                if tag == T::KwInline
                    && self.i < self.end
                    && self.t.tags()[self.i as usize] == T::KwSubckt
                {
                    self.i += 1;
                }
                let f = self.fields(Mode::Plain);
                (
                    Head::Open(self.ast.push(Tag::Subckt, first, f, NONE)),
                    Wait::No,
                )
            }
            T::KwEnds => (
                Head::Close(Tag::Subckt, self.head_fields(Tag::Card, Mode::Fields)),
                Wait::No,
            ),
            T::KwModel if opens => (
                Head::Open(self.head_fields(Tag::Model, Mode::Fields)),
                Wait::No,
            ),
            T::KwModel => flat(self.head_fields(Tag::Model, Mode::Fields)),
            T::KwParam => flat(self.head_fields(Tag::Param, Mode::Fields)),
            T::KwStatistics => block(self.head_fields(Tag::Statistics, Mode::Fields)),
            T::KwProcess | T::KwMismatch => block(self.head_fields(Tag::Process, Mode::Fields)),
            T::KwReal => block(self.head_fields(Tag::Func, Mode::Fields)),
            T::KwIf if self.t.text(first).starts_with('@') => {
                self.i += 1;
                let cond = self.condition();
                (
                    Head::Open(self.ast.push(Tag::If, first, cond, NONE)),
                    Wait::No,
                )
            }
            T::KwIf => {
                self.i += 1;
                let cond = self.paren_condition(self.end);
                let n = self.ast.push(Tag::If, first, cond, NONE);
                // `if (c) {`: the body follows; `if (c) stmt` or a `{` on
                // the next line: the branch waits.
                let wait = if opens && self.i == self.end {
                    Wait::No
                } else {
                    Wait::Brace
                };
                (Head::Open(n), wait)
            }
            T::KwElseif => {
                self.i += 1;
                let cond = self.condition();
                (
                    Head::Branch(self.ast.push(Tag::If, first, cond, NONE)),
                    Wait::No,
                )
            }
            T::KwElse => (
                Head::Branch(self.head_fields(Tag::Else, Mode::Fields)),
                Wait::No,
            ),
            T::KwEndif => (
                Head::Close(Tag::If, self.head_fields(Tag::Card, Mode::Fields)),
                Wait::No,
            ),
            T::KwControl => (
                Head::Open(self.head_fields(Tag::Control, Mode::Fields)),
                Wait::No,
            ),
            T::KwEndc => (
                Head::Close(Tag::Control, self.head_fields(Tag::Card, Mode::Fields)),
                Wait::No,
            ),
            T::KwGlobal => flat(self.head_fields(Tag::Global, Mode::Fields)),
            T::KwSave => flat(self.head_fields(Tag::Save, Mode::Fields)),
            T::KwIc => flat(self.head_fields(Tag::Ic, Mode::Fields)),
            T::KwNodeset => flat(self.head_fields(Tag::Nodeset, Mode::Fields)),
            T::KwReturn => {
                self.i += 1;
                let mut stop = self.end;
                while stop > self.i && self.t.tags()[stop as usize - 1] == T::Semicolon {
                    stop -= 1;
                }
                let saved = self.end;
                self.end = stop;
                let e = self.expr_all();
                self.end = saved;
                self.i = saved;
                let f = self.ast.push_list(&[e.0]);
                flat(self.ast.push(Tag::Card, first, f, NONE))
            }
            T::Ident if opens => (Head::Open(self.instance()), Wait::No),
            T::Ident if in_tag == Tag::Control => flat(self.head_fields(Tag::Card, Mode::Fields)),
            T::KwAnalysis => flat(self.analysis()),
            T::KwOptions => {
                self.i += 1;
                let args = self.items(Mode::Fields);
                let h = self.ast.push_header(NONE, NONE, NONE, &[], &args);
                flat(self.ast.push(Tag::Options, first, h, NONE))
            }
            T::Ident => flat(self.instance()),
            T::Number => flat(self.head_fields(Tag::Card, Mode::Fields)),
            // else: any other keyword (`ahdl_include`, `vary`, VACASK `load`,
            // `analysis` …) is a card read by its fields.
            kw if kw.lexeme().is_none() && !matches!(kw, T::String | T::Invalid | T::Directive) => {
                flat(self.head_fields(Tag::Card, Mode::Fields))
            }
            // else: a statement led by punctuation or a string has no head.
            _ => flat(self.verbatim(first)),
        }
    }

    /// A statement `tag` whose fields follow its head token.
    fn head_fields(&mut self, tag: Tag, mode: Mode) -> Node {
        let head = self.i;
        self.i += 1;
        let f = self.fields(mode);
        self.ast.push(tag, head, f, NONE)
    }

    /// `name [(nodes)] master params`, or `name nodes... master params`
    /// (spectre.peg `Inst`: without parentheses the master is the last bare
    /// word before the first `key=`); VACASK's master may be absent.
    fn instance(&mut self) -> Node {
        let first = self.i;
        let name = self.word();
        let mut nodes = Vec::new();
        let mut master = NONE;
        if self.i < self.end && self.t.tags()[self.i as usize] == T::LParen {
            self.i += 1;
            while self.i < self.end && self.t.tags()[self.i as usize] != T::RParen {
                if self.separates(self.i) {
                    self.i += 1;
                } else {
                    nodes.push(self.word().0);
                }
            }
            self.i = (self.i + 1).min(self.end);
            if self.i < self.end && !self.separates(self.i) && !self.pairstart(self.i) {
                master = self.word().0;
            }
        } else {
            while self.i < self.end && !self.separates(self.i) && !self.pairstart(self.i) {
                nodes.push(self.word().0);
            }
            master = nodes.pop().unwrap_or(NONE);
        }
        let args = self.items(Mode::Fields);
        let lex = self.t.lexicon_at(first);
        let word = (master != NONE).then(|| self.ast.name(Name(self.ast.rhs(Node(master)))));
        let (tag, code) = match word.and_then(|w| lex.master(w)) {
            Some(Statement::Analysis(a)) => (Tag::Analysis, a as u32),
            Some(Statement::Command(c)) if nodes.is_empty() => (Tag::Command, c as u32),
            Some(Statement::Options) if nodes.is_empty() => (Tag::Options, NONE),
            _ => {
                let device = word.and_then(|w| lex.device(w));
                (Tag::Instance, device.map_or(NONE, |d| d.primitive as u32))
            }
        };
        let h = self.ast.push_header(name.0, master, code, &nodes, &args);
        self.ast.push(tag, first, h, NONE)
    }

    /// VACASK `analysis name kind params`.
    fn analysis(&mut self) -> Node {
        let first = self.i;
        self.i += 1;
        let mut words = [NONE; 2];
        for w in &mut words {
            if self.i < self.end && !self.separates(self.i) && !self.pairstart(self.i) {
                *w = self.word().0;
            }
        }
        let [name, kind] = words;
        let lex = self.t.lexicon_at(first);
        let code = match kind {
            NONE => NONE,
            k => {
                let word = self.ast.name(Name(self.ast.rhs(Node(k))));
                lex.analysis(word).map_or(NONE, |a| a as u32)
            }
        };
        let args = self.items(Mode::Fields);
        let h = self.ast.push_header(name, kind, code, &[], &args);
        self.ast.push(Tag::Analysis, first, h, NONE)
    }
}
