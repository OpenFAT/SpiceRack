//! Library sections (`.lib name` ... `.endl`, Spectre `library`/`section`,
//! VACASK `section`) through the transpiler: read, written in every
//! dialect, read back.
//!
//! Oracles: what the hand-written fixtures under
//! `tests/fixtures/pdk_sections/` hold, section by section (counted by hand
//! from their text, read off the deck written in the fixture's own
//! dialect); read -> write -> read -> write as a text fixpoint in each
//! dialect; for the cross-dialect trips, the sections' meaning
//! ([`meaning`], of each side written as ngspice) with the dialect
//! differences listed at the test; and ngspice and ESPice running a corner
//! from the sections SpiceRack wrote against the same corner from the
//! fixture. Real PDK files: `test_sections_pdk.rs`. Unsupported fails, never
//! skips: run in `nix develop .#testing`.

use std::path::Path;

use spicerack::syntax::Dialect;
use spicerack::syntax::diag::Code;
use spicerack::syntax::render::{Written, render_text};

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
    let w = written(SPICE, Dialect::Ngspice, "models", Dialect::Ngspice);
    assert_eq!(codes(&w), [] as [Code; 0], "{}", w.notes.messages);
    // Nothing between the title and the first section, nothing after the
    // last but `.end`.
    assert!(w.text.starts_with("* models\n\n.lib tt\n"), "{}", w.text);
    assert!(w.text.ends_with(".endl ff\n.end"), "{}", w.text);
    let m = meaning(&w.text);
    let names: Vec<&str> = m.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["tt", "ff"]);
    let tt = &m[0];
    assert_eq!(tt.params, pairs(&[("vth0n", "4.5e-1"), ("u0n", "3e-2")]));
    assert_eq!(tt.includes, ["shared.spice"]);
    assert_eq!(tt.libs, pairs(&[("res.lib", "res_tt")]));
    let models: Vec<(&str, &str, usize)> = tt
        .models
        .iter()
        .map(|(n, k, p)| (n.as_str(), k.as_str(), p.len()))
        .collect();
    // Nine parameters each, LEVEL left out.
    assert_eq!(models, [("nch.1", "nmos", 8), ("nch.2", "nmos", 8)]);
    assert!(
        tt.models[1]
            .2
            .contains(&("vth0".into(), "vth0n+0.01".into()))
    );
    assert_eq!(
        tt.subckts,
        [(
            "nch_lvt".to_string(),
            ["d", "g", "s", "b"].map(String::from).to_vec(),
            pairs(&[("w", "1e-6"), ("l", "1e-7")]),
            1
        )]
    );
    assert!(
        w.text.contains("\nM0 d g s b nch w='w' l='l'\n"),
        "{}",
        w.text
    );
    assert_eq!(m[1].params[0], ("vth0n".into(), "4e-1".into()));
}

/// Sections do not nest: a `.lib name` inside one is noted and kept, in the
/// section, and the first `.endl` closes it (as ngspice reads it).
#[test]
fn a_nested_section_is_unparsed_and_kept_in_its_section() {
    let deck = "t\n.lib a\n.param x=1\n.lib b\n.param y=2\n.endl b\n.param z=3\n.endl a\n";
    let w = written(deck, Dialect::Ngspice, "t", Dialect::Ngspice);
    // `.lib b` is a section inside a section, kept as text (line 4); `.endl
    // a` a closer with no block open, kept as text at the top (line 8).
    assert_eq!(
        codes(&w),
        [
            Code::P0001,
            Code::P0009,
            Code::W0027,
            Code::P0004,
            Code::W0027
        ],
        "{}",
        w.notes.messages
    );
    // `.endl b` closed `a`: `.param z` is the top's, `.endl a` a stray.
    assert_eq!(
        w.text,
        "* t\n.param z=3\n\n.lib a\n.param x=1\n.param y=2\n.lib b\n.endl a\n.endl a\n.end"
    );
    // A section only a deck's top defines: one in a `.subckt` stays text.
    let s = written(
        "t\n.subckt x a\n.lib c\n.param q=1\n.endl c\n.ends x\n",
        Dialect::Ngspice,
        "t",
        Dialect::Ngspice,
    );
    assert!(
        s.text
            .contains(".subckt x a\n.lib c\n.param q=1\n.endl c\n.ends x\n"),
        "{}",
        s.text
    );
    assert_eq!(codes(&s), [Code::W0027], "{}", s.notes.messages);
}

