//! A file's [`Ast`] (and its kit) → the [`Deck`] plan the SPICE-family
//! writer spells: which statement is written in which block and in which of
//! the deck's places, as the old reader placed it, and the read-only views
//! (`Card`, `Val`, `Bench`, …) the writer asks of each statement. The plan
//! holds node ids and the few names the reader makes up (a hoisted
//! subcircuit's `parent__name`, the name map of instance masters); every
//! other word is spelled from the statement's tokens when it is written.
//!
//! **The old reader** is the PEG grammars and the lowering at `daa0ad0`,
//! which `syntax::adapter` ports onto the AST for the old writers; this
//! reader decides what each statement is by the same rules (`// quirk:`
//! marks each rule of the old reader it keeps), so the renderer's output is
//! the old pipeline's, byte for byte, which the golden gate checks
//! (TESTING.md, "Golden gate"). Phase 6 deletes the adapter and leaves these
//! rules here.
//!
//! This file is the spine: the plan's types and the statement dispatch;
//! `blocks` walks the blocks on a heap stack, `text` joins tokens as the old
//! line stage did, `refused` sorts what the old grammar refused, `values`,
//! `shapes`, `elements`, `devices` and `sources` read element cards,
//! `cards`, `testbench`, `outputs` and `options` dot cards.
//!
//! Cites: `docs/research/transpiler-architecture.md` §2.5; the old lowering
//! and its SPICE grammar's `card`, `Raw`, `Unparsed` (as `syntax::adapter`
//! cites them).
//!
//! Contents:
//! - `pub(in crate::syntax::render) struct Rd`, `enum Kept`, `struct Deck`, `struct Block`, `enum Comp`,
//!   `enum Raw`, `struct Cond`, `struct Section`, `enum Func`
//! - `fn read(ast: &Ast, t: &Tokens, d: Dialect, name: &str, kit: Option<(&Kit, FileId)>) -> (Rd, Deck)`
//! - `Rd::kit_split`, `Rd::master_name`, `Rd::named`, `Rd::tag`, `Rd::main`, `Rd::fields`, `Rd::body`,
//!   `Rd::ttag`, `Rd::raw`, `Rd::unparsed`, `Rd::keep`, `Rd::card`, `Rd::section`, `Rd::if_head_ok`
//! - `fn unwrapped(t: &str) -> String`

use std::collections::{HashMap, HashSet};

use crate::codegen::Dialect;
use crate::syntax::ast::{Ast, NONE, Node, Tag};
use crate::syntax::resolve::{FileId, Kit, Split};
use crate::syntax::token::{Tag as T, Tokens};
use blocks::Opened;

mod blocks;
mod cards;
mod devices;
mod elements;
mod named;
mod named_bench;
mod named_blocks;
mod named_elements;
mod options;
mod outputs;
mod refused;
mod shapes;
mod sources;
mod testbench;
mod text;
mod unlift;
mod values;

pub(in crate::syntax::render) use elements::Card;
pub(in crate::syntax::render) use options::Opt;
pub(in crate::syntax::render) use sources::{Num, Wave, WaveKind};
pub(in crate::syntax::render) use testbench::{Ana, BenchCard, Sweep};
pub(in crate::syntax::render) use values::Val;

/// The reader: the file, its kit, and the deck-wide facts every statement's
/// view reads (the old lowering's `Cx`, less what it built).
pub(in crate::syntax::render) struct Rd<'a> {
    pub(in crate::syntax::render) ast: &'a Ast<'a>,
    pub(in crate::syntax::render) t: &'a Tokens<'a>,
    /// The dialect the file was read in (its family: ESPice reads ngspice).
    pub(in crate::syntax::render) d: Dialect,
    /// The deck's `.model` names, lowercased (the old lowering).
    models: HashSet<String>,
    /// Spectre/VACASK: model kinds by name, and subcircuit names.
    kinds: HashMap<String, String>,
    subckts: Vec<String>,
    kit: Option<(&'a Kit, FileId)>,
    /// The name map: an `X` card's master as written, after the renames of
    /// the hoisted subcircuits it names (`parent__name`).
    masters: HashMap<u32, String>,
    /// The Monte Carlo read-back rule's fixups (`unlift`).
    unlifted: unlift::Unlift,
}

