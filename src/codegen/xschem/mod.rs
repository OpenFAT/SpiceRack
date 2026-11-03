//! Decks laid out the way xschem writes them.
//!
//! [`write`] lays a [`Deck`] (a [`CircuitIR`] plus what an xschem deck carries beside
//! it) out in the order and spelling of xschem's netlister, so a schematic tool keeps
//! byte-for-byte parity with xschem's golden netlists while SpiceRack writes the text:
//!
//! ```text
//! title           * <title>   (// for Spectre and Verilog; Spectre adds `simulator lang=spectre`)
//! globals         .global a b            (Spectre `global 0 a b`; CDL `.GLOBAL`)
//! parameters      .param k=v             (Spectre one `parameters` line; not CDL or Verilog)
//! setup           .lib / .include / .temp in deck order (not CDL or Verilog)
//! top block       the top's elements; wrapped as a .SUBCKT for CDL, a module for Verilog
//! bench text      the testbench's extra_lines, verbatim
//! definitions     subcircuit_defs in IR order, each its elements then its raw_spice
//! Verilog-A       one load per source, then the top's models, then the bodies
//! end             .end (CDL .END; none for Spectre and Verilog)
//! ```
//!
//! A block's elements are its packed [`Lines`], its components and its instances,
//! sorted bytewise by element name (the first token, the second for Verilog's
//! `<module> <instance> (…);`). The sort is stable and lines come first, then
//! components, then instances. A multi-line element counts as one unit.
//!
//! Dialects: ngspice, ESPice, Xyce, HSPICE, LTspice, CDL, Spectre, Verilog (VACASK
//! reads its own language and has no xschem layout). A component a dialect cannot
//! spell becomes a comment naming it. Only the testbench's `extra_lines` are
//! written; an xschem deck carries its analyses as text.

mod block;
mod deck;
mod sections;
mod spell;

use std::collections::BTreeMap;

use super::{CodeGenError, Dialect};
use crate::ir::*;
pub use deck::{Deck, Lines, SetupLine};
use sections::Writer;
pub use spell::{
    comment, element_name, is_spice, push_element_name, push_ident, push_instance, word,
};

