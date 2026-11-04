//! Two golden trees -> `IDENTICAL`, or every differing file, under a strict
//! xfail list.
//!
//! A difference is keyed by `(<set>/<relpath>, <target>)`: the input's
//! directory and the output file's stem (a dialect name, or `parse`), so
//! `ngspice.txt` and `ngspice.refused` are one key. A baseline's `ir.json`
//! and `report.json` (the IR and its report, gone in phase 6) are passed
//! over, as is every file of the target `--skip` names. An xfail
//! line is `<set>/<relpath> <target> <reason>`; `#` starts a comment. Strict,
//! as VerA's: an unlisted difference fails, and so does a listed key that is
//! now identical (`XPASS`). `MANIFEST` is compared without its `output`
//! lines (the files say that); any other difference there (an input's hash,
//! a set, a kit's read order) means the two trees read different inputs,
//! which no xfail line excuses.
//!
//! A refusal the renderer wrote (`codes: …` on its first line) is compared
//! by its codes, never its messages: two such files with one code line are
//! the same.
//!
//! With a refusal report (`--refusal-status-only <file>`), a key refused on
//! both sides is the same whatever the two texts say (the old refusals are
//! free text, the renderer's are `W` codes until phase 5's messages), and
//! each such pair whose texts differ is written to the report, old above
//! new, for review; a listed key is then no XPASS. Written versus refused
//! still differs.
//!
//! Cites: VerA `tools/vera.xfail:1-5` (strict xfail).
//!
//! Contents:
//! - `pub(super) const CODES: &str`
//! - `pub(super) fn diff(a: &Path, b: &Path, xfail: Option<&Path>, report: Option<&Path>, skip: Option<&str>) -> std::io::Result<bool>`

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use spicerack::syntax::Dialect;

/// The first line of a renderer's refusal starts so, its codes after it.
pub(super) const CODES: &str = "codes: ";

/// Output-file stems that are not dialects.
const STEMS: [&str; 1] = ["parse"];

/// Output-file stems a baseline holds that no record writes any more.
const GONE: [&str; 2] = ["ir", "report"];

/// Diffs `a` against `b` and prints the verdict; `Ok(true)` when every
/// difference is listed in `xfail` and every listed key differs. With
/// `skip`, that target's files are not compared and its xfail lines not
/// checked.
pub(super) fn diff(
    a: &Path,
    b: &Path,
    xfail: Option<&Path>,
    report: Option<&Path>,
    skip: Option<&str>,
) -> std::io::Result<bool> {
    let listed = match xfail {
        Some(f) => parse_xfail(&std::fs::read_to_string(f)?)
            .map_err(|e| std::io::Error::other(format!("{}: {e}", f.display())))?,
        None => BTreeMap::new(),
    };
    let mut ok = true;
    // MANIFEST, less its output lines.
    let inputs = |root: &Path| -> std::io::Result<BTreeSet<String>> {
        let text = std::fs::read_to_string(root.join("MANIFEST"))?;
        let lines = text.lines().filter(|l| !l.starts_with("output "));
        Ok(lines.map(str::to_string).collect())
    };
    let (ma, mb) = (inputs(a)?, inputs(b)?);
    for l in ma.difference(&mb) {
        println!("MANIFEST -{l}");
        ok = false;
    }
    for l in mb.difference(&ma) {
        println!("MANIFEST +{l}");
        ok = false;
    }
    // Every output file, by key.
    let (mut fa, mut fb) = (BTreeSet::new(), BTreeSet::new());
    walk(a, "", &mut fa);
    walk(b, "", &mut fb);
    let mut differ: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut refusals = String::new();
    // Keys refused on both sides with other texts: the same, in the mode.
    let mut refused_both: BTreeSet<(String, String)> = BTreeSet::new();
    let passed = |rel: &str| {
        let t = key(rel).1;
        GONE.contains(&t.as_str()) || Some(t.as_str()) == skip
    };
    for rel in fa.union(&fb).filter(|r| !passed(r)) {
        let both = fa.contains(rel) && fb.contains(rel);
        let (ta, tb) = match both {
            true => (std::fs::read(a.join(rel))?, std::fs::read(b.join(rel))?),
            false => (vec![], vec![]),
        };
        if both && ta == tb {
            continue;
        }
        if both && rel.ends_with(".refused") && codes(&ta).is_some_and(|c| Some(c) == codes(&tb)) {
            continue;
        }
        if both && report.is_some() && rel.ends_with(".refused") {
            let k = key(rel);
            refused_both.insert(k.clone());
            refusals.push_str(&format!(
                "== {} {}\n-- {}\n{}-- {}\n{}\n",
                k.0,
                k.1,
                a.display(),
                String::from_utf8_lossy(&ta),
                b.display(),
                String::from_utf8_lossy(&tb)
            ));
            continue;
        }
        differ.entry(key(rel)).or_default().push(rel.clone());
    }
    if let Some(r) = report {
        std::fs::write(r, refusals)?;
    }
    let mut xfailed = 0;
    for (k, files) in &differ {
        if listed.contains_key(k) {
            xfailed += 1;
            continue;
        }
        ok = false;
        for rel in files {
            let side = |f: &BTreeSet<String>| if f.contains(rel) { "" } else { " (absent)" };
            println!(
                "DIFF {rel}: {}{} vs {}{}",
                a.display(),
                side(&fa),
                b.display(),
                side(&fb)
            );
        }
    }
    for (k, reason) in &listed {
        // A listed refusal text is no XPASS when only the status is compared.
        let skipped = Some(k.1.as_str()) == skip;
        if !differ.contains_key(k) && !refused_both.contains(k) && !skipped {
            println!("XPASS {} {}: listed ({reason}) but identical", k.0, k.1);
            ok = false;
        }
    }
    match (ok, xfailed) {
        (true, 0) => println!("IDENTICAL"),
        (true, n) => println!("IDENTICAL except {n} listed xfail(s)"),
        (false, _) => {}
    }
    Ok(ok)
}

