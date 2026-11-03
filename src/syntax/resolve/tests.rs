//! Resolve, one test per answer: each names the manual clause it reads,
//! the answer derived by hand from that clause and the files written here,
//! the diagnostic codes it expects, and how many checks it makes (rule 10
//! of the compiler-construction skill). Files come from an in-memory
//! loader; no test touches the file system.
//!
//! Contents: none (tests only).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::syntax::ast::Header;

const TOP_SCS: &str = "// top\nsimulator lang=spectre\ninclude \"lib.scs\"\nsubckt top (a b)\nx1 (a b) leaf w=1u\nx2 a b leaf w=2u\nm1 (a b 0 0) nch_mod w=1u\nf1 (a b) cccs probe=v1 gain=2\nends top\n";
const LIB_SCS: &str = "// lib\nsimulator lang=spectre\nsubckt leaf (p n)\nr0 (p n) resistor r=1k\nends leaf\nmodel nch_mod bsim4 type=n\n";

// ---- private ----

/// A kit of `files` (path → text) entered at `entry`, read in `d`.
fn kit(files: &[(&str, &str)], entry: &str, form: Form, d: Dialect) -> Kit {
    let map: HashMap<PathBuf, String> = files
        .iter()
        .map(|(p, s)| (PathBuf::from(p), s.to_string()))
        .collect();
    let mut load = |p: &Path| {
        if p.starts_with("/outside") {
            return Err(LoadError::Refused);
        }
        map.get(p).cloned().ok_or(LoadError::NotFound)
    };
    Kit::load(&[PathBuf::from(entry)], form, d, &mut load)
}

/// File `f`'s tree, rebuilt from its text.
fn tree(k: &Kit, f: u32) -> (Vec<u32>, Vec<(String, Node)>) {
    let lex = lexicon::of(k.dialect()).expect("a reader");
    let text = &k.file(FileId(f)).text;
    let t = tokenize(text, lex);
    let a = parse(&t, lex);
    assert_eq!(a.len(), k.node_count(FileId(f)), "the same tree");
    // Every instance, by name, in source order.
    let mut out = Vec::new();
    let mut stmts = Vec::new();
    walk(&a, |n, _, _| {
        stmts.push(n);
        if a.tag(Node(n)) == Tag::Instance {
            let Header { name, .. } = a.header(Node(n));
            out.push((declare::word_text(&a, name).unwrap().to_string(), Node(n)));
        }
    });
    (stmts, out)
}

/// The binding of the instance named `name` in file `f`.
fn bound(k: &Kit, f: u32, name: &str) -> Binding {
    let (_, insts) = tree(k, f);
    let n = insts
        .iter()
        .find(|(s, _)| s == name)
        .expect("the instance")
        .1;
    k.binding(FileId(f), n).expect("an instance binds")
}

fn def_name(k: &Kit, b: Binding) -> (&str, &DefKind) {
    match b {
        Binding::Def(id) => (&k.definition(id).name, &k.definition(id).kind),
        b => panic!("no declaration: {b:?}"),
    }
}

fn codes(k: &Kit, f: u32) -> Vec<Code> {
    k.file(FileId(f)).diags.iter().map(|d| d.code).collect()
}

#[test]
fn spectre_masters_resolve_across_files() {
    // tests/fixtures/transpiler/top.scs and lib.scs; Spectre UG p.66: an
    // instance's master is a subcircuit, a model or a primitive; p.98:
    // `include` reads the file in place. `leaf` and `nch_mod` are declared
    // only in lib.scs. Checks: 8.
    let k = kit(
        &[("/k/top.scs", TOP_SCS), ("/k/lib.scs", LIB_SCS)],
        "/k/top.scs",
        Form::Deck,
        Dialect::Spectre,
    );
    assert_eq!(k.files().len(), 2);
    assert_eq!(k.file(FileId(1)).path, Path::new("/k/lib.scs"));
    for x in ["x1", "x2"] {
        let (name, kind) = def_name(&k, bound(&k, 0, x));
        assert_eq!(name, "leaf");
        assert_eq!(
            kind,
            &DefKind::Subckt {
                ports: vec!["p".into(), "n".into()],
                params: vec![]
            }
        );
    }
    let (name, kind) = def_name(&k, bound(&k, 0, "m1"));
    assert_eq!(name, "nch_mod");
    assert!(matches!(
        kind,
        DefKind::Model {
            kind: Some(Primitive::Mosfet),
            polarity: Some(Polarity::N),
            ..
        }
    ));
    assert_eq!(bound(&k, 0, "f1"), Binding::Primitive(Primitive::Cccs));
    assert_eq!(bound(&k, 1, "r0"), Binding::Primitive(Primitive::Resistor));
    assert!(codes(&k, 0).is_empty() && codes(&k, 1).is_empty());
}

