//! A SPICE-family statement head → its node: a dot card by its keyword (a
//! block opener, a closer, or a flat card whose fields the shared reader
//! takes), or an element by its letter (`Instance`). An `X` card's master is
//! the word the card's shape names: the last bare word before its
//! parameters, or the one after them (sky130's `special_pfet_latch`).
//!
//! Cites: `grammar/spice.peg` (`card`, `dotcard`, `element`, `X`, `LibDef`,
//! `If`, `Block`) and the dialect overlays; ngspice manual §2.5 (`.subckt`),
//! §2.7 (`.lib`), §2.15 (`.if`), §3.1 (element letters), ch.13 (`.control`).
//!
//! Contents:
//! - `Parser::spice(&mut self, in_tag: Tag) -> Head`
//! - `Parser::dot_card(&mut self, kw: T) -> Node`

use crate::syntax::ast::{NONE, Node, PRIMITIVE, Tag};
use crate::syntax::diag::Code;
use crate::syntax::fields::Mode;
use crate::syntax::lexicon::Element;
use crate::syntax::parse::{Head, Parser};
use crate::syntax::token::Tag as T;

impl Parser<'_, '_> {
    /// The SPICE statement at the cursor, inside a block tagged `in_tag`.
    pub(super) fn spice(&mut self, in_tag: Tag) -> Head {
        let first = self.i;
        let lex = self.t.lexicon_at(first);
        match self.t.tags()[first as usize] {
            T::Title => {
                self.i = self.end;
                Head::Flat(self.ast.push(Tag::Title, first, NONE, NONE))
            }
            T::KwEnd => {
                self.i = self.end;
                Head::Flat(self.ast.push(Tag::End, first, NONE, NONE))
            }
            T::KwSubckt => Head::Open(self.head_only(Tag::Subckt, Mode::Plain)),
            T::KwEnds => Head::Close(Tag::Subckt, self.head_only(Tag::Card, Mode::Fields)),
            T::KwLib if lex.lib_sections && self.one_word_after(first) => {
                if in_tag == Tag::Section {
                    self.ast.diag(Code::P0009, first);
                    return Head::Flat(self.verbatim(first));
                }
                Head::Open(self.head_only(Tag::Section, Mode::Plain))
            }
            T::KwEndl => Head::Close(Tag::Section, self.head_only(Tag::Card, Mode::Fields)),
            T::KwIf => {
                self.i += 1;
                let cond = self.condition();
                Head::Open(self.ast.push(Tag::If, first, cond, NONE))
            }
            T::KwElseif => {
                self.i += 1;
                let cond = self.condition();
                Head::Branch(self.ast.push(Tag::If, first, cond, NONE))
            }
            T::KwElse => Head::Branch(self.head_only(Tag::Else, Mode::Fields)),
            T::KwEndif => Head::Close(Tag::If, self.head_only(Tag::Card, Mode::Fields)),
            T::KwControl => Head::Open(self.head_only(Tag::Control, Mode::Fields)),
            T::KwData | T::KwVariation => Head::Open(self.head_only(Tag::Data, Mode::Fields)),
            T::KwEndc => Head::Close(Tag::Control, self.head_only(Tag::Card, Mode::Fields)),
            T::KwEnddata | T::KwEndVariation => {
                Head::Close(Tag::Data, self.head_only(Tag::Card, Mode::Fields))
            }
            T::Ident => Head::Flat(self.element()),
            kw if kw == T::Directive || kw.lexeme().is_none() && is_keyword(kw) => {
                Head::Flat(self.dot_card(kw))
            }
            // else: a statement led by a number or punctuation (`5 a b`, `@x`)
            // has no SPICE head.
            _ => Head::Flat(self.verbatim(first)),
        }
    }

    /// A flat dot card led by keyword `kw` at the cursor.
    pub(super) fn dot_card(&mut self, kw: T) -> Node {
        let (tag, mode) = shape(kw);
        self.head_only(tag, mode)
    }
}

// ---- private ----

