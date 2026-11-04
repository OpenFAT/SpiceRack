//! The renderer (`syntax::render`) against the old pipeline on decks the
//! golden corpus holds none of: Spectre `statistics` blocks lowered for the
//! SPICE family (two-layer `.param` draws; Spectre Reference 19.1
//! pp.175-179) and Laplace sources; the same tables for Spectre and VACASK;
//! functions whose bodies draw, inlined for Spectre at each call.
//! The made-up fixtures (`tests/fixtures/transpiler/stats_*.scs`,
//! `laplace.{sp,cir,net}`, `func_stats.{sp,cir}`, golden inputs too) are written by both and pinned under
//! `tests/fixtures/transpiler/pinned/<file>_<ext>/`.
//!
//! Oracle: `codegen::netlist(import::parse(deck))` at this commit, the
//! pipeline the golden gate's baseline recorded (TESTING.md, "Golden
//! gate"): each deck written byte for byte the same, or refused by both.
//! That pipeline's statistics text is the one `test_monte_carlo.rs` T-R3
//! and T-R4 check by hand,
//! and its Laplace cards the ones `test_laplace_ngspice.rs` runs on real
//! ngspice. `SPICERACK_BLESS=1` rewrites the pinned files.
//!
//! Contents: none (tests only).

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::parse;
use spicerack::syntax::diag::{Code, Kind, info};
use spicerack::syntax::render::render_text;

const SPICE: [Dialect; 5] = [
    Dialect::Ngspice,
    Dialect::Espice,
    Dialect::Hspice,
    Dialect::Xyce,
    Dialect::Ltspice,
];

/// T-R3's table: process (percent, lnorm, unif) and mismatch.
const TABLE: &str = "// t\nsimulator lang=spectre\nparameters p=1k q=2 r=3\nstatistics {\n  process {\n    vary p dist=gauss std=10 percent=yes\n    vary q dist=lnorm std=0.1\n    vary r dist=unif N=0.2\n  }\n  mismatch { vary q dist=gauss std=0.05 }\n}\nsubckt s (a b)\nr1 (a b) resistor r=q*1k\nends s\nr1 (a 0) resistor r=p\nx1 (a 0) s\n";

/// T-R4's base: a correlation, `CORR` its operands.
const CORR: &str = "// t\nsimulator lang=spectre\nparameters a=1 b=2\nstatistics {\n  process {\n    vary a dist=gauss std=0.1\n    vary b dist=gauss std=0.2\n  }\n  correlate CORR\n}\nr1 (x 0) resistor r=a+b\n";

/// The made-up statistics, Laplace and drawing-function fixtures, each
/// read in its dialect.
const FIXTURES: [(&str, Dialect); 9] = [
    ("stats_process.scs", Dialect::Spectre),
    ("stats_correlate.scs", Dialect::Spectre),
    ("stats_correlate_cc.scs", Dialect::Spectre),
    ("stats_refused.scs", Dialect::Spectre),
    ("laplace.sp", Dialect::Ngspice),
    ("laplace.cir", Dialect::Xyce),
    ("laplace.net", Dialect::Ltspice),
    ("func_stats.sp", Dialect::Ngspice),
    ("func_stats.cir", Dialect::Xyce),
];

// ---- private ----

#[test]
fn statistics_tables_are_written_as_the_old_lowering_wrote_them() {
    for deck in &statistics_decks() {
        let (ir, _) = parse(deck, "t", Dialect::Spectre).unwrap();
        for d in SPICE {
            let old = netlist(&ir, d).map_err(|e| e.to_string());
            let new = render_text(deck, Dialect::Spectre, "t", d).map(|w| w.text);
            match (&old, &new) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a, b, "{d:?}\n{deck}");
                    assert!(
                        a.contains("__mc=") || a.contains("__z=") || a.contains("__mm="),
                        "{a}"
                    );
                }
                (Err(_), Err(_)) => {}
                _ => panic!("{d:?}: old {old:?}, new {new:?}\n{deck}"),
            }
        }
    }
}

