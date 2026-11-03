//! A deck in one dialect -> emitted, read back and emitted again: the IR and
//! the text must not change. No simulator.
//!
//! Cites: none (a round trip through SpiceRack's own reader and writer).
//!
//! Contents:
//! - `pub(super) fn fixpoint(ir: &CircuitIR, d: Dialect) -> Result<String, String>`
//! - `pub(super) fn corpus_fixpoint(var: &str, d: Dialect, ext: &str, keep: fn(&str) -> bool)`

use super::*;

/// Emitting `ir` in `d` and reading it back gives `ir` again, and emitting
/// that gives the same text: what SpiceRack writes, it reads as written.
/// The emitted text is returned.
pub(super) fn fixpoint(ir: &CircuitIR, d: Dialect) -> Result<String, String> {
    let text = netlist(ir, d).map_err(|e| format!("{d:?} emit: {e}"))?;
    let (again, _) = parse(&text, &ir.top.name, d)?;
    if &again != ir || netlist(&again, d).ok().as_ref() != Some(&text) {
        return Err(format!(
            "{d:?} is no fixpoint: re-reading the emitted deck changes the IR"
        ));
    }
    Ok(text)
}

/// Every deck under `$<var>` whose path passes `keep` is a fixpoint in `d`.
/// No simulator.
pub(super) fn corpus_fixpoint(var: &str, d: Dialect, ext: &str, keep: fn(&str) -> bool) {
    let root = std::env::var(var).unwrap_or_else(|_| {
        panic!("{var} not set: run the tests in `nix develop .#testing`, which sets it")
    });
    let mut all = Vec::new();
    files(Path::new(&root), ext, &mut all);
    all.retain(|p| keep(&p.to_string_lossy()));
    assert!(!all.is_empty(), "no {d:?} decks under {root}");
    let bad: Vec<String> = all
        .iter()
        .filter_map(|p| {
            let src = spicerack::import::decode(&std::fs::read(p).ok()?);
            let parsed = parse(&src, "t", d);
            // `$GOLDEN_DIR`: each deck's IR and report as JSON, for diffing two
            // importers byte for byte (`diff -r before after`).
            if let Ok(dir) = std::env::var("GOLDEN_DIR") {
                let rel = p.strip_prefix(&root).unwrap_or(p);
                let out = Path::new(&dir)
                    .join(format!("{d:?}"))
                    .join(rel)
                    .with_extension("json");
                std::fs::create_dir_all(out.parent().unwrap()).unwrap();
                let body = match &parsed {
                    Ok((ir, r)) => {
                        format!("{}\n{:?}\n", serde_json::to_string_pretty(ir).unwrap(), r)
                    }
                    Err(e) => format!("error: {e}\n"),
                };
                std::fs::write(out, body).unwrap();
            }
            let why = match parsed {
                Ok((ir, _)) => fixpoint(&ir, d).err()?,
                Err(e) => e,
            };
            Some(format!("{}: {why}", p.display()))
        })
        .collect();
    eprintln!("{} {d:?} decks, {} not fixpoints", all.len(), bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
