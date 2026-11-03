//! The transpiler front end over every input of the golden gate (TESTING.md,
//! "Golden gate": the corpora, the PDK kits and `tests/**` fixtures, found by
//! `tools/golden/inputs.rs`):
//!
//! - `lang_switch`: the Spectre fixtures' `simulator lang=spice` regions are
//!   HSPICE tokens, against `bare.scs` read by hand;
//! - `throughput` (`--ignored`): tokenize, parse and `ast_to_ir`, timed.
//!
//! The comparisons phase 1 and 2 made here against the old reader in
//! process (its logical lines, its number values, its IR) were retired when
//! `import::parse` switched to this front end (phase 2b): with one reader the
//! IR comparison would compare the reader with itself, and the recorded
//! `golden` baseline (`daa0ad0`) holds every input's IR, report and text,
//! which `golden diff` compares file for file; the lines and numbers it does
//! not hold are what the IR is made of.
//!
//!   nix develop .#testing -c env IHP_PDK=<checkout> cargo test -j 4 --release \
//!     --no-default-features --features corpus --test test_syntax -- --test-threads=4
//!
//! The corpus tier (`required-features = ["corpus"]`); outside `nix develop
//! .#testing` the corpora are absent and the test fails.
//!
//! Cites: `tests/fixtures/pdk_sections/bare.scs` (read by hand); Spectre UG
//! p.53 (`simulator lang=`).
//!
//! Contents: none (tests only).

use std::path::Path;
use std::time::Instant;

use spicerack::codegen::Dialect;
use spicerack::import::decode;
use spicerack::syntax::lexicon::{self, HSPICE, SPECTRE};
use spicerack::syntax::token::Tag;
use spicerack::syntax::{ast_to_ir, parse, tokenize};

#[path = "../tools/golden/inputs.rs"]
#[allow(dead_code)]
mod inputs;

// ---- private ----

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

/// Tokenize, parse and `ast_to_ir` throughput over the gate's inputs,
/// printed; `--ignored` runs it.
#[test]
#[ignore]
fn throughput() {
    let (mut bytes, mut tokens, mut nodes) = (0usize, 0usize, 0usize);
    let (mut lexing, mut parsing, mut adapting) = (0.0f64, 0.0f64, 0.0f64);
    for (key, src, d) in every_input() {
        let lex = lexicon::of(d).expect("lexicon");
        let at = Instant::now();
        let t = tokenize(&src, lex);
        lexing += at.elapsed().as_secs_f64();
        let at = Instant::now();
        let ast = parse(&t, lex);
        parsing += at.elapsed().as_secs_f64();
        let at = Instant::now();
        std::hint::black_box(ast_to_ir(&ast, &t, d, &key));
        adapting += at.elapsed().as_secs_f64();
        (bytes, tokens, nodes) = (bytes + src.len(), tokens + t.len(), nodes + ast.len());
    }
    let mb = bytes as f64 / 1e6;
    println!(
        "{mb:.1} MB, {tokens} tokens, {nodes} nodes: tokenize {lexing:.2} s ({:.1} MB/s), \
         parse {parsing:.2} s ({:.1} MB/s), ast_to_ir {adapting:.2} s",
        mb / lexing,
        mb / parsing
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
