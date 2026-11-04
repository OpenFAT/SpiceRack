//! The deck → CDL in the xschem layout: the SPICE writer's CDL row. Each
//! block (the top, then each subcircuit) is one `.SUBCKT … .ENDS` around
//! its elements, each an ngspice card ([`W::card`]), sorted by element
//! name; then the deck's VACASK control text, a comment per Verilog-A load,
//! the top's models, `.END`. A schematic tool's LVS netlist: no analyses,
//! sections, branches or includes are written, each noted (`W0028`).
//!
//! The layout is the old writer's (`codegen::xschem::write` of an IR with
//! nothing beside it), byte for byte (TESTING.md, "Golden gate"); where it
//! differs from the SPICE spine it is marked `// quirk:`. An element the
//! ngspice card has no form for is a comment naming its `W` code, where the
//! old writer named its free-text error (no corpus deck has one), and a
//! note (`W0021`), the deck still written.
//!
//! Cites: Cadence CDL (`.SUBCKT`, `*.PININFO`, `.ENDS`, `.END`); the old
//! xschem layout's module notes (`codegen::xschem`).
//!
//! Contents:
//! - `W::cdl(&mut self)`

use super::W;
use crate::syntax::diag::{Code, Diag, Kind, info};
use crate::syntax::render::out::{Out, Space};
use crate::syntax::render::read::{Bench, Block, Card, Comp, Func};

impl W<'_, '_> {
    /// The deck, in the xschem layout.
    pub(super) fn cdl(&mut self) {
        let deck = self.deck;
        // quirk: the layout titles and ends every file, a library too, where
        // `role::frame` writes a library bare; phase 7 asks the frame.
        self.o.token(&format!("* {}", deck.top.name), Space::Line);
        self.cdl_block(&deck.top, true);
        let extra = self.fold().extra;
        for l in &extra {
            self.o.token(l, Space::Line);
        }
        for sc in &deck.defs {
            self.cdl_block(sc, false);
        }
        for &n in &deck.top.loads {
            let path = self.rd.unquote(self.rd.fields(n)[0]);
            let line = format!("* error: cdl loads no Verilog-A ({path})");
            self.o.token(&line, Space::Line);
            self.note(Code::W0021, self.rd.main(n));
        }
        // quirk: the top's models alone, each `k=v` as read, none of a
        // subcircuit's.
        for &(m, bin) in &deck.top.models {
            let m = self.rd.model_view(m, bin);
            let mut line = format!(".model {} {}", m.name, m.kind);
            for (k, v) in &m.params {
                line.push_str(&format!(" {k}={v}"));
            }
            self.o.token(&line, Space::Line);
        }
        self.o.token(".END", Space::Line);
        self.unwritten();
    }
}

// ---- private ----

impl W<'_, '_> {
    /// One block as `.SUBCKT name ports params`, `*.PININFO`, its elements
    /// sorted by name, its text kept as lines, `.ENDS name`.
    fn cdl_block(&mut self, b: &Block, top: bool) {
        let mut line = format!(".SUBCKT {}", b.name);
        let mut ports = Vec::new();
        // CDL has no `.param`: a block's parameters are its `.SUBCKT` line's.
        let mut params: Vec<(String, String)> = Vec::new();
        if top {
            params.extend(b.params.iter().map(|&a| self.rd.param_view(a)));
        } else {
            let head = self
                .rd
                .subckt_head(b.node)
                .expect("the reader read the head");
            ports = head.ports;
            params = head.params;
            params.extend(b.params.iter().map(|&a| self.rd.param_view(a)));
        }
        for p in &ports {
            line.push(' ');
            line.push_str(p);
        }
        for (k, v) in &params {
            line.push_str(&format!(" {k}={v}"));
        }
        self.o.token(&line, Space::Line);
        if !ports.is_empty() {
            let pins: Vec<String> = ports.iter().map(|p| format!(" {p}:B")).collect();
            self.o
                .token(&format!("*.PININFO{}", pins.concat()), Space::Line);
        }
        let mut lines = self.cdl_elements(b);
        // A stable sort by element name (the first word), as xschem writes.
        lines.sort_by(|x, y| first_word(x).cmp(first_word(y)));
        for l in lines.iter().filter(|l| !l.is_empty()) {
            self.o.token(l.strip_suffix('\n').unwrap_or(l), Space::Line);
        }
        for l in self.raw_lines(&b.raw) {
            self.o.token(&l, Space::Line);
        }
        self.o.token(&format!(".ENDS {}", b.name), Space::Line);
    }