/// A card that means something to the whole deck stays text in a section
/// instead of leaking out of it; a function is the section's.
#[test]
fn a_testbench_card_in_a_section_stays_in_it() {
    let w = written(
        "t\n.lib a\n.global vdd\n.func f(x) {x*2}\n.option reltol=1e-4\n.op\n.endl a\n",
        Dialect::Ngspice,
        "t",
        Dialect::Ngspice,
    );
    assert_eq!(
        w.text,
        "* t\n\n.lib a\n.func f(x) {x*2}\n.global vdd\n.option reltol=1e-4\n.op\n.endl a\n.end"
    );
    assert_eq!(codes(&w), [Code::W0027; 3], "{}", w.notes.messages);
}

/// Read -> write -> read -> write gives the same text in each dialect.
#[test]
fn each_dialect_is_a_fixpoint() {
    for d in [
        Dialect::Ngspice,
        Dialect::Hspice,
        Dialect::Xyce,
        Dialect::Espice,
    ] {
        let text = fixpoint(SPICE, d, "models");
        let back = written(&text, d, "models", Dialect::Ngspice);
        assert_eq!(meaning(&back.text).len(), 2, "{d:?}");
    }
    fixpoint(SPECTRE, Dialect::Spectre, "models");
    // VACASK has no PDK fixture of its own: the SPICE one, written as VACASK.
    let text = written(SPICE, Dialect::Ngspice, "models", Dialect::Vacask).text;
    // Known gap: VACASK selects no bin by geometry, so the wrappers' `m0 ...
    // nch` names no VACASK model and reads back as text, one per corner.
    let again = written(&text, Dialect::Vacask, "models", Dialect::Vacask);
    let kept = codes(&again).iter().filter(|&&c| c == Code::W0027).count();
    assert_eq!(kept, 2, "{text}");
    // The rest has the SPICE meaning: the kept lines left out, and with
    // them the wrappers' element counts.
    let rest: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("m0 "))
        .map(|l| format!("{l}\n"))
        .collect();
    let unbinned = |text: &str| {
        meaning(text)
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
    let back = written(&rest, Dialect::Vacask, "models", Dialect::Ngspice);
    let spice = written(SPICE, Dialect::Ngspice, "models", Dialect::Ngspice);
    assert_eq!(unbinned(&back.text), unbinned(&spice.text), "{text}");
    fixpoint(&text, Dialect::Vacask, "models");
}