// ---- private ----

/// Every file under `root/rel`, as `/`-separated paths, less `MANIFEST`.
fn walk(root: &Path, rel: &str, out: &mut BTreeSet<String>) {
    let Ok(rd) = std::fs::read_dir(root.join(rel)) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if e.path().is_dir() {
            walk(root, &path, out);
        } else if path != "MANIFEST" {
            out.insert(path);
        }
    }
}

/// A renderer's refusal's code line; `None` for another text.
fn codes(text: &[u8]) -> Option<&[u8]> {
    let line = text.split(|&b| b == b'\n').next()?;
    line.starts_with(CODES.as_bytes()).then_some(line)
}

/// `<set>/<relpath>/<target>.<ext>` -> `(<set>/<relpath>, <target>)`.
fn key(rel: &str) -> (String, String) {
    let (input, file) = rel.rsplit_once('/').unwrap_or(("", rel));
    let target = file.split_once('.').map_or(file, |(t, _)| t);
    (input.to_string(), target.to_string())
}

/// The xfail list: key -> reason. The target is the first word that names
/// one, so a path may hold spaces.
fn parse_xfail(text: &str) -> Result<BTreeMap<(String, String), String>, String> {
    let targets: Vec<&str> = Dialect::ALL.iter().map(|d| d.name()).chain(STEMS).collect();
    let mut out = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split(' ').collect();
        let Some(i) = (1..words.len()).find(|&i| targets.contains(&words[i])) else {
            return Err(format!("line {}: no target in `{line}`", n + 1));
        };
        let reason = words[i + 1..].join(" ");
        if reason.trim().is_empty() {
            return Err(format!("line {}: no reason in `{line}`", n + 1));
        }
        let k = (words[..i].join(" "), words[i].to_string());
        if out.insert(k, reason).is_some() {
            return Err(format!("line {}: listed twice", n + 1));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_the_input_and_the_file_stem() {
        let k = key("spice-bench/a b/c.sp/ngspice.refused");
        assert_eq!(k, ("spice-bench/a b/c.sp".into(), "ngspice".into()));
        assert_eq!(key("fixtures/x.scs/parse.refused").1, "parse");
    }

    #[test]
    fn a_coded_refusal_is_its_code_line() {
        let a = b"codes: W0013 W0013\nt:3:1: W0013 a conditional block the target lacks\n";
        let b = b"codes: W0013 W0013\nt:4:1: W0013 reworded\n";
        assert_eq!(codes(a), codes(b));
        assert!(codes(a).is_some());
        assert_ne!(codes(a), codes(b"codes: W0013\n"));
        assert_eq!(codes(b"Unsupported component\n"), None);
    }

    #[test]
    fn xfail_lines_need_a_target_and_a_reason() {
        let got = parse_xfail("# c\nset/a b.sp spectre HashMap order\n").unwrap();
        let k = ("set/a b.sp".to_string(), "spectre".to_string());
        assert_eq!(got[&k], "HashMap order");
        assert!(parse_xfail("set/a.sp spectre\n").is_err());
        assert!(parse_xfail("set/a.sp nothing here\n").is_err());
        assert!(parse_xfail("s/a ir r\ns/a ir r\n").is_err());
    }
}
