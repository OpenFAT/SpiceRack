//! Conditional blocks and functions through the transpiler: ngspice `.if`
//! ... `.endif`, Spectre's structural `if`/`else` and `real` functions,
//! VACASK `@if` ... `@end`.
//!
//! Oracles: the hand-written fixtures under `tests/fixtures/conditionals/`
//! read by hand, against the deck written in their own dialect; read ->
//! write -> read -> write as a text fixpoint per dialect; real ngspice
//! running the deck written back to the original's operating point bit for
//! bit, and running the Spectre kit written as ngspice to each branch's
//! closed-form resistance (worked by hand beside the test). Unsupported
//! fails, never skips: run in `nix develop .#testing`.
//!
//! Contents: none (tests only).

use spicerack::syntax::Dialect;
use spicerack::syntax::diag::Code;
use spicerack::syntax::render::render_text;

const KIT: &str = include_str!("fixtures/conditionals/kit.scs");
const IF: &str = include_str!("fixtures/conditionals/if.sp");

// ---- private ----

/// `src` (read in `d`) written for `to`, with no note on it.
fn written(src: &str, d: Dialect, to: Dialect) -> String {
    let w = render_text(src, d, "t", to).unwrap_or_else(|e| panic!("{to:?}: {}", e.messages));
    let codes: Vec<Code> = w.notes.diags.iter().map(|d| d.code).collect();
    assert_eq!(codes, [] as [Code; 0], "{to:?}: {}", w.notes.messages);
    w.text
}

/// Read -> write -> read -> write gives the same text.
fn fixpoint(src: &str, d: Dialect) -> String {
    let text = written(src, d, d);
    assert_eq!(written(&text, d, d), text, "{d:?}");
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
/// read with nothing kept as text; a fixpoint in Spectre.
#[test]
fn spectre_kit_reads_into_functions_and_conditionals() {
    let text = fixpoint(KIT, Dialect::Spectre);
    // By hand from kit.scs: the three functions, each its arguments and
    // body, in the section; then the subcircuit's one `if` chain, its
    // conditions in order (the two-line one joined), the third branch
    // nesting a two-branch `if`, every other branch one resistor.
    let want = "library kit\nsection tt\n\
                real rscale(real w, real l) {\n    return 1 + 0.1*(l/w);\n}\n\
                real rsheet(real t) {\n    return 100*(1 + 1e-3*(t - 27));\n}\n\
                real lim(real x, real lo, real hi) {\n    return x < lo ? lo : (x > hi ? hi : x);\n}\n\
                parameters rsh=rsheet(27)\nmodel nch bsim4 type=n vth0=400m\n\
                subckt res (a b)\nparameters w=1u l=10u mode=0 hot=0\n\
                if (mode == 0) {\nr0 (a b) resistor r=rsh*l/w\n\
                } else if (mode == 1 && hot == 0) {\nr0 (a b) resistor r=2*rsh*l/w*rscale(w, l)\n\
                } else if (mode == 2) {\n\
                if (hot == 1) {\nr0 (a b) resistor r=lim(3*rsh*l/w, 1, 1e4)\n\
                } else {\nr0 (a b) resistor r=3*rsh*l/w\n}\n\
                } else {\nr0 (a b) resistor r=1k\n}\n\
                ends res\nendsection tt\nendlibrary kit\n";
    assert!(text.contains(want), "{text}");
}

/// The kit written as ngspice: ngspice runs each branch to its closed-form
/// resistance. rsh = rsheet(27) = 100; w = 1u, l = 10u, so rsh*l/w = 1k:
/// mode 0: 1k; mode 1, hot 0: 2*1k*rscale = 2k*(1 + 0.1*10) = 4k; mode 2,
/// hot 1: lim(3k, 1, 1e4) = 3k; mode 2, hot 0: 3k; mode 3: 1k.
#[test]
fn spectre_kit_runs_on_ngspice() {
    let sp = written(KIT, Dialect::Spectre, Dialect::Ngspice);
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
/// operating point bit for bit; LTspice and Xyce, with no `.if`, refuse it.
#[test]
fn ngspice_conditionals_round_trip_and_run() {
    let text = fixpoint(IF, Dialect::Ngspice);
    // By hand from if.sp: the subcircuit's chain of three branches, the
    // second nesting two; at the top, the element chain and the `.model`
    // chain, `D1` in each of its three branches.
    assert!(
        text.contains(".subckt sel a b PARAMS: k=0\n.if (k == 0)\nR1 a b 1k\n.elseif (k == 1)\n.if (ok2)\nR1 a b 2k\n.else\nR1 a b 3k\n.endif\n.else\nR1 a b 4k\n.endif\n.ends sel\n"),
        "{text}"
    );
    assert!(
        text.contains(".if (ok && ok2)\nR11 1 0 2\n.else\nR11 1 0 500m\n.endif\n"),
        "{text}"
    );
    assert_eq!(text.matches(".model D1 D (").count(), 3, "{text}");
    let want = op(IF);
    assert_eq!(op(&text), want);
    let hs = written(IF, Dialect::Ngspice, Dialect::Hspice);
    assert!(hs.contains(".if (ok && ok2)\nR11 1 0 2\n.else\n"), "{hs}");
    assert_eq!(
        op(&format!("{hs}\n").replacen('\n', "\n*spicerack ngbehavior=hsa\n", 1)),
        want
    );
    let scs = written(IF, Dialect::Ngspice, Dialect::Spectre);
    assert!(
        scs.contains("if (k == 0) {\nr1 (a b) resistor r=1k\n} else if (k == 1) {\nif (ok2) {"),
        "{scs}"
    );
    let back = fixpoint(&scs, Dialect::Spectre);
    let sel = &back[back.find("subckt sel").unwrap()..back.find("ends sel").unwrap()];
    assert_eq!(
        sel.matches("} else ").count(),
        3,
        "three branches, one nested: {back}"
    );
    let sim = written(IF, Dialect::Ngspice, Dialect::Vacask);
    assert!(
        sim.contains("@if k == 0\n") && sim.contains("@end\n"),
        "{sim}"
    );
    fixpoint(&sim, Dialect::Vacask);
    for d in [Dialect::Ltspice, Dialect::Xyce] {
        let refused = render_text(IF, Dialect::Ngspice, "t", d).unwrap_err();
        let codes: Vec<Code> = refused.diags.iter().map(|d| d.code).collect();
        assert_eq!(codes, [Code::W0013; 3], "{d:?}: {}", refused.messages);
    }
}
