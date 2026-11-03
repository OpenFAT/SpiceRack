//! The files of a kit → a [`Kit`]: every file its entries reach through
//! includes, `.lib` calls, `include … section=` and Verilog-A sources, read in
//! breadth-first order, and a kit-wide symbol table with its answers in side
//! tables keyed by `(file, node)`. Resolve rewrites nothing: an AST read from
//! a kit file is the same tree [`crate::syntax::parse`] makes, and every
//! answer here names its nodes by id.
//!
//! Two passes, as Zig's AstGen does names (`compiler-architecture-reference.md`
//! §5.2, §5.3): `declare` pre-scans every file's declarations (subcircuits,
//! models and their bins, parameters, functions, sections) and its
//! references, then `lookup` re-reads each file and binds each instance's
//! master, records each statement's scope, corrects the parser's node and
//! model split where the kit's models say otherwise (a range terminal count,
//! a model among an R/C/L card's arguments), and points each include at the
//! file and section it reads. A file's role (deck or library) is read from
//! its statements. What resolve cannot answer keeps its name and gets an `R`
//! diagnostic ([`crate::syntax::diag::Code`]).
//!
//! Reading a file is the caller's job, through `load` (a path → its text):
//! the kit never touches a file system, so confining paths to a root is the
//! loader's rule, and a refused or absent file is an `R` row, never an
//! error.
//!
//! Cites: `docs/research/transpiler-architecture.md` §2.4, §2.5 (what the
//! renderer asks), §5 phase 3; `compiler-architecture-reference.md` §5,
//! §10.3, §10.5; ngspice manual §2.4.3 (`X`), §2.5 (`.subckt`), §2.6
//! (`.include`), §2.7 (`.lib`), §2.15 (`.if`); Spectre UG p.66 (instances),
//! p.94 (`subckt`), p.98 (`include`), p.108 (model groups), Spectre
//! Reference 19.1 p.498 (`library`, `section`).
//!
//! Contents:
//! - `pub struct Kit`, `pub struct FileId(pub u32)`, `pub struct DefId(pub u32)`
//! - `pub struct File`, `pub enum Form`, `pub enum Role`, `pub enum LoadError`
//! - `pub struct Def`, `pub enum DefKind`, `pub enum Polarity`, `pub enum Scope`
//! - `pub enum Binding`, `pub struct Split`, `pub struct Target`
//! - `pub const DEPTH_MAX: u8`, `pub const FILES_MAX: usize`
//! - `Kit::load(entries: &[PathBuf], form: Form, dialect: Dialect, load: &mut dyn FnMut(&Path) -> Result<String, LoadError>) -> Kit`
//! - `Kit::dialect(&self) -> Dialect`, `Kit::files(&self) -> &[File]`, `Kit::file(&self, id: FileId) -> &File`,
//!   `Kit::definitions(&self) -> &[Def]`, `Kit::definition(&self, id: DefId) -> &Def`,
//!   `Kit::binding(&self, file: FileId, n: Node) -> Option<Binding>`,
//!   `Kit::scope(&self, file: FileId, n: Node) -> Option<Scope>`,
//!   `Kit::split(&self, file: FileId, n: Node) -> Option<Split>`,
//!   `Kit::target(&self, file: FileId, n: Node) -> Option<Target>`,
//!   `Kit::node_count(&self, file: FileId) -> usize`
//! - `Def::base(&self) -> &str`
//! - `pub(super) fn walk(ast: &Ast, visit: impl FnMut(u32, Scope, &[u32]))`,
//!   `pub(super) fn key(name: &str, fold: bool) -> String`

use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};

use crate::codegen::Dialect;
use crate::syntax::ast::{Ast, NONE, Node, Tag};
use crate::syntax::device::Primitive;
use crate::syntax::diag::{Code, Diag};
use crate::syntax::{lexicon, parse, tokenize};

mod declare;
mod lookup;

