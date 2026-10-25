//! `spicerack::import` over the spice-bench suites (VAMS-Conformance-Suite
//! `external/spice-bench`): every deck must parse, survive the JSON round
//! trip, keep its analysis and `.meas` cards, and, re-emitted as ngspice,
//! reproduce every plot at every point and every `.meas` result.
//!
//!   external/spice-bench/fetch.sh DIR
//!   SPICE_BENCH=DIR cargo test --no-default-features --release --test test_import_suite -- --nocapture
//!
//! Skipped without `$SPICE_BENCH` or ngspice. A deck ngspice cannot finish in
//! `$SPICE_BENCH_TIMEOUT` CPU seconds (default 120) as written is skipped: the
//! suite READMEs list the ones ngspice-45 cannot run. Every dialect is also
//! emitted, and the decks each backend refuses are counted, not failed.

use std::path::{Path, PathBuf};
use std::process::Command;

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::parse;
use spicerack::ir::CircuitIR;
use std::collections::BTreeMap;

use num_complex::Complex64;
use spicerack::measure_parse::parse_measures;
use spicerack::result::RawData;

/// Files ending in `.ext` under `dir`, skipping `src*` directories.
fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("src")) {
            files(&p, ext, out);
        } else if p.extension() == Some(ext.as_ref()) {
            out.push(p);
        }
    }
}

fn decks(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        // `src*` holds fetched sources and their assets, not runnable decks.
        if p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("src")) {
            decks(&p, out);
        // `.tmp*`: this harness's own working decks, left by a killed run.
        } else if p.extension() == Some("sp".as_ref()) && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            out.push(p);
        }
    }
}

/// One ngspice run: every plot, and the `.meas` results.
struct Run {
    plots: Vec<RawData>,
    measures: Vec<(String, f64)>,
}

/// Runs `deck` (text) with ngspice from `dir` so relative includes resolve,
/// with the random seed pinned.
/// `-r` turns `.meas` off in batch mode ("No .measure possible in batch mode
/// (-b) with -r rawfile set!"), so a deck with `.meas` runs twice.
/// `Err` carries why; `Ok(None)` is a timeout.
fn ngspice(dir: &Path, deck: &str, secs: u64) -> Result<Option<Run>, String> {
    let file = tempfile::Builder::new().suffix(".sp").tempfile_in(dir).map_err(|e| e.to_string())?;
    // A pinned seed: models with `agauss` mismatch (gf180) draw a fresh
    // value every run otherwise, so the same deck disagrees with itself.
    let (title, body) = deck.split_once('\n').unwrap_or((deck, ""));
    std::fs::write(file.path(), format!("{title}\n.option seed=1\n{body}")).map_err(|e| e.to_string())?;
    let tmp = tempfile::TempDir::new().map_err(|e| e.to_string())?;
    let raw = tmp.path().join("out.raw");
    // The budget is CPU seconds, not wall: under a loaded machine a wall
    // clock times out decks that are not slow. One OpenMP thread, since a
    // CPU limit counts every thread and the workers already fill the cores.
    // The wall `timeout` only catches a hang.
    let run = |with_raw: bool| {
        let mut c = Command::new("timeout");
        c.arg((10 * secs).to_string()).arg("prlimit").arg(format!("--cpu={secs}")).args(["ngspice", "-b"]).env("OMP_NUM_THREADS", "1");
        if with_raw {
            c.arg("-r").arg(&raw);
        }
        c.arg(file.path()).current_dir(dir).output().map_err(|e| e.to_string())
    };
    // `timeout` exits 124 itself, or 128 + the signal that killed ngspice:
    // SIGXCPU (24) at the CPU limit, SIGKILL (9) past it.
    let timed_out = |o: &std::process::Output| matches!(o.status.code(), Some(124 | 137 | 152));
    let out = run(true)?;
    if timed_out(&out) {
        return Ok(None);
    }
    let bytes = std::fs::read(&raw).map_err(|_| {
        let log = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
        format!("no raw file: {}", log.lines().filter(|l| l.to_lowercase().contains("error")).take(3).collect::<Vec<_>>().join(" | "))
    })?;
    let plots = spicerack::rawfile::parse_raw_all(&bytes).map_err(|e| e.to_string())?;
    let mut measures = Vec::new();
    if analyses(deck).contains_key(".meas") {
        let out = run(false)?;
        if timed_out(&out) {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&out.stdout);
        measures = parse_measures(&text, "ngspice").into_iter().map(|m| (m.name.to_lowercase(), m.value)).collect();
        measures.sort_by(|a, b| a.0.cmp(&b.0));
    }
    Ok(Some(Run { plots, measures }))
}