#[test]
fn statistics_tables_reach_the_named_family_as_the_old_writers_wrote_them() {
    // VACASK lowers a table to one-layer draws (`render::draws`,
    // `Layers::One`); Spectre keeps it. A subcircuit's mismatch draw is
    // VACASK's only written form of a `mismatch` vary.
    let mut vacask = 0;
    for deck in &statistics_decks() {
        let (ir, _) = parse(deck, "t", Dialect::Spectre).unwrap();
        for d in [Dialect::Spectre, Dialect::Vacask] {
            let old = netlist(&ir, d).map_err(|e| e.to_string());
            let new = render_text(deck, Dialect::Spectre, "t", d).map(|w| w.text);
            match (&old, &new) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a, b, "{d:?}\n{deck}");
                    vacask += usize::from(d == Dialect::Vacask && a.contains("agauss("));
                }
                (Err(_), Err(_)) => {}
                _ => panic!("{d:?}: old {old:?}, new {new:?}\n{deck}"),
            }
        }
    }
    assert!(vacask >= 6, "VACASK wrote {vacask} of the tables");
}

/// Spectre decks with `statistics` tables: T-R3's, T-R4's correlations
/// (numeric, expression, 3x3), mismatch in several subcircuits, a library's
/// sections.
fn statistics_decks() -> Vec<String> {
    vec![
        TABLE.to_string(),
        CORR.replace("CORR", "param=[a b] cc=0.6"),
        CORR.replace("CORR", "param=[a b] cc=k").replace("a=1 b=2", "a=1 b=2 k=0.5"),
        CORR.replace("CORR", "param=[a b] cc=0.6\n  truncate tr=4"),
        // Three parameters, numeric: the Cholesky factor.
        "// t\nsimulator lang=spectre\nparameters a=1 b=2 c=3\nstatistics {\n  process {\n    vary a dist=gauss std=0.1\n    vary b dist=gauss std=0.2\n    vary c dist=gauss std=0.3 percent=yes\n  }\n  correlate param=[a b c] cc=0.5\n}\nr1 (x 0) resistor r=a+b+c\n".into(),
        // Mismatch in two subcircuits, one reading it in a model and an
        // instance parameter, one not at all; two mismatch parameters.
        "// t\nsimulator lang=spectre\nparameters w=1u l=2u\nstatistics {\n  mismatch {\n    vary w dist=gauss std=0.01\n    vary l dist=unif N=0.02\n  }\n}\nsubckt s1 (a b)\nparameters m=w*2\nr1 (a b) resistor r=w*1meg+l*1meg\nends s1\nsubckt s2 (a b)\nr1 (a b) resistor r=1k\nends s2\nsubckt s3 (a b)\nx1 (a b) s1 m=w\nends s3\nx1 (a 0) s1\nx2 (a 0) s2\n".into(),
        // A library's sections, each with its own table.
        "// t\nsimulator lang=spectre\nlibrary lib\nsection tt\nparameters v=1\nstatistics {\n  process { vary v dist=gauss std=0.1 }\n}\nends tt\nsection ss\nparameters v=2\nstatistics {\n  process { vary v dist=unif N=0.1 }\n  mismatch { vary v dist=gauss std=0.01 }\n}\nsubckt s (a b)\nr1 (a b) resistor r=v\nends s\nends ss\nendlibrary\n".into(),
    ]
}

#[test]
fn statistics_the_spice_family_cannot_hold_are_refused_by_code() {
    let cases = [
        (CORR.replace("CORR", "dev=[x1 x2] cc=0.6"), Code::W0018),
        (
            CORR.replace("CORR", "param=[a b] cc=0.6\n  truncate tr=3"),
            Code::W0018,
        ),
        (
            TABLE.replace("dist=lnorm std=0.1", "dist=lnorm std=0.1 percent=yes"),
            Code::W0018,
        ),
        (
            CORR.replace("vary b dist=gauss", "vary b dist=unif"),
            Code::W0018,
        ),
        (TABLE.replace("vary r ", "vary z "), Code::W0019),
        (CORR.replace("CORR", "param=[a b] cc=1.5"), Code::W0020),
    ];
    for (deck, code) in &cases {
        let deck = deck.replace("CORR", "param=[a b] cc=0.6");
        let (ir, _) = parse(&deck, "t", Dialect::Spectre).unwrap();
        assert!(
            netlist(&ir, Dialect::Ngspice).is_err(),
            "the old writer refused\n{deck}"
        );
        let Err(refused) = render_text(&deck, Dialect::Spectre, "t", Dialect::Ngspice) else {
            panic!("refused\n{deck}");
        };
        assert_eq!(
            refused.diags.iter().map(|d| d.code).collect::<Vec<_>>(),
            [*code],
            "{deck}"
        );
    }
}

