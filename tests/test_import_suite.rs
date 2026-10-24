//! `spicerack::import` over the spice-bench suites (VAMS-Conformance-Suite
//! `external/spice-bench`): every deck must parse, survive the JSON round
//! trip, and, re-emitted as ngspice, solve to the original deck's answer.
//!
//!   external/spice-bench/fetch.sh DIR
//!   SPICE_BENCH=DIR cargo test --no-default-features --release --test test_import_suite -- --nocapture
//!
//! Skipped without `$SPICE_BENCH` or ngspice. A deck ngspice cannot finish in
//! `$SPICE_BENCH_TIMEOUT` seconds (default 120) as written is skipped: the
//! suite READMEs list the ones ngspice-45 cannot run. Every dialect is also
//! emitted, and the decks each backend refuses are counted, not failed.

use std::path::{Path, PathBuf};
use std::process::Command;

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::parse;
use spicerack::ir::CircuitIR;
use spicerack::result::RawData;

fn decks(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        // `src*` holds fetched sources and their assets, not runnable decks.
        if p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("src")) {
            decks(&p, out);
        } else if p.extension() == Some("sp".as_ref()) {
            out.push(p);
        }
    }
}

/// Runs `deck` (text) with ngspice from `dir` so relative includes resolve.
/// `Err` carries why; `Ok(None)` is a timeout.
fn ngspice(dir: &Path, deck: &str, secs: u64) -> Result<Option<RawData>, String> {
    let file = tempfile::Builder::new().suffix(".sp").tempfile_in(dir).map_err(|e| e.to_string())?;
    std::fs::write(file.path(), deck).map_err(|e| e.to_string())?;
    let raw = tempfile::TempDir::new().unwrap().keep().join("out.raw");
    let out = Command::new("timeout")
        .arg(secs.to_string())
        .args(["ngspice", "-b", "-r"])
        .arg(&raw)
        .arg(file.path())
        .current_dir(dir)
        .output()
        .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&raw);
    let _ = std::fs::remove_dir_all(raw.parent().unwrap());
    if out.status.code() == Some(124) {
        return Ok(None);
    }
    let bytes = bytes.map_err(|_| {
        let log = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
        format!("no raw file: {}", log.lines().filter(|l| l.to_lowercase().contains("error")).take(3).collect::<Vec<_>>().join(" | "))
    })?;
    spicerack::rawfile::parse_raw(&bytes).map(Some).map_err(|e| e.to_string())
}

/// The last point of every vector both runs saved must agree.
fn compare(a: &RawData, b: &RawData) -> Result<(), String> {
    let mut shared = 0;
    for (i, v) in a.variables.iter().enumerate() {
        let Some(j) = b.variables.iter().position(|w| w.name.eq_ignore_ascii_case(&v.name)) else { continue };
        shared += 1;
        let (Some(x), Some(y)) = (a.real_data.get(i).and_then(|c| c.last()), b.real_data.get(j).and_then(|c| c.last())) else { continue };
        if (x - y).abs() > 1e-6 + 1e-3 * x.abs().max(y.abs()) {
            return Err(format!("{}: {x} vs {y}", v.name));
        }
    }
    if shared == 0 && !a.variables.is_empty() {
        return Err(format!("no vector in common: {:?}", b.variables.iter().map(|v| &v.name).take(5).collect::<Vec<_>>()));
    }
    Ok(())
}

enum Outcome {
    Pass,
    Skip(String),
    Fail(String),
}

fn check(deck: &Path, secs: u64, refused: &std::sync::Mutex<std::collections::BTreeMap<String, usize>>) -> Outcome {
    let src = match std::fs::read(deck) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return Outcome::Fail(e.to_string()),
    };
    let stem = deck.file_stem().unwrap().to_string_lossy();
    let ir = parse(&src, &stem);
    let back: CircuitIR = serde_json::from_str(&serde_json::to_string(&ir).unwrap()).unwrap();
    if back != ir {
        return Outcome::Fail("JSON round trip changed the IR".into());
    }
    for d in [Dialect::Ngspice, Dialect::Hspice, Dialect::Ltspice, Dialect::Spectre, Dialect::Vacask, Dialect::Espice] {
        if netlist(&ir, d).is_err() {
            *refused.lock().unwrap().entry(format!("{d:?}")).or_default() += 1;
        }
    }
    let converted = match netlist(&ir, Dialect::Ngspice) {
        Ok(t) => t,
        Err(e) => return Outcome::Fail(format!("ngspice emit: {e}")),
    };
    let dir = deck.parent().unwrap();
    let original = match ngspice(dir, &src, secs) {
        Ok(Some(r)) => r,
        Ok(None) => return Outcome::Skip("original timed out".into()),
        Err(e) => return Outcome::Skip(format!("original fails: {e}")),
    };
    match ngspice(dir, &converted, secs) {
        Ok(Some(r)) => compare(&original, &r).map_or_else(Outcome::Fail, |()| Outcome::Pass),
        Ok(None) => Outcome::Fail("converted timed out, original did not".into()),
        Err(e) => Outcome::Fail(format!("converted fails: {e}")),
    }
}

#[test]
fn spice_bench_round_trip() {
    let Ok(root) = std::env::var("SPICE_BENCH") else {
        eprintln!("SPICE_BENCH not set — skipping");
        return;
    };
    if Command::new("ngspice").arg("--version").output().is_err() {
        eprintln!("ngspice not on PATH — skipping");
        return;
    }
    let secs = std::env::var("SPICE_BENCH_TIMEOUT").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    let mut all = Vec::new();
    decks(Path::new(&root), &mut all);
    all.sort();
    assert!(!all.is_empty(), "no *.sp under {root}");

    let refused = std::sync::Mutex::new(std::collections::BTreeMap::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(deck) = all.get(i) else { break };
                let o = check(deck, secs, &refused);
                results.lock().unwrap().push((deck.clone(), o));
            });
        }
    });

    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.0.cmp(&b.0));
    let (mut pass, mut skip, mut fail) = (0, 0, Vec::new());
    for (deck, o) in &results {
        let rel = deck.strip_prefix(&root).unwrap_or(deck).display();
        match o {
            Outcome::Pass => pass += 1,
            Outcome::Skip(why) => {
                skip += 1;
                eprintln!("skip {rel}: {why}");
            }
            Outcome::Fail(why) => fail.push(format!("{rel}: {why}")),
        }
    }
    eprintln!("\n{} decks: {pass} pass, {skip} skip, {} fail", results.len(), fail.len());
    eprintln!("decks each backend could not express: {:?}", refused.into_inner().unwrap());
    assert!(fail.is_empty(), "round trip failed:\n{}", fail.join("\n"));
}
