//! Library sections -> [`LibSection`]s: a SPICE `.lib name` ... `.endl`, a
//! Spectre `library` of sections or a bare `section`, a VACASK `section`.
//! A section's cards lower as at the top of a deck, into the section's own
//! body and definitions; a card that would reach the whole deck stays raw
//! in it.
//!
//! Cites: ngspice manual §2.7 (`.lib`); Spectre Reference 19.1 p.498
//! (`library`, `section`); VACASK lib/dfllexer.l L536-L557.
//!
//! Contents:
//! - `Cx::library(&mut self, card: usize, here: &mut Subcircuit)`
//! - `Cx::section(&mut self, card: usize, library: Option<String>) -> LibSection`
//! - `Cx::leaks(&self, card: usize) -> bool`

use super::{Cx, Scope, sub};
use crate::import::Tag;
use crate::ir::*;

impl Cx<'_> {
    /// A Spectre `library`: each section in it, the library named; any other
    /// row (an `Unparsed` line) onto `here`.
    pub(super) fn library(&mut self, card: usize, here: &mut Subcircuit) {
        let lib = self.name(self.kids(card)[0]);
        for k in self.kids(card).into_iter().skip(1) {
            match self.tag(k) {
                Tag::LibDef => {
                    let s = self.section(k, Some(lib.clone()));
                    self.sections.push(s);
                }
                _ => self.card(k, here, Scope::Top),
            }
        }
    }

    /// A section definition: its cards lowered as at the top of a deck,
    /// into the section's own body and definitions.
    pub(super) fn section(&mut self, card: usize, library: Option<String>) -> LibSection {
        let kids = self.kids(card);
        assert_eq!(
            self.tag(kids[0]),
            Tag::Name,
            "a section's first row is its name"
        );
        let mut body = sub(&self.name(kids[0]));
        let (first, first_f) = (self.defs.len(), self.functions.len());
        for &k in &kids[1..] {
            self.card(k, &mut body, Scope::Section);
        }
        let functions = self.functions.split_off(first_f);
        let subcircuit_defs = self.defs.split_off(first);
        assert_eq!(
            self.defs.len(),
            first,
            "a section's definitions are its own"
        );
        LibSection {
            library,
            body,
            subcircuit_defs,
            functions,
        }
    }

    /// Whether a card in a section would reach past it: a testbench card or
    /// a `.global`. (A function stays the section's: `LibSection::functions`.)
    pub(super) fn leaks(&self, card: usize) -> bool {
        matches!(
            self.tag(card),
            Tag::Global
                | Tag::Control
                | Tag::Save
                | Tag::SpiceRegion
                | Tag::ControlBlock
                | Tag::Ctl
                | Tag::Ana
        )
    }
}
