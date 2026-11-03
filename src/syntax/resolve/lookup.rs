//! One file's tree and the kit's declarations → the file's answers: each
//! statement's scope, each instance master's binding, the corrected splits
//! of range-count and R/C/L cards, and the `R` rows for what does not
//! resolve. Zig's `localVarRef` (`compiler-architecture-reference.md`
//! §5.3): the nearest declaration wins, a tie between kinds is ambiguous,
//! nothing found is undeclared.
//!
//! The nearest declaration: one in an enclosing subcircuit of the same file
//! (innermost first), then one at that file's top (its sections and `if`
//! branches included), then one at another file's top, in read order. An
//! exact name beats a bin's base name (`nch` names `nch.1` only when no
//! `nch` is declared as near).
//!
//! Cites: ngspice manual §2.4.3 (`X`: the subcircuit named), §2.5 (a
//! subcircuit's own definitions are local), ch.3 (`RXXXXXXX n+ n- <value>
//! <mname>`), ch.8 (`QXXXXXXX nc nb ne <ns> <tj> mname <area>`), ch.11
//! (`MXXXXXXX nd ng ns nb mname`; HSPICE's optional bulk, BSIMSOI's up to
//! 7 nodes), §2.4 (binned models `name.N`); Spectre UG p.66 (`name (nodes)
//! master`, master a model, a subcircuit or a primitive), p.108 (model
//! groups).
//!
//! Contents:
//! - `pub(super) struct Table<'k>`, `Table::new(defs: &'k [Def], fold: bool) -> Table<'k>`,
//!   `Table::section(&self, file: FileId, name: &str) -> Option<Node>`
//! - `pub(super) fn lookup(table: &Table, file: FileId, t: &Tokens, ast: &Ast) -> (Facts, Vec<Diag>)`

use std::collections::HashMap;

use super::declare::word_text;
use super::{Binding, Def, DefId, DefKind, Facts, FileId, Split, key, walk};
use crate::syntax::ast::{Ast, NONE, Node, Tag};
use crate::syntax::device::{Model, Primitive, Terminals};
use crate::syntax::diag::{Code, Diag};
use crate::syntax::token::Tokens;

/// The kit's declarations, indexed by lookup key.
pub(super) struct Table<'k> {
    defs: &'k [Def],
    /// Key → the declarations named so (a bin also under its base), in
    /// read order.
    index: HashMap<String, Vec<DefId>>,
    fold: bool,
}

/// Where a name is looked up from: a file and the subcircuits around the
/// statement, outermost first.
struct Site<'a, 'k> {
    table: &'a Table<'k>,
    file: FileId,
    chain: &'a [u32],
}

/// A lookup's result.
enum Found {
    None,
    One(DefId),
    /// Declarations of different kinds equally near: the first.
    Ambiguous(DefId),
}

/// Which declarations a lookup wants.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    /// A subcircuit or a model: what an instance names.
    Master,
    /// A model: what an element's model word names.
    Model,
    /// A parameter or a function: a master that names one is wrong.
    Value,
}