#[test]
fn laplace_sources_are_written_as_the_old_writer_wrote_them() {
    let h = [
        "Laplace(v(in), 1/(1+s*1.59e-4))",
        "Laplace(V(in), (1+s*1e-3)/(1+s*1e-6))",
        "Laplace(v(in), 4/(1+s*2+s^2*3))",
        "Laplace(v(a), 1/(1+s**2))",
        "Laplace(v(a), 1/(1+s*s))",
        "Laplace(v(in), s/(1+s*1e-3))",
        "Laplace(v(in), 1/(1+s*159u))",
        "Laplace(v(in), 1/(1+s/1meg))",
        "LAPLACE (V(A,B), 2/(1+s*1.591549431e-9))",
        "Laplace(v(in), 2.5)",
        "Laplace(v(in), 10/4)",
        "laplace(v(in), (s+1)^-1)",
        "{Laplace(v(in), 1/(1+s))}",
        // Refused by both: symbolic, improper, current input, arithmetic
        // around the call, a zero denominator, a fractional power.
        "Laplace(v(in), 1/(1+s*tau))",
        "Laplace(v(in), s^2)",
        "Laplace(I(V1), 1/(1+s))",
        "2*Laplace(v(in), 1/(1+s))",
        "Laplace(v(in), 1/(1+s)) + 1",
        "Laplace(v(in), 1/(s-s))",
        "Laplace(v(in), s^0.5/(1+s))",
        "Laplace(v(in))",
    ];
    let mut lifted = 0;
    for e in h {
        for key in ["V", "I"] {
            let deck =
                format!("t\nV1 in 0 1\nB1 out 0 {key}={e}\nR1 out 0 1k\n.tran 1u 1m\n.end\n");
            let (ir, _) = parse(&deck, "t", Dialect::Ngspice).unwrap();
            for d in SPICE {
                let old = netlist(&ir, d).map_err(|e| e.to_string());
                let new = render_text(&deck, Dialect::Ngspice, "t", d).map(|w| w.text);
                match (&old, &new) {
                    (Ok(a), Ok(b)) => {
                        assert_eq!(a, b, "{d:?}\n{deck}");
                        lifted += usize::from(
                            ["_xfer", "LAPLACE", "Laplace="]
                                .iter()
                                .any(|w| a.contains(w)),
                        );
                    }
                    (Err(_), Err(_)) => {}
                    _ => panic!("{d:?}: old {old:?}, new {new:?}\n{deck}"),
                }
            }
        }
    }
    // The 11 transfer functions, as V and I, on ngspice, ESPice, HSPICE and
    // LTspice (Xyce refuses every one, LTspice the two with `^`, which it
    // reads as XOR).
    assert_eq!(lifted, (11 * 4 - 2) * 2);
}