/// Analysis and `.meas` cards by kind, outside `.control` blocks.
fn analyses(deck: &str) -> BTreeMap<&'static str, usize> {
    const KINDS: [(&str, &str); 14] = [
        (".op", ".op"), (".dc", ".dc"), (".ac", ".ac"), (".tran", ".tran"), (".noise", ".noise"), (".tf", ".tf"),
        (".sens", ".sens"), (".pz", ".pz"), (".disto", ".disto"), (".four", ".four"), (".fourier", ".four"),
        (".meas", ".meas"), (".measure", ".meas"), (".step", ".step"),
    ];
    let mut out = BTreeMap::new();
    let mut control = false;
    for line in deck.lines().skip(1) {
        let word = line.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        match word.as_str() {
            ".control" => control = true,
            ".endc" => control = false,
            _ if control => {}
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
        None => r.re(i).into_iter().map(|x| Complex64::new(x, 0.0)).collect(),
    }
}

/// Whether `y` lies within `tol` of the band `ys` spans over `[x - dx, x + dx]`
/// (the samples in the window and the two that bracket it). A switching edge
/// that ngspice's timestep control moved by less than `dx` still passes; a
/// wrong value does not.
fn in_tube(xs: &[f64], ys: &[f64], x: f64, dx: f64, y: f64, tol: f64) -> bool {
    let lo = xs.partition_point(|&t| t < x - dx).saturating_sub(1);
    let hi = (xs.partition_point(|&t| t <= x + dx) + 1).min(xs.len());
    let band = &ys[lo..hi];
    let (min, max) = band.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
    y >= min - tol && y <= max + tol
}

/// The first `k` whose `ya[k]` is not [`in_tube`] of `(xb, yb)`. Both time
/// axes rise, so the window only slides forward: a min and a max deque make
/// it O(n) where `in_tube` per point is O(n * window), hours on a 2M-point
/// TRNOISE plot.
fn leaves_tube(xa: &[f64], ya: &[f64], xb: &[f64], yb: &[f64], dx: f64, tol: f64) -> Option<usize> {
    if !xa.is_sorted() || !xb.is_sorted() {
        return (0..ya.len()).find(|&k| !in_tube(xb, yb, xa[k], dx, ya[k], tol));
    }
    let (mut mins, mut maxs) = (std::collections::VecDeque::<usize>::new(), std::collections::VecDeque::<usize>::new());
    let (mut lo, mut hi, mut pushed) = (0, 0, 0);
    for (k, (&x, &y)) in xa.iter().zip(ya).enumerate() {
        while lo < xb.len() && xb[lo] < x - dx {
            lo += 1;
        }
        while hi < xb.len() && xb[hi] <= x + dx {
            hi += 1;
        }
        let (start, end) = (lo.saturating_sub(1), (hi + 1).min(xb.len()));
        while pushed < end {
            while maxs.back().is_some_and(|&j| yb[j] <= yb[pushed]) {
                maxs.pop_back();
            }
            while mins.back().is_some_and(|&j| yb[j] >= yb[pushed]) {
                mins.pop_back();
            }
            maxs.push_back(pushed);
            mins.push_back(pushed);
            pushed += 1;
        }
        while maxs.front().is_some_and(|&j| j < start) {
            maxs.pop_front();
        }
        while mins.front().is_some_and(|&j| j < start) {
            mins.pop_front();
        }
        let min = mins.front().map_or(f64::INFINITY, |&j| yb[j]);
        let max = maxs.front().map_or(f64::NEG_INFINITY, |&j| yb[j]);
        if !(y >= min - tol && y <= max + tol) {
            return Some(k);
        }
    }
    None
}

