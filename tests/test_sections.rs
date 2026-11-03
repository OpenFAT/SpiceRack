//! Library sections (`.lib name` ... `.endl`, Spectre `library`/`section`,
//! VACASK `section`) through the IR: read, written in every dialect, read
//! back.
//!
//! Oracles: what the hand-written fixtures under
//! `tests/fixtures/pdk_sections/` hold, section by section (counted by hand
//! from their text); read -> write -> read as a fixpoint in each dialect;
//! for the cross-dialect trips, the sections' meaning ([`meaning`]) with the
//! dialect differences listed at the test; and ngspice and ESPice running a
//! corner from the sections SpiceRack wrote against the same corner from the
//! fixture. Real PDK files: `test_sections_pdk.rs`. Unsupported fails, never
//! skips: run in `nix develop .#testing`.

use std::path::Path;

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::parse;
use spicerack::ir::*;

#[path = "common/sections.rs"]
mod sections;
use sections::{Meaning, meaning};

const SPICE: &str = include_str!("fixtures/pdk_sections/models.lib");
const SPECTRE: &str = include_str!("fixtures/pdk_sections/models.scs");
const BARE: &str = include_str!("fixtures/pdk_sections/bare.scs");

// ---- private ----

/// What the fixture's `tt` section holds, by hand from `models.lib`.
#[test]
fn spice_sections_hold_their_cards_and_nothing_leaks_to_the_top() {
    let (ir, report) = parse(SPICE, "models", Dialect::Ngspice).unwrap();
    assert_eq!(report, Default::default());
    let names: Vec<&str> = ir.sections.iter().map(|s| s.body.name.as_str()).collect();
    assert_eq!(names, ["tt", "ff"]);
    let top = &ir.top;
    assert!(
        top.parameters.is_empty()
            && top.models.is_empty()
            && top.includes.is_empty()
            && top.libs.is_empty()
            && top.raw_spice.is_empty(),
        "{top:?}"
    );
    assert!(ir.subcircuit_defs.is_empty());

    let tt = &ir.sections[0];
    assert_eq!(tt.library, None);
    let params: Vec<(&str, &str)> = tt
        .body
        .parameters
        .iter()
        .map(|p| (p.name.as_str(), p.default.as_deref().unwrap()))
        .collect();
    assert_eq!(params, [("vth0n", "0.45"), ("u0n", "0.03")]);
    assert_eq!(tt.body.includes, ["shared.spice"]);
    assert_eq!(
        tt.body.libs,
        [("res.lib".to_string(), "res_tt".to_string())]
    );
    let models: Vec<(&str, &str, usize)> = tt
        .body
        .models
        .iter()
        .map(|m| (m.name.as_str(), m.kind.as_str(), m.parameters.len()))
        .collect();
    assert_eq!(models, [("nch.1", "nmos", 9), ("nch.2", "nmos", 9)]);
    assert_eq!(
        tt.body.models[1].parameters[6],
        ("vth0".to_string(), "'vth0n+0.01'".to_string())
    );
    let [sc] = &tt.subcircuit_defs[..] else {
        panic!("{:?}", tt.subcircuit_defs)
    };
    assert_eq!(sc.name, "nch_lvt");
    assert_eq!(
        sc.ports.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["d", "g", "s", "b"]
    );
    assert_eq!(
        sc.parameters,
        [
            ParamDef {
                name: "w".into(),
                default: Some("1u".into())
            },
            ParamDef {
                name: "l".into(),
                default: Some("100n".into())
            }
        ]
    );
    assert!(matches!(&sc.components[..], [Component::Mosfet { model, .. }] if model == "nch"));
    assert!(tt.body.raw_spice.is_empty() && tt.body.components.is_empty());
    assert_eq!(
        ir.sections[1].body.parameters[0].default.as_deref(),
        Some("0.40")
    );
}

