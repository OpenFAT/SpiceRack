//! The transpiler rewrite's phase-2 gate, in process (`docs/research/
//! transpiler-architecture.md` §5): `syntax::ast_to_ir(syntax::parse(
//! syntax::tokenize(..)))` against today's `import::parse`, over every input
//! of the golden gate (TESTING.md, "Golden gate"), comparing the `CircuitIR`
//! and the `Report` as the golden record writes them (pretty JSON). Equal
//! IR means equal text from every writer, so this is the fast form of
//! `golden diff` with `golden record --frontend syntax`, which stays the
//! gate of record.
//!
//! `SYNTAX_SET=<set>` limits the run to one input set; `SYNTAX_SHOW=<n>`
//! prints the first `n` differences (default 5) with their first differing
//! JSON line. Strict xfail: `migration.xfail`'s `ir` and `report` lines.
//!
//!   nix develop .#testing -c env IHP_PDK=<checkout> cargo test -j 4 --release \
//!     --no-default-features --features corpus --test test_syntax_adapter -- --test-threads=4
//!
//! Cites: today's reader (`import::parse`) as the oracle; VerA
//! `tools/vera.xfail` (strict xfail).
//!
//! Contents: none (tests only).

use std::collections::BTreeMap;
use std::path::Path;

use spicerack::import::{decode, parse};
use spicerack::syntax::{ast_to_ir, lexicon, tokenize};

#[path = "../tools/golden/inputs.rs"]
#[allow(dead_code)]
mod inputs;

const XFAIL: &str = include_str!("../migration.xfail");

// ---- private ----

#[test]
fn ast_to_ir_matches_todays_reader() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let only = std::env::var("SYNTAX_SET").ok();
    let show: usize = std::env::var("SYNTAX_SHOW")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let mut manifest = Vec::new();
    let all = inputs::inputs(repo, &mut manifest);
    let absent: Vec<&String> = manifest
        .iter()
        .filter(|l| l.starts_with("absent"))
        .collect();
    assert!(
        absent.is_empty(),
        "run in `nix develop .#testing` with $IHP_PDK: {absent:?}"
    );
    let mut per_set: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    for i in &all {
        if only.as_deref().is_some_and(|s| s != i.set) {
            continue;
        }
        let text = decode(&std::fs::read(&i.path).expect("input readable"));
        let (src, name) = inputs::source(&text, i.form, &i.path);
        let key = format!("{}/{}", i.set, i.rel);
        let lex = lexicon::of(i.dialect).expect("a lexicon");
        let (want_ir, want_rep) = parse(&src, &name, i.dialect).expect("today reads every input");
        let t = tokenize(&src, lex);
        let ast = spicerack::syntax::parse(&t, lex);
        let (got_ir, got_rep) = ast_to_ir(&ast, &t, i.dialect, &name);
        let e = per_set.entry(i.set).or_default();
        e.0 += 1;
        for (target, want, got) in [
            ("ir", json(&want_ir), json(&got_ir)),
            ("report", report(&want_rep), report(&got_rep)),
        ] {
            if want != got {
                if target == "ir" {
                    e.1 += 1;
                }
                found.insert(format!("{key} {target}"), first_diff(&want, &got));
            }
        }
    }
    for (set, (n, bad)) in &per_set {
        println!("{set}: {n} inputs, {bad} IR differences");
    }
    let listed: Vec<String> = XFAIL
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let w: Vec<&str> = l.splitn(3, ' ').collect();
            (w.len() == 3 && matches!(w[1], "ir" | "report")).then(|| format!("{} {}", w[0], w[1]))
        })
        .filter(|k| {
            only.as_deref()
                .is_none_or(|s| k.starts_with(&format!("{s}/")))
        })
        .collect();
    let unlisted: Vec<_> = found.iter().filter(|(k, _)| !listed.contains(k)).collect();
    let xpass: Vec<_> = listed.iter().filter(|k| !found.contains_key(*k)).collect();
    for (k, why) in unlisted.iter().take(show) {
        println!("DIFF {k}\n{why}");
    }
    assert!(
        unlisted.is_empty() && xpass.is_empty(),
        "{} unlisted differences, {} xfail lines that pass: {:?}",
        unlisted.len(),
        xpass.len(),
        xpass
    );
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string_pretty(v).expect("IR serialises")
}

fn report(r: &spicerack::import::Report) -> String {
    let v = serde_json::json!({ "unparsed": r.unparsed, "raw": r.raw });
    format!("{v:#}")
}

/// The first differing line of two texts, with two lines of context.
fn first_diff(want: &str, got: &str) -> String {
    let (w, g): (Vec<&str>, Vec<&str>) = (want.lines().collect(), got.lines().collect());
    let k = (0..w.len().max(g.len()))
        .find(|&k| w.get(k) != g.get(k))
        .unwrap_or(0);
    let at = |v: &[&str]| v.get(k).map_or("<end>", |l| l.trim()).to_string();
    format!("  line {k}: today {} | syntax {}", at(&w), at(&g))
}