#[test]
fn fixtures_render_as_the_old_writer_did_and_as_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transpiler");
    let bless = std::env::var_os("SPICERACK_BLESS").is_some();
    let targets = SPICE
        .into_iter()
        .chain([Dialect::Cdl, Dialect::Spectre, Dialect::Vacask]);
    for (file, d) in FIXTURES {
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        let stem = file.split('.').next().unwrap();
        let (ir, _) = parse(&text, stem, d).unwrap();
        for t in targets.clone() {
            let old = netlist(&ir, t).map_err(|e| e.to_string());
            let new = render_text(&text, d, stem, t).map(|w| w.text);
            let (name, body) = match (&old, &new) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a, b, "{file} {t:?}");
                    (format!("{}.txt", t.name()), a.clone())
                }
                (Err(e), Err(codes)) => {
                    let codes: Vec<String> = codes
                        .diags
                        .iter()
                        .map(|c| format!("{:?}", c.code))
                        .collect();
                    let body = format!("old: {e}\nnew: {}\n", codes.join(" "));
                    (format!("{}.refused", t.name()), body)
                }
                _ => panic!("{file} {t:?}: old {old:?}, new {new:?}"),
            };
            let pin = dir.join("pinned").join(file.replace('.', "_")).join(&name);
            if bless {
                std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
                std::fs::write(&pin, &body).unwrap();
            }
            let want = std::fs::read_to_string(&pin)
                .unwrap_or_else(|e| panic!("{}: {e} (SPICERACK_BLESS=1 writes it)", pin.display()));
            assert_eq!(body, want, "{}", pin.display());
        }
    }
    // Each fixture reaches the rule it is for.
    let pinned = |f: &str, t: &str| {
        std::fs::read_to_string(dir.join("pinned").join(f.replace('.', "_")).join(t)).unwrap()
    };
    for f in [
        "stats_process.scs",
        "stats_correlate.scs",
        "stats_correlate_cc.scs",
    ] {
        let sp = pinned(f, "ngspice.txt");
        assert!(
            sp.contains("__mc={") || sp.contains("__z__r={"),
            "{f}\n{sp}"
        );
    }
    assert!(pinned("stats_process.scs", "ngspice.txt").contains("__mm_r={"));
    assert!(pinned("stats_refused.scs", "ngspice.refused").contains("new: W0018"));
    // Spectre keeps a table; VACASK draws it in one layer, a subcircuit's
    // mismatch draw first among its parameters.
    assert!(pinned("stats_process.scs", "spectre.txt").contains("statistics {"));
    let va = pinned("stats_process.scs", "vacask.txt");
    assert!(
        va.contains("rsh=agauss(") && va.contains("__mm=agauss("),
        "{va}"
    );
    assert!(pinned("stats_refused.scs", "vacask.refused").contains("new: W0018"));
    for f in ["laplace.sp", "laplace.cir", "laplace.net"] {
        assert!(
            pinned(f, "ngspice.txt").contains("s_xfer(num_coeff=["),
            "{f}"
        );
        assert!(pinned(f, "hspice.txt").contains(" LAPLACE "), "{f}");
        assert!(pinned(f, "xyce.refused").contains(" W0011"), "{f}");
    }
    // LTspice reads the ngspice deck's `s^2` as XOR: refused there, written
    // from the two decks that spell it `s*s`.
    assert!(pinned("laplace.sp", "ltspice.refused").contains("new: W0002"));
    for f in ["laplace.cir", "laplace.net"] {
        assert!(pinned(f, "ltspice.txt").contains(" Laplace="), "{f}");
    }
    // Spectre inlines a drawing function at each call, each drawn on its
    // own, and writes no `real` function for it; VACASK has no functions.
    let sp = pinned("func_stats.sp", "spectre.txt");
    assert!(
        sp.contains("r=((1k)*rack_agauss(1, 0.1, 3, rack_r1_r_1))"),
        "{sp}"
    );
    assert!(sp.contains("vary rack_s_r4_r_1 dist=gauss std=1"), "{sp}");
    assert!(
        !sp.contains("real f(") && !sp.contains("real unused("),
        "{sp}"
    );
    assert!(sp.contains("real h(real x)"), "{sp}");
    let cir = pinned("func_stats.cir", "spectre.txt");
    assert!(
        cir.contains("r=(min(max((3),  0),  2) + rack_agauss((3), 1,1, rack_r1_r_1))*1k"),
        "{cir}"
    );
    for f in ["func_stats.sp", "func_stats.cir"] {
        assert!(pinned(f, "vacask.refused").contains("new: W0106"), "{f}");
    }
}

