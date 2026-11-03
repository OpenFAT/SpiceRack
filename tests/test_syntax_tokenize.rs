//! The transpiler rewrite's phase-1 gate (`docs/research/transpiler-architecture.md`
//! §5): `spicerack::syntax::tokenize` against today's reader, over every input
//! of the golden gate (TESTING.md, "Golden gate": the corpora, the PDK kits and
//! `tests/**` fixtures, found by `tools/golden/inputs.rs`).
//!
//! - `lines`: each file's statements, rebuilt from its tokens, equal today's
//!   logical lines (`import::logical_lines`), whitespace collapsed.
//! - `numbers`: every `Number` token's value equals today's
//!   (`import::number_value`: the dialect's `Num` rule and the lowering's
//!   arithmetic) to the bit, and NaN exactly where today reads no number.
//! - `lang_switch`: the Spectre fixtures' `simulator lang=spice` regions are
//!   HSPICE tokens.
//!
//! Every difference is a line of `tests/syntax_tokenize.xfail`, which is
//! strict: an unlisted difference fails, and so does a listed one that no
//! longer differs.
//!
//!   nix develop .#testing -c env IHP_PDK=<checkout> cargo test -j 4 --release \
//!     --no-default-features --features corpus --test test_syntax_tokenize -- --test-threads=4
//!
//! The corpus tier (`required-features = ["corpus"]`); outside `nix develop
//! .#testing` the corpora are absent and the test fails.
//!
//! Cites: today's reader (`grammar/*.peg` line stages and `Num` rules,
//! `import/lower/values.rs`) as the oracle; VerA `tools/vera.xfail` (strict
//! xfail).
//!
//! Contents: none (tests only).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Instant;

use spicerack::codegen::Dialect;
use spicerack::import::{decode, logical_lines, number_value};
use spicerack::syntax::lexicon::{self, Backslash, HSPICE, SPECTRE};
use spicerack::syntax::token::{Tag, Tokens};
use spicerack::syntax::tokenize;

#[path = "../tools/golden/inputs.rs"]
#[allow(dead_code)]
mod inputs;

const XFAIL: &str = include_str!("syntax_tokenize.xfail");

// ---- private ----

#[test]
fn tokens_match_todays_lines_and_numbers() {
    let mut found = BTreeMap::new();
    let mut today: HashMap<(&str, String), Option<u64>> = HashMap::new();
    let (mut files, mut lines, mut numbers) = (0, 0, 0);
    for (key, src, d) in every_input() {
        let lex = lexicon::of(d).expect("every input dialect has a lexicon");
        let t = tokenize(&src, lex);
        files += 1;
        match logical_lines(&src, d) {
            Ok(want) => {
                // A statement may hold newlines (a VACASK `<<<` string, an
                // HSPICE quote across `\\`), so the texts compare whole.
                let want = norm(&want, d);
                let got = norm(
                    &statements(&t)
                        .iter()
                        .map(|s| format!("{s}\n"))
                        .collect::<String>(),
                    d,
                );
                let (want, got): (Vec<&str>, Vec<&str>) =
                    (want.lines().collect(), got.lines().collect());
                lines += want.len();
                if let Some(k) = (0..want.len().max(got.len())).find(|&k| want.get(k) != got.get(k))
                {
                    found.insert(
                        (key.clone(), "lines"),
                        format!("line {k}: today {:?}, tokens {:?}", want.get(k), got.get(k)),
                    );
                }
            }
            Err(e) => {
                found.insert(
                    (key.clone(), "lines"),
                    format!("today refuses the file: {e}"),
                );
            }
        }
        for i in (0..t.len() as u32).filter(|&i| t.tags()[i as usize] == Tag::Number) {
            let d = t.lexicon_at(i).dialect;
            let text = t.text(i);
            let want = *today
                .entry((d.name(), text.to_string()))
                .or_insert_with(|| number_value(text, d).map(f64::to_bits));
            let got = t.value(i);
            numbers += 1;
            let same = match want {
                Some(bits) => bits == got.to_bits(),
                None => got.is_nan(),
            };
            if !same {
                let at = t.location(i);
                found.entry((key.clone(), "numbers")).or_insert(format!(
                    "{text:?} at {}:{}: today {:?}, tokens {got:?}",
                    at.line,
                    at.column,
                    want.map(f64::from_bits)
                ));
            }
        }
    }
    println!(
        "{files} files, {lines} logical lines, {numbers} numbers, {} differences",
        found.len()
    );
    check_xfail(&found);
}

