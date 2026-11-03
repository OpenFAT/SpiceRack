//! A body's statements → the IR, the blocks inside it walked on one heap
//! stack: `.subckt`s (nested ones hoisted as `<parent>__<name>`, the
//! parent's references renamed), `.if` chains and Spectre `if`/`else`,
//! VACASK `@if`, blocks whose head the grammar refused (dissolved where they
//! sit) and Spectre libraries. [`Cx::card`] hands each such block back
//! ([`Opened`]) instead of walking it, so a deck nested 10⁴ deep in any mix
//! of them needs no deeper native stack than a flat one (rule 7 of the
//! compiler-construction skill). The IR is the old lowering's, statement for
//! statement: each block's own walk ran in this order.
//!
//! Cites: the old lowering's `cards`, `subckt`, `conditional` and its
//! named `conditional`, `dissolve` and the `library` arm (ported); ngspice
//! manual §2.5 (`.subckt` nests), §2.15 (`.if`); Spectre Reference 19.1
//! p.491 (`if`), p.498 (`library`).
//!
//! Contents:
//! - `pub(super) enum Opened`
//! - `Cx::cards(&mut self, stmts: &[u32], here: &mut Subcircuit, scope: Scope)`

use super::{Cx, Scope, SubcktHead, sub, unwrapped};
use crate::ir::*;
use crate::syntax::ast::{Node, Tag};

/// A block [`Cx::card`] leaves to [`Cx::cards`]'s stack.
pub(super) enum Opened {
    /// `.subckt` … `.ends` with the head the grammar read.
    Subckt(u32, SubcktHead),
    /// An `.if` chain (`named`: a Spectre or VACASK one).
    If { node: u32, named: bool },
    /// A block whose head the grammar refused (its head already
    /// `Unparsed`): its body read as cards where it sits, its closer
    /// refused.
    Dissolve(u32),
    /// A Spectre `library`: its sections, and its other statements as at
    /// the top.
    Library(u32),
}

/// An open block and the body it builds.
struct Frame {
    body: Subcircuit,
    /// The work left, the next item last.
    pending: Vec<Work>,
    scope: Scope,
    kind: Kind,
}

enum Kind {
    /// The body `cards` was called on.
    Root,
    Subckt {
        /// `self.defs.len()` when it opened: its hoisted definitions follow.
        first: usize,
        /// What its nested definitions renamed.
        renames: Vec<(String, String)>,
        /// Opened directly in a subckt's body (so hoisted under it).
        nested: bool,
    },
    If {
        /// The branch being read.
        branch: Node,
        /// The branches read so far.
        cond: Conditional,
        /// Where the chain sits among its parent's components.
        at: u32,
        named: bool,
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

impl<'a> Cx<'a> {
    /// Each statement of `stmts`, in order, into `here` (inside `scope`).
    pub(super) fn cards(&mut self, stmts: &[u32], here: &mut Subcircuit, scope: Scope) {
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
                if let Some(body) = self.close(done, &mut stack) {
                    assert!(stack.is_empty(), "the root closes last");
                    *here = body;
                    return;
                }
                continue;
            };
            let scope = top.scope;
            let direct = matches!(work, Work::Stmt(_));
            let opened = match work {
                Work::Stmt(s) | Work::Loose(s) => self.card(s, &mut top.body, scope),
                Work::Closer(c) => {
                    let k = self.refused(c);
                    self.keep(c, &mut top.body, k);
                    None
                }
                Work::Section(k, lib) => {
                    let s = self.section(k, Some(lib));
                    self.sections.push(s);
                    None
                }
            };
            if let Some(o) = opened {
                self.open(o, direct, &mut stack);
            }
        }
    }
}

// ---- private ----

impl<'a> Cx<'a> {
    /// Puts block `o`, opened by the top frame's statement (`direct`: one of
    /// its own body), on the stack.
    fn open(&mut self, o: Opened, direct: bool, stack: &mut Vec<Frame>) {
        let top = stack.last_mut().expect("a frame opened it");
        match o {
            Opened::Subckt(node, head) => {
                let mut sc = sub(&head.name);
                sc.ports = head.ports;
                sc.parameters = head.params;
                let nested = direct && matches!(top.kind, Kind::Subckt { .. });
                stack.push(Frame {
                    body: sc,
                    pending: self.work(node),
                    scope: Scope::Subckt,
                    kind: Kind::Subckt {
                        first: self.defs.len(),
                        renames: Vec::new(),
                        nested,
                    },
                });
            }
            Opened::If { node, named } => {
                let at = top.body.components.len() as u32;
                let scope = top.scope;
                stack.push(Frame {
                    body: Subcircuit::default(),
                    pending: self.work(node),
                    scope,
                    kind: Kind::If {
                        branch: Node(node),
                        cond: Conditional::default(),
                        at,
                        named,
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
    fn close(&mut self, done: Frame, stack: &mut Vec<Frame>) -> Option<Subcircuit> {
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
                let mut sc = self.close_subckt(body, first, &renames);
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
                self.defs.push(sc);
            }
            Kind::If {
                branch,
                mut cond,
                at,
                named,
            } => {
                let condition = self.condition(branch, named);
                cond.branches.push(Branch { condition, body });
                match self.ast.next(branch) {
                    Some(next) => stack.push(Frame {
                        body: Subcircuit::default(),
                        pending: self.work(next.0),
                        scope,
                        kind: Kind::If {
                            branch: next,
                            cond,
                            at,
                            named,
                        },
                    }),
                    None => {
                        let parent = stack.last_mut().expect("an if sits in a frame");
                        parent.body.conditionals.push(Conditional {
                            at: Some(at),
                            ..cond
                        });
                    }
                }
            }
        }
        None
    }

    /// The finished definition `sc`, its and its hoisted definitions
    /// (`self.defs[first..]`) references to the nested ones renamed.
    fn close_subckt(
        &mut self,
        mut sc: Subcircuit,
        first: usize,
        renames: &[(String, String)],
    ) -> Subcircuit {
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

    /// An `if` branch's condition: `None` for an `else`.
    fn condition(&self, b: Node, named: bool) -> Option<String> {
        if self.tag(b.0) != Tag::If {
            return None;
        }
        if named {
            return Some(unwrapped(&self.text(self.fields(b.0)[0])));
        }
        let first = self.main(b.0);
        Some(unwrapped(&self.span(first + 1, self.line_last(first))))
    }
}
