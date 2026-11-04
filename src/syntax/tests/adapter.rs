//! The adapter over block nesting and kit answers: a deck nested 10⁴ deep
//! in `.subckt`s and `.if`s reaches the IR in Debug, and what
//! `syntax::resolve` answers for a kit file reaches its IR. Each test names
//! its clause, the IR derived by hand, and how many checks it makes.
//!
//! Contents: none (tests only).

use crate::syntax::lexicon::{NGSPICE, SPECTRE};
use crate::syntax::{parse, tokenize};

// ---- private ----

#[test]
fn nested_subckts_ten_thousand_deep_reach_the_ir() {
    // ngspice manual §2.5: definitions nest; the adapter hoists each as
    // `<parent>__<name>` (as the old lowering did), walking the nesting on a heap
    // stack (rule 7 of the compiler-construction skill), so this runs in
    // Debug on a test thread's 2 MiB stack. Checks: 3.
    let n = 10_000;
    let mut src = String::from("t\n");
    for k in 0..n {
        src.push_str(&format!(".subckt s{k} a\nR{k} a 0 1\n"));
    }
    for _ in 0..n {
        src.push_str(".ends\n");
    }
    let t = tokenize(&src, &NGSPICE);
    let a = parse(&t, &NGSPICE);
    let (ir, report) =
        crate::syntax::ast_to_ir(&a, &t, crate::codegen::Dialect::Ngspice, "deep", None);
    assert_eq!(ir.subcircuit_defs.len(), n);
    assert!(report.unparsed.is_empty() && report.raw == 0);
    let inner = format!("s{}__s{}", n - 2, n - 1);
    assert!(ir.subcircuit_defs.iter().any(|d| d.name == inner));
}

#[test]
fn nested_ifs_ten_thousand_deep_reach_the_ir() {
    // ngspice manual §2.15: `.if` blocks nest; Spectre Reference 19.1 p.491:
    // `if` blocks nest; a `.subckt` may sit in a branch and a branch in a
    // `.subckt`. The adapter walks every block on one heap stack (rule 7 of
    // the compiler-construction skill), so these run in Debug on a test
    // thread's 2 MiB stack. Checks: 4. The IR built is itself a tree of
    // nested `Vec`s whose drop recurses (the old IR's shape, gone with it in
    // phase 6), so the deep ones are forgotten, not dropped.
    use crate::codegen::Dialect;
    use crate::ir::Conditional;
    let n = 10_000;
    let depth = |c: &Conditional| {
        let mut d = 0;
        let mut at = Some(c);
        while let Some(c) = at {
            d += 1;
            at = c.branches[0].body.conditionals.first();
        }
        d
    };
    let mut spice = String::from("t\n");
    let mut spectre = String::from("t\n");
    let mut mixed = String::from("t\n");
    for k in 0..n {
        spice.push_str(".if (a == 1)\n");
        spectre.push_str("if (a == 1) {\n");
        mixed.push_str(&format!(".subckt s{k} a\n.if (a == 1)\n"));
    }
    spice.push_str("R1 a 0 1\n");
    spectre.push_str("r1 (a 0) resistor r=1\n");
    for _ in 0..n {
        spice.push_str(".endif\n");
        spectre.push_str("}\n");
        mixed.push_str(".endif\n.ends\n");
    }
    for (src, lex, d) in [
        (&spice, &NGSPICE, Dialect::Ngspice),
        (&spectre, &SPECTRE, Dialect::Spectre),
    ] {
        let t = tokenize(src, lex);
        let a = parse(&t, lex);
        let (ir, _) = crate::syntax::ast_to_ir(&a, &t, d, "deep", None);
        assert_eq!(depth(&ir.top.conditionals[0]), n, "{d:?}");
        std::mem::forget(ir);
    }
    let t = tokenize(&mixed, &NGSPICE);
    let a = parse(&t, &NGSPICE);
    let (ir, report) = crate::syntax::ast_to_ir(&a, &t, Dialect::Ngspice, "deep", None);
    assert_eq!(ir.subcircuit_defs.len(), n);
    assert!(report.unparsed.is_empty());
    std::mem::forget(ir);
}

#[test]
fn kit_answers_reach_the_ir() {
    // Phase 3 (TESTING.md, "Golden gate"): the adapter reads the
    // kit where the file alone leaves a master open. top.scs's `x1`, `x2`
    // name `leaf` and `m1` names `nch_mod` (Spectre UG p.66), both in
    // lib.scs; ngspice ch.3 `R1 a b 1k rmod`, ch.8 `Q1 c b e mod 1.5`
    // (models in another file). Checks: 7.
    use crate::codegen::Dialect;
    use crate::ir::Component;
    use crate::syntax::resolve::{FileId, Form, Kit, LoadError};
    use std::path::{Path, PathBuf};
    let read = |files: &[(&str, &str)], d: Dialect| {
        let mut load = |p: &Path| {
            let f = files.iter().find(|(q, _)| Path::new(q) == p);
            f.map(|(_, s)| s.to_string()).ok_or(LoadError::NotFound)
        };
        let k = Kit::load(&[PathBuf::from(files[0].0)], Form::Deck, d, &mut load);
        crate::import::parse_kit(&k, FileId(0), "top")
    };
    let top = "// top\ninclude \"lib.scs\"\nsubckt top (a b)\nx1 (a b) leaf w=1u\nx2 a b leaf w=2u\nm1 (a b 0 0) nch_mod w=1u\nends top\n";
    let lib = "// lib\nsubckt leaf (p n)\nends leaf\nmodel nch_mod bsim4 type=n\n";
    let (ir, report) = read(&[("/top.scs", top), ("/lib.scs", lib)], Dialect::Spectre);
    let sc = &ir.subcircuit_defs[0];
    let masters: Vec<&str> = sc.instances.iter().map(|i| i.subcircuit.as_str()).collect();
    assert_eq!(masters, ["leaf", "leaf"]);
    assert!(matches!(&sc.components[..], [Component::Mosfet { model, .. }] if model == "nch_mod"));
    assert_eq!(report.raw, 0);
    let deck = "t\n.include m.sp\nR1 a b 1k rmod\nQ1 c b e mod 1.5\n";
    let models = "* m\n.model rmod r rsh=1\n.model mod npn\n";
    let (ir, report) = read(&[("/d.sp", deck), ("/m.sp", models)], Dialect::Ngspice);
    let [r, q] = &ir.top.components[..] else {
        panic!("two components: {:?}", ir.top.components)
    };
    assert!(matches!(r, Component::Resistor { model: Some(m), .. } if m == "rmod"));
    assert!(
        matches!(q, Component::Bjt { nc, nb, ne, extra_nodes, model, .. }
            if (nc, nb, ne, model) == (&"c".into(), &"b".into(), &"e".into(), &"mod".to_string())
                && extra_nodes.is_empty())
    );
    assert_eq!(report.raw, 0);
}