/// What the fixture's Spectre `tt` section holds, by hand from `models.scs`.
#[test]
fn spectre_sections_hold_their_cards() {
    let w = written(SPECTRE, Dialect::Spectre, "models", Dialect::Spectre);
    assert_eq!(codes(&w), [] as [Code; 0], "{}", w.notes.messages);
    // Nothing at the top; both sections in their library.
    assert!(
        w.text
            .starts_with("// models\n\nsimulator lang=spectre\n\n\nlibrary mylib\nsection tt\n"),
        "{}",
        w.text
    );
    assert!(
        w.text.contains("\nendsection tt\nsection ff\n")
            && w.text.contains("\nendsection ff\nendlibrary mylib\n"),
        "{}",
        w.text
    );
    let tt = &w.text[..w.text.find("endsection tt").unwrap()];
    for line in [
        "parameters vth0n=450m u0n=30m",
        "include \"shared.scs\"",
        "include \"res.scs\" section=res_tt",
        // The model group, one model per bin, as SPICE names bins.
        "model nch bsim4 type=n version=4.5 vth0=vth0n u0=u0n toxe=4n",
        "model nchb bsim4 {",
        "2: type=n lmin=1u lmax=100u vth0=vth0n+0.01",
        "subckt nch_lvt (d g s b)",
        "parameters w=1u l=100n",
        "m0 (d g s b) nch w=w l=l",
    ] {
        assert!(tt.lines().any(|l| l == line), "{line}\n{}", w.text);
    }
    let sp = written(SPECTRE, Dialect::Spectre, "models", Dialect::Ngspice);
    let models: Vec<(String, String)> = meaning(&sp.text)[0]
        .models
        .iter()
        .map(|(n, k, _)| (n.clone(), k.clone()))
        .collect();
    assert_eq!(
        models,
        [("nch", "nmos"), ("nchb.1", "nmos"), ("nchb.2", "nmos")]
            .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    assert!(
        sp.text
            .contains("\n.model nchb.2 nmos (level=54 lmin=1u lmax=100u vth0=vth0n+0.01)\n")
    );
}

/// Bare Spectre sections (no `library`, as a commercial PDK's corner files
/// hold them) are sections like any other and are written back bare; a
/// `simulator lang=spice` region in one stays in it, kept as the SPICE text
/// it is (by hand from `bare.scs`).
#[test]
fn bare_spectre_sections_with_a_spice_region() {
    let text = fixpoint(BARE, Dialect::Spectre, "bare");
    let w = written(BARE, Dialect::Spectre, "bare", Dialect::Spectre);
    assert_eq!(codes(&w), [Code::W0027], "{}", w.notes.messages);
    assert_eq!(
        text,
        "// bare\n\nsimulator lang=spectre\n\nsection tt\ninclude \"shared.scs\"\nparameters vth0n=450m u0n=30m after=1\nsimulator lang=spice\n.param rsh_spice=100 rvc=0.01\nsimulator lang=spectre\nendsection tt\nsection ff\ninclude \"shared.scs\"\nparameters vth0n=400m u0n=35m\nendsection ff\n\n"
    );
    assert!(!text.contains("library") && text.contains("\nsection tt\n"));
}

/// The value forms sky130 uses read as ngspice evaluates them (each checked
/// on ngspice 45 by hand first): `{"e"}` is `{e}`; a braced group with an
/// unbraced tail is one expression; a lone `+` between two assignments
/// separates them; an empty `.param` sets nothing; a `.param` in a
/// `.subckt` is a local. ngspice runs the deck written back to the same
/// currents. A statistical function reaches HSPICE and is refused, by code,
/// by LTspice, which has none.
#[test]
fn ngspice_value_forms_and_statistical_functions() {
    let deck = "forms\n.param x=2\n.param a = '-1*x' + b = '0 - 2.5'\n.param c = {1+x} + x*0*AGAUSS(0,1,1)\n.param\n.model rr r rsh=10 dw={\"-a-b/8\"}\n.subckt two p n\n.param k={x*2}\nR1 p n {k+c}\n.ends twoV1 1 0 1\nR1 1 0 rr l=10 w=1\nX1 1 0 two\nVb 2 0 1\nRb 2 0 {b+10}\n.op\n.end\n";
    let w = written(deck, Dialect::Ngspice, "forms", Dialect::Ngspice);
    assert_eq!(codes(&w), [] as [Code; 0], "{}", w.notes.messages);
    for card in [
        ".param x=2",
        ".param a='-1*x'",
        ".param b='0 - 2.5'",
        ".param c={(1+x) + x*0*AGAUSS(0,1,1)}",
        ".model rr r (rsh=10 dw={-a-b/8})",
        ".subckt two p n",
        ".param k={x*2}",
    ] {
        assert!(w.text.lines().any(|l| l == card), "{card}\n{}", w.text);
    }
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
    let text = fixpoint(deck, Dialect::Ngspice, "forms");
    assert_eq!(currents(&text), currents(deck));

    let stat = "t\n.subckt m a b\n.param mis='agauss(0,1m,1)'\nR1 a b {1k+mis}\n.ends m\n";
    let hs = written(stat, Dialect::Ngspice, "t", Dialect::Hspice);
    assert!(
        hs.text.contains(".param mis='agauss(0,1m,1)'"),
        "{}",
        hs.text
    );
    // Spectre lifts the call, VACASK has it (tests/test_monte_carlo.rs);
    // LTspice has none.
    let refused = render_text(stat, Dialect::Ngspice, "t", Dialect::Ltspice).unwrap_err();
    let got: Vec<Code> = refused.diags.iter().map(|d| d.code).collect();
    assert_eq!(got, [Code::W0003], "{}", refused.messages);
}

/// sky130's two element forms ngspice reads its own way:
/// - a charge-defined capacitor (ngspice §3.3.9 `C a b q='expr'`):
///   ngspice runs the deck written back to the same transient, and so does
///   it read as HSPICE (`hsa`); Spectre gets a `bsource q=` (Reference 19.1
///   p.459); VACASK, with no charge element, refuses it (`W0105`, a
///   device with no VACASK master);
/// - an instance naming its subcircuit after its parameters (the
///   `special_pfet_latch` alias, which ngspice 45 itself rejects: "unknown
///   subckt") reads as that subcircuit; no other shape does.
#[test]
fn charge_capacitor_and_late_subcircuit_name() {
    let deck = "q\nV1 a 0 pulse(0 1 0 1u 1u 1m 2m)\nR1 a b 1k\nCq b 0 q='1u*v(b)+0.5u*v(b)*v(b)'\n.tran 10u 2m\n.end\n";
    let w = written(deck, Dialect::Ngspice, "q", Dialect::Ngspice);
    assert_eq!(codes(&w), [] as [Code; 0], "{}", w.notes.messages);
    assert!(
        w.text.contains("\nCq b 0 Q='1u*v(b)+0.5u*v(b)*v(b)'\n"),
        "{}",
        w.text
    );
    let tran = |text: &str| {
        let text = format!("{text}\n").replacen("\n", "\n*spicerack ngbehavior=hsa\n", 1);
        let raw = Dialect::Ngspice
            .run(&text)
            .unwrap_or_else(|e| panic!("{e}\n{text}"));
        raw.real_data
            .iter()
            .flatten()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>()
    };
    let want = tran(deck);
    assert_eq!(tran(&fixpoint(deck, Dialect::Ngspice, "q")), want);
    let hs = written(deck, Dialect::Ngspice, "q", Dialect::Hspice).text;
    assert!(hs.contains("\nCq b 0 Q='1u*v(b)+0.5u*v(b)*v(b)'\n"), "{hs}");
    assert_eq!(tran(&hs), want);
    let scs = written(deck, Dialect::Ngspice, "q", Dialect::Spectre).text;
    assert!(
        scs.contains("\ncq (b 0) bsource q=1u*v(b)+0.5u*v(b)*v(b)\n"),
        "{scs}"
    );
    let refused = render_text(deck, Dialect::Ngspice, "q", Dialect::Vacask).unwrap_err();
    let got: Vec<Code> = refused.diags.iter().map(|d| d.code).collect();
    assert_eq!(got, [Code::W0105], "{}", refused.messages);

    let late = "t\n.subckt pass d g s b w=1\nR1 d s {w}\n.ends pass\n.subckt latch d g s b\nxp d g s b w = {2} pass\n.ends latch\n";
    let w = written(late, Dialect::Ngspice, "t", Dialect::Ngspice);
    assert_eq!(codes(&w), [] as [Code; 0], "{}", w.notes.messages);
    assert!(fixpoint(late, Dialect::Ngspice, "t").contains("\nXp d g s b pass w={2}\n"));
    // Two bare words after the parameters, or one before them: not that
    // shape; the first is kept as text, the second an instance of `c`.
    let other = written(
        "t\nxq a b w=1 c d\nxr a b c w=1\n",
        Dialect::Ngspice,
        "t",
        Dialect::Ngspice,
    );
    assert_eq!(codes(&other), [Code::W0027], "{}", other.notes.messages);
    assert!(other.text.contains("\nXr a b c w=1\n"), "{}", other.text);
}

/// LTspice reads `.lib tt` as the file `tt`: no section form, so the writer
/// refuses rather than write a different deck.
#[test]
fn ltspice_refuses_a_section() {
    let refused = render_text(SPICE, Dialect::Ngspice, "models", Dialect::Ltspice).unwrap_err();
    let got: Vec<Code> = refused.diags.iter().map(|d| d.code).collect();
    assert_eq!(got, [Code::W0012; 2], "{}", refused.messages);
}

/// SPICE -> Spectre -> SPICE and Spectre -> SPICE -> Spectre keep every
/// section's parameters, models, includes, library calls and subcircuits.
/// The trip changes spellings, not meaning, except for the last two:
/// - numbers are respelled (`0.45` is `450m`) and SPICE's expression
///   delimiters dropped (`'vth0n'` is `vth0n`); [`meaning`] compares values;
/// - Spectre writes a SPICE model's parameter names lowercased and moves
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
    let spice = written(SPICE, Dialect::Ngspice, "models", Dialect::Ngspice).text;
    let scs = written(SPICE, Dialect::Ngspice, "models", Dialect::Spectre).text;
    assert!(
        scs.contains("\nsection tt\n")
            && scs.contains("\nendsection ff\n")
            && !scs.contains("library"),
        "{scs}"
    );
    let back = written(&scs, Dialect::Spectre, "models", Dialect::Ngspice);
    assert_eq!(codes(&back), [] as [Code; 0], "{scs}");
    assert_eq!(meaning(&back.text), meaning(&spice), "{scs}");
    assert!(
        scs.contains("model nch bsim4 {\n1: type=n version=4.5 lmin=100n"),
        "{scs}"
    );

    let sp = written(SPECTRE, Dialect::Spectre, "models", Dialect::Ngspice).text;
    assert!(
        sp.contains("\n.model nchb.2 nmos (level=54 lmin=1u lmax=100u vth0=vth0n+0.01)\n"),
        "{sp}"
    );
    let again = written(&sp, Dialect::Ngspice, "models", Dialect::Spectre);
    assert_eq!(codes(&again), [] as [Code; 0], "{sp}");
    let round = written(&again.text, Dialect::Spectre, "models", Dialect::Ngspice).text;
    assert_eq!(meaning(&round), meaning(&sp), "{sp}");
    assert!(!again.text.contains("library"), "{}", again.text);
}

