//! [`Ast`] → the [`CircuitIR`] and [`Report`] the old reader built: the
//! temporary bridge of the rewrite (`docs/research/transpiler-architecture.md`
//! §5), so the new front end feeds the old writers while they still run.
//!
//! **The old reader** is the PEG grammars and the lowering this front end
//! replaced, as they stood at `daa0ad0`, the golden gate's oracle
//! (`git show daa0ad0:grammar/spice.peg`, `daa0ad0:src/import/lower/`); they
//! are gone from the tree. A citation "the old SPICE grammar's `card`" names
//! a rule there; "the old lowering's `raw`" a function.
//!
//! This is a port of the old lowering onto AST nodes, plus what the old
//! grammars decided and the AST does not: which card shapes they accepted,
//! which refusals were `Raw` and which `Unparsed`, the text of a logical
//! line as the old line stage joined it. Each such rule is marked
//! `// quirk:` and states what the old reader read, so the comment stands
//! without the file; that list is the inventory of the old reader, and
//! phase 7 turns the wrong ones into behaviour changes (`migration.xfail`
//! lists what this cannot reproduce).
//!
//! This file is the spine: the per-deck state and the card dispatch;
//! `blocks` walks the blocks (`.subckt`, `.if`, libraries) on a heap stack.
//! `text` joins tokens as the line
//! stage did, `refused` sorts what the grammar refused into `Raw` and
//! `Unparsed`, `values` reads values, `shapes`, `elements`, `devices` and
//! `sources` SPICE element cards, `cards`, `testbench`, `outputs` and
//! `options` SPICE dot cards, `named`, `named_blocks`, `named_elements` and
//! `named_analyses` Spectre and VACASK statements.
//!
//! Cites: the old lowering (ported) and line stage (`logical`); the old
//! SPICE grammar's `card`, `Raw`, `Unparsed`.
//!
//! Contents:
//! - `fn ast_to_ir(ast: &Ast, t: &Tokens, d: Dialect, name: &str) -> (CircuitIR, Report)`
//! - `Cx::named(&self) -> bool`
//! - `Cx::tag(&self, n: u32) -> Tag`
//! - `Cx::main(&self, n: u32) -> u32`
//! - `Cx::fields(&self, n: u32) -> &'a [u32]`
//! - `Cx::body(&self, n: u32) -> &'a [u32]`
//! - `Cx::ttag(&self, k: u32) -> T`
//! - `Cx::raw(&mut self, n: u32, here: &mut Subcircuit, block: bool)`
//! - `Cx::unparsed(&mut self, n: u32, here: &mut Subcircuit)`
//! - `Cx::keep(&mut self, n: u32, here: &mut Subcircuit, k: Kept)`
//! - `Cx::card(&mut self, n: u32, here: &mut Subcircuit, scope: Scope) -> Option<Opened>`
//! - `Cx::section(&mut self, n: u32, library: Option<String>) -> LibSection`
//! - `Cx::if_head_ok(&self, n: u32) -> bool`

use std::collections::{HashMap, HashSet};

use crate::codegen::Dialect;
use crate::import::Report;
use crate::ir::*;
use crate::syntax::ast::{Ast, Node, Tag};
use crate::syntax::token::{Tag as T, Tokens};
use blocks::Opened;

mod blocks;
mod cards;
mod devices;
mod elements;
mod named;
mod named_analyses;
mod named_blocks;
mod named_elements;
mod options;
mod outputs;
mod refused;
mod shapes;
mod sources;
mod testbench;
mod text;
mod values;

/// The per-deck state, as `lower::Cx`.
struct Cx<'a> {
    ast: &'a Ast<'a>,
    t: &'a Tokens<'a>,
    d: Dialect,
    /// The deck's `.model` names, lowercased (the old lowering).
    models: HashSet<String>,
    report: Report,
    tb: Testbench,
    globals: Vec<String>,
    functions: Vec<FuncDef>,
    defs: Vec<Subcircuit>,
    sections: Vec<LibSection>,
    /// Spectre/VACASK: model kinds by name, and subcircuit names.
    kinds: HashMap<String, String>,
    subckts: Vec<String>,
}

