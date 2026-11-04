//! A body's statements → its [`Block`], the blocks inside it walked on one
//! heap stack: `.subckt`s (nested ones hoisted as `<parent>__<name>`, the
//! parent's references renamed in the name map), `.if` chains, blocks whose
//! head the grammar refused (dissolved where they sit). [`Rd::card`] hands
//! each such block back ([`Opened`]) instead of walking it, so a deck nested
//! 10⁴ deep needs no deeper native stack than a flat one (rule 7 of the
//! compiler-construction skill). Each block's statements land where the old
//! lowering put them, in its order.
//!
//! Cites: the old lowering's `cards`, `subckt`, `conditional`, `dissolve`;
//! ngspice manual §2.5
//! (`.subckt` nests), §2.15 (`.if`).
//!
//! Contents:
//! - `pub(super) enum Opened`
//! - `Rd::cards(&mut self, stmts: &[u32], here: &mut Block, scope: Scope, acc: &mut Acc)`
//! - `Rd::condition(&self, b: u32) -> Option<String>`

use super::{Acc, Block, Cond, Rd, Scope, unwrapped};
use crate::syntax::ast::{Node, Tag};

/// A block [`Rd::card`] leaves to [`Rd::cards`]'s stack.
pub(super) enum Opened {
    /// `.subckt` … `.ends` with a head the grammar read.
    Subckt(u32),
    /// An `.if` chain.
    If(u32),
    /// A block whose head the grammar refused (its head already kept as a
    /// line): its body read as cards where it sits, its closer refused.
    Dissolve(u32),
    /// A Spectre `library`: its sections, and its other statements as at
    /// the top.
    Library(u32),
}

/// An open block and the body it builds.
struct Frame {
    body: Block,
    /// The work left, the next item last.
    pending: Vec<Work>,
    scope: Scope,
    kind: Kind,
}

enum Kind {
    /// The body `cards` was called on.
    Root,
    Subckt {
        /// `acc.defs.len()` when it opened: its hoisted definitions follow.
        first: usize,
        /// What its nested definitions renamed.
        renames: Vec<(String, String)>,
        /// Opened directly in a subckt's body (so hoisted under it).
        nested: bool,
    },
    If {
        /// The branch being read.
        branch: u32,
        /// The branches read so far.
        cond: Cond,
        /// Where the chain sits among its parent's elements.
        at: u32,
    },
}

/// One step of a frame.
enum Work {
    /// A statement of the frame's own body.
    Stmt(u32),
    /// A statement of a block dissolved into the frame: a subckt it opens
    /// is not nested in the frame's (the old `dissolve` read it afresh).
    Loose(u32),
    /// A dissolved block's closer.
    Closer(u32),
    /// A library's section, with the library's name.
    Section(u32, String),
}

impl<'a> Rd<'a> {
    /// Each statement of `stmts`, in order, into `here` (inside `scope`).
    pub(super) fn cards(&mut self, stmts: &[u32], here: &mut Block, scope: Scope, acc: &mut Acc) {
        let mut stack = vec![Frame {
            body: std::mem::take(here),
            pending: stmts.iter().rev().map(|&s| Work::Stmt(s)).collect(),
            scope,
            kind: Kind::Root,
        }];
        loop {
            let top = stack.last_mut().expect("the root frame stays to the end");
            let Some(work) = top.pending.pop() else {
                let done = stack.pop().expect("the top frame");
                if let Some(body) = self.close(done, &mut stack, acc) {
                    assert!(stack.is_empty(), "the root closes last");
                    *here = body;
                    return;
                }
                continue;
            };
            let scope = top.scope;
            let direct = matches!(work, Work::Stmt(_));
            let opened = match work {
                Work::Stmt(s) | Work::Loose(s) => self.card(s, &mut top.body, scope, acc),
                Work::Closer(c) => {
                    let k = self.refused(c);
                    self.keep(c, &mut top.body, k);
                    None
                }
                Work::Section(k, lib) => {
                    let s = self.section(k, Some(lib), acc);
                    acc.sections.push(s);
                    None
                }
            };
            if let Some(o) = opened {
                self.open(o, direct, &mut stack, acc);
            }
        }
    }