#[test]
fn an_unknown_master_keeps_its_name_with_r0001() {
    // tests/fixtures/transpiler/f1.scs's `othercell`: declared nowhere.
    // Checks: 3.
    let src = "t\nsubckt cell (a b)\nx1 (a b) othercell w=1\nends\n";
    let k = kit(&[("/f1.scs", src)], "/f1.scs", Form::Deck, Dialect::Spectre);
    assert_eq!(bound(&k, 0, "x1"), Binding::Unknown);
    assert_eq!(codes(&k, 0), [Code::R0001]);
    let d = k.file(FileId(0)).diags[0];
    let t = tokenize(&k.file(FileId(0)).text, &lexicon::SPECTRE);
    assert_eq!(t.text(d.token), "othercell");
}

#[test]
fn a_bin_base_names_its_bins() {
    // ngspice manual §2.4 (model binning): `nch.1`, `nch.2` are bins of
    // `nch`, which an M card names. Checks: 3.
    let src = "t\n.model nch.1 nmos level=54 lmin=0 lmax=1\n.model nch.2 nmos level=54\nM1 d g s b nch w=1\n";
    let k = kit(&[("/a.sp", src)], "/a.sp", Form::Deck, Dialect::Ngspice);
    let Binding::Def(id) = bound(&k, 0, "M1") else {
        panic!("M1 binds")
    };
    let d = k.definition(id);
    assert_eq!((d.name.as_str(), d.base()), ("nch.1", "nch"));
    assert!(matches!(
        d.kind,
        DefKind::Model {
            kind: Some(Primitive::Mosfet),
            level: Some(54),
            ..
        }
    ));
}

#[test]
fn a_range_card_splits_where_its_model_is() {
    // ngspice manual ch.8: `QXXXXXXX nc nb ne <ns> <tj> mname <area>`. The
    // parser reads the longest node list (`mod` a node, `1.5` the model);
    // `mod` is a model, so the split is 3 nodes and `mod`. A card whose
    // model the parser found keeps its header. Checks: 4.
    let src = "t\n.model mod npn\nQ1 c b e mod 1.5\nQ2 c b e s mod\n";
    let k = kit(&[("/a.sp", src)], "/a.sp", Form::Deck, Dialect::Ngspice);
    let (_, insts) = tree(&k, 0);
    let (q1, q2) = (insts[0].1, insts[1].1);
    let s = k.split(FileId(0), q1).expect("a corrected split");
    assert_eq!(s.nodes, 3);
    assert!(matches!(def_name(&k, bound(&k, 0, "Q1")).0, "mod"));
    assert_eq!(k.split(FileId(0), q2), None);
    assert!(codes(&k, 0).is_empty());
}

#[test]
fn an_rlc_card_names_its_model_among_its_values() {
    // ngspice manual ch.3: `RXXXXXXX n+ n- <value> <mname> <l=length>`, the
    // model after the value or alone. `R3`'s `1k` names nothing: a value.
    // Checks: 5.
    let src = "t\n.model rmod r rsh=1\nR1 a b 1k rmod l=1u\nR2 a b rmod l=1u\nR3 a b 1k\n";
    let k = kit(&[("/a.sp", src)], "/a.sp", Form::Deck, Dialect::Ngspice);
    let (_, insts) = tree(&k, 0);
    for (i, name) in [(0, "R1"), (1, "R2")] {
        let s = k.split(FileId(0), insts[i].1).expect("a split");
        assert_eq!(s.nodes, 2);
        assert_eq!(def_name(&k, bound(&k, 0, name)).0, "rmod");
    }
    assert_eq!(bound(&k, 0, "R3"), Binding::Primitive(Primitive::Resistor));
}