/// ngspice and ESPice each run the `tt` and `ff` corners from the sections
/// SpiceRack wrote for them and get the drain current they get from the
/// hand-written file, bit for bit; `ff` (lower VTH0, higher U0) conducts
/// more than `tt`.
#[test]
fn simulators_run_each_corner_from_the_written_sections() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdk_sections");
    for sim in [Dialect::Ngspice, Dialect::Espice] {
        let dir = tempfile::tempdir().unwrap();
        for f in ["shared.spice", "res.lib"] {
            std::fs::copy(fixtures.join(f), dir.path().join(f)).unwrap();
        }
        let text = written(SPICE, Dialect::Ngspice, "models", sim).text;
        std::fs::write(dir.path().join("models.lib"), text).unwrap();
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

/// `src` (read in `d`) written for `to`, which must not refuse it.
fn written(src: &str, d: Dialect, name: &str, to: Dialect) -> Written {
    render_text(src, d, name, to).unwrap_or_else(|e| panic!("{to:?}: {}\n{src}", e.messages))
}

/// The codes of the notes on a written deck, in source order.
fn codes(w: &Written) -> Vec<Code> {
    w.notes.diags.iter().map(|d| d.code).collect()
}

/// `src` written in `d`, read back and written again: the same text, which
/// is returned.
fn fixpoint(src: &str, d: Dialect, name: &str) -> String {
    let text = written(src, d, name, d).text;
    let again = written(&text, d, name, d).text;
    assert_eq!(again, text, "{d:?}");
    text
}

/// `(k, v)` pairs as [`meaning`] holds them.
fn pairs(ps: &[(&str, &str)]) -> Vec<(String, String)> {
    ps.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