/// The deepest include chain a kit follows (an entry is depth 0); a deeper
/// include is an `R0008` and not read. Interop's `spice::MAX_INCLUDE_DEPTH`.
pub const DEPTH_MAX: u8 = 16;

/// The most files a kit reads; one more is an `R0009` and not read.
/// Interop's `spice::MAX_FILES`.
pub const FILES_MAX: usize = 1 << 12;

/// A kit's files and its symbol table, built once by [`Kit::load`] and
/// read-only after.
///
/// Invariants: `files` and `facts` have one row per file, in read order;
/// every [`DefId`], [`FileId`] and node id an answer holds is in range; each
/// fact table is sorted by node.
pub struct Kit {
    dialect: Dialect,
    files: Vec<File>,
    defs: Vec<Def>,
    facts: Vec<Facts>,
}

/// A file's index in [`Kit::files`], its read order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

/// A declaration's index in [`Kit::definitions`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct DefId(pub u32);

/// One file the kit reached.
#[derive(Debug)]
pub struct File {
    /// The path, as the includer joined it to its directory (`.` and `..`
    /// resolved lexically); an entry's as given.
    pub path: PathBuf,
    pub role: Role,
    /// The text the kit parsed: the loaded text, behind one empty title
    /// line for a file read in [`Form::Library`] (so token and line numbers
    /// count that line); a Verilog-A source's text as loaded; empty for an
    /// unread file. `tokenize(&text, lexicon::of(kit.dialect()))` and
    /// `parse` rebuild the tree every answer's node ids name.
    pub text: String,
    /// The parse's `P` rows and resolve's `R` rows, sorted by token. An `R`
    /// row about an included file sits on the statement that names it; an
    /// unread entry's sits on its own row, at token 0.
    pub diags: Vec<Diag>,
}

/// How an entry is read: its line 1 is a title, or it has none. A file the
/// kit includes is always a library.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    Deck,
    Library,
}

/// What a file is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// It holds an analysis, a control block or a top-level independent
    /// source.
    Deck,
    /// It holds none of those: definitions only.
    Library,
    /// A Verilog-A source (`ahdl_include`, VACASK `load`): loaded, not read.
    VerilogA,
    /// The loader did not return it.
    Unread(LoadError),
}

/// Why the loader returned no text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoadError {
    /// No such file (`R0006`).
    NotFound,
    /// The loader refused the path, e.g. one outside its root (`R0007`).
    Refused,
}

/// One declaration.
#[derive(Clone, Debug)]
pub struct Def {
    /// The name as written.
    pub name: String,
    pub kind: DefKind,
    pub file: FileId,
    /// The declaring statement.
    pub node: Node,
    /// The innermost block it is declared in.
    pub scope: Scope,
    /// The nearest subcircuit enclosing it, `NONE` at a file's top: a
    /// declaration inside a subcircuit is seen only from inside it.
    within: u32,
}

/// What a declaration declares.
#[derive(Clone, PartialEq, Debug)]
pub enum DefKind {
    /// A subcircuit: its ports in order and its parameter names (SPICE:
    /// the head's `k=v`s; Spectre, VACASK: its own `parameters`), never the
    /// defaults.
    Subckt {
        ports: Vec<String>,
        params: Vec<String>,
    },
    /// A model, or one bin of one (`nch.1`; [`Def::base`] is `nch`).
    Model {
        /// The type or master word as written (`nmos`, `bsim4`,
        /// `sp_bsim4v8`).
        word: String,
        /// The primitive its instances are, when the dialect's tables know
        /// the word.
        kind: Option<Primitive>,
        /// Its `level=`, when an integer.
        level: Option<u32>,
        /// From the word (`nmos`, `pnp`) or `type=` (Spectre `p`, VACASK
        /// `-1`); a Spectre model group's from its first bin.
        polarity: Option<Polarity>,
    },
    /// A `.param` / `parameters` name.
    Param,
    /// A `.func` or Spectre `real` function.
    Func,
    /// A `.lib name` … `.endl` or Spectre/VACASK `section`.
    Section,
}

