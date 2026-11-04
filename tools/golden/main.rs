//! Every input file -> the reader and every writer -> a tree of golden
//! files: what each phase of the transpiler rewrite must reproduce byte for
//! byte against the recorded `daa0ad0` baseline (TESTING.md, "Golden
//! gate").
//!
//!   golden record [--writer syntax] <out_dir>
//!   golden diff <a> <b> [--xfail migration.xfail] [--refusal-status-only <report>]
//!   golden check-determinism
//!
//! `record` reads every input `inputs.rs` finds (run it in `nix develop
//! .#testing`, which sets the corpus and PDK variables; `$IHP_PDK` names an
//! IHP-Open-PDK checkout) and writes, under `<out_dir>/<set>/<relpath>/`:
//! - `ir.json`, the `CircuitIR` as pretty JSON, and `report.json`, the
//!   `Report` (`unparsed` lines, `raw` count);
//! - for every target dialect, `<target>.txt`, the exact text of
//!   `codegen::netlist`, or `<target>.refused`, its error text;
//! - `parse.refused` instead of all of these when `parse` refuses the file;
//! - `<name>.panicked`, the panic message, when a call panics.
//!
//! With `--writer syntax`, every target `syntax::render::writes` names is
//! written by the transpiler's renderer (`syntax::render`, phase 4) from the
//! file's AST and kit instead of `codegen::netlist` from its IR; a refusal
//! is its codes, sorted, on a first line (`codes: P0001 W0013`), then its
//! messages (`diag::messages`: `<deck>:<line>:<column>: <code> <title>`),
//! which `diff` does not compare. The other targets, `ir.json` and
//! `report.json` are recorded as without it.
//!
//! The reader is the transpiler's front end (phase 2b): `import::parse` for
//! a corpus deck, `import::parse_kit` for a PDK kit file or a fixture, read
//! in its `syntax::resolve::Kit` (phase 3). A record of `daa0ad0` (the PEG
//! reader it replaced) is the oracle a new record is diffed against.
//!
//! `MANIFEST` lists the sets, each kit's read order, each input's content
//! hash and each output file's. `diff` prints `IDENTICAL` or every differing
//! file and exits 1, with strict xfail (`diff.rs`). `check-determinism` runs
//! `record` twice, in two processes, and diffs the trees.
//!
//! Cites: the compiler-construction skill, rule 9; VerA `tools/vera.xfail`
//! (strict xfail).
//!
//! Contents: none (a binary; `main` is its entry).

use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::{decode, parse, parse_kit};
use spicerack::syntax::render::{self, Diags};

mod diff;
mod inputs;

use inputs::{Form, Input, inputs, source};

const USAGE: &str = "usage: golden record [--writer syntax] <out_dir> | \
                     golden diff <a> <b> [--xfail <file>] [--refusal-status-only <report>] | \
                     golden check-determinism";

/// Input dialects no set reads, and why: the manifest's `skip` lines.
/// Every target is written for every input that parses; none is skipped.
// The text is recorded in every MANIFEST, which `diff` compares, so it stays
// byte for byte as the `daa0ad0` baseline holds it (`import::lang` was the
// PEG reader's; `import::parse` now refuses the two dialects the same way).
const SKIPPED: [&str; 3] = [
    "skip input espice: reads ngspice text (Dialect::family), so an ESPice input is an ngspice input",
    "skip input cdl: no reader (import::lang refuses it)",
    "skip input verilog: no reader (import::lang refuses it)",
];