impl<'k> Table<'k> {
    /// Indexes `defs` (names compared case-blind where `fold`).
    pub(super) fn new(defs: &'k [Def], fold: bool) -> Table<'k> {
        let mut index: HashMap<String, Vec<DefId>> = HashMap::new();
        for (k, d) in defs.iter().enumerate() {
            let id = DefId(u32::try_from(k).expect("fewer than u32::MAX declarations"));
            if d.kind == DefKind::Section {
                continue;
            }
            index.entry(key(&d.name, fold)).or_default().push(id);
            if d.base() != d.name {
                index.entry(key(d.base(), fold)).or_default().push(id);
            }
        }
        Table { defs, index, fold }
    }

    /// The section named `name` at `file`'s top (or in a library there).
    pub(super) fn section(&self, file: FileId, name: &str) -> Option<Node> {
        let want = key(name, self.fold);
        self.defs
            .iter()
            .find(|d| {
                d.file == file && d.kind == DefKind::Section && key(&d.name, self.fold) == want
            })
            .map(|d| d.node)
    }
}

/// Resolves file `file` (`ast` read from `t`) against `table`: its facts
/// (targets left empty) and its `R` rows.
pub(super) fn lookup(table: &Table, file: FileId, t: &Tokens, ast: &Ast) -> (Facts, Vec<Diag>) {
    let mut facts = Facts {
        nodes: ast.len(),
        ..Facts::default()
    };
    let mut diags = Vec::new();
    walk(ast, |n, scope, chain| {
        facts.scopes.push((n, scope));
        if ast.tag(Node(n)) != Tag::Instance {
            return;
        }
        let at = Site { table, file, chain };
        if let Some((b, split, code)) = at.instance(t, ast, Node(n)) {
            facts.bindings.push((n, b));
            facts.splits.extend(split.map(|s| (n, s)));
            diags.extend(code);
        }
    });
    facts.scopes.sort_unstable_by_key(|e| e.0);
    facts.bindings.sort_unstable_by_key(|e| e.0);
    facts.splits.sort_unstable_by_key(|e| e.0);
    (facts, diags)
}

// ---- private ----

impl Site<'_, '_> {
    /// Instance `n`'s binding, its corrected split, and its `R` row.
    fn instance(
        &self,
        t: &Tokens,
        ast: &Ast,
        n: Node,
    ) -> Option<(Binding, Option<Split>, Option<Diag>)> {
        let h = ast.header(n);
        let lex = t.lexicon_at(ast.main_token(n));
        if lex.named {
            if let Some(p) = Primitive::from_u32(h.code) {
                return Some((Binding::Primitive(p), None, None));
            }
            let (b, d) = self.master(ast, h.master, h.nodes.len(), n);
            return Some((b, None, d));
        }
        let name = word_text(ast, h.name)?;
        let row = name.get(..1).and_then(|l| lex.device(l))?;
        let items = items(ast, n);
        match (row.model, row.terminals) {
            (Model::None, _) => Some((Binding::Primitive(row.primitive), None, None)),
            (Model::InArgs, _) => {
                // A positional word after the nodes that names a model.
                let k = h.nodes.len();
                let model = items[k.min(items.len())..].iter().find_map(|&w| {
                    let id = self.model_word(ast, w)?;
                    Some((w, id))
                });
                Some(match model {
                    Some((w, id)) => (
                        Binding::Def(id),
                        Some(Split {
                            nodes: k as u32,
                            master: Node(w),
                        }),
                        None,
                    ),
                    None => (Binding::Primitive(row.primitive), None, None),
                })
            }
            (Model::AfterNodes, Terminals::Range(lo, hi)) => {
                let parsed = h.nodes.len();
                if h.master != NONE
                    && let Found::One(id) | Found::Ambiguous(id) =
                        self.find(word_text(ast, h.master), Want::Master)
                {
                    let (b, d) = self.bound(id, false, ast, h.master, parsed, n);
                    return Some((b, None, d));
                }
                // The parser read the longest node list; a shorter one whose
                // next word names a model is the card's split.
                let leading = items
                    .iter()
                    .take_while(|&&k| ast.tag(Node(k)) == Tag::Word)
                    .count();
                let most = usize::from(hi).min(leading.saturating_sub(1));
                let split = (usize::from(lo)..=most).rev().find_map(|k| {
                    let w = items[k];
                    (k != parsed).then_some(())?;
                    Some((k, w, self.model_word(ast, w)?))
                });
                Some(match split {
                    Some((k, w, id)) => (
                        Binding::Def(id),
                        Some(Split {
                            nodes: k as u32,
                            master: Node(w),
                        }),
                        None,
                    ),
                    None => {
                        let (b, d) = self.master(ast, h.master, parsed, n);
                        (b, None, d)
                    }
                })
            }
            (Model::AfterNodes | Model::Subckt, _) => {
                let (b, d) = self.master(ast, h.master, h.nodes.len(), n);
                Some((b, None, d))
            }
        }
    }

    /// Master word `m` (`NONE`: the card names none) of instance `n`, with
    /// `nodes` nodes → its binding and `R` row.
    fn master(&self, ast: &Ast, m: u32, nodes: usize, n: Node) -> (Binding, Option<Diag>) {
        let anchor = |m: u32| match m {
            NONE => ast.main_token(n),
            m => ast.main_token(Node(m)),
        };
        let name = word_text(ast, m);
        match self.find(name, Want::Master) {
            Found::One(id) => self.bound(id, false, ast, m, nodes, n),
            Found::Ambiguous(id) => self.bound(id, true, ast, m, nodes, n),
            Found::None => match self.find(name, Want::Value) {
                Found::One(id) | Found::Ambiguous(id) => (
                    Binding::Def(id),
                    Some(Diag {
                        code: Code::R0005,
                        token: anchor(m),
                    }),
                ),
                Found::None => (
                    Binding::Unknown,
                    Some(Diag {
                        code: Code::R0001,
                        token: anchor(m),
                    }),
                ),
            },
        }
    }