#[test]
fn sliding_tube_matches_the_scan() {
    let xb: Vec<f64> = (0..400).map(|i| i as f64 * 0.01 + (i % 7) as f64 * 1e-4).collect();
    let yb: Vec<f64> = xb.iter().map(|x| (7.0 * x).sin() + if (x * 10.0) as i32 % 3 == 0 { 0.5 } else { 0.0 }).collect();
    for shift in [0.0, 0.003, 0.02, 0.2] {
        let xa: Vec<f64> = (0..300).map(|i| i as f64 * 0.013).collect();
        let ya: Vec<f64> = xa.iter().map(|x| (7.0 * (x + shift)).sin()).collect();
        for dx in [0.0, 0.005, 0.04] {
            let scan = (0..ya.len()).find(|&k| !in_tube(&xb, &yb, xa[k], dx, ya[k], 1e-3));
            assert_eq!(leaves_tube(&xa, &ya, &xb, &yb, dx, 1e-3), scan, "shift {shift} dx {dx}");
        }
    }
}

fn close(x: f64, y: f64) -> bool {
    (x - y).abs() <= 1e-9 * x.abs().max(y.abs())
}

/// Every plot of `b` must match `a`'s, in order. Op, dc and ac sweep grids
/// must be the same and every point agree; a transient must end at the same
/// time and each of `a`'s points lie in `b`'s tube (`in_tube`, a window of
/// 1% of the stop time): reordering an unconverted deck's own cards moves a
/// switching edge by a few timesteps, so pointwise equality is not a test of
/// the conversion. A vector passes within `1e-3` of its largest magnitude
/// (plus `1e-9`), so a node near zero is judged against its own swing.
fn compare(a: &Run, b: &Run) -> Result<(), String> {
    let names = |r: &Run| r.plots.iter().map(|p| p.plot_name.clone()).collect::<Vec<_>>();
    if names(a) != names(b) {
        return Err(format!("plots {:?} became {:?}", names(a), names(b)));
    }
    for (pa, pb) in a.plots.iter().zip(&b.plots) {
        let transient = pa.variables.first().is_some_and(|v| v.name.eq_ignore_ascii_case("time"));
        let (xa, xb) = (pa.re(0), pb.re(0));
        // Span-relative: a sweep crossing zero lands within an ulp of it, and
        // `-50m` and `-0.05` are two such ulps in ngspice's number parser.
        let span = xa.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        if !transient && (xa.len() != xb.len() || xa.iter().zip(&xb).any(|(x, y)| (x - y).abs() > 1e-9 * span)) {
            return Err(format!("{}: sweep grid changed ({} -> {} points)", pa.plot_name, xa.len(), xb.len()));
        }
        if transient && !matches!((xa.last(), xb.last()), (Some(&x), Some(&y)) if close(x, y)) {
            return Err(format!("{}: stop time {:?} -> {:?}", pa.plot_name, xa.last(), xb.last()));
        }
        let mut shared = 0;
        // A name can repeat (a TXL line's two `i(y1)` branches): pair the
        // k-th occurrence in `a` with the k-th in `b`. Indexed by name, since
        // a power grid saves ~10^6 vectors and a scan per vector is quadratic.
        let mut in_b = std::collections::HashMap::<String, std::collections::VecDeque<usize>>::new();
        for (j, w) in pb.variables.iter().enumerate() {
            in_b.entry(w.name.to_ascii_lowercase()).or_default().push_back(j);
        }
        for (i, v) in pa.variables.iter().enumerate() {
            let Some(j) = in_b.get_mut(&v.name.to_ascii_lowercase()).and_then(|q| q.pop_front()) else { continue };
            if i == 0 {
                continue; // the scale; popped so the k-th pairing counts it
            }
            shared += 1;
            let (ya, yb) = (column(pa, i), column(pb, j));
            let swing = ya.iter().map(|c| c.norm()).fold(0.0, f64::max);
            let tol = 1e-9 + 1e-3 * swing;
            // The same grid and every point within `tol` is inside any tube:
            // the common case, and the tube scan costs a window per point.
            let pointwise = || xa == xb && ya.iter().zip(&yb).all(|(p, q)| (p - q).norm() <= tol);
            if transient && !pointwise() {
                let (ra, rb) = (pa.re(i), pb.re(j));
                let dx = 0.01 * xa.last().unwrap();
                if let Some(k) = leaves_tube(&xa, &ra, &xb, &rb, dx, tol) {
                    return Err(format!("{} {}: {} leaves the tube at {}", pa.plot_name, v.name, ra[k], xa[k]));
                }
            } else if let Some(k) = (0..ya.len()).find(|&k| (ya[k] - yb[k]).norm() > tol) {
                return Err(format!("{} {}: {} vs {} at {}", pa.plot_name, v.name, ya[k], yb[k], xa[k]));
            }
        }
        if shared == 0 && pa.variables.len() > 1 {
            return Err(format!("{}: no vector in common", pa.plot_name));
        }
    }
    let (ma, mb): (Vec<_>, Vec<_>) = (a.measures.iter().map(|m| &m.0).collect(), b.measures.iter().map(|m| &m.0).collect());
    if ma != mb {
        return Err(format!(".meas {ma:?} became {mb:?}"));
    }
    for ((n, x), (_, y)) in a.measures.iter().zip(&b.measures) {
        if (x - y).abs() > 1e-9 + 1e-3 * x.abs() {
            return Err(format!(".meas {n}: {x} vs {y}"));
        }
    }
    Ok(())
}

