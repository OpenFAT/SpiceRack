//! `spicerack::import` over the spice-bench suites (VAMS-Conformance-Suite
//! `external/spice-bench`): every deck must parse, survive the JSON round
//! trip, keep its analysis and `.meas` cards, and, re-emitted as ngspice,
//! reproduce every plot at every point and every `.meas` result.
//!
//!   nix develop .#testing -c cargo test --no-default-features --features corpus --release --test test_import_suite -- --nocapture
//!
//! The nightly tier (`.github/workflows/nightly.yaml`); the merge tier does
//! not build it (`required-features = ["corpus"]` in Cargo.toml).
//!
//! `nix develop .#testing` sets `$SPICE_BENCH` and the `$*_CORPUS` variables;
//! without them, or without ngspice, the tests fail. A deck ngspice cannot
//! finish in `$SPICE_BENCH_TIMEOUT` CPU seconds (default 120) as written, or
//! cannot run at all, is a named known failure in
//! `tests/fixtures/spice_bench_known_failures.txt`: the suite READMEs list the
//! ones ngspice-45 cannot run. Every dialect is also emitted, and the decks
//! each backend refuses are counted, not failed.

use std::path::{Path, PathBuf};
use std::process::Command;

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::parse;
use spicerack::ir::CircuitIR;
use std::collections::BTreeMap;

use num_complex::Complex64;
use spicerack::measure_parse::parse_measures;
use spicerack::result::RawData;

mod compare;
mod fixpoint;

use compare::*;
use fixpoint::*;
/// Files ending in `.ext` under `dir`, skipping `src*` directories.
fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir()
            && !p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("src"))
        {
            files(&p, ext, out);
        } else if p.extension() == Some(ext.as_ref()) {
            out.push(p);
        }
    }
}

fn decks(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        // `src*` holds fetched sources and their assets, not runnable decks.
        if p.is_dir()
            && !p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("src"))
        {
            decks(&p, out);
        // `.tmp*`: this harness's own working decks, left by a killed run.
        } else if p.extension() == Some("sp".as_ref())
            && !p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            out.push(p);
        }
    }
}

/// One ngspice run: every plot, and the `.meas` results.
struct Run {
    plots: Vec<RawData>,
    measures: Vec<(String, f64)>,
}

