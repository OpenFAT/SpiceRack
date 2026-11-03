//! [`Ast`] → today's [`CircuitIR`] and [`Report`], exactly as
//! `import::parse` builds them: the temporary bridge of the rewrite's phase
//! 2 (`docs/research/transpiler-architecture.md` §5), so the new front end
//! feeds the old writers while they still run.
//!
//! It is a port of `import/lower/*` onto AST nodes, plus what the old
//! grammars decided and the AST does not: which card shapes `grammar/*.peg`
//! accepted, which refusals were `Raw` and which `Unparsed`, the text of a
//! logical line as `import::logical` joined it. Each such rule is marked
//! `// quirk:` with the grammar rule or lowering line it mirrors; that list
//! is the inventory of the old reader, and phase 7 turns the wrong ones into
//! behaviour changes (`migration.xfail` lists what this cannot reproduce).
//!
//! This file is the spine: the per-deck state, the text of tokens as the
//! line stage joined them, the walk over statements and blocks, and the
//! cards that only place rows. `values` reads values, `elements` SPICE
//! element cards, `cards` SPICE dot cards, `named` Spectre and VACASK.
//!
//! Cites: `import/lower/mod.rs` (the lowering ported), `import/mod.rs`
//! `logical`; `grammar/spice.peg` `card`, `Raw`, `Unparsed`.
//!
//! Contents:
//! - `pub fn ast_to_ir(ast: &Ast, t: &Tokens, d: Dialect, name: &str) -> (CircuitIR, Report)`

use std::collections::{HashMap, HashSet};

use crate::codegen::Dialect;
use crate::import::Report;
use crate::ir::*;
use crate::syntax::ast::{Ast, NONE, Node, Tag};
use crate::syntax::lexicon::Lexicon;
use crate::syntax::token::{Tag as T, Tokens};

mod cards;
mod elements;
mod named;
mod testbench;
mod values;