/// Where a card sits (the old lowering).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Top,
    Subckt,
    Section,
}

/// What the old reader made of a card it could not lower.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kept {
    /// The grammar read it, the IR has no field: a raw line, counted.
    Raw,
    /// The grammar refused it: an `Unparsed` line.
    Unparsed,
}

/// A `.subckt` head the grammar read: its name, ports and defaults.
struct SubcktHead {
    name: String,
    ports: Vec<Port>,
    params: Vec<ParamDef>,
}

/// Builds the `CircuitIR` named `name` and the `Report` that
/// `import::parse(t.source(), name, d)` returns, from `ast`, the parse of
/// `t`, read in dialect `d` (`syntax::lexicon::of(d)`).
///
/// # Panics
/// When `d` has no lexicon (CDL, Verilog).
pub fn ast_to_ir(ast: &Ast, t: &Tokens, d: Dialect, name: &str) -> (CircuitIR, Report) {
    let d = d.family();
    assert!(
        crate::syntax::lexicon::of(d).is_some(),
        "ast_to_ir: a dialect with a reader"
    );
    let mut cx = Cx {
        ast,
        t,
        d,
        models: HashSet::new(),
        report: Report::default(),
        tb: Testbench::default(),
        globals: vec![],
        functions: vec![],
        defs: vec![],
        sections: vec![],
        kinds: HashMap::new(),
        subckts: vec![],
    };
    let top_stmts = ast.fields(ast.root()).to_vec();
    if cx.named() {
        (cx.kinds, cx.subckts) = cx.declarations(&top_stmts);
    } else {
        cx.models = cx.model_names(&top_stmts);
    }
    let mut top = sub(name);
    cx.cards(&top_stmts, &mut top, Scope::Top);
    let Cx {
        report,
        tb,
        globals,
        functions,
        mut defs,
        sections,
        ..
    } = cx;
    // LTspice inductors carry an implicit Rser=1m (the old lowering).
    if d == Dialect::Ltspice {
        for sc in std::iter::once(&mut top).chain(defs.iter_mut()) {
            implicit_rser(&mut sc.components);
        }
    }
    if d == Dialect::Vacask {
        top.osdi_loads.sort();
    }
    let mut ir = CircuitIR {
        raw_dialect: Some(d),
        globals,
        functions,
        top,
        testbench: Some(tb),
        subcircuit_defs: defs,
        model_libraries: vec![],
        sections,
    };
    if d == Dialect::Spectre {
        crate::codegen::statistics::unlift(&mut ir);
    }
    (ir, report)
}

impl<'a> Cx<'a> {
    pub(super) fn named(&self) -> bool {
        matches!(self.d, Dialect::Spectre | Dialect::Vacask)
    }

    pub(super) fn tag(&self, n: u32) -> Tag {
        self.ast.tag(Node(n))
    }

    pub(super) fn main(&self, n: u32) -> u32 {
        self.ast.main_token(Node(n))
    }