/// A device's polarity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Polarity {
    N,
    P,
}

/// The block a statement sits in: a file's top, or the innermost
/// subcircuit, section or `if` branch (`If` or `Else` node) around it.
/// Libraries, `simulator lang=` regions and the other bodies (`.control`,
/// model groups, `statistics`) are no scope of their own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Top,
    Subckt(Node),
    Section(Node),
    Branch(Node),
}

/// What an instance's master binds to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Binding {
    /// A primitive the card names by letter or master word (`R1 a b 1k`,
    /// `r1 (a b) resistor`).
    Primitive(Primitive),
    /// A declaration: a subcircuit or model (a parameter or function, with
    /// an `R0005`).
    Def(DefId),
    /// Nothing the kit declares (`R0001`): the master keeps its name.
    Unknown,
}

/// A corrected split of an element card's items (its nodes, fields and
/// master in source order, after its name): the first `nodes` items are its
/// nodes and `master` is the word naming its model; the other positional
/// items are values. Recorded only where the kit's models overrule the
/// parser's header: a range terminal count (`Q1 c b e mod 1.5`, where the
/// parser read `mod` as a node) and a model among an R/C/L card's arguments
/// (`R1 a b 1k rmod`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Split {
    pub nodes: u32,
    pub master: Node,
}

/// The file an include, `.lib` call or Verilog-A source reads, and for a
/// `.lib file section` or `include … section=` the `Section` node it
/// selects there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    pub file: FileId,
    pub section: Option<Node>,
}

/// One file's answers, each sorted by node.
#[derive(Default, Debug)]
struct Facts {
    /// The node count of the tree the answers were made on.
    nodes: usize,
    scopes: Vec<(u32, Scope)>,
    bindings: Vec<(u32, Binding)>,
    splits: Vec<(u32, Split)>,
    targets: Vec<(u32, Target)>,
}

/// A file the walk has yet to read.
struct Pending {
    path: PathBuf,
    /// The statement that names it: its file and token (`None` for an
    /// entry).
    from: Option<(FileId, u32)>,
    depth: u8,
    verilog_a: bool,
}

/// An open block of [`walk`].
struct Open<'a> {
    items: &'a [u32],
    at: usize,
    scope: Scope,
    /// The scope around the block.
    outer: Scope,
    /// Whether the block is an `if` branch, whose chain's next follows it.
    branch: bool,
    /// Whether the block is a subcircuit.
    subckt: bool,
    node: u32,
}