/// Runs `deck` (text) with ngspice from `dir` so relative includes resolve
/// (ngspice falls back to the working directory), with the random seed
/// pinned. The deck itself goes to the temp dir: `dir` is read-only in the
/// nix store.
/// `-r` turns `.meas` off in batch mode ("No .measure possible in batch mode
/// (-b) with -r rawfile set!"), and with it `.option autostop`, which then
/// stops a transient at t = 0. So a deck with `.meas` runs once without
/// `-r`, from a `.control` block that ends in `write`: the converted deck
/// already runs its analysis from one (the codegen writes it, and the block
/// gets the `write`, as `backend::ngspice` does); any other gets `run` +
/// `write`. An original whose own `.control` block runs its analysis keeps
/// the `-r` run (`converted` is false), so which originals ngspice can run
/// as written does not change.
/// `Err` carries why; `Ok(None)` is a timeout.
fn ngspice(dir: &Path, deck: &str, secs: u64, converted: bool) -> Result<Option<Run>, String> {
    let file = tempfile::Builder::new()
        .suffix(".sp")
        .tempfile()
        .map_err(|e| e.to_string())?;
    // A pinned seed: models with `agauss` mismatch (gf180) draw a fresh
    // value every run otherwise, so the same deck disagrees with itself.
    let (title, body) = deck.split_once('\n').unwrap_or((deck, ""));
    std::fs::write(file.path(), format!("{title}\n.option seed=1\n{body}"))
        .map_err(|e| e.to_string())?;
    let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
    let raw = tmp.path().join("out.raw");
    let meas = analyses(deck).contains_key(".meas");
    let block = match runs_in_control(deck) {
        true if converted => Some(""),
        false if meas => Some("run\n"),
        _ => None,
    };
    let control = block.is_some();
    if let Some(run) = block {
        let end = body
            .lines()
            .position(|l| l.trim().eq_ignore_ascii_case(".end"))
            .unwrap_or(body.lines().count());
        let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
        lines.insert(
            end,
            format!(".control\n{run}write {}\n.endc", raw.display()),
        );
        std::fs::write(
            file.path(),
            format!("{title}\n.option seed=1\n{}\n", lines.join("\n")),
        )
        .map_err(|e| e.to_string())?;
    }
    // The budget is CPU seconds, not wall: under a loaded machine a wall
    // clock times out decks that are not slow. One OpenMP thread, since a
    // CPU limit counts every thread and the workers already fill the cores.
    // The wall `timeout` only catches a hang.
    let run = |with_raw: bool| {
        let mut c = Command::new("timeout");
        c.arg((10 * secs).to_string())
            .arg("prlimit")
            .arg(format!("--cpu={secs}"))
            .args(["ngspice", "-b"])
            .env("OMP_NUM_THREADS", "1");
        if with_raw {
            c.arg("-r").arg(&raw);
        }
        c.arg(file.path())
            .current_dir(dir)
            .output()
            .map_err(|e| e.to_string())
    };
    // `timeout` exits 124 itself; when the CPU limit kills ngspice (SIGXCPU,
    // or SIGKILL at the hard limit) it re-raises that signal on itself, so
    // there is no exit code. 137 and 152 are the same deaths through a shell.
    let timed_out = |o: &std::process::Output| {
        use std::os::unix::process::ExitStatusExt;
        matches!(o.status.code(), Some(124 | 137 | 152))
            || matches!(o.status.signal(), Some(9 | 24))
    };
    let out = run(!control)?;
    if timed_out(&out) {
        return Ok(None);
    }
    let bytes = std::fs::read(&raw).map_err(|_| {
        let log = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
        format!(
            "no raw file: {}",
            log.lines()
                .filter(|l| l.to_lowercase().contains("error"))
                .take(3)
                .collect::<Vec<_>>()
                .join(" | ")
        )
    })?;
    let plots = spicerack::rawfile::parse_raw_all(&bytes).map_err(|e| e.to_string())?;
    let mut measures = Vec::new();
    if meas {
        let out = if control { out } else { run(false)? };
        if timed_out(&out) {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&out.stdout);
        measures = parse_measures(&text, Some(spicerack::ir::Dialect::Ngspice))
            .into_iter()
            .map(|m| (m.name.to_lowercase(), m.value))
            .collect();
        measures.sort_by(|a, b| a.0.cmp(&b.0));
    }
    Ok(Some(Run { plots, measures }))
}

/// True if a `.control` block in `deck` issues a run command.
fn runs_in_control(deck: &str) -> bool {
    let mut inside = false;
    for line in deck.lines() {
        match line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            ".control" => inside = true,
            ".endc" => inside = false,
            "run" | "op" | "ac" | "tran" | "dc" if inside => return true,
            _ => {}
        }
    }
    false
}

/// Analysis and `.meas` cards by kind; inside a `.control` block, the
/// command of the same name (`tran`, `fourier`, `meas`): the codegen runs a
/// deck's `.meas`/`.four` analysis from one.
fn analyses(deck: &str) -> BTreeMap<&'static str, usize> {
    const KINDS: [(&str, &str); 14] = [
        (".op", ".op"),
        (".dc", ".dc"),
        (".ac", ".ac"),
        (".tran", ".tran"),
        (".noise", ".noise"),
        (".tf", ".tf"),
        (".sens", ".sens"),
        (".pz", ".pz"),
        (".disto", ".disto"),
        (".four", ".four"),
        (".fourier", ".four"),
        (".meas", ".meas"),
        (".measure", ".meas"),
        (".step", ".step"),
    ];
    let mut out = BTreeMap::new();
    let mut control = false;
    for line in deck.lines().skip(1) {
        let word = line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match word.as_str() {
            ".control" => control = true,
            ".endc" => control = false,
            w if control => {
                if let Some((_, k)) = KINDS.iter().find(|(spelling, _)| spelling[1..] == *w) {
                    *out.entry(*k).or_default() += 1;
                }
            }
            w => {
                if let Some((_, k)) = KINDS.iter().find(|(spelling, _)| *spelling == w) {
                    *out.entry(*k).or_default() += 1;
                }
            }
        }
    }
    out
}