    pub(super) fn fields(&self, n: u32) -> &'a [u32] {
        self.ast.fields(Node(n))
    }

    pub(super) fn body(&self, n: u32) -> &'a [u32] {
        self.ast.body(Node(n))
    }

    pub(super) fn ttag(&self, k: u32) -> T {
        self.t.tags()[k as usize]
    }

    /// A card kept as text (the old lowering's `raw`): a block one raw line per
    /// line, a SPICE element in its place among the components.
    pub(super) fn raw(&mut self, n: u32, here: &mut Subcircuit, block: bool) {
        self.report.raw += 1;
        if block {
            here.raw_spice.extend(self.lines(n));
            return;
        }
        let text = self.line(n);
        if !self.named() && text.starts_with(|c: char| c.is_ascii_alphabetic()) {
            here.components.push(Component::RawSpice { line: text });
        } else {
            here.raw_spice.push(text);
        }
    }

    /// A card the grammar refused: a raw line and a `Report::unparsed` one.
    pub(super) fn unparsed(&mut self, n: u32, here: &mut Subcircuit) {
        let line = self.line(n);
        self.report.unparsed.push(line.clone());
        here.raw_spice.push(line);
    }

    pub(super) fn keep(&mut self, n: u32, here: &mut Subcircuit, k: Kept) {
        match k {
            Kept::Raw => self.raw(n, here, false),
            Kept::Unparsed => self.unparsed(n, here),
        }
    }

    /// One card, as `lower::Cx::card`; a block with a body of cards
    /// (`.subckt`, `.if`, a refused head, a library) is handed back for
    /// [`Cx::cards`] to walk.
    pub(super) fn card(&mut self, n: u32, here: &mut Subcircuit, scope: Scope) -> Option<Opened> {
        if self.named() {
            return self.named_card_spine(n, here, scope);
        }
        let inside = scope == Scope::Subckt;
        if scope == Scope::Section && self.leaks(n) {
            // A `.control` kept raw is one line holding its newlines
            // (quirk: the old lowering's `raw` splits only `Block`-like rows).
            match self.tag(n) {
                Tag::Control => {
                    self.report.raw += 1;
                    here.raw_spice.push(self.lines(n).join("\n"));
                }
                _ => self.raw(n, here, false),
            }
            return None;
        }
        match self.tag(n) {
            Tag::Title | Tag::End => {}
            Tag::Verbatim | Tag::Line => self.unparsed(n, here),
            Tag::Subckt => match self.subckt_head(n) {
                Some(head) => return Some(Opened::Subckt(n, head)),
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
                let s = self.section(n, None);
                self.sections.push(s);
            }
            Tag::If => match self.if_head_ok(n) {
                true => {
                    return Some(Opened::If {
                        node: n,
                        named: false,
                    });
                }
                // quirk: the old SPICE grammar's `ifblock`: an `.if` with no
                // condition in parentheses is a `Block`, kept whole.
                false => self.raw(n, here, true),
            },
            Tag::Data => self.raw(n, here, true),
            Tag::Control => {
                for &c in self.body(n) {
                    if self.tag(c) == Tag::Analysis {
                        self.control(c);
                    }
                }
            }
            Tag::Instance => self.element(n, here),
            _ => self.dot_card(n, here, scope, inside),
        }
        None
    }

    /// `.lib name` … `.endl`: its cards lowered as at the top, into the
    /// section's own body and definitions (the old lowering's `section`).
    pub(super) fn section(&mut self, n: u32, library: Option<String>) -> LibSection {
        let name_item = self.fields(n)[0];
        let mut body = sub(&self.name_of(name_item));
        let (first, first_f) = (self.defs.len(), self.functions.len());
        let stmts = self.body(n).to_vec();
        self.cards(&stmts, &mut body, Scope::Section);
        let functions = self.functions.split_off(first_f);
        let subcircuit_defs = self.defs.split_off(first);
        LibSection {
            library,
            body,
            subcircuit_defs,
            functions,
        }
    }

    /// quirk: the old SPICE grammar's `Branch <- '.if' s? Cond s? nl`: the
    /// condition is one parenthesized group ending the line.
    pub(super) fn if_head_ok(&self, n: u32) -> bool {
        let first = self.main(n);
        let last = self.line_last(first);
        first < last
            && self.ttag(first + 1) == T::LParen
            && self.paren_close(first + 1) == Some(last)
    }
}

// ---- private ----

impl<'a> Cx<'a> {
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

fn implicit_rser(components: &mut [Component]) {
    let coupled: HashSet<String> = components
        .iter()
        .filter_map(|c| match c {
            Component::MutualInductor {
                inductor1,
                inductor2,
                ..
            } => Some([inductor1.clone(), inductor2.clone()]),
            _ => None,
        })
        .flatten()
        .collect();
    for c in components.iter_mut() {
        if let Component::Inductor { name, params, .. } = c
            && !coupled.contains(name)
            && !params.iter().any(|(k, _)| k.eq_ignore_ascii_case("rser"))
        {
            params.push(("Rser".to_string(), "1m".to_string()));
        }
    }
}

fn sub(name: &str) -> Subcircuit {
    Subcircuit {
        name: name.into(),
        ..Default::default()
    }
}

/// A condition without the parentheses around it, blanks folded
/// (the old lowering's `unwrapped`).
fn unwrapped(t: &str) -> String {
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