impl Kit {
    /// Reads `entries` (in `form`) and every file they reach, in `dialect`,
    /// and resolves them. `load` returns a path's text (decoded); a path it
    /// refuses or lacks is a [`Role::Unread`] row with an `R0006` or
    /// `R0007`. Files are read breadth first, each once (by path), relative
    /// to the including file's directory, to [`DEPTH_MAX`] and [`FILES_MAX`].
    ///
    /// Cost: two tokenize-and-parse passes per file; one file's tree is
    /// alive at a time.
    ///
    /// # Panics
    /// When `dialect` has no reader (CDL, Verilog).
    pub fn load(
        entries: &[PathBuf],
        form: Form,
        dialect: Dialect,
        load: &mut dyn FnMut(&Path) -> Result<String, LoadError>,
    ) -> Kit {
        let lex = lexicon::of(dialect).expect("Kit::load: a dialect with a reader");
        let mut kit = Kit {
            dialect,
            files: Vec::new(),
            defs: Vec::new(),
            facts: Vec::new(),
        };
        let mut refs: Vec<Vec<declare::Ref>> = Vec::new();
        let mut seen: HashMap<PathBuf, FileId> = HashMap::new();
        let mut todo: VecDeque<Pending> = entries
            .iter()
            .map(|e| Pending {
                path: normalize(e),
                from: None,
                depth: 0,
                verilog_a: false,
            })
            .collect();

        // Pass 1: read breadth first, each file's declarations and references.
        while let Some(p) = todo.pop_front() {
            if seen.contains_key(&p.path) {
                continue;
            }
            if kit.files.len() == FILES_MAX {
                kit.diag_from(p.from, Code::R0009);
                continue;
            }
            let id = FileId(kit.files.len() as u32);
            seen.insert(p.path.clone(), id);
            let mut file = File {
                path: p.path,
                role: Role::Library,
                text: String::new(),
                diags: Vec::new(),
            };
            let mut found = Vec::new();
            match load(&file.path) {
                Err(e) => {
                    file.role = Role::Unread(e);
                    if p.from.is_none() {
                        file.diags.push(Diag {
                            code: unread_code(e),
                            token: 0,
                        });
                    }
                }
                Ok(text) if p.verilog_a => {
                    file.role = Role::VerilogA;
                    file.text = text;
                }
                Ok(text) => {
                    file.text = match (form, p.from) {
                        (Form::Deck, None) => text,
                        _ => format!("\n{text}"),
                    };
                    let t = tokenize(&file.text, lex);
                    let ast = parse(&t, lex);
                    let d = declare::declare(id, &t, &ast, lex.fold_case);
                    file.role = d.role;
                    file.diags.extend(ast.diags());
                    file.diags.extend(d.diags);
                    kit.defs.extend(d.defs);
                    found = d.refs;
                }
            }
            let dir = file.path.parent().unwrap_or(Path::new("")).to_path_buf();
            kit.files.push(file);
            for r in &found {
                if p.depth >= DEPTH_MAX {
                    kit.files[id.0 as usize].diags.push(Diag {
                        code: Code::R0008,
                        token: r.token,
                    });
                    continue;
                }
                todo.push_back(Pending {
                    path: normalize(&dir.join(&r.path)),
                    from: Some((id, r.token)),
                    depth: p.depth + 1,
                    verilog_a: r.verilog_a,
                });
            }
            refs.push(found);
        }

        // Pass 2: bind each file's names against every file's declarations.
        let table = lookup::Table::new(&kit.defs, lex.fold_case);
        let unread: Vec<Option<LoadError>> = kit
            .files
            .iter()
            .map(|f| match f.role {
                Role::Unread(e) => Some(e),
                Role::Deck | Role::Library | Role::VerilogA => None,
            })
            .collect();
        let mut facts = Vec::with_capacity(kit.files.len());
        for (k, file) in kit.files.iter_mut().enumerate() {
            let id = FileId(k as u32);
            if matches!(file.role, Role::VerilogA | Role::Unread(_)) {
                facts.push(Facts::default());
                continue;
            }
            let t = tokenize(&file.text, lex);
            let ast = parse(&t, lex);
            let (mut f, diags) = lookup::lookup(&table, id, &t, &ast);
            file.diags.extend(diags);
            f.targets = targets(
                &refs[k],
                &file.path,
                &seen,
                &unread,
                &table,
                &mut file.diags,
            );
            facts.push(f);
        }
        drop(table);
        kit.facts = facts;
        for (file, f) in kit.files.iter_mut().zip(&kit.facts) {
            file.diags.sort_by_key(|d| d.token);
            debug_assert!(f.bindings.is_sorted_by_key(|b| b.0));
            debug_assert!(f.scopes.is_sorted_by_key(|s| s.0));
        }
        assert_eq!(kit.files.len(), kit.facts.len());
        kit
    }

    /// The dialect every file was read in.
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Every file the walk reached, in read order (unread ones included).
    pub fn files(&self) -> &[File] {
        &self.files
    }

    /// File `id`.
    ///
    /// # Panics
    /// When `id` is no file of this kit.
    pub fn file(&self, id: FileId) -> &File {
        &self.files[id.0 as usize]
    }

    /// Every declaration, in read order: by file, then in source order.
    pub fn definitions(&self) -> &[Def] {
        &self.defs
    }