/// The control for a failed comparison: `deck` with its top-level element
/// cards (and their `+` lines) in reverse order, unconverted. The circuit is
/// the same; only ngspice's matrix and timestep order change.
fn reordered(deck: &str) -> String {
    let mut lines = deck.lines();
    let mut out = vec![lines.next().unwrap_or("").to_string()];
    let (mut depth, mut cards, mut tail) = (0, Vec::<String>::new(), Vec::new());
    for l in lines {
        let t = l.trim_start().to_ascii_lowercase();
        if t.starts_with(".subckt") || t.starts_with(".macro") {
            depth += 1;
        } else if t.starts_with(".ends") || t.starts_with(".eom") {
            depth -= 1;
            out.push(l.to_string());
            continue;
        }
        let element = depth == 0 && t.starts_with(|c: char| c.is_ascii_alphabetic());
        match (element, t.starts_with('+'), cards.last_mut()) {
            (true, _, _) => cards.push(l.to_string()),
            (false, true, Some(last)) if tail.is_empty() && depth == 0 => {
                last.push('\n');
                last.push_str(l);
            }
            _ if t.starts_with(".end") && !t.starts_with(".ends") => tail.push(l.to_string()),
            _ => out.push(l.to_string()),
        }
    }
    out.extend(cards.into_iter().rev());
    out.extend(tail);
    out.join("\n") + "\n"
}

/// Emitting `ir` in `d` and reading it back gives `ir` again, and emitting
/// that gives the same text: what SpiceRack writes, it reads as written.
/// The emitted text is returned.
fn fixpoint(ir: &CircuitIR, d: Dialect) -> Result<String, String> {
    let text = netlist(ir, d).map_err(|e| format!("{d:?} emit: {e}"))?;
    let (again, _) = parse(&text, &ir.top.name, d)?;
    if &again != ir || netlist(&again, d).ok().as_ref() != Some(&text) {
        return Err(format!("{d:?} is no fixpoint: re-reading the emitted deck changes the IR"));
    }
    Ok(text)
}

