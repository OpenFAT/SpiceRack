//! The deck → structural Verilog: the SPICE writer's Verilog row. Each
//! block (the top, then each subcircuit) is one `module name (ports);` …
//! `endmodule` around its ports (`inout`), the nets its instances use
//! (`wire`) and its instances, each `master name (.port(net), …);` paired
//! by the master's ports where the deck defines it, by position
//! otherwise. Verilog has a module only for a subcircuit: every other
//! element, and text kept as written, is a `// W0029 …` comment in its
//! place, and what else the deck holds is not written; each is a `W0029`
//! note, the deck still written.
//!
//! Cites: IEEE 1364-2005 §3.7.1 (escaped identifiers: `\` up to a blank),
//! §12.1 (module declarations), §12.2.2 (connections by name and by
//! position); the old xschem layout writer, whose
//! element lines this writer does not keep (it wrote the old IR's `Debug`
//! text).
//!
//! Contents:
//! - `W::verilog(&mut self)`

use std::collections::BTreeSet;

use super::W;
use crate::syntax::diag::{Code, info};
use crate::syntax::render::out::Space;
use crate::syntax::render::read::{Block, Card, Comp};

impl W<'_, '_> {
    /// The deck, as one module per block.
    pub(super) fn verilog(&mut self) {
        let deck = self.deck;
        self.o.token(&format!("// {}", deck.top.name), Space::Line);
        self.module(&deck.top, true);
        for sc in &deck.defs {
            self.module(sc, false);
        }
        // What a module holds no form for: analyses and outputs, sections,
        // branches, includes, models, functions, loads.
        let mut at: Vec<u32> = deck.sections.iter().map(|s| s.node).collect();
        for b in std::iter::once(&deck.top).chain(&deck.defs) {
            at.extend(b.conds.iter().map(|&(n, _)| n));
            at.extend(b.includes.iter().chain(&b.libs).chain(&b.loads));
            at.extend(b.models.iter().map(|&(m, _)| m));
            at.extend(b.params.iter().chain(&b.locals));
        }
        if let Some(&first) = at.first() {
            self.note(Code::W0029, self.rd.main(first));
        }
        if !deck.bench.is_empty() || !deck.funcs.is_empty() {
            self.note(Code::W0029, 0);
        }
    }
}

// ---- private ----

impl W<'_, '_> {
    /// One block as a module: its ports, the nets its instances use, its
    /// instances, a comment per element no module stands for.
    fn module(&mut self, b: &Block, top: bool) {
        let ports = match top {
            true => Vec::new(),
            false => {
                let head = self.rd.subckt_head(b.node);
                head.expect("the reader read the head").ports
            }
        };
        let names: Vec<String> = ports.iter().map(|p| ident(p)).collect();
        let head = format!("module {} ({});", ident(&b.name), names.join(", "));
        self.o.token(&head, Space::Line);
        for p in &names {
            self.o.token(&format!("  inout {p};"), Space::Line);
        }
        let insts: Vec<Card> = b.insts.iter().map(|&(_, n)| self.rd.card_view(n)).collect();
        let mut wires = BTreeSet::new();
        for i in &insts {
            if let Card::Inst { ports: nets, .. } = i {
                wires.extend(nets.iter().filter(|n| !ports.contains(n) && *n != "0"));
            }
        }
        for w in wires {
            self.o.token(&format!("  wire {};", ident(w)), Space::Line);
        }
        for (i, &(_, n)) in insts.iter().zip(&b.insts) {
            let line = self.instance(i);
            self.o.token(&line, Space::Line);
            if matches!(i, Card::Inst { params, .. } if !params.is_empty()) {
                self.note(Code::W0029, self.rd.main(n));
            }
        }
        let title = info(Code::W0029).title;
        for &c in &b.comps {
            let (Comp::Card(n) | Comp::Raw(n)) = c;
            self.note(Code::W0029, self.rd.main(n));
            let name = self.rd.t.text(self.rd.main(n));
            self.o
                .token(&format!("  // W0029 {title}: {name}"), Space::Line);
        }
        self.o.token("endmodule", Space::Line);
    }

    /// `master name (.port(net), …);`, by name where the deck defines the
    /// master, else `master name (net, …);`.
    fn instance(&self, i: &Card) -> String {
        let Card::Inst {
            name,
            ports: nets,
            master,
            ..
        } = i
        else {
            unreachable!("an instance the reader placed");
        };
        let def = self
            .deck
            .defs
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(master));
        let pins = def
            .and_then(|d| self.rd.subckt_head(d.node))
            .map(|h| h.ports);
        let conns: Vec<String> = match pins.filter(|p| p.len() == nets.len()) {
            Some(pins) => pins
                .iter()
                .zip(nets)
                .map(|(p, n)| format!(".{}({})", ident(p), ident(n)))
                .collect(),
            None => nets.iter().map(|n| ident(n)).collect(),
        };
        // The view's name follows the card's letter; the instance keeps it.
        let name = format!("X{name}");
        format!(
            "  {} {} ({});",
            ident(master),
            ident(&name),
            conns.join(", ")
        )
    }
}

/// `name` as a Verilog identifier: as is when it is a simple one
/// (`[A-Za-z_][A-Za-z0-9_$]*`), else escaped (`\name `, IEEE 1364-2005
/// §3.7.1).
// ponytail: a simple name that is a Verilog keyword (`module`, `if`) is not
// escaped; escape it when a deck names a subcircuit so.
fn ident(name: &str) -> String {
    let b = name.as_bytes();
    let simple = b
        .first()
        .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'$');
    match simple {
        true => name.to_string(),
        false => format!("\\{name} "),
    }
}

#[cfg(test)]
mod tests {
    use crate::syntax::Dialect;
    use crate::syntax::diag::Code;
    use crate::syntax::render::render_text;

    #[test]
    fn subcircuits_are_modules_and_instances_connect_by_port() {
        // IEEE 1364-2005 §12.2.2: `inv u1 (.a(n1), .y(out));` connects by
        // name; a master the deck does not define connects by position.
        // A ground net `0` and a net `n<1>` are escaped (§3.7.1). Checks: 3.
        let deck = "t\n.subckt inv a y\nM1 y a 0 0 nmos\n.ends\nX1 in n<1> inv\nX2 n<1> out pdk_cell\nR1 out 0 1k\n.op\n.end\n";
        let w = render_text(deck, Dialect::Ngspice, "t", Dialect::Verilog).expect("written");
        let want = "// t\nmodule t ();\n  wire in;\n  wire \\n<1> ;\n  wire out;\n  inv X1 (.a(in), .y(\\n<1> ));\n  pdk_cell X2 (\\n<1> , out);\n  // W0029 a statement structural Verilog does not write: R1\nendmodule\nmodule inv (a, y);\n  inout a;\n  inout y;\n  // W0029 a statement structural Verilog does not write: M1\nendmodule\n";
        assert_eq!(w.text, want);
        let codes: Vec<Code> = w.notes.diags.iter().map(|d| d.code).collect();
        assert!(codes.iter().all(|&c| c == Code::W0029), "{codes:?}");
        assert_eq!(codes.len(), 3, "R1, M1, the analysis");
    }
}
