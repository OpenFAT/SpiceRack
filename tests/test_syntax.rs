//! The transpiler front end over every input of the golden gate (TESTING.md,
//! "Golden gate": the corpora, the PDK kits and `tests/**` fixtures, found by
//! `tools/golden/inputs.rs`):
//!
//! - `lang_switch`: the Spectre fixtures' `simulator lang=spice` regions are
//!   HSPICE tokens, against `bare.scs` read by hand;
//! - `kits_resolve`: every PDK kit and fixture read as one `syntax::resolve`
//!   kit: its read order is the manifest's (`inputs.rs`'s walk), and every
//!   instance master binds or carries an `R0001`; the counts are printed;
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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use std::collections::BTreeMap;

use spicerack::codegen::Dialect;
use spicerack::import::decode;
use spicerack::syntax::ast::{self, NONE, Node};
use spicerack::syntax::diag::Code;
use spicerack::syntax::lexicon::{self, HSPICE, SPECTRE};
use spicerack::syntax::resolve::{Binding, DefKind, FileId, Kit, LoadError, Role};
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

#[test]
fn kits_resolve() {
    // Every kit file's every instance binds: to a primitive, a declaration
    // of the kit, or `Unknown` with an R0001 at its master (resolve's
    // contract); and the
    // kit reads its files in the order the golden walk records.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut manifest = Vec::new();
    let all = inputs::inputs(repo, &mut manifest);
    assert!(
        !manifest.iter().any(|l| l.starts_with("absent")),
        "run in `nix develop .#testing` with $IHP_PDK"
    );
    // Each kit once, with its set and the root its paths are shown against.
    let mut kits: Vec<(&str, PathBuf, Arc<Kit>)> = Vec::new();
    for i in &all {
        if let Some((k, _)) = &i.kit
            && !kits.iter().any(|(_, _, x)| Arc::ptr_eq(x, k))
        {
            let path = i.path.to_string_lossy();
            let root = path.strip_suffix(&i.rel).expect("rel ends the path");
            kits.push((i.set, PathBuf::from(root), Arc::clone(k)));
        }
    }
    let mut counts: BTreeMap<&str, BTreeMap<String, usize>> = BTreeMap::new();
    for (kset, kroot, kit) in &kits {
        let (kset, kroot): (&str, &Path) = (kset, kroot);
        if kset != "fixtures" {
            // The walk's files are the kit's, in the kit's order; the kit
            // also follows includes the walk skips (inside a subckt or an
            // `.if` branch), which are no golden inputs.
            let kit_files: Vec<(String, &'static str)> = kit
                .files()
                .iter()
                .map(|f| {
                    let shown = f.path.strip_prefix(kroot).unwrap_or(&f.path);
                    let what = match f.role {
                        Role::Unread(LoadError::Refused) => "outside",
                        Role::Unread(LoadError::NotFound) => "missing",
                        Role::VerilogA | Role::Deck | Role::Library => "order",
                    };
                    (shown.to_string_lossy().replace('\\', "/"), what)
                })
                .collect();
            let walked: Vec<(String, &'static str)> = manifest
                .iter()
                .filter_map(|l| {
                    let w: Vec<&str> = l.split(' ').collect();
                    match w.as_slice() {
                        ["order", set, _, path, ..] if *set == kset => {
                            Some((path.to_string(), "order"))
                        }
                        [what @ ("missing" | "outside"), set, path] if *set == kset => Some((
                            path.to_string(),
                            if *what == "missing" {
                                "missing"
                            } else {
                                "outside"
                            },
                        )),
                        _ => None,
                    }
                })
                .collect();
            let shared: Vec<&(String, &str)> =
                kit_files.iter().filter(|f| walked.contains(f)).collect();
            assert_eq!(
                shared,
                walked.iter().collect::<Vec<_>>(),
                "{}: read order",
                kset
            );
            let extra: Vec<&String> = kit_files
                .iter()
                .filter(|f| !walked.contains(f))
                .map(|f| &f.0)
                .collect();
            println!(
                "{}: {} files, {} past the walk: {extra:?}",
                kset,
                kit_files.len(),
                extra.len()
            );
        }
        let c = counts.entry(kset).or_default();
        let lex = lexicon::of(kit.dialect()).expect("a reader");
        for (k, file) in kit.files().iter().enumerate() {
            if !matches!(file.role, Role::Deck | Role::Library) {
                continue;
            }
            let id = FileId(k as u32);
            let t = tokenize(&file.text, lex);
            let a = parse(&t, lex);
            assert_eq!(a.len(), kit.node_count(id));
            let r0001: Vec<u32> = file
                .diags
                .iter()
                .filter(|d| d.code == Code::R0001)
                .map(|d| d.token)
                .collect();
            let mut unknown = 0;
            for n in (0..a.len() as u32).map(Node) {
                if a.tag(n) != ast::Tag::Instance {
                    continue;
                }
                let what = match kit.binding(id, n) {
                    None => "no device (letter)".to_string(),
                    Some(Binding::Primitive(p)) => format!("primitive {p:?}"),
                    Some(Binding::Def(d)) => match &kit.definition(d).kind {
                        DefKind::Subckt { .. } => "subckt".into(),
                        DefKind::Model { kind, .. } => format!("model {kind:?}"),
                        DefKind::Param | DefKind::Func | DefKind::Section => "param/func".into(),
                    },
                    Some(Binding::Unknown) => {
                        unknown += 1;
                        let m = a.header(n).master;
                        let at = if m == NONE {
                            a.main_token(n)
                        } else {
                            a.main_token(Node(m))
                        };
                        assert!(
                            r0001.contains(&at),
                            "{}: an unknown master has its R0001",
                            file.path.display()
                        );
                        "unknown".into()
                    }
                };
                *c.entry(what).or_default() += 1;
            }
            assert_eq!(
                unknown,
                r0001.len(),
                "{}: one R0001 per unknown",
                file.path.display()
            );
            for d in &file.diags {
                *c.entry(format!("diag {:?}", d.code)).or_default() += 1;
                if d.code == Code::R0003 {
                    let shown = file.path.strip_prefix(kroot).unwrap_or(&file.path);
                    println!("{kset}: R0003 {} at {:?}", shown.display(), t.text(d.token));
                }
            }
        }
    }
    for (set, c) in &counts {
        println!("{set}: {c:?}");
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
        std::hint::black_box(ast_to_ir(&ast, &t, d, &key, None));
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