    /// An `if` branch's condition: `None` for an `else`.
    pub(in crate::syntax::render) fn condition(&self, b: u32) -> Option<String> {
        if self.tag(b) != Tag::If {
            return None;
        }
        if self.named() {
            return Some(unwrapped(&self.text(self.fields(b)[0])));
        }
        let first = self.main(b);
        Some(unwrapped(&self.span(first + 1, self.line_last(first))))
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// Puts block `o`, opened by the top frame's statement (`direct`: one of
    /// its own body), on the stack.
    fn open(&mut self, o: Opened, direct: bool, stack: &mut Vec<Frame>, acc: &Acc) {
        let top = stack.last_mut().expect("a frame opened it");
        match o {
            Opened::Subckt(node) => {
                let name = self.name_of(self.fields(node)[0]);
                let nested = direct && matches!(top.kind, Kind::Subckt { .. });
                stack.push(Frame {
                    body: Block {
                        name,
                        node,
                        ..Block::default()
                    },
                    pending: self.work(node),
                    scope: Scope::Subckt,
                    kind: Kind::Subckt {
                        first: acc.defs.len(),
                        renames: Vec::new(),
                        nested,
                    },
                });
            }
            Opened::If(node) => {
                let at = top.body.comps.len() as u32;
                let scope = top.scope;
                stack.push(Frame {
                    body: Block::default(),
                    pending: self.work(node),
                    scope,
                    kind: Kind::If {
                        branch: node,
                        cond: Cond { branches: vec![] },
                        at,
                    },
                });
            }
            Opened::Dissolve(n) => {
                if let Some(c) = self.ast.closer(Node(n)) {
                    top.pending.push(Work::Closer(c.0));
                }
                let body = self.body(n);
                top.pending
                    .extend(body.iter().rev().map(|&s| Work::Loose(s)));
            }
            Opened::Library(n) => {
                let lib = self.name_of(self.fields(n)[0]);
                let body = self.body(n);
                top.pending
                    .extend(body.iter().rev().map(|&k| match self.tag(k) {
                        Tag::Section => Work::Section(k, lib.clone()),
                        _ => Work::Stmt(k),
                    }));
            }
        }
    }

    /// Block `n`'s body statements as work, the first last.
    fn work(&self, n: u32) -> Vec<Work> {
        self.body(n).iter().rev().map(|&s| Work::Stmt(s)).collect()
    }

    /// Finishes frame `done` (popped off `stack`) into its parent; the root's
    /// body is returned.
    fn close(&mut self, done: Frame, stack: &mut Vec<Frame>, acc: &mut Acc) -> Option<Block> {
        let Frame {
            body, scope, kind, ..
        } = done;
        match kind {
            Kind::Root => return Some(body),
            Kind::Subckt {
                first,
                renames,
                nested,
            } => {
                let mut sc = body;
                self.rename(&sc, &acc.defs[first..], &renames);
                if nested {
                    let Some(Frame {
                        kind: Kind::Subckt { renames, .. },
                        body: parent,
                        ..
                    }) = stack.last_mut()
                    else {
                        unreachable!("a nested subckt's parent frame is a subckt");
                    };
                    let scoped = format!("{}__{}", parent.name, sc.name);
                    renames.push((std::mem::replace(&mut sc.name, scoped.clone()), scoped));
                }
                acc.defs.push(sc);
            }
            Kind::If {
                branch,
                mut cond,
                at,
            } => {
                cond.branches.push((branch, body));
                match self.ast.next(Node(branch)) {
                    Some(next) => stack.push(Frame {
                        body: Block::default(),
                        pending: self.work(next.0),
                        scope,
                        kind: Kind::If {
                            branch: next.0,
                            cond,
                            at,
                        },
                    }),
                    None => {
                        let parent = stack.last_mut().expect("an if sits in a frame");
                        parent.body.conds.push((at, cond));
                    }
                }
            }
        }
        None
    }

    /// The name map after a subcircuit closes: its instances' and its
    /// hoisted definitions' masters that name a nested definition get its
    /// hoisted name (the first rename matching, case-blind).
    fn rename(&mut self, sc: &Block, hoisted: &[Block], renames: &[(String, String)]) {
        if renames.is_empty() {
            return;
        }
        let insts = sc.insts.iter().chain(hoisted.iter().flat_map(|d| &d.insts));
        for &(_, inst) in insts {
            let master = self.master_name(inst);
            if let Some((_, to)) = renames
                .iter()
                .find(|(from, _)| from.eq_ignore_ascii_case(&master))
            {
                self.masters.insert(inst, to.clone());
            }
        }
    }
}