#[test]
fn scopes_are_subckt_section_and_branch() {
    // ngspice manual §2.5 (`.subckt`), §2.7 (`.lib name` … `.endl`), §2.15
    // (`.if`/`.else`). Checks: 5.
    let src = "t\n.lib tt\n.subckt s a\nR1 a 0 1\n.ends\n.endl tt\n.if (x)\nR2 a 0 1\n.else\nR3 a 0 1\n.endif\nR4 a 0 1\n";
    let k = kit(&[("/a.sp", src)], "/a.sp", Form::Deck, Dialect::Ngspice);
    let (_, insts) = tree(&k, 0);
    let scope = |i: usize| k.scope(FileId(0), insts[i].1).expect("a statement");
    let Scope::Subckt(s) = scope(0) else {
        panic!("R1 in the subckt")
    };
    assert!(matches!(k.scope(FileId(0), s), Some(Scope::Section(_))));
    let (Scope::Branch(b2), Scope::Branch(b3)) = (scope(1), scope(2)) else {
        panic!("R2, R3 in branches")
    };
    assert_ne!(b2, b3);
    assert_eq!(scope(3), Scope::Top);
}

#[test]
fn a_subckt_local_model_is_seen_only_inside() {
    // ngspice manual §2.5: a `.model` inside a `.subckt` is local to it.
    // `M2` at the top does not see `m`. Checks: 3.
    let src = "t\n.subckt s d g\n.model m nmos\nM1 d g 0 0 m\n.ends\nM2 d g 0 0 m\n";
    let k = kit(&[("/a.sp", src)], "/a.sp", Form::Deck, Dialect::Ngspice);
    assert_eq!(def_name(&k, bound(&k, 0, "M1")).0, "m");
    assert_eq!(bound(&k, 0, "M2"), Binding::Unknown);
    assert_eq!(codes(&k, 0), [Code::R0001]);
}

#[test]
fn lib_calls_select_their_section() {
    // tests/fixtures/pdk_sections/models.lib: ngspice manual §2.7, `.lib
    // file section` reads that section of file; the `.include` reads the
    // file whole; a section the file lacks is R0010. Checks: 6.
    let models = "* m\n.lib tt\n.include 'shared.spice'\n.lib 'res.lib' res_tt\n.lib 'res.lib' res_xx\n.endl tt\n";
    let res = "* r\n.lib res_tt\n.param rsh=100\n.endl res_tt\n";
    let k = kit(
        &[
            ("/p/models.lib", models),
            ("/p/shared.spice", ".param t=1\n"),
            ("/p/res.lib", res),
        ],
        "/p/models.lib",
        Form::Library,
        Dialect::Ngspice,
    );
    let paths: Vec<&Path> = k.files().iter().map(|f| f.path.as_path()).collect();
    assert_eq!(
        paths,
        ["/p/models.lib", "/p/shared.spice", "/p/res.lib"].map(Path::new)
    );
    let (stmts, _) = tree(&k, 0);
    let targets: Vec<Target> = stmts
        .iter()
        .filter_map(|&n| k.target(FileId(0), Node(n)))
        .collect();
    assert_eq!(targets.len(), 3);
    assert_eq!((targets[0].file, targets[0].section), (FileId(1), None));
    let tt = k
        .definitions()
        .iter()
        .find(|d| d.kind == DefKind::Section && d.name == "res_tt")
        .expect("res_tt");
    assert_eq!(
        targets[1],
        Target {
            file: FileId(2),
            section: Some(tt.node)
        }
    );
    assert_eq!(codes(&k, 0), [Code::R0010]);
}