fn column(r: &RawData, i: usize) -> Vec<Complex64> {
    match r.complex(i) {
        Some(c) => c.to_vec(),
        None => r.re(i).iter().map(|&x| Complex64::new(x, 0.0)).collect(),
    }
}

#[test]
fn sliding_tube_matches_the_scan() {
    let xb: Vec<f64> = (0..400)
        .map(|i| i as f64 * 0.01 + (i % 7) as f64 * 1e-4)
        .collect();
    let yb: Vec<f64> = xb
        .iter()
        .map(|x| (7.0 * x).sin() + if (x * 10.0) as i32 % 3 == 0 { 0.5 } else { 0.0 })
        .collect();
    for shift in [0.0, 0.003, 0.02, 0.2] {
        let xa: Vec<f64> = (0..300).map(|i| i as f64 * 0.013).collect();
        let ya: Vec<f64> = xa.iter().map(|x| (7.0 * (x + shift)).sin()).collect();
        for dx in [0.0, 0.005, 0.04] {
            let scan = (0..ya.len()).find(|&k| !in_tube(&xb, &yb, xa[k], dx, ya[k], 1e-3));
            assert_eq!(
                leaves_tube(&xa, &ya, &xb, &yb, dx, 1e-3),
                scan,
                "shift {shift} dx {dx}"
            );
        }
    }
}

/// ngspice decks (`$NGSPICE_CORPUS`, e.g. spice-bench's `*.sp`).
#[test]
fn ngspice_corpus_fixpoint() {
    corpus_fixpoint("NGSPICE_CORPUS", Dialect::Ngspice, "sp", |p| {
        !p.contains("/src") && !p.contains("/.tmp")
    });
}

/// ESPice's HSPICE corpus (`$HSPICE_CORPUS`, its `tests/conformance/tests/netlist`).
#[test]
fn hspice_corpus_fixpoint() {
    corpus_fixpoint("HSPICE_CORPUS", Dialect::Hspice, "sp", |p| {
        p.contains("hspice") && !p.contains("invalid")
    });
}

/// Xyce's regression decks (`$XYCE_CORPUS`, Xyce_Regression's `Netlists`).
#[test]
fn xyce_corpus_fixpoint() {
    corpus_fixpoint("XYCE_CORPUS", Dialect::Xyce, "cir", |_| true);
}

/// Spectre decks (`$SPECTRE_CORPUS`; `err_*` decks are deliberately invalid).
///
/// Known failure: ESPice's `tests/frontend/spectre_fixtures`, the corpus this
/// read, is gone from its public history and no other public Spectre corpus
/// is pinned, so `.#testing` leaves the variable unset. Point it at a corpus
/// and the test passes, which fails until this marker goes.
#[test]
#[should_panic(expected = "SPECTRE_CORPUS not set")]
fn spectre_corpus_fixpoint() {
    corpus_fixpoint("SPECTRE_CORPUS", Dialect::Spectre, "scs", |p| {
        !p.contains("/err_")
    });
}

/// VACASK decks (`$VACASK_CORPUS`, e.g. VACASK's `test`, `demo`, `benchmark`).
#[test]
fn vacask_corpus_fixpoint() {
    corpus_fixpoint("VACASK_CORPUS", Dialect::Vacask, "sim", |_| true);
}