    /// Declaration `id`.
    ///
    /// # Panics
    /// When `id` is no declaration of this kit.
    pub fn definition(&self, id: DefId) -> &Def {
        &self.defs[id.0 as usize]
    }

    /// What `Instance` node `n` of `file` binds to; `None` for a node that
    /// is no instance, or an element whose letter names no device.
    pub fn binding(&self, file: FileId, n: Node) -> Option<Binding> {
        find(&self.facts[file.0 as usize].bindings, n)
    }

    /// The scope of statement `n` of `file` (an `.elseif`/`.else` branch's
    /// head: the chain's); `None` for a node that is no statement.
    pub fn scope(&self, file: FileId, n: Node) -> Option<Scope> {
        find(&self.facts[file.0 as usize].scopes, n)
    }

    /// The corrected split of element `n` of `file`; `None` where the
    /// parser's header stands.
    pub fn split(&self, file: FileId, n: Node) -> Option<Split> {
        find(&self.facts[file.0 as usize].splits, n)
    }

    /// The file (and section) include, `.lib` call or Verilog-A load `n` of
    /// `file` reads; `None` when it reads no file of the kit.
    pub fn target(&self, file: FileId, n: Node) -> Option<Target> {
        find(&self.facts[file.0 as usize].targets, n)
    }

    /// The node count of the tree `file`'s answers were made on (0 for an
    /// unread file): a reader of `file.text` asserts its tree has as many.
    pub fn node_count(&self, file: FileId) -> usize {
        self.facts[file.0 as usize].nodes
    }
}

impl Def {
    /// The name less a bin suffix: `nch` for a model `nch.1`, else the name.
    pub fn base(&self) -> &str {
        match (&self.kind, self.name.rsplit_once('.')) {
            (DefKind::Model { .. }, Some((base, n)))
                if !base.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) =>
            {
                base
            }
            _ => &self.name,
        }
    }
}

/// Visits every statement of `ast` in source order (a block's head, then
/// its body, then a chain's next branch), with its [`Scope`] and the
/// subcircuits around it, outermost first. A heap stack, so a file nested
/// 10⁴ deep needs no deeper native stack than a flat one.
pub(super) fn walk(ast: &Ast, mut visit: impl FnMut(u32, Scope, &[u32])) {
    let mut stack = vec![Open {
        items: ast.fields(ast.root()),
        at: 0,
        scope: Scope::Top,
        outer: Scope::Top,
        branch: false,
        subckt: false,
        node: 0,
    }];
    let mut chain: Vec<u32> = Vec::new();
    while let Some(top) = stack.last_mut() {
        if let Some(&n) = top.items.get(top.at) {
            top.at += 1;
            let scope = top.scope;
            visit(n, scope, &chain);
            if let Some(open) = opened(ast, n, scope) {
                if open.subckt {
                    chain.push(n);
                }
                stack.push(open);
            }
            continue;
        }
        let done = stack.pop().expect("the top frame");
        if done.subckt {
            assert_eq!(
                chain.pop(),
                Some(done.node),
                "the chain ends in this subckt"
            );
        }
        if done.branch
            && let Some(next) = ast.next(Node(done.node))
        {
            visit(next.0, done.outer, &chain);
            stack.extend(opened(ast, next.0, done.outer));
        }
    }
    assert!(chain.is_empty(), "every subckt frame popped");
}

/// `name`'s lookup key: lowercased where the dialect folds case.
pub(super) fn key(name: &str, fold: bool) -> String {
    if fold {
        name.to_ascii_lowercase()
    } else {
        name.to_string()
    }
}

// ---- private ----