/// The node and field mode of a dot card led by `kw`.
fn shape(kw: T) -> (Tag, Mode) {
    match kw {
        T::KwParam => (Tag::Param, Mode::Fields),
        T::KwFunc => (Tag::Func, Mode::Fields),
        T::KwModel => (Tag::Model, Mode::Model),
        T::KwInclude => (Tag::Include, Mode::Chunks),
        T::KwLib => (Tag::LibCall, Mode::Chunks),
        T::KwGlobal => (Tag::Global, Mode::Fields),
        T::KwOptions => (Tag::Options, Mode::Fields),
        T::KwTemp => (Tag::Temp, Mode::Fields),
        T::KwIc => (Tag::Ic, Mode::Fields),
        T::KwNodeset => (Tag::Nodeset, Mode::Fields),
        T::KwSave | T::KwProbe => (Tag::Save, Mode::Fields),
        T::KwPrint => (Tag::Print, Mode::Fields),
        T::KwMeas => (Tag::Measure, Mode::Fields),
        T::KwOp
        | T::KwDc
        | T::KwAc
        | T::KwTran
        | T::KwNoise
        | T::KwTf
        | T::KwSens
        | T::KwPz
        | T::KwFour
        | T::KwSp => (Tag::Analysis, Mode::Fields),
        T::KwStep => (Tag::Step, Mode::Fields),
        // else: any other keyword or directive (`.mc`, `.backanno`, a
        // Spectre word in a SPICE region) is a generic card.
        _ => (Tag::Card, Mode::Fields),
    }
}

/// Whether `tag` is a keyword (one of the `Kw*` tags).
fn is_keyword(tag: T) -> bool {
    !matches!(
        tag,
        T::Title
            | T::EndOfStatement
            | T::LangSpice
            | T::LangSpectre
            | T::Invalid
            | T::Ident
            | T::Number
            | T::String
            | T::Directive
    )
}

impl Parser<'_, '_> {
    /// A statement `tag` whose fields follow its head token, its body (if
    /// any) to be filled by the block stack.
    fn head_only(&mut self, tag: Tag, mode: Mode) -> Node {
        let head = self.i;
        self.i += 1;
        let f = self.fields(mode);
        self.ast.push(tag, head, f, NONE)
    }

    /// Whether the statement is its head token and one word (`.lib name`).
    fn one_word_after(&self, head: u32) -> bool {
        let w = head + 1;
        w < self.end && !self.separates(w) && self.word_end(w) == self.end - 1
    }

    /// An element card: its name word, every item after it, and its master.
    fn element(&mut self) -> Node {
        let first = self.i;
        let name = self.word();
        let letter = self.t.text(first).as_bytes()[0];
        let element = self.t.lexicon_at(first).element(letter);
        let fields = self.fields(Mode::Element);
        let items: Vec<u32> = self.ast.list(fields).to_vec();
        let (master, nodes) = match element {
            None => (NONE, Vec::new()),
            Some(Element::Instance) => self.x_master(&items),
            Some(e) => (PRIMITIVE | e as u32, Vec::new()),
        };
        let mut words = vec![name.0, master, nodes.len() as u32];
        words.extend(nodes);
        let at = self.ast.push_extra(&words);
        self.ast.push(Tag::Instance, first, fields, at)
    }

    /// An `X` card's master and nodes (spice.peg `X`): the last bare word
    /// before `params:` and the `k=v`s; or, with parameters before one last
    /// word, that word. `NONE` when the card has neither shape.
    fn x_master(&self, items: &[u32]) -> (u32, Vec<u32>) {
        let word = |n: u32| self.ast.tag(Node(n)) == Tag::Word;
        let assign = |n: u32| self.ast.tag(Node(n)) == Tag::Assign;
        let params = |n: u32| {
            word(n)
                && self
                    .t
                    .text(self.ast.main_token(Node(n)))
                    .eq_ignore_ascii_case("params:")
        };
        let k = items
            .iter()
            .position(|&n| assign(n) || params(n))
            .unwrap_or(items.len());
        let after = &items[k..];
        let rest_ok = after.first().is_none_or(|&n| assign(n) || params(n))
            && after.iter().skip(1).all(|&n| assign(n));
        if k >= 1 && rest_ok && items[..k].iter().all(|&n| word(n) && !params(n)) {
            return (items[k - 1], items[..k - 1].to_vec());
        }
        // The late name: nodes, `k=v`s, one word, the card's end.
        if let [nodes @ .., last] = items
            && word(*last)
            && k < items.len() - 1
            && items[k..items.len() - 1].iter().all(|&n| assign(n))
            && nodes[..k].iter().all(|&n| word(n))
        {
            return (*last, items[..k].to_vec());
        }
        (NONE, Vec::new())
    }
}
