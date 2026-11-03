//! Conditional blocks -> [`Conditional`]s: ngspice and HSPICE `.if` ...
//! `.endif`, Spectre's structural `if (c) { } else ...`, VACASK `@if` ...
//! `@end`; each branch's cards lowered as where the block sits.
//!
//! Cites: ngspice manual §2.15; Spectre Reference 19.1 p.491; VACASK
//! lib/dflparser.y P462-P509.
//!
//! Contents:
//! - `Cx::conditional(&mut self, card: usize, scope: Scope) -> Conditional`

use super::{Cx, Scope};
use crate::import::Tag;
use crate::ir::*;

impl Cx<'_> {
    pub(super) fn conditional(&mut self, card: usize, scope: Scope) -> Conditional {
        let mut c = Conditional::default();
        for b in self.all(card, Tag::Branch) {
            let mut body = Subcircuit::default();
            let mut condition = None;
            for k in self.kids(b) {
                match self.tag(k) {
                    Tag::Cond => condition = Some(unwrapped(self.text(k))),
                    _ => self.card(k, &mut body, scope),
                }
            }
            c.branches.push(Branch { condition, body });
        }
        c
    }
}

// ---- private ----

/// A condition without the parentheses around it, blanks and line breaks
/// folded (a Spectre condition may span lines).
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