/// The body statement `n` opens, if any, and the scope inside it.
fn opened<'a>(ast: &'a Ast, n: u32, scope: Scope) -> Option<Open<'a>> {
    let node = Node(n);
    let tag = ast.tag(node);
    if !has_body(tag) || ast.rhs(node) == NONE {
        return None;
    }
    let inner = match tag {
        Tag::Subckt => Scope::Subckt(node),
        Tag::Section => Scope::Section(node),
        Tag::If | Tag::Else => Scope::Branch(node),
        // else: the other bodies are no scope of their own.
        _ => scope,
    };
    Some(Open {
        items: ast.body(node),
        at: 0,
        scope: inner,
        outer: scope,
        branch: matches!(tag, Tag::If | Tag::Else),
        subckt: tag == Tag::Subckt,
        node: n,
    })
}

/// Whether a `tag` statement's `rhs` is a body when it is not `NONE`.
fn has_body(tag: Tag) -> bool {
    match tag {
        Tag::Subckt
        | Tag::Section
        | Tag::Library
        | Tag::If
        | Tag::Else
        | Tag::Control
        | Tag::Region
        | Tag::Data
        | Tag::Func
        | Tag::Model
        | Tag::Analysis
        | Tag::Command
        | Tag::Statistics
        | Tag::Process
        | Tag::Card
        | Tag::Instance => true,
        Tag::Root
        | Tag::Title
        | Tag::End
        | Tag::Verbatim
        | Tag::Include
        | Tag::LibCall
        | Tag::Line
        | Tag::Param
        | Tag::Options
        | Tag::Save
        | Tag::Print
        | Tag::Measure
        | Tag::Ic
        | Tag::Nodeset
        | Tag::Temp
        | Tag::Global
        | Tag::Step
        | Tag::Word
        | Tag::Assign
        | Tag::Group
        | Tag::Seq
        | Tag::Delimited
        | Tag::Num
        | Tag::Ident
        | Tag::Str
        | Tag::Unary
        | Tag::Binary
        | Tag::Ternary
        | Tag::Call
        | Tag::Probe
        | Tag::Index
        | Tag::Paren
        | Tag::Vector
        | Tag::List
        | Tag::Bad => false,
    }
}

impl Kit {
    /// Appends `code` at the statement `from` names; nothing for an entry.
    fn diag_from(&mut self, from: Option<(FileId, u32)>, code: Code) {
        if let Some((f, token)) = from {
            self.files[f.0 as usize].diags.push(Diag { code, token });
        }
    }
}

fn unread_code(e: LoadError) -> Code {
    match e {
        LoadError::NotFound => Code::R0006,
        LoadError::Refused => Code::R0007,
    }
}

/// The answer for node `n` in a table sorted by node.
fn find<T: Copy>(table: &[(u32, T)], n: Node) -> Option<T> {
    table
        .binary_search_by_key(&n.0, |e| e.0)
        .ok()
        .map(|i| table[i].1)
}

/// Each reference of a file (`refs`, read from `path`) → the file it reads
/// and the section it selects; an unread file is an `R0006`/`R0007` at the
/// reference, a missing section an `R0010`.
fn targets(
    refs: &[declare::Ref],
    path: &Path,
    seen: &HashMap<PathBuf, FileId>,
    unread: &[Option<LoadError>],
    table: &lookup::Table,
    diags: &mut Vec<Diag>,
) -> Vec<(u32, Target)> {
    let dir = path.parent().unwrap_or(Path::new(""));
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        // A reference past a cap was never read: its `R0008`/`R0009` is there.
        let Some(&file) = seen.get(&normalize(&dir.join(&r.path))) else {
            continue;
        };
        if let Some(e) = unread[file.0 as usize] {
            diags.push(Diag {
                code: unread_code(e),
                token: r.token,
            });
            continue;
        }
        let section = match &r.section {
            None => None,
            Some(s) => match table.section(file, s) {
                Some(n) => Some(n),
                None => {
                    diags.push(Diag {
                        code: Code::R0010,
                        token: r.token,
                    });
                    None
                }
            },
        };
        out.push((r.node, Target { file, section }));
    }
    out.sort_by_key(|e| e.0);
    out
}

/// `p` with `.` and `..` resolved lexically.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests;