    /// A master bound to `id` (`ambiguous`: with an `R0003`), checked
    /// against a subcircuit's port count.
    fn bound(
        &self,
        id: DefId,
        ambiguous: bool,
        ast: &Ast,
        m: u32,
        nodes: usize,
        n: Node,
    ) -> (Binding, Option<Diag>) {
        let token = match m {
            NONE => ast.main_token(n),
            m => ast.main_token(Node(m)),
        };
        let code = match &self.table.defs[id.0 as usize].kind {
            _ if ambiguous => Some(Code::R0003),
            DefKind::Subckt { ports, .. } if ports.len() != nodes => Some(Code::R0004),
            // else: a model, or a subckt whose ports the nodes match.
            _ => None,
        };
        (Binding::Def(id), code.map(|code| Diag { code, token }))
    }

    /// The model word item `w` names, if it is a word naming one.
    fn model_word(&self, ast: &Ast, w: u32) -> Option<DefId> {
        match self.find(word_text(ast, w), Want::Model) {
            Found::One(id) | Found::Ambiguous(id) => Some(id),
            Found::None => None,
        }
    }

    /// The nearest declaration named `name` that `want` accepts.
    fn find(&self, name: Option<&str>, want: Want) -> Found {
        let Some(name) = name else {
            return Found::None;
        };
        let Some(ids) = self.table.index.get(&key(name, self.table.fold)) else {
            return Found::None;
        };
        let defs = self.table.defs;
        let exact = |d: &Def| key(&d.name, self.table.fold) == key(name, self.table.fold);
        let mut best: Option<(usize, bool, DefId)> = None;
        let mut ambiguous = false;
        for &id in ids {
            let d = &defs[id.0 as usize];
            let accepts = match (&d.kind, want) {
                (DefKind::Subckt { .. }, Want::Master)
                | (DefKind::Model { .. }, Want::Master | Want::Model)
                | (DefKind::Param | DefKind::Func, Want::Value) => true,
                // else: the kind is not one `want` takes.
                _ => false,
            };
            let Some(rank) = self.rank(d).filter(|_| accepts) else {
                continue;
            };
            // Nearer first, then an exact name before a bin's base.
            let this = (rank, !exact(d), id);
            match best {
                Some(b) if (b.0, b.1) < (this.0, this.1) => {}
                Some(b) if (b.0, b.1) == (this.0, this.1) => {
                    ambiguous |= class(&defs[b.2.0 as usize].kind) != class(&d.kind);
                }
                _ => {
                    best = Some(this);
                    ambiguous = false;
                }
            }
        }
        match best {
            None => Found::None,
            Some((_, _, id)) if ambiguous => Found::Ambiguous(id),
            Some((_, _, id)) => Found::One(id),
        }
    }

    /// How near `d` is from here (0 nearest); `None` when it is out of
    /// sight (inside a subcircuit that does not enclose this statement).
    fn rank(&self, d: &Def) -> Option<usize> {
        let depth = self.chain.len();
        if d.file != self.file {
            return (d.within == NONE).then_some(depth + 1);
        }
        if d.within == NONE {
            return Some(depth);
        }
        let at = self.chain.iter().position(|&s| s == d.within)?;
        Some(depth - 1 - at)
    }
}

/// A declaration kind's class for ambiguity: devices of different classes
/// are what a master cannot tell apart.
fn class(k: &DefKind) -> u8 {
    match k {
        DefKind::Subckt { .. } => 0,
        DefKind::Model { .. } => 1,
        DefKind::Param | DefKind::Func | DefKind::Section => 2,
    }
}

/// An element card's items after its name, in source order: its nodes,
/// fields and master merged back.
fn items(ast: &Ast, n: Node) -> Vec<u32> {
    let h = ast.header(n);
    let mut items: Vec<u32> = h.nodes.iter().chain(ast.fields(n)).copied().collect();
    if h.master != NONE {
        items.push(h.master);
    }
    items.sort_unstable_by_key(|&k| ast.main_token(Node(k)));
    items
}
