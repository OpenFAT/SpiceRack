//! Conditional blocks and functions through the IR: ngspice `.if` ...
//! `.endif`, Spectre's structural `if`/`else` and `real` functions,
//! VACASK `@if` ... `@end`.
//!
//! Oracles: the hand-written fixtures under `tests/fixtures/conditionals/`
//! counted by hand; read -> write -> read as a fixpoint per dialect; real
//! ngspice running the deck written back to the original's operating point
//! bit for bit, and running the Spectre kit written as ngspice to each
//! branch's closed-form resistance (worked by hand beside the test).
//! Unsupported fails, never skips: run in `nix develop .#testing`.
//!
//! Contents: none (tests only).

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::parse;
use spicerack::ir::*;

const KIT: &str = include_str!("fixtures/conditionals/kit.scs");
const IF: &str = include_str!("fixtures/conditionals/if.sp");

// ---- private ----

/// Read -> write -> read gives the same IR and the same text.
fn fixpoint(ir: &CircuitIR, d: Dialect) -> String {
    let text = netlist(ir, d).unwrap_or_else(|e| panic!("{d:?}: {e}"));
    let (again, report) = parse(&text, &ir.top.name, d).unwrap();
    assert!(
        report.unparsed.is_empty(),
        "{d:?}: {:?}\n{text}",
        report.unparsed
    );
    assert_eq!(&again, ir, "{d:?}:\n{text}");
    assert_eq!(netlist(&again, d).unwrap(), text, "{d:?}");
    text
}

/// Every branch current of an operating point, bit for bit.
fn op(deck: &str) -> Vec<(String, u64)> {
    let raw = Dialect::Ngspice
        .run(deck)
        .unwrap_or_else(|e| panic!("{e}\n{deck}"));
    raw.variables
        .iter()
        .zip(&raw.real_data)
        .map(|(v, d)| (v.name.clone(), d[0].to_bits()))
        .collect()
}

/// The Spectre kit: three functions in the section, one subcircuit whose
/// body is one conditional of four branches (the third nesting two), all
/// read with nothing raw; a fixpoint in Spectre.
#[test]
fn spectre_kit_reads_into_functions_and_conditionals() {
    let (ir, report) = parse(KIT, "kit", Dialect::Spectre).unwrap();
    assert_eq!(report, Default::default());
    let tt = &ir.sections[0];
    let names: Vec<(&str, usize)> = tt
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.args.len()))
        .collect();
    assert_eq!(names, [("rscale", 2), ("rsheet", 1), ("lim", 3)]);
    assert_eq!(tt.functions[1].body, "100*(1 + 1e-3*(t - 27))");
    let res = &tt.subcircuit_defs[0];
    assert!(
        res.components.is_empty() && res.raw_spice.is_empty(),
        "{res:?}"
    );
    let [c] = &res.conditionals[..] else {
        panic!("{res:?}")
    };
    let conds: Vec<Option<&str>> = c.branches.iter().map(|b| b.condition.as_deref()).collect();
    assert_eq!(
        conds,
        [
            Some("mode == 0"),
            Some("mode == 1 && hot == 0"),
            Some("mode == 2"),
            None
        ]
    );
    let nested = &c.branches[2].body.conditionals[0];
    assert_eq!(
        nested
            .branches
            .iter()
            .map(|b| b.condition.as_deref())
            .collect::<Vec<_>>(),
        [Some("hot == 1"), None]
    );
    assert!(
        c.branches
            .iter()
            .enumerate()
            .all(|(i, b)| (i == 2) == b.body.components.is_empty())
    );
    fixpoint(&ir, Dialect::Spectre);
}

