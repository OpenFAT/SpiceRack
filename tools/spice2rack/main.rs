//! SPICE deck -> SpiceRack IR -> every dialect.
//!
//!   cargo run --no-default-features --bin spice2rack -- -o out/ deck.sp [deck2.sp ...]
//!
//! For each deck writes `out/<stem>.json` (the `CircuitIR`, SpiceRack's own
//! form) and one deck per backend: `.ngspice.sp`, `.hspice.sp`, `.ltspice.cir`,
//! `.spectre.scs`, `.vacask.sim`, `.espice.sp`. A backend that cannot express
//! the circuit (VACASK has no B sources, ngspice no `.step`) is reported and
//! skipped; the others are still written. The reader is `spicerack::import`.
//!
//! `--python` also writes `<stem>.py`: the deck as SpiceRack Python, using
//! the public API only. A construct the API cannot express goes through
//! `raw_spice()` under a `# gap:` comment, and `--python` ends with the gap
//! tally over all decks: the API's missing pieces, ranked by real decks.
//! Running the script prints `tb.netlist(<dialect>)`, which must hold the
//! same cards as the `.ngspice.sp` written beside it (`--check` runs it and
//! compares them as a multiset: a `raw_spice()` fallback moves its card into
//! the raw block, after the components).
//! `--check` also requires each deck to be a fixpoint: written in its own
//! dialect and read back, it gives the same IR. `--dialect hspice` reads
//! HSPICE decks (default ngspice).
//!
//! Cites: none (a CLI over `spicerack::import`, `codegen` and the Python API).
//!
//! Contents: none (a binary; `main` is its entry).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use spicerack::codegen::{Dialect, card, netlist};
use spicerack::import::parse;
use spicerack::ir::*;

mod python;

use python::*;

// ── Output ──

const BACKENDS: [(Dialect, &str); 6] = [
    (Dialect::Ngspice, "ngspice.sp"),
    (Dialect::Hspice, "hspice.sp"),
    (Dialect::Ltspice, "ltspice.cir"),
    (Dialect::Spectre, "spectre.scs"),
    (Dialect::Vacask, "vacask.sim"),
    (Dialect::Espice, "espice.sp"),
];

struct Opts {
    out: PathBuf,
    dialect: Dialect,
    python: bool,
    check: bool,
}

// ---- private ----

/// Converts one deck; returns the Python gaps it hit (empty without `--python`).
fn convert(deck: &Path, o: &Opts) -> std::io::Result<Vec<&'static str>> {
    let src = spicerack::import::decode(&std::fs::read(deck)?);
    let stem = deck.file_stem().and_then(|s| s.to_str()).unwrap_or("deck");
    let (ir, report) = parse(&src, stem, o.dialect).map_err(std::io::Error::other)?;
    if report.raw > 0 {
        eprintln!(
            "{}: {} card(s) kept verbatim (no IR form)",
            deck.display(),
            report.raw
        );
    }
    for card in &report.unparsed {
        eprintln!("{}: grammar refused, kept verbatim: {card}", deck.display());
    }
    std::fs::write(
        o.out.join(format!("{stem}.json")),
        serde_json::to_string_pretty(&ir).map_err(std::io::Error::other)?,
    )?;
    for (d, ext) in BACKENDS {
        match netlist(&ir, d) {
            Ok(text) => std::fs::write(o.out.join(format!("{stem}.{ext}")), text + "\n")?,
            Err(e) => eprintln!("{}: {d:?} skipped: {e}", deck.display()),
        }
    }
    // Fixpoint: what SpiceRack writes in the deck's own dialect, it reads
    // back as the same IR, and writes back as the same text.
    if o.check {
        let text = netlist(&ir, o.dialect).map_err(std::io::Error::other)?;
        let (again, _) = parse(&text, stem, o.dialect).map_err(std::io::Error::other)?;
        if again != ir || netlist(&again, o.dialect).ok().as_ref() != Some(&text) {
            return Err(std::io::Error::other(format!(
                "not a fixpoint in {:?}: re-reading the emitted deck changes the IR",
                o.dialect
            )));
        }
    }
    if !o.python {
        return Ok(vec![]);
    }
    let py = python(&ir);
    let script = o.out.join(format!("{stem}.py"));
    std::fs::write(&script, &py.out)?;
    if o.check {
        let want = netlist(&ir, Dialect::Ngspice).map_err(std::io::Error::other)?;
        let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".into());
        let run = std::process::Command::new(python).arg(&script).output()?;
        let got = String::from_utf8_lossy(&run.stdout);
        let cards = |t: &str| {
            let mut v: Vec<String> = t.lines().map(str::to_string).collect();
            v.sort();
            v
        };
        if cards(&got) != cards(&want) {
            let err = String::from_utf8_lossy(&run.stderr);
            return Err(std::io::Error::other(format!(
                "{} prints a different netlist{}",
                script.display(),
                err.lines()
                    .last()
                    .map(|l| format!(": {l}"))
                    .unwrap_or_default()
            )));
        }
    }
    Ok(py.gaps)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .map(|i| args.remove(i))
            .is_some()
    };
    let (python, check) = (flag("--python"), flag("--check"));
    let mut value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .filter(|i| i + 1 < args.len())
            .map(|i| {
                let v = args.remove(i + 1);
                args.remove(i);
                v
            })
    };
    let out = PathBuf::from(value("-o").unwrap_or_else(|| ".".into()));
    let dialect = match value("--dialect").map_or(Ok(Dialect::Ngspice), |v| v.parse()) {
        Ok(
            d @ (Dialect::Ngspice
            | Dialect::Hspice
            | Dialect::Ltspice
            | Dialect::Xyce
            | Dialect::Spectre
            | Dialect::Vacask),
        ) => d,
        _ => {
            eprintln!("--dialect: reads ngspice, hspice, ltspice, xyce, spectre or vacask");
            std::process::exit(2);
        }
    };
    if args.is_empty() {
        eprintln!(
            "usage: spice2rack [-o DIR] [--dialect ngspice|hspice|ltspice|xyce|spectre|vacask] [--python [--check]] DECK..."
        );
        std::process::exit(2);
    }
    std::fs::create_dir_all(&out).expect("output directory");
    let o = Opts {
        out,
        dialect,
        python,
        check,
    };
    let (mut failed, mut tally) = (0, BTreeMap::<&str, (usize, usize)>::new());
    for deck in &args {
        match convert(Path::new(deck), &o) {
            Ok(gaps) => {
                let mut seen: Vec<&str> = gaps.clone();
                seen.sort();
                seen.dedup();
                for g in &gaps {
                    tally.entry(g).or_default().0 += 1;
                }
                for g in seen {
                    tally.entry(g).or_default().1 += 1;
                }
            }
            Err(e) => {
                eprintln!("{deck}: {e}");
                failed += 1;
            }
        }
    }
    if o.python {
        let mut rows: Vec<_> = tally.into_iter().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1.1));
        let mut report = format!(
            "Python API gaps over {} decks (cards, decks):\n",
            args.len()
        );
        for (g, (cards, decks)) in rows {
            let _ = writeln!(report, "  {decks:5} decks {cards:7} cards  {g}");
        }
        eprint!("{report}");
    }
    if failed > 0 {
        eprintln!("{failed} deck(s) failed");
    }
    std::process::exit((failed > 0) as i32);
}