/// Sections do not nest: a `.lib name` inside one is reported and kept, in
/// the section, and the first `.endl` closes it (as ngspice reads it).
#[test]
fn a_nested_section_is_unparsed_and_kept_in_its_section() {
    let deck = "t\n.lib a\n.param x=1\n.lib b\n.param y=2\n.endl b\n.param z=3\n.endl a\n";
    let (ir, report) = parse(deck, "t", Dialect::Ngspice).unwrap();
    assert_eq!(report.unparsed, [".lib b", ".endl a"]);
    let [a] = &ir.sections[..] else {
        panic!("{:?}", ir.sections)
    };
    assert_eq!(a.body.raw_spice, [".lib b"]);
    assert_eq!(
        a.body
            .parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["x", "y"]
    );
    // `.endl b` closed `a`: `.param z` is the top's, `.endl a` a stray.
    assert_eq!(ir.top.parameters[0].name, "z");
    assert_eq!(ir.top.raw_spice, [".endl a"]);
    // A section only a deck's top defines: one in a `.subckt` stays raw.
    let (s, _) = parse(
        "t\n.subckt x a\n.lib c\n.param q=1\n.endl c\n.ends x\n",
        "t",
        Dialect::Ngspice,
    )
    .unwrap();
    assert!(s.sections.is_empty());
    assert_eq!(
        s.subcircuit_defs[0].raw_spice,
        [".lib c", ".param q=1", ".endl c"]
    );
}

/// A card that means something to the whole deck stays raw in a section
/// instead of leaking out of it.
#[test]
fn a_testbench_card_in_a_section_stays_in_it() {
    let (ir, _) = parse(
        "t\n.lib a\n.global vdd\n.func f(x) {x*2}\n.option reltol=1e-4\n.op\n.endl a\n",
        "t",
        Dialect::Ngspice,
    )
    .unwrap();
    assert_eq!(
        ir.sections[0].body.raw_spice,
        [
            ".global vdd",
            ".func f(x) {x*2}",
            ".option reltol=1e-4",
            ".op"
        ]
    );
    assert!(ir.globals.is_empty() && ir.functions.is_empty());
    assert_eq!(ir.testbench.unwrap(), Testbench::default());
}