#[test]
fn lang_switch() {
    // tests/fixtures/pdk_sections/bare.scs, read by hand: one region holding
    // `.param rsh_spice=100` continued by `+ rvc=0.01`.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdk_sections/bare.scs");
    let src = decode(&std::fs::read(&path).expect("bare.scs"));
    let t = tokenize(&src, &SPECTRE);
    let tags = t.tags();
    let at = |tag: Tag| tags.iter().position(|&x| x == tag).expect("tag present") as u32;
    let (on, off) = (at(Tag::LangSpice), at(Tag::LangSpectre));
    assert_eq!(tags.iter().filter(|&&x| x == Tag::LangSpice).count(), 1);
    let region: Vec<&str> = (on + 1..off).map(|i| t.text(i)).collect();
    assert_eq!(
        region,
        [
            ".param",
            "rsh_spice",
            "=",
            "100",
            "rvc",
            "=",
            "0.01",
            "",
            "simulator",
            "lang",
            "=",
            "spectre",
            ""
        ]
    );
    assert_eq!(tags[on as usize + 1], Tag::KwParam);
    assert!((on + 1..off).all(|i| std::ptr::eq(t.lexicon_at(i), &HSPICE)));
    assert_eq!(tags[off as usize + 1], Tag::KwParam);
    assert!(std::ptr::eq(t.lexicon_at(off + 1), &SPECTRE));

    // Every Spectre fixture: one marker per `simulator lang=spice` line, and
    // nothing in a region lexed by Spectre's rules.
    for (key, src, d) in
        every_input().filter(|(k, _, d)| *d == Dialect::Spectre && k.starts_with("fixtures/"))
    {
        let t = tokenize(&src, &SPECTRE);
        let lines = src
            .lines()
            .filter(|l| {
                let w: String = l.chars().filter(|c| !c.is_whitespace()).collect();
                w.starts_with("simulatorlang=spice") && !w.starts_with("simulatorlang=spectre")
            })
            .count();
        let markers = t.tags().iter().filter(|&&x| x == Tag::LangSpice).count();
        assert_eq!(markers, lines, "{key}: {d:?}");
    }
}

/// Tokenize throughput over the corpora, printed; `--ignored` runs it.
#[test]
#[ignore]
fn throughput() {
    let (mut bytes, mut tokens, mut spent) = (0usize, 0usize, 0.0f64);
    for (_, src, d) in every_input() {
        let lex = lexicon::of(d).expect("lexicon");
        let start = Instant::now();
        let t = tokenize(&src, lex);
        spent += start.elapsed().as_secs_f64();
        bytes += src.len();
        tokens += t.len();
    }
    println!(
        "tokenize: {bytes} bytes, {tokens} tokens in {spent:.3} s: {:.1} MB/s",
        bytes as f64 / spent / 1e6
    );
}

/// Every golden-gate input as `(set/relpath, the text today reads, dialect)`,
/// read one at a time.
fn every_input() -> impl Iterator<Item = (String, String, Dialect)> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
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
    all.into_iter().map(|i| {
        let text = decode(&std::fs::read(&i.path).expect("input readable"));
        let (src, _) = inputs::source(&text, i.form, &i.path);
        (format!("{}/{}", i.set, i.rel), src, i.dialect)
    })
}