/// The deck, `\n` line ends, ending in a newline.
///
/// # Errors
///
/// VACASK, which has no xschem layout.
pub fn write(deck: &Deck<'_>, d: Dialect) -> Result<String, CodeGenError> {
    if d == Dialect::Vacask {
        return Err(CodeGenError::Other("VACASK has no xschem layout".into()));
    }
    let ir = deck.ir;
    let blocks = 1 + ir.subcircuit_defs.len();
    assert!(
        deck.lines.len() <= blocks && deck.nets.len() <= blocks,
        "one entry per block"
    );
    let text: usize = deck
        .lines
        .iter()
        .flatten()
        .map(|l| l.text.len() + l.len())
        .sum();
    let raw: usize = std::iter::once(&ir.top)
        .chain(&ir.subcircuit_defs)
        .map(|s| 48 * (s.components.len() + s.instances.len()))
        .sum();
    let bodies: usize = deck.bodies.iter().map(String::len).sum();
    let mut out = String::with_capacity(4096 + text + raw + bodies);
    let ports: BTreeMap<&str, &[Port]> = match d {
        Dialect::Verilog => ir
            .subcircuit_defs
            .iter()
            .map(|s| (s.name.as_str(), s.ports.as_slice()))
            .collect(),
        _ => BTreeMap::new(),
    };
    let w = Writer { d, deck, ports };
    w.header(&mut out);
    w.globals(&mut out);
    w.params(&mut out);
    w.setup(&mut out);
    w.block(
        &mut out,
        0,
        &ir.top,
        matches!(d, Dialect::Cdl | Dialect::Verilog),
    );
    for l in ir.testbench.iter().flat_map(|tb| &tb.extra_lines) {
        out.push_str(l);
        out.push('\n');
    }
    for (k, sc) in ir.subcircuit_defs.iter().enumerate() {
        w.block(&mut out, k + 1, sc, true);
    }
    w.loads(&mut out);
    for m in &ir.top.models {
        out.push_str(if is_spice(d) { ".model " } else { "model " });
        out.push_str(&m.name);
        out.push(' ');
        out.push_str(&m.kind);
        for (k, v) in &m.parameters {
            out.push(' ');
            out.push_str(k);
            out.push('=');
            out.push_str(v);
        }
        out.push('\n');
    }
    for b in &deck.bodies {
        debug_assert!(
            b.is_empty() || b.ends_with('\n'),
            "a body ends in a newline"
        );
        if d == Dialect::Spectre {
            out.push_str("simulator lang=spice\n");
        }
        out.push_str(b);
        if d == Dialect::Spectre {
            out.push_str("simulator lang=spectre\n");
        }
    }
    out.push_str(match d {
        Dialect::Cdl => ".END\n",
        Dialect::Spectre | Dialect::Verilog => "",
        _ => ".end\n",
    });
    debug_assert!(out.ends_with('\n'));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(line: &str) -> Component {
        Component::RawSpice { line: line.into() }
    }

    fn port(name: &str, direction: PortDirection) -> Port {
        Port {
            name: name.into(),
            direction,
        }
    }

    fn ir() -> CircuitIR {
        CircuitIR {
            top: Subcircuit {
                name: "tb".into(),
                parameters: vec![ParamDef {
                    name: "vdd".into(),
                    default: Some("1.8".into()),
                }],
                components: vec![raw("V1 vdd! 0 1.8")],
                instances: vec![Instance {
                    name: "1".into(),
                    subcircuit: "inv".into(),
                    port_mapping: vec!["in".into(), "out[3]".into()],
                    ..Default::default()
                }],
                osdi_loads: vec!["dev.va".into()],
                models: vec![ModelDef {
                    name: "m1".into(),
                    kind: "dev".into(),
                    ..Default::default()
                }],
                ..Subcircuit::default()
            },
            testbench: Some(Testbench {
                extra_lines: vec![".op".into()],
                ..Testbench::default()
            }),
            subcircuit_defs: vec![Subcircuit {
                name: "inv".into(),
                ports: vec![
                    port("a", PortDirection::Input),
                    port("y", PortDirection::Output),
                ],
                components: vec![raw("M2 y a 0 0 n"), raw("M1 y a vdd! vdd! p")],
                raw_spice: vec!["* own code".into()],
                ..Subcircuit::default()
            }],
            ..CircuitIR::default()
        }
    }

    fn deck(ir: &CircuitIR) -> Deck<'_> {
        let mut top = Lines::default();
        top.push("C1 out 0 1e-12");
        Deck {
            title: Some("tb (test)".into()),
            globals: vec!["0".into(), "vdd!".into()],
            setup: vec![
                SetupLine::Lib {
                    path: "m.lib".into(),
                    section: "tt".into(),
                },
                SetupLine::Temp { value: "27".into() },
                SetupLine::Include {
                    path: "x.inc".into(),
                },
            ],
            bodies: vec![".model d1 d\n".into()],
            lines: vec![vec![top]],
            nets: vec![vec![], vec!["mid".into()]],
            ..Deck::new(ir)
        }
    }

    fn emit(d: Dialect) -> String {
        write(&deck(&ir()), d).unwrap()
    }

    #[test]
    fn ngspice_layout() {
        assert_eq!(
            emit(Dialect::Ngspice),
            "* tb (test)\n.global vdd!\n.param vdd=1.8\n.lib m.lib tt\n.temp 27\n.include x.inc\n\
             C1 out 0 1e-12\nV1 vdd! 0 1.8\nX1 in out[3] inv\n.op\n\
             .subckt inv a y\nM1 y a vdd! vdd! p\nM2 y a 0 0 n\n* own code\n.ends inv\n\
             .control\npre_osdi dev.osdi\n.endc\n.model m1 dev\n.model d1 d\n.end\n"
        );
    }

    #[test]
    fn dialect_spellings() {
        let x = emit(Dialect::Xyce);
        assert!(x.contains(".options device temp=27\n"), "{x}");
        let e = emit(Dialect::Espice);
        assert!(
            e.contains(".hdl \"dev.va\"\n") && !e.contains("pre_osdi"),
            "{e}"
        );
        let h = emit(Dialect::Hspice);
        assert!(
            h.contains(".lib 'm.lib' tt\n.temp 27\n.include 'x.inc'\n")
                && h.contains(".hdl \"dev.va\"\n"),
            "{h}"
        );
        let l = emit(Dialect::Ltspice);
        assert!(
            l.starts_with("* tb (test)\n.options plotwinsize=0 numdgt=15\n"),
            "{l}"
        );
        assert!(
            l.contains("* error: ltspice loads no Verilog-A (dev.va)\n"),
            "{l}"
        );
        let c = emit(Dialect::Cdl);
        assert!(
            c.starts_with("* tb (test)\n.GLOBAL vdd!\n.SUBCKT tb vdd=1.8\n"),
            "{c}"
        );
        assert!(
            c.contains(".SUBCKT inv a y\n*.PININFO a:I y:O\n") && c.ends_with(".END\n"),
            "{c}"
        );
        assert!(!c.contains(".lib") && !c.contains(".param"), "{c}");
        let s = emit(Dialect::Spectre);
        assert!(
            s.starts_with(
                "// tb (test)\nsimulator lang=spectre\nglobal 0 vdd\\!\nparameters vdd=1.8\n"
            ),
            "{s}"
        );
        assert!(
            s.contains("include \"m.lib\" section=tt\ncorner_temp options temp=27\n"),
            "{s}"
        );
        assert!(
            s.contains("1 (in out\\[3\\]) inv\n") && s.contains("subckt inv (a y)\n"),
            "{s}"
        );
        assert!(
            s.contains("simulator lang=spice\n.model d1 d\nsimulator lang=spectre\n"),
            "{s}"
        );
        let v = emit(Dialect::Verilog);
        assert!(
            v.contains("module tb ();\n  inv \\1  (.a(in), .y(\\out[3] ));\n  C1 out"),
            "{v}"
        );
        assert!(
            v.contains("module inv (a, y);\n  input a;\n  output y;\n  wire mid;\n"),
            "{v}"
        );
        assert!(write(&deck(&ir()), Dialect::Vacask).is_err());
        // The plain codegen entry writes the dialects only this layout has.
        assert!(
            super::super::netlist(&ir(), Dialect::Cdl)
                .unwrap()
                .starts_with("* tb\n")
        );
    }

    #[test]
    fn element_names() {
        let mut s = String::new();
        for n in ["X1", "x2", "M1"] {
            push_element_name(&mut s, n, Dialect::Ngspice);
            s.push(' ');
        }
        assert_eq!(s, "X1 x2 XM1 ");
        assert_eq!(element_name("XM1 a b", Dialect::Cdl), "XM1");
        assert_eq!(element_name("  inv u1 (.a(x));", Dialect::Verilog), "u1");
        assert_eq!(element_name("", Dialect::Ngspice), "");
        for (name, d, want) in [
            ("a[0]", Dialect::Cdl, "a[0]"),
            ("a[0]", Dialect::Spectre, r"a\[0\]"),
            ("_x$1", Dialect::Verilog, "_x$1"),
            ("1a", Dialect::Verilog, r"\1a "),
            ("", Dialect::Verilog, r"\ "),
        ] {
            let mut out = String::from(">");
            push_ident(&mut out, name, d);
            assert_eq!(out, format!(">{want}"), "{name} {d:?}");
        }
    }

    #[test]
    fn lines_pack() {
        let mut a = Lines::default();
        a.push("R1 a b 1");
        a.push("R2 c d 2\n+ tc=1\n");
        a.push("");
        assert_eq!(
            (a.len(), a.get(0), a.get(1), a.get(2)),
            (3, "R1 a b 1", "R2 c d 2\n+ tc=1\n", "")
        );
        // Runs of one block are its lines in order.
        let ir = ir();
        let mut d = deck(&ir);
        let mut second = Lines::default();
        second.push("B1 x 0 v=1");
        d.lines[0].push(second);
        let n = write(&d, Dialect::Ngspice).unwrap();
        assert!(n.contains("B1 x 0 v=1\nC1 out 0 1e-12\nV1"), "{n}");
    }

    /// A run marked sorted writes as an unmarked one: in place when its neighbours keep
    /// the order, line by line among them when they do not.
    #[test]
    fn sorted_runs_write_as_unmarked_ones() {
        let ir = ir();
        for (lines, marked) in [
            (["R1 a 0 1", "R2 a 0 1", "Y1 b"], [true, false]),
            (["A1 a 0 1", "B2 a 0 1", "C1 b"], [true, true]),
        ] {
            let run = |sorted: bool, ls: &[&str]| {
                let mut l = Lines::default();
                ls.iter().for_each(|s| l.push(&format!("{s}\n")));
                l.sorted = sorted;
                l
            };
            let deck = |m: [bool; 2]| Deck {
                lines: vec![vec![run(m[0], &lines[..2]), run(m[1], &lines[2..])]],
                ..Deck::new(&ir)
            };
            for d in [Dialect::Ngspice, Dialect::Spectre] {
                assert_eq!(
                    write(&deck(marked), d).unwrap(),
                    write(&deck([false, false]), d).unwrap(),
                    "{d:?}"
                );
            }
        }
        // The top's `V1` and `X1` sort between the marked run's lines.
        let n = write(&Deck::new(&ir), Dialect::Ngspice).unwrap();
        assert!(n.contains("V1 vdd! 0 1.8\nX1 in out[3] inv\n"), "{n}");
        let mut l = Lines::default();
        l.push("A1 a\n");
        l.push("W1 a\n");
        l.sorted = true;
        let d = Deck {
            lines: vec![vec![l]],
            ..Deck::new(&ir)
        };
        let n = write(&d, Dialect::Ngspice).unwrap();
        assert!(
            n.contains("A1 a\nV1 vdd! 0 1.8\nW1 a\nX1 in out[3] inv\n"),
            "{n}"
        );
    }

    /// `push_instance` spells an instance as the deck writes an [`Instance`].
    #[test]
    fn push_instance_spells_as_the_deck() {
        let ports = ["a", "y"];
        for (d, want) in [
            (Dialect::Ngspice, "X1 in ? inv w=2"),
            (Dialect::Spectre, r"1 (in ?) inv w=2"),
            (Dialect::Verilog, r"inv \1  (.a(in), .y(?));"),
        ] {
            let mut out = String::new();
            push_instance(&mut out, "1", "inv", ["in", "?"], ports, [("w", "2")], d);
            assert_eq!(out, want, "{d:?}");
        }
    }

    /// Determinism: one IR gives one deck, a clone included (every map on the emit
    /// path is ordered), and elements out of name order are sorted stably.
    #[test]
    fn two_emits_are_byte_identical_and_sorted() {
        for d in [
            Dialect::Ngspice,
            Dialect::Espice,
            Dialect::Xyce,
            Dialect::Hspice,
            Dialect::Ltspice,
            Dialect::Cdl,
            Dialect::Spectre,
            Dialect::Verilog,
        ] {
            let a = emit(d);
            let ir2 = ir().clone();
            assert_eq!(a, write(&deck(&ir2), d).unwrap(), "{d:?}");
        }
        let mut ir = ir();
        ir.top.components = vec![raw("R2 a 0 1"), raw("R1 a 0 1"), raw("R2 b 0 2")];
        ir.top.instances.clear();
        let n = write(&Deck::new(&ir), Dialect::Ngspice).unwrap();
        assert!(n.contains("R1 a 0 1\nR2 a 0 1\nR2 b 0 2\n"), "{n}");
    }
}
