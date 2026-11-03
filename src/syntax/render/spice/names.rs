//! The deck's elements → the name map of the SPICE family's letter policy
//! (`docs/research/transpiler-architecture.md` §2.5, "names are renamed as
//! a whole"): each element's written name (its type letter, then its name
//! as the reader keeps it: `r1` is `R1`; a Spectre `in1 (a 0) vsource` is
//! `Vin1`), built once per deck, and each declaration found from a
//! reference to it in its scope, so a reference is spelled as its
//! declaration is. The letter is added here and nowhere else.
//!
//! Where the old writer spelled a reference otherwise (as written, while its
//! declaration gained a letter), the writer keeps the old bytes beside a
//! `// quirk:` until phase 7 (`tests/fixtures/transpiler/PHASE4-STATUS.md`).
//!
//! Cites: ngspice manual §2.1.2 (the element letter), §3.2.12 (`K`),
//! §5.2-§5.3 (the controlling source of `F`, `H`); ngspice and HSPICE fold
//! name case, Spectre does not (Spectre UG p.66).
//!
//! Contents:
//! - `pub(super) struct Names`
//! - `Names::new(rd: &Rd, deck: &Deck) -> Names`, `Names::decl(&self, n: u32) -> &str`,
//!   `Names::reference(&self, scope: u32, name: &str, letter: char) -> Option<&str>`

use std::collections::HashMap;

use crate::syntax::ast::{NONE, Node};
use crate::syntax::render::read::{Block, Card, Comp, Deck, Rd};

/// The deck's element names, written.
pub(super) struct Names {
    /// Each element's written name, by its node.
    decl: HashMap<u32, String>,
    /// Each element's written name, by its scope (the subcircuit's node,
    /// `NONE` at the top) and its name as written (case folded in a SPICE
    /// deck).
    by_name: HashMap<(u32, String), String>,
    /// Names fold case (a SPICE deck).
    fold: bool,
}

impl Names {
    /// The map of every element `deck` writes, in every block.
    pub(super) fn new(rd: &Rd, deck: &Deck) -> Names {
        let mut names = Names {
            decl: HashMap::new(),
            by_name: HashMap::new(),
            fold: !rd.named(),
        };
        let tops = std::iter::once(&deck.top).chain(deck.sections.iter().map(|s| &s.body));
        let defs = deck
            .defs
            .iter()
            .chain(deck.sections.iter().flat_map(|s| &s.defs));
        let mut work: Vec<(&Block, u32)> = tops.map(|b| (b, NONE)).collect();
        work.extend(defs.map(|b| (b, b.node)));
        while let Some((b, scope)) = work.pop() {
            let cards = b.comps.iter().filter_map(|c| match c {
                Comp::Card(n) => Some(*n),
                Comp::Raw(_) => None,
            });
            for n in cards.chain(b.insts.iter().map(|&(_, n)| n)) {
                let written = written(rd.card_view(n));
                let source = rd.name_of(rd.ast.header(Node(n)).name);
                names
                    .by_name
                    .entry((scope, names.key(&source)))
                    .or_insert_with(|| written.clone());
                names.decl.insert(n, written);
            }
            let branches = b.conds.iter().flat_map(|(_, c)| &c.branches);
            work.extend(branches.map(|(_, body)| (body, scope)));
        }
        names
    }

    /// Element `n`'s written name.
    ///
    /// # Panics
    /// When `n` is no element the deck writes.
    pub(super) fn decl(&self, n: u32) -> &str {
        self.decl.get(&n).expect("an element the map holds")
    }

    /// The written name of the element `name` (as a reference wrote it)
    /// declares in `scope`, when one does with type letter `letter`.
    pub(super) fn reference(&self, scope: u32, name: &str, letter: char) -> Option<&str> {
        let w = self.by_name.get(&(scope, self.key(name)))?;
        w.starts_with(letter).then_some(w.as_str())
    }
}

// ---- private ----

impl Names {
    fn key(&self, name: &str) -> String {
        match self.fold {
            true => name.to_ascii_lowercase(),
            false => name.to_string(),
        }
    }
}

/// A card's written name: its type letter, then its name.
fn written(c: Card) -> String {
    let (letter, name) = match c {
        Card::Rcl { letter, name, .. }
        | Card::Source { letter, name, .. }
        | Card::Linear { letter, name, .. }
        | Card::Poly { letter, name, .. }
        | Card::Semi { letter, name, .. }
        | Card::Switch { letter, name, .. } => (letter, name),
        Card::Charge { name, .. } => ('C', name),
        Card::Coupling { name, .. } => ('K', name),
        Card::Behavioural { name, .. } => ('B', name),
        Card::Line { name, .. } => ('T', name),
        Card::Inst { name, .. } => ('X', name),
    };
    assert!(letter.is_ascii_uppercase(), "a type letter");
    format!("{letter}{name}")
}
