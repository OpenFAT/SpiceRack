//! A deck in one dialect -> written in it, read back and written again: the
//! text must not change. No simulator.
//!
//! Cites: none (a round trip through SpiceRack's own reader and writer).
//!
//! Contents:
//! - `pub(super) fn fixpoint(src: &str, name: &str, d: Dialect) -> Result<String, String>`
//! - `pub(super) fn corpus_fixpoint(var: &str, d: Dialect, ext: &str, keep: fn(&str) -> bool)`

use super::*;

/// `src` written in `d`, read back and written again gives the same text:
/// what SpiceRack writes, it reads as written. The written text is
/// returned; `Err` names the refusal or the first change.
pub(super) fn fixpoint(src: &str, name: &str, d: Dialect) -> Result<String, String> {
    let once = render_text(src, d, name, d)
        .map_err(|e| format!("{d:?} write: {}", e.messages))?
        .text;
    let twice = render_text(&once, d, name, d)
        .map_err(|e| format!("{d:?} rewrite: {}", e.messages))?
        .text;
    if twice != once {
        return Err(format!(
            "{d:?} is no fixpoint: re-reading the written deck changes it"
        ));
    }
    Ok(once)
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
            let why = fixpoint(&src, "t", d).err()?;
            Some(format!("{}: {why}", p.display()))
        })
        .collect();
    eprintln!("{} {d:?} decks, {} not fixpoints", all.len(), bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