/// Read -> write -> read gives the same IR, and the same text again.
#[test]
fn each_dialect_is_a_fixpoint() {
    for d in [
        Dialect::Ngspice,
        Dialect::Hspice,
        Dialect::Xyce,
        Dialect::Espice,
    ] {
        let (ir, _) = parse(SPICE, "models", d).unwrap();
        assert_eq!(ir.sections.len(), 2, "{d:?}");
        fixpoint(&ir, d);
    }
    let (spectre, _) = parse(SPECTRE, "models", Dialect::Spectre).unwrap();
    fixpoint(&spectre, Dialect::Spectre);
    // VACASK has no PDK fixture of its own: the SPICE one, written as VACASK.
    let (spice, _) = parse(SPICE, "models", Dialect::Ngspice).unwrap();
    let text = netlist(&spice, Dialect::Vacask).unwrap();
    let (vacask, report) = parse(&text, "models", Dialect::Vacask).unwrap();
    // Known gap: VACASK selects no bin by geometry, so the wrappers' `m0 ...
    // nch` names no VACASK model and reads back as raw text, one per corner.
    assert_eq!((report.unparsed.len(), report.raw), (0, 2), "{text}");
    let unbinned = |ir: &CircuitIR| {
        meaning(ir)
            .into_iter()
            .map(|m| Meaning {
                subckts: m
                    .subckts
                    .into_iter()
                    .map(|(n, p, d, _)| (n, p, d, 0))
                    .collect(),
                ..m
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(unbinned(&vacask), unbinned(&spice), "{text}");
    fixpoint(&vacask, Dialect::Vacask);
}

/// What the fixture's Spectre `tt` section holds, by hand from `models.scs`.
#[test]
fn spectre_sections_hold_their_cards() {
    let (ir, report) = parse(SPECTRE, "models", Dialect::Spectre).unwrap();
    assert_eq!(report, Default::default());
    assert_eq!(
        ir.top,
        Subcircuit {
            name: "models".into(),
            ..Default::default()
        }
    );
    let names: Vec<(Option<&str>, &str)> = ir
        .sections
        .iter()
        .map(|s| (s.library.as_deref(), s.body.name.as_str()))
        .collect();
    assert_eq!(names, [(Some("mylib"), "tt"), (Some("mylib"), "ff")]);
    let tt = &ir.sections[0].body;
    assert_eq!(
        tt.parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["vth0n", "u0n"]
    );
    assert_eq!(tt.includes, ["shared.scs"]);
    assert_eq!(tt.libs, [("res.scs".to_string(), "res_tt".to_string())]);
    // The model group is one model per bin, as SPICE names bins.
    let models: Vec<(&str, &str)> = tt
        .models
        .iter()
        .map(|m| (m.name.as_str(), m.kind.as_str()))
        .collect();
    assert_eq!(
        models,
        [("nch", "nmos"), ("nchb.1", "nmos"), ("nchb.2", "nmos")]
    );
    let p = |k: &str| {
        tt.models[2]
            .parameters
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(
        (p("level"), p("lmin"), p("vth0"), p("type")),
        (Some("54"), Some("1u"), Some("vth0n+0.01"), None)
    );
    assert!(tt.raw_spice.is_empty(), "{:?}", tt.raw_spice);
    let sc = &ir.sections[0].subcircuit_defs[0];
    assert_eq!(
        (sc.name.as_str(), sc.parameters.len(), sc.components.len()),
        ("nch_lvt", 2, 1)
    );
}

/// Bare Spectre sections (no `library`, as a commercial PDK's corner files
/// hold them) are sections like any other and are written back bare; a
/// `simulator lang=spice` region in one stays in it, kept as the SPICE text
/// it is (by hand from `bare.scs`).
#[test]
fn bare_spectre_sections_with_a_spice_region() {
    let (ir, report) = parse(BARE, "bare", Dialect::Spectre).unwrap();
    assert_eq!(
        report,
        spicerack::import::Report {
            unparsed: vec![],
            raw: 1
        }
    );
    assert_eq!(
        ir.top,
        Subcircuit {
            name: "bare".into(),
            ..Default::default()
        }
    );
    assert_eq!(ir.testbench.as_ref().unwrap(), &Testbench::default());
    let names: Vec<(Option<&str>, &str)> = ir
        .sections
        .iter()
        .map(|s| (s.library.as_deref(), s.body.name.as_str()))
        .collect();
    assert_eq!(names, [(None, "tt"), (None, "ff")]);
    let tt = &ir.sections[0].body;
    assert_eq!(
        tt.parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["vth0n", "u0n", "after"]
    );
    assert_eq!(tt.includes, ["shared.scs"]);
    assert_eq!(
        tt.raw_spice,
        [
            "simulator lang=spice",
            ".param rsh_spice=100 rvc=0.01",
            "simulator lang=spectre"
        ]
    );
    let text = fixpoint(&ir, Dialect::Spectre);
    assert!(
        !text.contains("library") && text.contains("\nsection tt\n"),
        "{text}"
    );
}

/// The value forms sky130 uses read as ngspice evaluates them (each checked
/// on ngspice 45 by hand first): `{"e"}` is `{e}`; a braced group with an
/// unbraced tail is one expression; a lone `+` between two assignments
/// separates them; an empty `.param` sets nothing; a `.param` in a
/// `.subckt` is a local. ngspice runs the deck written from the IR to the same currents.
/// A statistical function reaches HSPICE and is refused, by name, where the
/// dialect has none.
#[test]
fn ngspice_value_forms_and_statistical_functions() {
    let deck = "forms\n.param x=2\n.param a = '-1*x' + b = '0 - 2.5'\n.param c = {1+x} + x*0*AGAUSS(0,1,1)\n.param\n.model rr r rsh=10 dw={\"-a-b/8\"}\n.subckt two p n\n.param k={x*2}\nR1 p n {k+c}\n.ends twoV1 1 0 1\nR1 1 0 rr l=10 w=1\nX1 1 0 two\nVb 2 0 1\nRb 2 0 {b+10}\n.op\n.end\n";
    let (ir, report) = parse(deck, "forms", Dialect::Ngspice).unwrap();
    assert_eq!(report, Default::default());
    let top: Vec<(&str, &str)> = ir
        .top
        .parameters
        .iter()
        .map(|p| (p.name.as_str(), p.default.as_deref().unwrap()))
        .collect();
    assert_eq!(
        top,
        [
            ("x", "2"),
            ("a", "'-1*x'"),
            ("b", "'0 - 2.5'"),
            ("c", "{(1+x) + x*0*AGAUSS(0,1,1)}")
        ]
    );
    assert_eq!(
        ir.top.models[0].parameters[1],
        ("dw".to_string(), "{-a-b/8}".to_string())
    );
    assert_eq!(
        ir.subcircuit_defs[0].locals,
        [ParamDef {
            name: "k".into(),
            default: Some("{x*2}".into())
        }]
    );
    let currents = |deck: &str| {
        let raw = Dialect::Ngspice
            .run(deck)
            .unwrap_or_else(|e| panic!("{e}\n{deck}"));
        raw.variables
            .iter()
            .zip(&raw.real_data)
            .filter(|(v, _)| v.name.contains('#'))
            .map(|(v, d)| (v.name.clone(), d[0].to_bits()))
            .collect::<Vec<_>>()
    };
    let written = fixpoint(&ir, Dialect::Ngspice);
    assert_eq!(currents(&written), currents(deck));

    let (stat, _) = parse(
        "t\n.subckt m a b\n.param mis='agauss(0,1m,1)'\nR1 a b {1k+mis}\n.ends m\n",
        "t",
        Dialect::Ngspice,
    )
    .unwrap();
    assert!(
        netlist(&stat, Dialect::Hspice)
            .unwrap()
            .contains(".param mis='agauss(0,1m,1)'")
    );
    for d in [Dialect::Spectre, Dialect::Vacask, Dialect::Ltspice] {
        let err = netlist(&stat, d).unwrap_err().to_string();
        assert!(
            err.contains("statistical function agauss()"),
            "{d:?}: {err}"
        );
    }
}

/// LTspice reads `.lib tt` as the file `tt`: no section form, so the
/// codegen refuses rather than write a different deck.
#[test]
fn ltspice_refuses_a_section() {
    let (ir, _) = parse(SPICE, "models", Dialect::Ngspice).unwrap();
    let err = netlist(&ir, Dialect::Ltspice).unwrap_err().to_string();
    assert!(err.contains("library section 'tt'"), "{err}");
}

/// SPICE -> Spectre -> IR and Spectre -> SPICE -> IR keep every section's
/// parameters, models, includes, library calls and subcircuits. The trip
/// changes spellings, not meaning, except for the last two:
/// - numbers are respelled (`0.45` is `450m`) and SPICE's expression
///   delimiters dropped (`'vth0n'` is `vth0n`); [`meaning`] compares values;
/// - Spectre writes a SPICE IR's model parameter names lowercased and moves
///   LEVEL first, its `type=` the polarity;
/// - SPICE has no library: a SPICE section is a bare Spectre one, and a
///   Spectre library's name is dropped;
/// - include paths are kept as written: a SPICE file is not converted by
///   including it from Spectre;
/// - SPICE's bins (`nch.1`, `nch.2`) are a Spectre model group
///   (`model nch bsim4 { 1: ... }`) and back; a SPICE instance naming one
///   bin (`nch.1`) has no Spectre spelling.
#[test]
fn sections_cross_between_spice_and_spectre() {
    let (spice, _) = parse(SPICE, "models", Dialect::Ngspice).unwrap();
    let scs = netlist(&spice, Dialect::Spectre).unwrap();
    assert!(
        scs.contains("\nsection tt\n")
            && scs.contains("\nendsection ff\n")
            && !scs.contains("library"),
        "{scs}"
    );
    let (back, report) = parse(&scs, "models", Dialect::Spectre).unwrap();
    assert_eq!(report, Default::default(), "{scs}");
    assert_eq!(meaning(&back), meaning(&spice), "{scs}");

    assert!(
        scs.contains("model nch bsim4 {\n1: type=n version=4.5 lmin=100n"),
        "{scs}"
    );

    let (spectre, _) = parse(SPECTRE, "models", Dialect::Spectre).unwrap();
    let sp = netlist(&spectre, Dialect::Ngspice).unwrap();
    assert!(
        sp.contains("\n.model nchb.2 nmos (level=54 lmin=1u lmax=100u vth0=vth0n+0.01)\n"),
        "{sp}"
    );
    let (back, report) = parse(&sp, "models", Dialect::Ngspice).unwrap();
    assert_eq!(report, Default::default(), "{sp}");
    assert_eq!(meaning(&back), meaning(&spectre), "{sp}");
    assert!(back.sections.iter().all(|s| s.library.is_none()));
}

/// ngspice and ESPice each run the `tt` and `ff` corners from the sections
/// SpiceRack wrote for them and get the drain current they get from the
/// hand-written file, bit for bit; `ff` (lower VTH0, higher U0) conducts
/// more than `tt`.
#[test]
fn simulators_run_each_corner_from_the_written_sections() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdk_sections");
    let (ir, _) = parse(SPICE, "models", Dialect::Ngspice).unwrap();
    for sim in [Dialect::Ngspice, Dialect::Espice] {
        let dir = tempfile::tempdir().unwrap();
        for f in ["shared.spice", "res.lib"] {
            std::fs::copy(fixtures.join(f), dir.path().join(f)).unwrap();
        }
        std::fs::write(dir.path().join("models.lib"), netlist(&ir, sim).unwrap()).unwrap();
        let id = |lib: &Path, corner: &str| {
            let deck = format!(
                "corner\n.lib '{}' {corner}\nX1 d g 0 0 nch_lvt w=1u l=100n\nVd d 0 1\nVg g 0 1\n.op\n.end\n",
                lib.display()
            );
            let raw = sim
                .run(&deck)
                .unwrap_or_else(|e| panic!("{sim:?}: {e}\n{deck}"));
            let i = raw
                .variables
                .iter()
                .position(|v| {
                    v.name.eq_ignore_ascii_case("vd#branch") || v.name.eq_ignore_ascii_case("i(vd)")
                })
                .unwrap_or_else(|| panic!("{:?}", raw.variables));
            -raw.real_data[i][0]
        };
        for corner in ["tt", "ff"] {
            let (want, got) = (
                id(&fixtures.join("models.lib"), corner),
                id(&dir.path().join("models.lib"), corner),
            );
            assert!(want > 0.0, "{sim:?} {corner}: {want}");
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "{sim:?} {corner}: {got} != {want}"
            );
        }
        assert!(
            id(&fixtures.join("models.lib"), "ff") > id(&fixtures.join("models.lib"), "tt"),
            "{sim:?}"
        );
    }
}

/// Emitting `ir` in `d` and reading it back gives `ir` again, and emitting
/// that gives the same text. Returns the text.
fn fixpoint(ir: &CircuitIR, d: Dialect) -> String {
    let text = netlist(ir, d).unwrap_or_else(|e| panic!("{d:?}: {e}"));
    let (again, _) = parse(&text, &ir.top.name, d).unwrap();
    assert_eq!(&again.sections, &ir.sections, "{d:?}:\n{text}");
    assert_eq!(&again, ir, "{d:?}:\n{text}");
    assert_eq!(netlist(&again, d).unwrap(), text, "{d:?}");
    text
}