/// What the reader made of a file: every block and the deck-wide lists, in
/// the order the writer reads them.
#[derive(Default)]
pub(in crate::syntax::render) struct Deck {
    pub(in crate::syntax::render) top: Block,
    /// Every subcircuit at the top or nested in one (hoisted), in the order
    /// each closed.
    pub(in crate::syntax::render) defs: Vec<Block>,
    pub(in crate::syntax::render) sections: Vec<Section>,
    pub(in crate::syntax::render) globals: Vec<String>,
    pub(in crate::syntax::render) funcs: Vec<Func>,
    /// The testbench statements at the top, in read order (`.control`'s
    /// analyses among them); [`Rd::bench_cards`] reads each.
    pub(in crate::syntax::render) bench: Vec<Bench>,
}

/// A testbench item: a statement, and how it adds to the testbench.
#[derive(Clone, Copy, Debug)]
pub(in crate::syntax::render) enum Bench {
    /// A SPICE testbench card (`BenchCard`).
    Card(u32),
    /// A `.meas` line of a Spectre `simulator lang=spice` region.
    Meas(u32),
    /// Spectre/VACASK `save`.
    Save(u32),
    /// A Spectre analysis.
    Analysis(u32),
    /// Spectre `options`.
    Options(u32),
    /// Spectre `ic`/`nodeset`.
    NodeVals(u32),
    /// A VACASK `control` block.
    Control(u32),
}

/// One block's statements by place: a subcircuit, the top, a section's
/// body, a branch.
pub(in crate::syntax::render) struct Block {
    /// The name it is written under.
    pub(in crate::syntax::render) name: String,
    /// Its `Subckt` node; `NONE` for the top, a section, a branch.
    pub(in crate::syntax::render) node: u32,
    /// Its elements in order: typed cards and raw element lines.
    pub(in crate::syntax::render) comps: Vec<Comp>,
    /// Its `X` cards, each with the count of `comps` before it.
    pub(in crate::syntax::render) insts: Vec<(u32, u32)>,
    /// Its `.if` chains, each with the count of `comps` before it.
    pub(in crate::syntax::render) conds: Vec<(u32, Cond)>,
    /// Its `.model` cards: the statement, and the bin of a Spectre model
    /// group (`NONE`: none).
    pub(in crate::syntax::render) models: Vec<(u32, u32)>,
    /// Its `.param` assignments (at a deck's or section's top).
    pub(in crate::syntax::render) params: Vec<u32>,
    /// Its `.param` assignments inside a subcircuit.
    pub(in crate::syntax::render) locals: Vec<u32>,
    /// Its `.include` cards.
    pub(in crate::syntax::render) includes: Vec<u32>,
    /// Its `.lib file section` cards.
    pub(in crate::syntax::render) libs: Vec<u32>,
    /// What it keeps as text, after its elements.
    pub(in crate::syntax::render) raw: Vec<Raw>,
    /// Its Verilog-A loads (Spectre `ahdl_include`, VACASK `load`).
    pub(in crate::syntax::render) loads: Vec<u32>,
    /// Its Spectre `statistics` blocks.
    pub(in crate::syntax::render) statistics: Vec<u32>,
}

/// One element of a block.
#[derive(Clone, Copy)]
pub(in crate::syntax::render) enum Comp {
    /// A card the writer spells (`Rd::card_view`).
    Card(u32),
    /// An element card kept as its line.
    Raw(u32),
}