/// The kit written as ngspice: ngspice runs each branch to its closed-form
/// resistance. rsh = rsheet(27) = 100; w = 1u, l = 10u, so rsh*l/w = 1k:
/// mode 0: 1k; mode 1, hot 0: 2*1k*rscale = 2k*(1 + 0.1*10) = 4k; mode 2,
/// hot 1: lim(3k, 1, 1e4) = 3k; mode 2, hot 0: 3k; mode 3: 1k.
#[test]
fn spectre_kit_runs_on_ngspice() {
    let (ir, _) = parse(KIT, "kit", Dialect::Spectre).unwrap();
    let sp = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(
        sp.contains(".if (mode == 0)\n") && sp.contains(".func rscale(w, l) {1 + 0.1*(l/w)}\n"),
        "{sp}"
    );
    let dir = tempfile::tempdir().unwrap();
    let lib = dir.path().join("kit.lib");
    std::fs::write(&lib, &sp).unwrap();
    let cases = [
        ("0", "0", 1e3),
        ("1", "0", 4e3),
        ("2", "1", 3e3),
        ("2", "0", 3e3),
        ("3", "0", 1e3),
    ];
    for (mode, hot, r) in cases {
        let deck = format!(
            "kit\n.lib '{}' tt\nV1 a 0 1\nX1 a 0 res mode={mode} hot={hot}\n.op\n.end\n",
            lib.display()
        );
        let raw = Dialect::Ngspice
            .run(&deck)
            .unwrap_or_else(|e| panic!("{e}\n{deck}\n{sp}"));
        let i = raw
            .variables
            .iter()
            .position(|v| {
                v.name.eq_ignore_ascii_case("v1#branch") || v.name.eq_ignore_ascii_case("i(v1)")
            })
            .unwrap();
        let got = -raw.real_data[i][0];
        assert!(
            (got * r - 1.0).abs() < 1e-9,
            "mode {mode} hot {hot}: {got} A, want {} A",
            1.0 / r
        );
    }
}

/// ngspice's own forms: a `.if` in a subcircuit over its parameter, nested,
/// a top-level one choosing an element, one choosing a `.model`. A fixpoint
/// in ngspice, HSPICE, Spectre and VACASK; ngspice runs the deck written
/// back, and the HSPICE text in its HSPICE mode, to the original's
/// operating point bit for bit.
#[test]
fn ngspice_conditionals_round_trip_and_run() {
    let (ir, report) = parse(IF, "if", Dialect::Ngspice).unwrap();
    assert_eq!(report, Default::default());
    assert_eq!(
        (
            ir.top.conditionals.len(),
            ir.subcircuit_defs[0].conditionals.len()
        ),
        (2, 1)
    );
    assert_eq!(ir.top.conditionals[1].branches[1].body.models[0].name, "D1");
    let want = op(IF);
    assert_eq!(op(&fixpoint(&ir, Dialect::Ngspice)), want);
    let hs = netlist(&ir, Dialect::Hspice).unwrap();
    assert!(hs.contains(".if (ok && ok2)\nR11 1 0 2\n.else\n"), "{hs}");
    assert_eq!(
        op(&format!("{hs}\n").replacen('\n', "\n*spicerack ngbehavior=hsa\n", 1)),
        want
    );
    let scs = netlist(&ir, Dialect::Spectre).unwrap();
    assert!(
        scs.contains("if (k == 0) {\nr1 (a b) resistor r=1k\n} else if (k == 1) {\nif (ok2) {"),
        "{scs}"
    );
    let (back, report) = parse(&scs, "if", Dialect::Spectre).unwrap();
    assert_eq!(report, Default::default(), "{scs}");
    assert_eq!(back.subcircuit_defs[0].conditionals[0].branches.len(), 3);
    fixpoint(&back, Dialect::Spectre);
    let sim = netlist(&ir, Dialect::Vacask).unwrap();
    assert!(
        sim.contains("@if k == 0\n") && sim.contains("@end\n"),
        "{sim}"
    );
    let (vback, report) = parse(&sim, "if", Dialect::Vacask).unwrap();
    assert!(report.unparsed.is_empty(), "{:?}\n{sim}", report.unparsed);
    fixpoint(&vback, Dialect::Vacask);
    for d in [Dialect::Ltspice, Dialect::Xyce] {
        let err = netlist(&ir, d).unwrap_err().to_string();
        assert!(err.contains("has no .if"), "{d:?}: {err}");
    }
}