#[test]
fn absent_and_refused_files_are_rows_with_codes() {
    // ngspice manual §2.6: `.include` names a file; one the kit lacks is
    // R0006, one the loader refuses R0007, each at its include. Checks: 4.
    let src = "* a\n.include missing.sp\n.include /outside/x.sp\n";
    let k = kit(&[("/a.sp", src)], "/a.sp", Form::Library, Dialect::Ngspice);
    let roles: Vec<Role> = k.files().iter().map(|f| f.role).collect();
    assert_eq!(
        roles,
        [
            Role::Library,
            Role::Unread(LoadError::NotFound),
            Role::Unread(LoadError::Refused)
        ]
    );
    assert_eq!(codes(&k, 0), [Code::R0006, Code::R0007]);
    let none = kit(&[], "/none.sp", Form::Deck, Dialect::Ngspice);
    assert_eq!(codes(&none, 0), [Code::R0006]);
    assert!(none.definitions().is_empty());
}

#[test]
fn the_include_chain_stops_at_its_cap() {
    // No clause: the kit's own cap, DEPTH_MAX. A chain f0 -> f1 -> ... reads
    // DEPTH_MAX + 1 files; the last one's include is R0008. Checks: 2.
    let n = usize::from(DEPTH_MAX) + 3;
    let texts: Vec<(String, String)> = (0..n)
        .map(|i| {
            (
                format!("/f{i}.sp"),
                format!("* f{i}\n.include f{}.sp\n", i + 1),
            )
        })
        .collect();
    let files: Vec<(&str, &str)> = texts
        .iter()
        .map(|(p, s)| (p.as_str(), s.as_str()))
        .collect();
    let k = kit(&files, "/f0.sp", Form::Library, Dialect::Ngspice);
    assert_eq!(k.files().len(), usize::from(DEPTH_MAX) + 1);
    assert_eq!(codes(&k, u32::from(DEPTH_MAX)), [Code::R0008]);
}

#[test]
fn the_file_count_stops_at_its_cap() {
    // No clause: the kit's own cap, FILES_MAX. An entry including FILES_MAX
    // files reads FILES_MAX in all; the one past it is R0009. Checks: 2.
    let mut top = String::from("* top\n");
    for i in 0..FILES_MAX {
        top.push_str(&format!(".include f{i}.sp\n"));
    }
    let map: HashMap<PathBuf, String> = (0..FILES_MAX)
        .map(|i| (PathBuf::from(format!("/f{i}.sp")), String::from("* f\n")))
        .chain([(PathBuf::from("/top.sp"), top)])
        .collect();
    let mut load = |p: &Path| map.get(p).cloned().ok_or(LoadError::NotFound);
    let k = Kit::load(
        &[PathBuf::from("/top.sp")],
        Form::Library,
        Dialect::Ngspice,
        &mut load,
    );
    assert_eq!(k.files().len(), FILES_MAX);
    assert_eq!(codes(&k, 0), [Code::R0009]);
}

#[test]
fn duplicates_ambiguity_arity_and_values() {
    // ngspice manual §2.5: one name per definition in a scope (R0002, the
    // second `.subckt s`); Spectre UG p.66: a master naming a subckt and a
    // model equally near is ambiguous (R0003, `x1`); §2.4.3: `X` nodes
    // match the subckt's ports (R0004, `x2`); a master naming a parameter
    // is no device (R0005, `x3`). Checks: 2.
    let a = "t\nsubckt s (a b)\nends\nsubckt s (a b)\nends\nx1 (a b) dual\nx2 (a b c) s\nparameters p=1\nx3 (a b) p\n";
    let b = "// b\nsubckt dual (a b)\nends\nmodel dual bsim4\n";
    let k = kit(
        &[
            ("/a.scs", &format!("{a}include \"b.scs\"\n")),
            ("/b.scs", b),
        ],
        "/a.scs",
        Form::Deck,
        Dialect::Spectre,
    );
    assert_eq!(
        codes(&k, 0),
        [Code::R0002, Code::R0003, Code::R0004, Code::R0005]
    );
    assert!(matches!(
        def_name(&k, bound(&k, 0, "x1")).1,
        DefKind::Subckt { .. }
    ));
}