/// A statement kept as text.
#[derive(Clone, Copy)]
pub(in crate::syntax::render) enum Raw {
    /// Its logical line.
    Line(u32),
    /// A block kept whole, one line per line.
    Lines(u32),
    /// A block kept whole as one line holding its newlines.
    Joined(u32),
}

/// An `.if` chain: each branch's head (`If` or `Else`) and body.
pub(in crate::syntax::render) struct Cond {
    pub(in crate::syntax::render) branches: Vec<(u32, Block)>,
}

/// A `.lib name` … `.endl` section.
pub(in crate::syntax::render) struct Section {
    /// The Spectre `library` it sits in.
    #[expect(dead_code, reason = "the named writer reads it")]
    pub(in crate::syntax::render) library: Option<String>,
    pub(in crate::syntax::render) body: Block,
    pub(in crate::syntax::render) defs: Vec<Block>,
    pub(in crate::syntax::render) funcs: Vec<Func>,
}

/// A function definition.
#[derive(Clone, Copy)]
pub(in crate::syntax::render) enum Func {
    /// `.param f(x)=…`: the assignment.
    Param(u32),
    /// `.func f(x) body`.
    Card(u32),
    /// Spectre `real f(real x) { return … }`.
    Real(u32),
}

/// What the old reader made of a card it could not lower.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::syntax::render) enum Kept {
    /// The grammar read it, the IR has no field: a raw line.
    Raw,
    /// The grammar refused it: an `Unparsed` line.
    Unparsed,
}

/// Where a card sits (the old lowering).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Top,
    Subckt,
    Section,
}

/// The deck state [`Rd::card`] appends to besides the block it writes into.
#[derive(Default)]
struct Acc {
    globals: Vec<String>,
    funcs: Vec<Func>,
    defs: Vec<Block>,
    sections: Vec<Section>,
    bench: Vec<Bench>,
}

impl Default for Block {
    fn default() -> Block {
        Block {
            name: String::new(),
            node: NONE,
            comps: vec![],
            insts: vec![],
            conds: vec![],
            models: vec![],
            params: vec![],
            locals: vec![],
            includes: vec![],
            libs: vec![],
            raw: vec![],
            loads: vec![],
            statistics: vec![],
        }
    }
}

/// Reads `ast` (the parse of `t`, in dialect `d`) into the plan the writer
/// spells, the top block named `name`; with `kit`, what the file alone
/// leaves open is the kit's answer, as `ast_to_ir` reads it.
///
/// # Panics
/// When `ast` is not `kit`'s tree of its file.
pub(in crate::syntax::render) fn read<'a>(
    ast: &'a Ast<'a>,
    t: &'a Tokens<'a>,
    d: Dialect,
    name: &str,
    kit: Option<(&'a Kit, FileId)>,
) -> (Rd<'a>, Deck) {
    let d = d.family();
    if let Some((k, f)) = kit {
        assert_eq!(ast.len(), k.node_count(f), "read: the kit file's tree");
    }
    let mut rd = Rd {
        ast,
        t,
        d,
        models: HashSet::new(),
        kinds: HashMap::new(),
        subckts: Vec::new(),
        kit,
        masters: HashMap::new(),
        unlifted: unlift::Unlift::default(),
    };
    let top_stmts = ast.fields(ast.root()).to_vec();
    if rd.named() {
        (rd.kinds, rd.subckts) = rd.declarations(&top_stmts);
    } else {
        rd.models = rd.model_names(&top_stmts);
    }
    let mut acc = Acc::default();
    let mut top = Block {
        name: name.into(),
        ..Block::default()
    };
    rd.cards(&top_stmts, &mut top, Scope::Top, &mut acc);
    if d == Dialect::Vacask {
        // The old reader sorted the top's loads by path.
        top.loads.sort_by_key(|&n| rd.unquote(rd.fields(n)[0]));
    }
    let mut deck = Deck {
        top,
        defs: acc.defs,
        sections: acc.sections,
        globals: acc.globals,
        funcs: acc.funcs,
        bench: acc.bench,
    };
    rd.unlift(&mut deck);
    (rd, deck)
}

