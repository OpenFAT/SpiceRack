//! SPICE deck -> SpiceRack IR -> every dialect.
//!
//!   cargo run --no-default-features --bin spice2rack -- -o out/ deck.sp [deck2.sp ...]
//!
//! For each deck writes `out/<stem>.json` (the `CircuitIR`, SpiceRack's own
//! form) and one deck per backend: `.ngspice.sp`, `.hspice.sp`, `.ltspice.cir`,
//! `.spectre.scs`, `.vacask.sim`, `.espice.sp`. A backend that cannot express
//! the circuit (VACASK has no B sources, ngspice no `.step`) is reported and
//! skipped; the others are still written. The reader is `spicerack::import`.

use std::path::{Path, PathBuf};

use spicerack::codegen::CodeGen;
use spicerack::codegen::espice::EspiceCodeGen;
use spicerack::codegen::spectre::SpectreCodeGen;
use spicerack::codegen::spice3::{Spice3CodeGen, Spice3Dialect};
use spicerack::codegen::vacask::VacaskCodeGen;
use spicerack::import::parse;

// ── Output ──

const NG: Spice3CodeGen = Spice3CodeGen { dialect: Spice3Dialect::Ngspice };
const HS: Spice3CodeGen = Spice3CodeGen { dialect: Spice3Dialect::Hspice };
const LT: Spice3CodeGen = Spice3CodeGen { dialect: Spice3Dialect::Ltspice };

fn backends() -> [(&'static dyn CodeGen, &'static str); 6] {
    [(&NG, "ngspice.sp"), (&HS, "hspice.sp"), (&LT, "ltspice.cir"), (&SpectreCodeGen, "spectre.scs"), (&VacaskCodeGen, "vacask.sim"), (&EspiceCodeGen, "espice.sp")]
}

fn convert(deck: &Path, out: &Path) -> std::io::Result<()> {
    let src = std::fs::read_to_string(deck)?;
    let stem = deck.file_stem().and_then(|s| s.to_str()).unwrap_or("deck");
    let ir = parse(&src, stem);
    let raw = ir.top.raw_spice.len() + ir.subcircuit_defs.iter().map(|s| s.raw_spice.len()).sum::<usize>() + ir.testbench.as_ref().map_or(0, |t| t.extra_lines.len());
    if raw > 0 {
        eprintln!("{}: {raw} card(s) kept verbatim (no IR form)", deck.display());
    }
    std::fs::write(out.join(format!("{stem}.json")), serde_json::to_string_pretty(&ir).map_err(std::io::Error::other)?)?;
    for (g, ext) in backends() {
        match g.emit_netlist(&ir) {
            Ok(text) => std::fs::write(out.join(format!("{stem}.{ext}")), text + "\n")?,
            Err(e) => eprintln!("{}: {} skipped: {e}", deck.display(), g.backend_name()),
        }
    }
    Ok(())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let out = match args.iter().position(|a| a == "-o") {
        Some(i) if i + 1 < args.len() => {
            let o = PathBuf::from(args.remove(i + 1));
            args.remove(i);
            o
        }
        _ => PathBuf::from("."),
    };
    if args.is_empty() {
        eprintln!("usage: spice2rack [-o DIR] DECK...");
        std::process::exit(2);
    }
    std::fs::create_dir_all(&out).expect("output directory");
    let mut failed = false;
    for deck in &args {
        if let Err(e) = convert(Path::new(deck), &out) {
            eprintln!("{deck}: {e}");
            failed = true;
        }
    }
    std::process::exit(failed as i32);
}