#[test]
fn a_spice_letter_picks_a_subckt_or_a_model() {
    // ngspice manual §2.4.3: `XYYYYYYY N1 <N2 N3 ...> SUBNAM`, a subcircuit;
    // sky130's res.model.spice declares both `.model short r` and `.subckt
    // short`, which SPICE keeps apart.
    let src = "t\n.model short r r=0.01\n.subckt short 1 2\nR0 1 2 short\n.ends\nX1 a b short\n";
    let k = kit(&[("/a.sp", src)], "/a.sp", Form::Deck, Dialect::Ngspice);
    assert!(matches!(
        def_name(&k, bound(&k, 0, "X1")).1,
        DefKind::Subckt { .. }
    ));
    assert!(codes(&k, 0).is_empty());
}

#[test]
fn vacask_masters_are_models() {
    // VACASK `share/doc/vacask/demo`: `model nmos sp_bsim4v8 type=1`, an
    // instance's master is a model; `type=-1` is a PMOS. Checks: 3.
    let src = "t\nmodel nm sp_bsim4v8 type=1\nmodel pm sp_bsim4v8 type=-1\nm1 (d g s b) nm\nm2 (d g s b) pm\n";
    let k = kit(&[("/a.sim", src)], "/a.sim", Form::Deck, Dialect::Vacask);
    let pol = |name: &str| match def_name(&k, bound(&k, 0, name)).1 {
        DefKind::Model { kind, polarity, .. } => (*kind, *polarity),
        other => panic!("{other:?}"),
    };
    assert_eq!(pol("m1"), (Some(Primitive::Mosfet), Some(Polarity::N)));
    assert_eq!(pol("m2"), (Some(Primitive::Mosfet), Some(Polarity::P)));
    assert!(codes(&k, 0).is_empty());
}

#[test]
fn a_spice_region_model_serves_a_spectre_instance() {
    // Spectre UG p.53: `simulator lang=spice` reads SPICE until `lang=spectre`;
    // its `.model` is a model of the file. Checks: 1.
    let src =
        "t\nsimulator lang=spice\n.model dd d is=1e-14\nsimulator lang=spectre\nd1 (a 0) dd\n";
    let k = kit(&[("/a.scs", src)], "/a.scs", Form::Deck, Dialect::Spectre);
    assert!(matches!(
        def_name(&k, bound(&k, 0, "d1")).1,
        DefKind::Model {
            kind: Some(Primitive::Diode),
            ..
        }
    ));
}

#[test]
fn roles_and_verilog_a() {
    // A deck holds an analysis or a stimulus; a library only definitions
    // (transpiler-architecture.md §2.5). Spectre UG p.118: `ahdl_include`
    // names a Verilog-A source, loaded and not read. Checks: 3.
    let deck = "t\nahdl_include \"dev.va\"\ninclude \"lib.scs\"\nv1 (a 0) vsource dc=1\ntran1 tran stop=1n\n";
    let k = kit(
        &[
            ("/d.scs", deck),
            ("/lib.scs", LIB_SCS),
            ("/dev.va", "module dev(a); endmodule\n"),
        ],
        "/d.scs",
        Form::Deck,
        Dialect::Spectre,
    );
    let roles: Vec<Role> = k.files().iter().map(|f| f.role).collect();
    assert_eq!(roles, [Role::Deck, Role::VerilogA, Role::Library]);
    assert!(k.file(FileId(1)).text.starts_with("module dev"));
    assert!(codes(&k, 0).is_empty());
}

#[test]
fn nesting_ten_thousand_deep_resolves_without_recursion() {
    // Rule 7 of the compiler-construction skill: every walk over input
    // depth is a heap worklist. 10⁴ nested `.if`s inside 10⁴ nested
    // `.subckt`s, in Debug on a test thread's 2 MiB stack. Checks: 2.
    let n = 10_000;
    let mut src = String::from("t\n.model m nmos\n");
    for k in 0..n {
        src.push_str(&format!(".subckt s{k} a\n.if (x)\n"));
    }
    src.push_str("M1 a a a a m\n");
    for _ in 0..n {
        src.push_str(".endif\n.ends\n");
    }
    let k = kit(
        &[("/deep.sp", &src)],
        "/deep.sp",
        Form::Deck,
        Dialect::Ngspice,
    );
    assert_eq!(def_name(&k, bound(&k, 0, "M1")).0, "m");
    assert!(codes(&k, 0).is_empty());
}