/// LTspice netlists (`$LTSPICE_CORPUS`, e.g. the spice-datasets collection's `*.net`).
#[test]
fn ltspice_corpus_fixpoint() {
    corpus_fixpoint("LTSPICE_CORPUS", Dialect::Ltspice, "net", |_| true);
}

enum Outcome {
    Pass,
    /// Every check but the simulation passed: the deck is in [`SLOW`].
    Slow,
    Skip(String),
    Fail(String),
}

fn check(
    deck: &Path,
    secs: u64,
    simulate: bool,
    refused: &std::sync::Mutex<std::collections::BTreeMap<String, usize>>,
) -> Outcome {
    let src = match std::fs::read(deck) {
        Ok(b) => spicerack::import::decode(&b),
        Err(e) => return Outcome::Fail(e.to_string()),
    };
    let stem = deck.file_stem().unwrap().to_string_lossy();
    let ir = match parse(&src, &stem, Dialect::Ngspice) {
        Ok((ir, _)) => ir,
        Err(e) => return Outcome::Fail(e),
    };
    let converted = match fixpoint(&ir, Dialect::Ngspice) {
        Ok(t) => t,
        Err(why) => return Outcome::Fail(why),
    };
    let back: CircuitIR = serde_json::from_str(&serde_json::to_string(&ir).unwrap()).unwrap();
    if back != ir {
        return Outcome::Fail("JSON round trip changed the IR".into());
    }
    for d in [
        Dialect::Hspice,
        Dialect::Ltspice,
        Dialect::Spectre,
        Dialect::Vacask,
        Dialect::Espice,
    ] {
        if netlist(&ir, d).is_err() {
            *refused.lock().unwrap().entry(format!("{d:?}")).or_default() += 1;
        }
    }
    // The analyses must survive whether or not ngspice can run the deck.
    let (before, after) = (analyses(&src), analyses(&converted));
    if before != after {
        return Outcome::Fail(format!("analyses {before:?} became {after:?}"));
    }
    if !simulate {
        return Outcome::Slow;
    }
    let dir = deck.parent().unwrap();
    let original = match ngspice(dir, &src, secs, false) {
        Ok(Some(r)) => r,
        Ok(None) => return Outcome::Skip("original timed out".into()),
        Err(e) => return Outcome::Skip(format!("original fails: {e}")),
    };
    if let Some(p) = original
        .plots
        .iter()
        .find(|p| p.variables.len() > 0 && p.re(0).is_empty())
    {
        return Outcome::Skip(format!("original fails: {} has no points", p.plot_name));
    }
    // Twice the budget: under a loaded machine a deck near the limit as
    // written crosses it converted with no change in the circuit.
    match ngspice(dir, &converted, 2 * secs, true) {
        Ok(Some(r)) => match compare(&original, &r) {
            Ok(()) => Outcome::Pass,
            // Fails only if the original agrees with itself (TRNOISE draws
            // a fresh stream per run, seed or no seed) and the unconverted,
            // reordered deck stays within the tolerance the converted one
            // broke.
            Err(why) => match ngspice(dir, &src, secs, false) {
                Ok(Some(again)) if compare(&original, &again).is_err() => Outcome::Skip(format!(
                    "original is not repeatable: two runs disagree ({why})"
                )),
                _ => match ngspice(dir, &reordered(&src), secs, false) {
                    Ok(Some(c)) if compare(&original, &c).is_err() => Outcome::Skip(format!(
                        "order-sensitive: reordering the original alone breaks it ({why})"
                    )),
                    _ => Outcome::Fail(why),
                },
            },
        },
        Ok(None) => Outcome::Fail("converted timed out, original did not".into()),
        Err(e) => Outcome::Fail(format!("converted fails: {e}")),
    }
}

/// The decks ngspice cannot run as written, one `<path under $SPICE_BENCH>
/// <reason>` per line.
const KNOWN_FAILURES: &str = include_str!("../fixtures/spice_bench_known_failures.txt");