    /// Block `b`'s elements, each its line: the cards in order, then the
    /// instances (the old IR's components, then its instances).
    fn cdl_elements(&mut self, b: &Block) -> Vec<String> {
        let coupled = match self.rd.d == crate::codegen::Dialect::Ltspice {
            true => Some(self.coupled(b)),
            false => None,
        };
        let mut out = Vec::with_capacity(b.comps.len() + b.insts.len());
        for &c in &b.comps {
            out.push(match c {
                Comp::Card(n) => self.cdl_card(n, coupled.as_deref()),
                Comp::Raw(n) => self.rd.line(n),
            });
        }
        for &(_, n) in &b.insts {
            out.push(self.cdl_instance(n));
        }
        out
    }

    /// Element `n`'s ngspice card; a comment naming its codes when the card
    /// has no form for it (a note, the deck still written).
    fn cdl_card(&mut self, n: u32, coupled: Option<&[String]>) -> String {
        let before = self.diags.len();
        let o = std::mem::replace(&mut self.o, Out::new(128));
        self.card(n, coupled);
        let line = std::mem::replace(&mut self.o, o).finish();
        let (refused, notes): (Vec<Diag>, Vec<Diag>) = self
            .diags
            .drain(before..)
            .partition(|d| info(d.code).kind == Kind::Refusal);
        self.diags.extend(notes);
        if refused.is_empty() {
            return line;
        }
        self.note(Code::W0021, self.rd.main(n));
        let codes: Vec<String> = refused
            .iter()
            .map(|d| format!("{:?} {}", d.code, info(d.code).title))
            .collect();
        format!("* error: {}", codes.join("; "))
    }

    /// An `X` card as the old layout spelled an instance: `X` and the name
    /// after its letter, unless that already starts with an `X`.
    fn cdl_instance(&self, n: u32) -> String {
        let Card::Inst {
            name,
            ports,
            master,
            params,
        } = self.rd.card_view(n)
        else {
            unreachable!("an instance the reader placed");
        };
        // quirk: the old layout's letter rule (`Xx2` is `x2`), not the name
        // map's; phase 7 writes the map's name.
        let x = if name.starts_with(['x', 'X']) {
            ""
        } else {
            "X"
        };
        let mut s = format!("{x}{name}");
        for p in &ports {
            s.push(' ');
            s.push_str(p);
        }
        s.push(' ');
        s.push_str(&master);
        for (k, v) in &params {
            s.push_str(&format!(" {k}={v}"));
        }
        s
    }
}

impl W<'_, '_> {
    /// Notes what the layout does not write (an LVS netlist's shape):
    /// analyses and outputs, sections, branches, includes, functions, a
    /// subcircuit's models and loads.
    fn unwritten(&mut self) {
        let deck = self.deck;
        let rd = self.rd;
        let mut at: Vec<u32> = Vec::new();
        for &b in &deck.bench {
            match b {
                // Its VACASK control text is written.
                Bench::Control(_) => {}
                Bench::Card(n)
                | Bench::Meas(n)
                | Bench::Save(n)
                | Bench::Analysis(n)
                | Bench::Options(n)
                | Bench::NodeVals(n) => at.push(n),
            }
        }
        at.extend(deck.sections.iter().map(|s| s.node));
        let funcs = deck
            .funcs
            .iter()
            .chain(deck.sections.iter().flat_map(|s| &s.funcs));
        at.extend(funcs.map(|&(Func::Param(n) | Func::Card(n) | Func::Real(n))| n));
        for b in std::iter::once(&deck.top).chain(&deck.defs) {
            at.extend(b.conds.iter().map(|&(n, _)| n));
            at.extend(b.includes.iter().chain(&b.libs).copied());
        }
        for sc in &deck.defs {
            at.extend(sc.models.iter().map(|&(m, _)| m));
            at.extend(sc.loads.iter().copied());
        }
        for n in at {
            self.note(Code::W0028, rd.main(n));
        }
    }
}

/// A written element's name: its first word.
fn first_word(line: &str) -> &str {
    line.split_ascii_whitespace().next().unwrap_or("")
}