impl<'a> Rd<'a> {
    pub(in crate::syntax::render) fn named(&self) -> bool {
        matches!(self.d, Dialect::Spectre | Dialect::Vacask)
    }

    pub(in crate::syntax::render) fn tag(&self, n: u32) -> Tag {
        self.ast.tag(Node(n))
    }

    pub(in crate::syntax::render) fn main(&self, n: u32) -> u32 {
        self.ast.main_token(Node(n))
    }

    pub(in crate::syntax::render) fn fields(&self, n: u32) -> &'a [u32] {
        self.ast.fields(Node(n))
    }

    pub(in crate::syntax::render) fn body(&self, n: u32) -> &'a [u32] {
        self.ast.body(Node(n))
    }

    pub(in crate::syntax::render) fn ttag(&self, k: u32) -> T {
        self.t.tags()[k as usize]
    }

    /// The kit's corrected split of element `n`.
    pub(in crate::syntax::render) fn kit_split(&self, n: u32) -> Option<Split> {
        let (kit, f) = self.kit?;
        kit.split(f, Node(n))
    }

    /// `X` card `n`'s master, as the name map holds it.
    pub(in crate::syntax::render) fn master_name(&self, n: u32) -> String {
        match self.masters.get(&n) {
            Some(m) => m.clone(),
            None => self.name_of(self.ast.header(Node(n)).master),
        }
    }
}