/// The logical statements but the title and `.end`, each its tokens joined
/// as the line stage joins its parts: nothing between tokens that touch or
/// that only an HSPICE `\\` glue separates, one blank otherwise.
fn statements(t: &Tokens) -> Vec<String> {
    let src = t.source();
    let (mut out, mut line, mut prev): (Vec<String>, String, Option<u32>) =
        (Vec::new(), String::new(), None);
    let mut skip = false;
    for i in 0..t.len() as u32 {
        match t.tags()[i as usize] {
            Tag::EndOfStatement => {
                if !line.is_empty() && !skip {
                    out.push(std::mem::take(&mut line));
                }
                (line, prev, skip) = (String::new(), None, false);
            }
            Tag::LangSpice | Tag::LangSpectre => {}
            tag => {
                skip |= prev.is_none() && matches!(tag, Tag::Title | Tag::KwEnd);
                if let Some(p) = prev {
                    let gap = &src[t.end(p) as usize..t.start(i) as usize];
                    let glue = t.lexicon_at(i).backslash == Backslash::Hspice && is_glue(gap);
                    if !gap.is_empty() && !glue {
                        line.push(' ');
                    }
                }
                line.push_str(t.text(i));
                prev = Some(i);
            }
        }
    }
    out
}

/// hspice.peg `l_glue_break`: blanks, `\\`, blanks, the newline, blanks.
fn is_glue(gap: &str) -> bool {
    gap.trim_matches([' ', '\t'])
        .strip_prefix("\\\\")
        .is_some_and(|r| r.trim_start_matches([' ', '\t', '\r']) == "\n")
}

/// `s` with each run of blanks one blank and every line trimmed. VACASK's
/// line stage joins a bracket to its neighbours by its own rule (`Glue`, no
/// blank before, one after), and blanks around a bracket mean nothing there,
/// so for VACASK they go.
fn norm(s: &str, d: Dialect) -> String {
    let bracket = |c: char| "()[]{}".contains(c);
    let mut out = String::new();
    for line in s.split('\n') {
        let line = line.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
        if d == Dialect::Vacask {
            let chars: Vec<char> = line.chars().collect();
            let keep = |k: usize| {
                chars[k] != ' '
                    || !(chars.get(k.wrapping_sub(1)).is_some_and(|&p| bracket(p))
                        || chars.get(k + 1).is_some_and(|&n| bracket(n)))
            };
            out.extend((0..chars.len()).filter(|&k| keep(k)).map(|k| chars[k]));
        } else {
            out.push_str(&line);
        }
        out.push('\n');
    }
    out
}

/// Fails on a difference `XFAIL` does not list, and on a listed one that is
/// gone.
fn check_xfail(found: &BTreeMap<(String, &'static str), String>) {
    let mut listed = BTreeMap::new();
    for line in XFAIL.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split(' ').collect();
        let i = (1..words.len())
            .find(|&i| matches!(words[i], "lines" | "numbers"))
            .unwrap_or_else(|| panic!("xfail line has no check: {line}"));
        assert!(i + 1 < words.len(), "xfail line has no reason: {line}");
        listed.insert((words[..i].join(" "), words[i].to_string()), ());
    }
    let unlisted: Vec<String> = found
        .iter()
        .filter(|((k, c), _)| !listed.contains_key(&(k.clone(), c.to_string())))
        .map(|((k, c), why)| format!("{k} {c}: {why}"))
        .collect();
    let xpass: Vec<String> = listed
        .keys()
        .filter(|(k, c)| !found.keys().any(|(fk, fc)| fk == k && fc == c))
        .map(|(k, c)| format!("{k} {c}"))
        .collect();
    assert!(
        unlisted.is_empty() && xpass.is_empty(),
        "{} unlisted differences:\n{}\n{} xfail lines that pass (delete them):\n{}",
        unlisted.len(),
        unlisted.join("\n"),
        xpass.len(),
        xpass.join("\n")
    );
}
