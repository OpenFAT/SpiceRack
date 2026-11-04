//! SPICE deck -> every dialect, through the transpiler (`syntax::render`).
//!
//!   cargo run --no-default-features --bin spice2rack -- -o out/ deck.sp [deck2.sp ...]
//!
//! For each deck writes one deck per backend: `.ngspice.sp`, `.hspice.sp`,
//! `.ltspice.cir`, `.spectre.scs`, `.vacask.sim`, `.espice.sp`. A backend
//! that cannot express the circuit (VACASK has no B sources, ngspice no
//! `.step`) is reported with its codes and skipped; the others are still
//! written, each with its notes (what it kept or left out) on stderr.
//! `--dialect hspice` reads HSPICE decks (default ngspice). `--check`
//! requires each deck to be a fixpoint: written in its own dialect and read
//! back, it writes the same text.
//!
//! A PDK model file converts like a deck: its library sections (`.lib tt`
//! ... `.endl tt`, Spectre and VACASK `section`) are written as sections in
//! every dialect that has them; LTspice has none and is refused.
//!
//! Cites: none (a CLI over `spicerack::import` and `syntax::render`).
//!
//! Contents: none (a binary; `main` is its entry).

use std::path::{Path, PathBuf};

use spicerack::syntax::Dialect;
use spicerack::syntax::render::render_text;

const BACKENDS: [(Dialect, &str); 6] = [
    (Dialect::Ngspice, "ngspice.sp"),
    (Dialect::Hspice, "hspice.sp"),
    (Dialect::Ltspice, "ltspice.cir"),
    (Dialect::Spectre, "spectre.scs"),
    (Dialect::Vacask, "vacask.sim"),
    (Dialect::Espice, "espice.sp"),
];

// ---- private ----

/// Converts one deck into every backend's dialect under `out`.
fn convert(deck: &Path, d: Dialect, out: &Path, check: bool) -> std::io::Result<()> {
    let src = spicerack::import::decode(&std::fs::read(deck)?);
    let stem = deck.file_stem().and_then(|s| s.to_str()).unwrap_or("deck");
    for (target, ext) in BACKENDS {
        match render_text(&src, d, stem, target) {
            Ok(w) => {
                eprint!("{}", w.notes.messages);
                std::fs::write(out.join(format!("{stem}.{ext}")), w.text + "\n")?;
            }
            Err(e) => eprint!("{}: {target:?} skipped:\n{}", deck.display(), e.messages),
        }
    }
    // Fixpoint: what SpiceRack writes in the deck's own dialect, it reads
    // back and writes as the same text.
    if check {
        let once = render_text(&src, d, stem, d).map_err(|e| std::io::Error::other(e.messages))?;
        let twice =
            render_text(&once.text, d, stem, d).map_err(|e| std::io::Error::other(e.messages))?;
        if once.text != twice.text {
            return Err(std::io::Error::other(format!(
                "not a fixpoint in {d:?}: re-reading the written deck changes it"
            )));
        }
    }
    Ok(())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let check = match args.iter().position(|a| a == "--check") {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
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
            "usage: spice2rack [-o DIR] [--dialect ngspice|hspice|ltspice|xyce|spectre|vacask] [--check] DECK..."
        );
        std::process::exit(2);
    }
    std::fs::create_dir_all(&out).expect("output directory");
    let mut failed = 0;
    for deck in &args {
        if let Err(e) = convert(Path::new(deck), dialect, &out, check) {
            eprintln!("{deck}: {e}");
            failed += 1;
        }
    }
    if failed > 0 {
        eprintln!("{failed} deck(s) failed");
    }
    std::process::exit((failed > 0) as i32);
}
