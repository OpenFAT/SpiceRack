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
//! This file is the spine: the per-deck state and the walk over statements
//! and blocks (`.subckt`, sections, `.if`). `text` joins tokens as the line
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
//! - `Cx::cards(&mut self, stmts: &[u32], here: &mut Subcircuit, scope: Scope)`
//! - `Cx::raw(&mut self, n: u32, here: &mut Subcircuit, block: bool)`
//! - `Cx::unparsed(&mut self, n: u32, here: &mut Subcircuit)`
//! - `Cx::keep(&mut self, n: u32, here: &mut Subcircuit, k: Kept)`
//! - `Cx::card(&mut self, n: u32, here: &mut Subcircuit, scope: Scope)`
//! - `Cx::section(&mut self, n: u32, library: Option<String>) -> LibSection`
//! - `Cx::if_head_ok(&self, n: u32) -> bool`
//! - `Cx::subckt(&mut self, n: u32, head: SubcktHead) -> Subcircuit`

use std::collections::{HashMap, HashSet};

use crate::codegen::Dialect;
use crate::import::Report;
use crate::ir::*;
use crate::syntax::ast::{Ast, Node, Tag};
use crate::syntax::token::{Tag as T, Tokens};

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

/// A definition [`Cx::subckt`] has open: its node, the body read so far,
/// and what its nested definitions renamed.
struct OpenSubckt {
    node: u32,
    /// The next body statement to read.
    next: usize,
    sc: Subcircuit,
    /// `self.defs.len()` when it opened: its hoisted definitions follow.
    first: usize,
    renames: Vec<(String, String)>,
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

    /// Each statement of `stmts`, in order.
    pub(super) fn cards(&mut self, stmts: &[u32], here: &mut Subcircuit, scope: Scope) {
        for &s in stmts {
            self.card(s, here, scope);
        }
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

    /// One card, as `lower::Cx::card`.
    pub(super) fn card(&mut self, n: u32, here: &mut Subcircuit, scope: Scope) {
        if self.named() {
            return self.named_card_spine(n, here, scope);
        }
        let inside = scope == Scope::Subckt;
        if scope == Scope::Section && self.leaks(n) {
            // A `.control` kept raw is one line holding its newlines
            // (quirk: the old lowering's `raw` splits only `Block`-like rows).
            return match self.tag(n) {
                Tag::Control => {
                    self.report.raw += 1;
                    here.raw_spice.push(self.lines(n).join("\n"));
                }
                _ => self.raw(n, here, false),
            };
        }
        match self.tag(n) {
            Tag::Title | Tag::End => {}
            Tag::Verbatim | Tag::Line => self.unparsed(n, here),
            Tag::Subckt => match self.subckt_head(n) {
                Some(head) => {
                    let sc = self.subckt(n, head);
                    self.defs.push(sc);
                }
                // quirk: a `.subckt` head the old SPICE grammar does not read is
                // `Unparsed`; its body then reads as cards here, and its
                // `.ends` is a `Raw` card.
                None => self.dissolve(n, here, scope),
            },
            Tag::Section if scope != Scope::Top => self.raw(n, here, true),
            Tag::Section => {
                let s = self.section(n, None);
                self.sections.push(s);
            }
            Tag::If => match self.if_head_ok(n) {
                true => {
                    let c = Conditional {
                        at: Some(here.components.len() as u32),
                        ..self.conditional(n, scope)
                    };
                    here.conditionals.push(c);
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

    /// `.subckt` … `.ends` (the old lowering's `subckt`): nested definitions
    /// hoisted as `<parent>__<name>`, the parent's references renamed. The
    /// nesting is walked on a heap stack of open definitions, so a deck
    /// nested 10⁴ deep needs no deeper native stack than a flat one.
    pub(super) fn subckt(&mut self, n: u32, head: SubcktHead) -> Subcircuit {
        let mut stack = vec![self.open_subckt(n, head)];
        loop {
            let top = stack.last_mut().expect("a definition is open");
            if let Some(&k) = self.body(top.node).get(top.next) {
                top.next += 1;
                if self.tag(k) == Tag::Subckt
                    && let Some(h) = self.subckt_head(k)
                {
                    let inner = self.open_subckt(k, h);
                    stack.push(inner);
                } else {
                    self.card(k, &mut top.sc, Scope::Subckt);
                }
                continue;
            }
            let done = stack.pop().expect("the top definition");
            let mut sc = self.close_subckt(done);
            let Some(parent) = stack.last_mut() else {
                return sc;
            };
            let scoped = format!("{}__{}", parent.sc.name, sc.name);
            parent
                .renames
                .push((std::mem::replace(&mut sc.name, scoped.clone()), scoped));
            self.defs.push(sc);
        }
    }
}

// ---- private ----

impl<'a> Cx<'a> {
    fn open_subckt(&self, node: u32, head: SubcktHead) -> OpenSubckt {
        let mut sc = sub(&head.name);
        sc.ports = head.ports;
        sc.parameters = head.params;
        OpenSubckt {
            node,
            next: 0,
            sc,
            first: self.defs.len(),
            renames: Vec::new(),
        }
    }

    /// The finished definition, its and its hoisted definitions'
    /// references to the nested ones renamed.
    fn close_subckt(&mut self, f: OpenSubckt) -> Subcircuit {
        let OpenSubckt {
            mut sc,
            first,
            renames,
            ..
        } = f;
        for inst in sc.instances.iter_mut().chain(
            self.defs[first..]
                .iter_mut()
                .flat_map(|d| d.instances.iter_mut()),
        ) {
            if let Some((_, to)) = renames
                .iter()
                .find(|(from, _)| from.eq_ignore_ascii_case(&inst.subcircuit))
            {
                inst.subcircuit = to.clone();
            }
        }
        sc
    }

    /// A block whose head the grammar refused: the head `Unparsed`, the
    /// body read as cards where the block sits, the closer refused too.
    fn dissolve(&mut self, n: u32, here: &mut Subcircuit, scope: Scope) {
        self.unparsed(n, here);
        let body = self.body(n).to_vec();
        self.cards(&body, here, scope);
        if let Some(c) = self.ast.closer(Node(n)) {
            let k = self.refused(c.0);
            self.keep(c.0, here, k);
        }
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

    /// A `.if` chain → a `Conditional` (the old lowering).
    fn conditional(&mut self, n: u32, scope: Scope) -> Conditional {
        let mut c = Conditional::default();
        let mut branch = Some(Node(n));
        while let Some(b) = branch {
            let condition = match self.tag(b.0) {
                Tag::If => {
                    let first = self.main(b.0);
                    Some(unwrapped(&self.span(first + 1, self.line_last(first))))
                }
                _ => None,
            };
            let mut body = Subcircuit::default();
            let stmts = self.body(b.0).to_vec();
            self.cards(&stmts, &mut body, scope);
            c.branches.push(Branch { condition, body });
            branch = self.ast.next(b);
        }
        c
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