#[test]
fn what_a_writer_leaves_out_or_copies_is_a_note_by_code() {
    // Each deck is still written; the note names what the target does not
    // get as read (diag.rs, `Kind::Note`).
    let spectre = "// t\nsimulator lang=spectre\nahdl_include \"m.va\"\nr1 (a 0) resistor r=1k\n";
    let cases: [(&str, Dialect, Dialect, Code); 11] = [
        (spectre, Dialect::Spectre, Dialect::Cdl, Code::W0021),
        (spectre, Dialect::Spectre, Dialect::Ltspice, Code::W0022),
        (
            "t\nR1 a 0 1k\n.tran 1n 10n\n.print tran v(a)\n.end\n",
            Dialect::Ngspice,
            Dialect::Ltspice,
            Code::W0023,
        ),
        (
            "t\nR1 a 0 1k\n.op\n.save v(a)\n.end\n",
            Dialect::Ngspice,
            Dialect::Xyce,
            Code::W0023,
        ),
        (
            "t\nR1 a 0 1k\n.op\n.options klu\n.end\n",
            Dialect::Ngspice,
            Dialect::Vacask,
            Code::W0024,
        ),
        (
            "t\nR1 a 0 1k\n.control\nlet x=1\nop\n.endc\n.end\n",
            Dialect::Ngspice,
            Dialect::Ngspice,
            Code::W0025,
        ),
        (
            "t\nV1 a 0 PULSE(0 1 0 1n 1n 5n 10n 3)\nR1 a 0 1k\n.tran 1n 10n\n.end\n",
            Dialect::Ngspice,
            Dialect::Hspice,
            Code::W0026,
        ),
        (
            "t\n.subckt s a\n.func f(x) {x}\nR1 a 0 1k\n.ends\n.end\n",
            Dialect::Ngspice,
            Dialect::Espice,
            Code::W0027,
        ),
        (
            "t\nR1 a 0 1k\n.op\n.end\n",
            Dialect::Ngspice,
            Dialect::Cdl,
            Code::W0028,
        ),
        (
            "t\n.lib models.lib\nR1 a 0 1k\n.op\n.end\n",
            Dialect::Ltspice,
            Dialect::Spectre,
            Code::W0110,
        ),
        (
            "t\n.param x=2\n.model m nmos level={x}\nM1 d g 0 0 m\n.op\n.end\n",
            Dialect::Ngspice,
            Dialect::Spectre,
            Code::W0111,
        ),
    ];
    for (deck, d, t, code) in cases {
        let written =
            render_text(deck, d, "t", t).unwrap_or_else(|e| panic!("{t:?} refused {e:?}\n{deck}"));
        let codes: Vec<Code> = written.notes.diags.iter().map(|d| d.code).collect();
        assert!(
            codes.contains(&code),
            "{t:?}: {codes:?}, not {code:?}\n{deck}"
        );
        assert!(
            codes.iter().all(|&c| info(c).kind == Kind::Note),
            "{codes:?}"
        );
        assert!(
            written.notes.messages.contains(&format!(" {code:?} ")),
            "{}",
            written.notes.messages
        );
    }
}

#[test]
fn a_diagnostic_is_anchored_at_its_statement_and_messages_are_in_source_order() {
    // LTspice reads `^` as XOR (W0002), and has no `aunif` form (W0003):
    // each anchored at the line holding it, not the deck's first token.
    let deck = "t\nR1 a 0 1k\nR2 a 0 {aunif(1k, 10)}\nR3 a 0 {2^3}\n.op\n.end\n";
    let Err(refused) = render_text(deck, Dialect::Ngspice, "t", Dialect::Ltspice) else {
        panic!("refused");
    };
    let codes: Vec<Code> = refused.diags.iter().map(|d| d.code).collect();
    assert_eq!(codes, [Code::W0003, Code::W0002]);
    assert_eq!(
        refused.messages,
        "t:3:1: W0003 a statistical function the target lacks\n\
         t:4:1: W0002 a `^` that means power in one dialect and XOR in the other\n"
    );
}