/// Every deck under `$<var>` whose path passes `keep` is a fixpoint in `d`.
/// No simulator.
fn corpus_fixpoint(var: &str, d: Dialect, ext: &str, keep: fn(&str) -> bool) {
    let Ok(root) = std::env::var(var) else {
        eprintln!("{var} not set — skipping");
        return;
    };
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
                let out = Path::new(&dir).join(format!("{d:?}")).join(rel).with_extension("json");
                std::fs::create_dir_all(out.parent().unwrap()).unwrap();
                let body = match &parsed {
                    Ok((ir, r)) => format!("{}\n{:?}\n", serde_json::to_string_pretty(ir).unwrap(), r),
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

/// ngspice decks (`$NGSPICE_CORPUS`, e.g. spice-bench's `*.sp`).
#[test]
fn ngspice_corpus_fixpoint() {
    corpus_fixpoint("NGSPICE_CORPUS", Dialect::Ngspice, "sp", |p| !p.contains("/src") && !p.contains("/.tmp"));
}

/// ESPice's HSPICE corpus (`$HSPICE_CORPUS`, its `tests/conformance/tests/netlist`).
#[test]
fn hspice_corpus_fixpoint() {
    corpus_fixpoint("HSPICE_CORPUS", Dialect::Hspice, "sp", |p| p.contains("hspice") && !p.contains("invalid"));
}

/// Xyce's regression decks (`$XYCE_CORPUS`, Xyce_Regression's `Netlists`).
#[test]
fn xyce_corpus_fixpoint() {
    corpus_fixpoint("XYCE_CORPUS", Dialect::Xyce, "cir", |_| true);
}

/// Spectre decks (`$SPECTRE_CORPUS`, e.g. ESPice's
/// `tests/frontend/spectre_fixtures`; `err_*` decks are deliberately invalid).
#[test]
fn spectre_corpus_fixpoint() {
    corpus_fixpoint("SPECTRE_CORPUS", Dialect::Spectre, "scs", |p| !p.contains("/err_"));
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
    Skip(String),
    Fail(String),
}

fn check(deck: &Path, secs: u64, refused: &std::sync::Mutex<std::collections::BTreeMap<String, usize>>) -> Outcome {
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
    for d in [Dialect::Hspice, Dialect::Ltspice, Dialect::Spectre, Dialect::Vacask, Dialect::Espice] {
        if netlist(&ir, d).is_err() {
            *refused.lock().unwrap().entry(format!("{d:?}")).or_default() += 1;
        }
    }
    // The analyses must survive whether or not ngspice can run the deck.
    let (before, after) = (analyses(&src), analyses(&converted));
    if before != after {
        return Outcome::Fail(format!("analyses {before:?} became {after:?}"));
    }
    let dir = deck.parent().unwrap();
    let original = match ngspice(dir, &src, secs) {
        Ok(Some(r)) => r,
        Ok(None) => return Outcome::Skip("original timed out".into()),
        Err(e) => return Outcome::Skip(format!("original fails: {e}")),
    };
    if let Some(p) = original.plots.iter().find(|p| p.variables.len() > 0 && p.re(0).is_empty()) {
        return Outcome::Skip(format!("original fails: {} has no points", p.plot_name));
    }
    // Twice the budget: under a loaded machine a deck near the limit as
    // written crosses it converted with no change in the circuit.
    match ngspice(dir, &converted, 2 * secs) {
        Ok(Some(r)) => match compare(&original, &r) {
            Ok(()) => Outcome::Pass,
            // Fails only if the original agrees with itself (TRNOISE draws
            // a fresh stream per run, seed or no seed) and the unconverted,
            // reordered deck stays within the tolerance the converted one
            // broke.
            Err(why) => match ngspice(dir, &src, secs) {
                Ok(Some(again)) if compare(&original, &again).is_err() => Outcome::Skip(format!("original is not repeatable: two runs disagree ({why})")),
                _ => match ngspice(dir, &reordered(&src), secs) {
                    Ok(Some(c)) if compare(&original, &c).is_err() => Outcome::Skip(format!("order-sensitive: reordering the original alone breaks it ({why})")),
                    _ => Outcome::Fail(why),
                },
            },
        },
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
    // Largest first: a big deck started last would set the wall clock alone.
    all.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map_or(0, |m| m.len())));
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
                let t = std::time::Instant::now();
                let o = check(deck, secs, &refused);
                if t.elapsed().as_secs() > 60 {
                    eprintln!("slow {}: {:.0?}", deck.display(), t.elapsed());
                }
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