// ---- private ----

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rest: Vec<&str> = args.iter().map(String::as_str).collect();
    let xfail = match rest.iter().position(|a| *a == "--xfail") {
        Some(i) if i + 1 < rest.len() => {
            let f = PathBuf::from(rest[i + 1]);
            rest.drain(i..i + 2);
            Some(f)
        }
        Some(_) => return usage(),
        None => None,
    };
    let Ok(report) = flag(&mut rest, "--refusal-status-only") else {
        return usage();
    };
    let syntax = match flag(&mut rest, "--writer") {
        Ok(None) => false,
        Ok(Some(w)) if w == "syntax" => true,
        _ => return usage(),
    };
    let run = match rest.as_slice() {
        ["record", out] if xfail.is_none() && report.is_none() => {
            record(Path::new(out), syntax).map(|()| true)
        }
        ["diff", a, b] if !syntax => diff::diff(
            Path::new(a),
            Path::new(b),
            xfail.as_deref(),
            report.as_deref().map(Path::new),
        ),
        ["check-determinism"] if xfail.is_none() && report.is_none() => check_determinism(syntax),
        _ => return usage(),
    };
    match run {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("golden: {e}");
            ExitCode::from(2)
        }
    }
}

/// Removes `name <value>` from `rest`; `Err` when the value is missing.
fn flag(rest: &mut Vec<&str>, name: &str) -> Result<Option<String>, ()> {
    match rest.iter().position(|a| *a == name) {
        Some(i) if i + 1 < rest.len() => {
            let v = rest[i + 1].to_string();
            rest.drain(i..i + 2);
            Ok(Some(v))
        }
        Some(_) => Err(()),
        None => Ok(None),
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

/// Records every input into `out`, which must be absent or empty, so no
/// stale file from an earlier record survives into the tree.
fn record(out: &Path, syntax: bool) -> std::io::Result<()> {
    if std::fs::read_dir(out).is_ok_and(|mut d| d.next().is_some()) {
        return Err(std::io::Error::other(format!(
            "{} is not empty",
            out.display()
        )));
    }
    let mut manifest: Vec<String> = SKIPPED.iter().map(|s| s.to_string()).collect();
    let all = inputs(Path::new(env!("CARGO_MANIFEST_DIR")), &mut manifest);
    eprintln!("golden: {} inputs", all.len());
    let jobs = std::env::var("GOLDEN_JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .unwrap_or(4);
    let next = AtomicUsize::new(0);
    let lines = Mutex::new(manifest);
    let failed = Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..jobs {
            // A deck nested deep in `.if`s is an IR nested as deep, whose
            // writers and drop recurse; the stack is address space, not
            // resident memory.
            let worker = std::thread::Builder::new().stack_size(256 << 20);
            let spawned = worker.spawn_scoped(s, || {
                while let Some(input) = all.get(next.fetch_add(1, Ordering::Relaxed)) {
                    match record_one(out, input, syntax) {
                        Ok(v) => lines.lock().expect("no worker panics").extend(v),
                        Err(e) => {
                            let why = format!("{}/{}: {e}", input.set, input.rel);
                            *failed.lock().expect("no worker panics") = Some(why);
                            next.store(all.len(), Ordering::Relaxed);
                        }
                    }
                }
            });
            spawned.expect("spawn a worker");
        }
    });
    if let Some(why) = failed.into_inner().expect("no worker panics") {
        return Err(std::io::Error::other(why));
    }
    let mut lines = lines.into_inner().expect("no worker panics");
    lines.sort();
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(out.join("MANIFEST"), text)
}

/// Records one input; returns its manifest lines (input and output hashes).
fn record_one(out: &Path, input: &Input, syntax: bool) -> std::io::Result<Vec<String>> {
    let key = format!("{}/{}", input.set, input.rel);
    let dir = out.join(&key);
    std::fs::create_dir_all(&dir)?;
    let bytes = std::fs::read(&input.path)?;
    let form = match input.form {
        Form::Deck => "deck",
        Form::Library => "library",
    };
    let mut lines = vec![format!(
        "input {key} {} {form} {}",
        input.dialect.name(),
        hash(&bytes)
    )];
    let mut put = |name: &str, body: &str| -> std::io::Result<()> {
        std::fs::File::create(dir.join(name))?.write_all(body.as_bytes())?;
        lines.push(format!("output {key}/{name} {}", hash(body.as_bytes())));
        Ok(())
    };
    let (src, name) = source(&decode(&bytes), input.form, &input.path);
    let read = || match &input.kit {
        Some((kit, f)) => {
            assert_eq!(kit.file(*f).text, src, "the kit read this text");
            Ok(parse_kit(kit, *f, &name))
        }
        None => parse(&src, &name, input.dialect),
    };
    let (ir, report) = match catch_unwind(AssertUnwindSafe(read)) {
        Ok(Ok(parsed)) => parsed,
        Ok(Err(e)) => {
            put("parse.refused", &format!("{e}\n"))?;
            return Ok(lines);
        }
        Err(p) => {
            put("parse.panicked", &panic_text(p))?;
            return Ok(lines);
        }
    };
    let ir_json = serde_json::to_string_pretty(&ir).map_err(std::io::Error::other)?;
    put("ir.json", &format!("{ir_json}\n"))?;
    let report = serde_json::json!({ "unparsed": report.unparsed, "raw": report.raw });
    put("report.json", &format!("{report:#}\n"))?;
    for d in Dialect::ALL {
        let t = d.name();
        let written = if syntax && render::writes(d) {
            let render = || match &input.kit {
                Some((kit, f)) => render::render_kit(kit, *f, &name, d),
                None => render::render_text(&src, input.dialect, &name, d),
            };
            catch_unwind(AssertUnwindSafe(render))
                .map(|r| r.map(|w| w.text).map_err(|e| refused(&e)))
        } else {
            catch_unwind(AssertUnwindSafe(|| netlist(&ir, d)))
                .map(|r| r.map_err(|e| format!("{e}\n")))
        };
        match written {
            Ok(Ok(text)) => put(&format!("{t}.txt"), &text)?,
            Ok(Err(e)) => put(&format!("{t}.refused"), &e)?,
            Err(p) => put(&format!("{t}.panicked"), &panic_text(p))?,
        }
    }
    Ok(lines)
}

/// A renderer's refusal as text: its codes, sorted, on one line (what
/// `diff` compares), then its messages.
fn refused(d: &Diags) -> String {
    let mut codes: Vec<String> = d.diags.iter().map(|d| format!("{:?}", d.code)).collect();
    codes.sort();
    format!("{}{}\n{}", diff::CODES, codes.join(" "), d.messages)
}

/// A panic's message, without its location (which moves with every edit).
fn panic_text(p: Box<dyn std::any::Any + Send>) -> String {
    let msg = p
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a non-string panic".into());
    format!("{msg}\n")
}

/// FNV-1a, 64 bits, as 16 hex digits: stable across builds and machines.
// ponytail: an identity check, not a security one; sha256 if a manifest ever
// gates something untrusted.
fn hash(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{h:016x}")
}

/// Records twice into a temp dir, each in its own process (a `HashMap`'s
/// seed is per process), and diffs the two trees with no xfail list (both
/// come from one reader, so any listed line would XPASS). Removes them on
/// success.
fn check_determinism(syntax: bool) -> std::io::Result<bool> {
    let base = std::env::temp_dir().join(format!("golden-determinism-{}", std::process::id()));
    let exe = std::env::current_exe()?;
    for run in ["a", "b"] {
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("record");
        if syntax {
            cmd.args(["--writer", "syntax"]);
        }
        let status = cmd.arg(base.join(run)).status()?;
        if !status.success() {
            return Err(std::io::Error::other(format!(
                "record {run} failed: {status}"
            )));
        }
    }
    let same = diff::diff(&base.join("a"), &base.join("b"), None, None)?;
    if same {
        std::fs::remove_dir_all(&base)?;
    } else {
        eprintln!("golden: kept both records under {}", base.display());
    }
    Ok(same)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_the_reference_vectors() {
        assert_eq!(hash(b""), "cbf29ce484222325");
        assert_eq!(hash(b"a"), "af63dc4c8601ec8c");
        assert_eq!(hash(b"foobar"), "85944171f73967e8");
    }
}