/// The decks ngspice cannot finish in the CPU budget, same format. Whether a
/// deck crosses a time budget depends on the machine, so these are
/// quarantined by name: every check but the simulation runs.
const SLOW: &str = include_str!("../fixtures/spice_bench_slow.txt");

/// The deck paths of a known-failure list.
fn names(list: &str) -> Vec<&str> {
    list.lines()
        .filter_map(|l| l.split_whitespace().next().filter(|w| !w.starts_with('#')))
        .collect()
}

#[test]
fn spice_bench_round_trip() {
    let root = std::env::var("SPICE_BENCH").unwrap_or_else(|_| {
        panic!("SPICE_BENCH not set: run the tests in `nix develop .#testing`, which sets it")
    });
    assert!(
        Command::new("ngspice").arg("--version").output().is_ok(),
        "ngspice not on PATH: run the tests in `nix develop .#testing`"
    );
    let secs = std::env::var("SPICE_BENCH_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let mut all = Vec::new();
    decks(Path::new(&root), &mut all);
    // Largest first: a big deck started last would set the wall clock alone.
    all.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map_or(0, |m| m.len())));
    assert!(!all.is_empty(), "no *.sp under {root}");
    let rel = |deck: &Path| {
        deck.strip_prefix(&root)
            .unwrap_or(deck)
            .to_string_lossy()
            .into_owned()
    };
    let (slow, known) = (names(SLOW), names(KNOWN_FAILURES));
    let stale: Vec<&str> = slow
        .iter()
        .chain(&known)
        .copied()
        .filter(|n| !all.iter().any(|d| rel(d) == *n))
        .collect();
    assert!(
        stale.is_empty(),
        "listed decks missing from the suite: {stale:#?}"
    );

    let refused = std::sync::Mutex::new(std::collections::BTreeMap::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(deck) = all.get(i) else { break };
                    let t = std::time::Instant::now();
                    let o = check(deck, secs, !slow.contains(&rel(deck).as_str()), &refused);
                    if t.elapsed().as_secs() > 60 {
                        eprintln!("slow {}: {:.0?}", deck.display(), t.elapsed());
                    }
                    results.lock().unwrap().push((deck.clone(), o));
                }
            });
        }
    });

    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.0.cmp(&b.0));
    let (mut pass, mut quarantined, mut skipped, mut fail) = (0, 0, Vec::new(), Vec::new());
    for (deck, o) in &results {
        let rel = rel(deck);
        match o {
            Outcome::Pass => pass += 1,
            Outcome::Slow => quarantined += 1,
            Outcome::Skip(why) => {
                eprintln!("skip {rel}: {why}");
                skipped.push(rel);
            }
            Outcome::Fail(why) => fail.push(format!("{rel}: {why}")),
        }
    }
    eprintln!(
        "\n{} decks: {pass} pass, {quarantined} slow, {} skip, {} fail",
        results.len(),
        skipped.len(),
        fail.len()
    );
    eprintln!(
        "decks each backend could not express: {:?}",
        refused.into_inner().unwrap()
    );
    // A skip is a named known failure: the deck's line in KNOWN_FAILURES
    // (or SLOW, for a timeout). The name lists must match: an unlisted skip
    // and a listed deck that ran both fail, until the list is edited.
    let unlisted: Vec<&String> = skipped
        .iter()
        .filter(|d| !known.contains(&d.as_str()))
        .collect();
    let ran: Vec<&&str> = known
        .iter()
        .filter(|k| !skipped.iter().any(|d| d == *k))
        .collect();
    assert!(fail.is_empty(), "round trip failed:\n{}", fail.join("\n"));
    assert!(
        unlisted.is_empty() && ran.is_empty(),
        "known failures out of date (tests/fixtures/spice_bench_known_failures.txt, or spice_bench_slow.txt for a timeout):\nskipped, not listed: {unlisted:#?}\nlisted, not skipped: {ran:#?}"
    );
}