/// Builds the `CircuitIR` named `name` and the `Report` that
/// `import::parse(t.source(), name, d)` returns, from `ast`, the parse of
/// `t`, read in dialect `d` (`syntax::lexicon::of(d)`).
///
/// # Panics
/// When `d` has no lexicon (CDL, Verilog).
pub fn ast_to_ir(ast: &Ast, t: &Tokens, d: Dialect, name: &str) -> (CircuitIR, Report) {
    let d = d.family();
    let lex = crate::syntax::lexicon::of(d).expect("ast_to_ir: a dialect with a reader");
    let mut cx = Cx {
        ast,
        t,
        d,
        lex,
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
    // LTspice inductors carry an implicit Rser=1m (lower/mod.rs:118-125).
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

// ---- private ----

/// The per-deck state, as `lower::Cx`.
struct Cx<'a> {
    ast: &'a Ast<'a>,
    t: &'a Tokens<'a>,
    d: Dialect,
    lex: &'static Lexicon,
    /// The deck's `.model` names, lowercased (lower/mod.rs:82-86).
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

/// Where a card sits (lower/mod.rs:66-73).
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

/// spice.peg `rawkind`: the dot words whose card, when no rule reads it,
/// is `Raw`; any other dot card no rule reads is `Unparsed`.
const RAWKIND: &[&str] = &[
    "acphasenoise",
    "osdi_include",
    "appendmodel",
    "temperature",
    "montecarlo",
    "phasenoise",
    "tran_noise",
    "trannoise",
    "unprotect",
    "envelope",
    "ptdnoise",
    "pre_osdi",
    "acmatch",
    "dcmatch",
    "hbnoise",
    "snnoise",
    "measure",
    "biaschk",
    "stimuli",
    "connect",
    "protect",
    "options",
    "nodeset",
    "include",
    "verilog",
    "dcsens",
    "pnoise",
    "jitter",
    "sample",
    "unprot",
    "option",
    "dcvolt",
    "global",
    "dcinc",
    "disto",
    "envlp",
    "hbosc",
    "hblin",
    "matex",
    "noise",
    "snosc",
    "check",
    "power",
    "store",
    "mosra",
    "print",
    "probe",
    "graph",
    "width",
    "title",
    "model",
    "acxf",
    "dcxf",
    "four",
    "hbac",
    "hbxf",
    "lstb",
    "qpss",
    "sens",
    "snac",
    "snxf",
    "tran",
    "meas",
    "dout",
    "stim",
    "plot",
    "prot",
    "step",
    "endc",
    "opts",
    "temp",
    "save",
    "fft",
    "lin",
    "net",
    "pac",
    "pss",
    "pxf",
    "stb",
    "pat",
    "opt",
    "hdl",
    "ac",
    "dc",
    "hb",
    "mc",
    "op",
    "pz",
    "sn",
    "sp",
    "tf",
    "ic",
];

impl<'a> Cx<'a> {
    fn named(&self) -> bool {
        matches!(self.d, Dialect::Spectre | Dialect::Vacask)
    }

    fn tag(&self, n: u32) -> Tag {
        self.ast.tag(Node(n))
    }

    fn main(&self, n: u32) -> u32 {
        self.ast.main_token(Node(n))
    }

    fn fields(&self, n: u32) -> &'a [u32] {
        self.ast.fields(Node(n))
    }

    fn body(&self, n: u32) -> &'a [u32] {
        self.ast.body(Node(n))
    }

    fn ttag(&self, k: u32) -> T {
        self.t.tags()[k as usize]
    }

    /// The last token of the statement starting at token `first`.
    fn line_last(&self, first: u32) -> u32 {
        let tags = self.t.tags();
        let eos = (first as usize..tags.len())
            .find(|&k| tags[k] == T::EndOfStatement)
            .expect("a statement ends");
        eos as u32 - 1
    }

    /// A flat statement's (or a block head's) logical-line text.
    fn line(&self, n: u32) -> String {
        let first = self.main(n);
        self.span(first, self.line_last(first))
    }

    /// The last token of item or expression `n`.
    fn last(&self, n: u32) -> u32 {
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

    fn last_of_key(&self, n: Node) -> u32 {
        // An `Assign` with no value ends at its `=`, the token after its key.
        self.last(self.ast.lhs(n)) + 1
    }

    /// The text of item `n` as the line stage joined it.
    fn text(&self, n: u32) -> String {
        self.span(self.main(n), self.last(n))
    }

    /// Tokens `a..=b` as `import::logical` joined them: the source inside
    /// one physical piece, one blank between pieces (a continuation, a
    /// comment), nothing across an HSPICE `\\` glue; VACASK glues a bracket
    /// to what precedes it and puts a blank after it.
    fn span(&self, a: u32, b: u32) -> String {
        let mut out = String::new();
        for k in a..=b {
            if k > a {
                out.push_str(&self.gap(k - 1, k));
            }
            out.push_str(self.t.text(k));
        }
        out
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

    /// Each statement of `stmts`, in order.
    fn cards(&mut self, stmts: &[u32], here: &mut Subcircuit, scope: Scope) {
        for &s in stmts {
            self.card(s, here, scope);
        }
    }

    /// Every line of statement `n`, a block's head, body and closer
    /// included: a `Block` kept whole.
    fn lines(&self, n: u32) -> Vec<String> {
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
            if self.ast.rhs(Node(s)) == NONE || self.tag(s) == Tag::Instance {
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

    /// A card kept as text (lower/mod.rs `raw`): a block one raw line per
    /// line, a SPICE element in its place among the components.
    fn raw(&mut self, n: u32, here: &mut Subcircuit, block: bool) {
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
    fn unparsed(&mut self, n: u32, here: &mut Subcircuit) {
        let line = self.line(n);
        self.report.unparsed.push(line.clone());
        here.raw_spice.push(line);
    }

    fn keep(&mut self, n: u32, here: &mut Subcircuit, k: Kept) {
        match k {
            Kept::Raw => self.raw(n, here, false),
            Kept::Unparsed => self.unparsed(n, here),
        }
    }

    /// quirk: spice.peg `Raw <- '.' rawkind !namechar rest / ends rest`: a
    /// dot card no rule reads is `Raw` when its word is a `rawkind` (or
    /// `.ends`/`.eom`), else `Unparsed` (and so are non-dot lines).
    fn refused(&self, n: u32) -> Kept {
        let word = self.t.text(self.main(n)).to_ascii_lowercase();
        let Some(w) = word.strip_prefix('.') else {
            return Kept::Unparsed;
        };
        if w == "ends" || w == "eom" || self.dialect_raw(w) || RAWKIND.contains(&w) {
            Kept::Raw
        } else {
            Kept::Unparsed
        }
    }

    /// quirk: the overlays' own raw dot words: ltspice.peg `Raw_lt`,
    /// xyce.peg `Raw_xy`.
    fn dialect_raw(&self, w: &str) -> bool {
        const LT: &[&str] = &[
            "endmachine",
            "savestate",
            "loadstate",
            "softstart",
            "backanno",
            "savebias",
            "loadbias",
            "machine",
            "fourier",
            "output",
            "ferret",
            "state",
            "wave",
            "rule",
            "endl",
            "fra",
        ];
        const XY: &[&str] = &[
            "embeddedsampling",
            "global_param",
            "temperature",
            "preprocess",
            "objective",
            "sampling",
            "initcond",
            "result",
            "mpde",
            "dcop",
            "temp",
            "save",
            "sens",
            "pce",
            "mor",
            "rol",
        ];
        match self.d {
            Dialect::Ltspice => LT.contains(&w),
            Dialect::Xyce => XY.contains(&w),
            _ => false,
        }
    }

    /// One card, as `lower::Cx::card`.
    fn card(&mut self, n: u32, here: &mut Subcircuit, scope: Scope) {
        if self.named() {
            return self.named_card_spine(n, here, scope);
        }
        let inside = scope == Scope::Subckt;
        if scope == Scope::Section && self.leaks(n) {
            // A `.control` kept raw is one line holding its newlines
            // (quirk: lower/mod.rs `raw` splits only `Block`-like rows).
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
                // quirk: a `.subckt` head spice.peg does not read is
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
                // quirk: spice.peg `ifblock`: an `.if` with no condition in
                // parentheses is a `Block`, kept whole.
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

    /// Whether a card in a section would reach past it (lower/sections.rs
    /// `leaks`).
    fn leaks(&self, n: u32) -> bool {
        match self.tag(n) {
            Tag::Global | Tag::Control => true,
            Tag::Save => self.ttag(self.main(n)) == T::KwSave,
            _ => false,
        }
    }

    /// `.lib name` … `.endl`: its cards lowered as at the top, into the
    /// section's own body and definitions (lower/sections.rs `section`).
    fn section(&mut self, n: u32, library: Option<String>) -> LibSection {
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

    /// quirk: spice.peg `Branch <- '.if' s? Cond s? nl`: the condition is
    /// one parenthesized group ending the line.
    fn if_head_ok(&self, n: u32) -> bool {
        let first = self.main(n);
        let last = self.line_last(first);
        first < last
            && self.ttag(first + 1) == T::LParen
            && self.paren_close(first + 1) == Some(last)
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

    /// A `.if` chain → a `Conditional` (lower/conditionals.rs).
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

    /// `.subckt` … `.ends` (lower/mod.rs `subckt`): nested definitions
    /// hoisted as `<parent>__<name>`, the parent's references renamed.
    fn subckt(&mut self, n: u32, head: SubcktHead) -> Subcircuit {
        let mut sc = sub(&head.name);
        sc.ports = head.ports;
        sc.parameters = head.params;
        let first = self.defs.len();
        let mut renames: Vec<(String, String)> = Vec::new();
        for &k in self.body(n) {
            if self.tag(k) == Tag::Subckt
                && let Some(h) = self.subckt_head(k)
            {
                let mut inner = self.subckt(k, h);
                let scoped = format!("{}__{}", sc.name, inner.name);
                renames.push((std::mem::replace(&mut inner.name, scoped.clone()), scoped));
                self.defs.push(inner);
            } else {
                self.card(k, &mut sc, Scope::Subckt);
            }
        }
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
}

/// A `.subckt` head the grammar read: its name, ports and defaults.
struct SubcktHead {
    name: String,
    ports: Vec<Port>,
    params: Vec<ParamDef>,
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
/// (lower/conditionals.rs `unwrapped`).
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