/// A condition without the parentheses around it, blanks folded
/// (the old lowering's `unwrapped`).
pub(in crate::syntax::render) fn unwrapped(t: &str) -> String {
    let t = t.trim();
    let inner = t
        .strip_prefix('(')
        .and_then(|r| r.strip_suffix(')'))
        .filter(|r| {
            let mut depth = 0i32;
            r.chars().all(|c| {
                depth += i32::from(c == '(') - i32::from(c == ')');
                depth >= 0
            })
        })
        .unwrap_or(t);
    inner.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---- private ----

impl<'a> Rd<'a> {
    /// A card kept as text (the old lowering's `raw`): a block one raw line
    /// per line, a SPICE element in its place among the elements.
    fn raw(&mut self, n: u32, here: &mut Block, block: bool) {
        if block {
            here.raw.push(Raw::Lines(n));
            return;
        }
        let letter = self
            .t
            .text(self.main(n))
            .starts_with(|c: char| c.is_ascii_alphabetic());
        if !self.named() && letter {
            here.comps.push(Comp::Raw(n));
        } else {
            here.raw.push(Raw::Line(n));
        }
    }

    /// A card the grammar refused: a raw line.
    fn unparsed(&mut self, n: u32, here: &mut Block) {
        here.raw.push(Raw::Line(n));
    }

    fn keep(&mut self, n: u32, here: &mut Block, k: Kept) {
        match k {
            Kept::Raw => self.raw(n, here, false),
            Kept::Unparsed => self.unparsed(n, here),
        }
    }

    /// One card, as the adapter's `Cx::card`; a block with a body of cards
    /// is handed back for [`Rd::cards`] to walk.
    fn card(&mut self, n: u32, here: &mut Block, scope: Scope, acc: &mut Acc) -> Option<Opened> {
        if self.named() {
            return self.named_card(n, here, scope, acc);
        }
        let inside = scope == Scope::Subckt;
        if scope == Scope::Section && self.leaks(n) {
            // A `.control` kept raw is one line holding its newlines
            // (quirk: the old lowering's `raw` splits only `Block`-like rows).
            match self.tag(n) {
                Tag::Control => here.raw.push(Raw::Joined(n)),
                _ => self.raw(n, here, false),
            }
            return None;
        }
        match self.tag(n) {
            Tag::Title | Tag::End => {}
            Tag::Verbatim | Tag::Line => self.unparsed(n, here),
            Tag::Subckt => match self.subckt_head(n) {
                Some(_) => return Some(Opened::Subckt(n)),
                // quirk: a `.subckt` head the old SPICE grammar does not read is
                // `Unparsed`; its body then reads as cards here, and its
                // `.ends` is a `Raw` card.
                None => {
                    self.unparsed(n, here);
                    return Some(Opened::Dissolve(n));
                }
            },
            Tag::Section if scope != Scope::Top => self.raw(n, here, true),
            Tag::Section => {
                let s = self.section(n, None, acc);
                acc.sections.push(s);
            }
            Tag::If => match self.if_head_ok(n) {
                true => return Some(Opened::If(n)),
                // quirk: the old SPICE grammar's `ifblock`: an `.if` with no
                // condition in parentheses is a `Block`, kept whole.
                false => self.raw(n, here, true),
            },
            Tag::Data => self.raw(n, here, true),
            Tag::Control => {
                for &c in self.body(n) {
                    if self.tag(c) == Tag::Analysis && self.testbench_peg(c) {
                        acc.bench.push(Bench::Card(c));
                    }
                }
            }
            Tag::Instance => self.element(n, here),
            _ => self.dot_card(n, here, scope, inside, acc),
        }
        None
    }

    /// `.lib name` … `.endl`: its cards read as at the top, into the
    /// section's own body and definitions (the old lowering's `section`).
    fn section(&mut self, n: u32, library: Option<String>, acc: &mut Acc) -> Section {
        let name_item = self.fields(n)[0];
        let mut body = Block {
            name: self.name_of(name_item),
            ..Block::default()
        };
        let (first, first_f) = (acc.defs.len(), acc.funcs.len());
        let stmts = self.body(n).to_vec();
        self.cards(&stmts, &mut body, Scope::Section, acc);
        Section {
            library,
            body,
            defs: acc.defs.split_off(first),
            funcs: acc.funcs.split_off(first_f),
        }
    }

    /// quirk: the old SPICE grammar's `Branch <- '.if' s? Cond s? nl`: the
    /// condition is one parenthesized group ending the line.
    fn if_head_ok(&self, n: u32) -> bool {
        let first = self.main(n);
        let last = self.line_last(first);
        first < last
            && self.ttag(first + 1) == T::LParen
            && self.paren_close(first + 1) == Some(last)
    }

    /// Whether a card in a section would reach past it (the old lowering's
    /// `leaks`).
    fn leaks(&self, n: u32) -> bool {
        match self.tag(n) {
            Tag::Global | Tag::Control => true,
            Tag::Save => self.ttag(self.main(n)) == T::KwSave,
            _ => false,
        }
    }

    /// The `)` matching the `(` at token `open`, on its statement.
    fn paren_close(&self, open: u32) -> Option<u32> {
        let last = self.line_last(open);
        let mut depth = 0i32;
        for k in open..=last {
            match self.ttag(k) {
                T::LParen => depth += 1,
                T::RParen => {
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

    /// The `.model` names the grammar read, lowercased, anywhere in the
    /// deck but kept-whole blocks.
    fn model_names(&self, stmts: &[u32]) -> HashSet<String> {
        let mut out = HashSet::new();
        let mut work: Vec<u32> = stmts.iter().rev().copied().collect();
        while let Some(s) = work.pop() {
            match self.tag(s) {
                Tag::Model => {
                    if let Some(m) = self.model_head(s) {
                        out.insert(self.text(m.0).to_ascii_lowercase());
                    }
                }
                Tag::Data | Tag::Control => {}
                Tag::If if !self.if_head_ok(s) => {}
                Tag::Subckt | Tag::Section | Tag::If | Tag::Else => {
                    if let Some(nx) = self.ast.next(Node(s)) {
                        work.push(nx.0);
                    }
                    work.extend(self.body(s).iter().rev());
                }
                _ => {}
            }
        }
        out
    }
}
