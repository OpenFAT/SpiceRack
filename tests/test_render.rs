//! The renderer (`syntax::render`) against the old pipeline on decks the
//! golden corpus holds none of: Spectre `statistics` blocks lowered for the
//! SPICE family (`docs/research/monte-carlo-spectre.md` §6b, §10.5) and
//! Laplace sources.
//!
//! Oracle: `codegen::netlist(import::parse(deck))` at this commit, the
//! pipeline the golden gate's baseline recorded (TESTING.md, "Golden
//! gate"): each deck written byte for byte the same, or refused by both.
//! `test_monte_carlo.rs` (T-R3, T-R4) and `test_laplace_ngspice.rs` check
//! the old pipeline's text against the research note and real ngspice.
//!
//! Contents: none (tests only).

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::parse;
use spicerack::syntax::diag::Code;
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

// ---- private ----

#[test]
fn statistics_tables_are_written_as_the_old_lowering_wrote_them() {
    let decks = [
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
    ];
    for deck in &decks {
        let (ir, _) = parse(deck, "t", Dialect::Spectre).unwrap();
        for d in SPICE {
            let old = netlist(&ir, d).map_err(|e| e.to_string());
            let new = render_text(deck, Dialect::Spectre, "t", d);
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
        let refused = render_text(&deck, Dialect::Spectre, "t", Dialect::Ngspice).unwrap_err();
        assert_eq!(
            refused.iter().map(|d| d.code).collect::<Vec<_>>(),
            [*code],
            "{deck}"
        );
    }
}
