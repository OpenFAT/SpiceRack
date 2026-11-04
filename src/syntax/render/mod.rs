//! [`Ast`] (and its [`Kit`]) → deck text in any dialect: the transpiler's
//! last stage (`docs/research/transpiler-architecture.md` §2.5). Two
//! structural writers, because there are two statement shapes: the SPICE
//! family (`spice`: ngspice, ESPice, HSPICE, Xyce, LTspice, CDL) and the named
//! family (`named`: Spectre, VACASK). Each reads the reader's plan
//! (`read`: which statement is written where, and its views), spells
//! through the target's [`spelling::Spelling`] row, asks
//! [`capability::action`] for everything a target may not say, and writes
//! every byte through one token choke point (`out`).
//!
//! A written deck ([`Written`]) is the old pipeline's text, byte for byte
//! (TESTING.md, "Golden gate"): the Python API's `codegen::netlist(&CircuitIR,
//! d)` stays until phase 6. Either way the file's diagnostics come with it
//! ([`Diags`]): the parser's and the kit's notes (`P`, `R`) and the
//! writer's (`W`), sorted by source position, with their messages; a
//! refusal is a writer's `W` code of `diag::Kind::Refusal`.
//!
//! Cites: `transpiler-architecture.md` §2.5, §2.6, §5 phase 4;
//! `compiler-architecture-reference.md` §4 (Zig `Ast/Render.zig`), §12.4,
//! §12.5.
//!
//! Contents:
//! - `pub mod capability`, `pub mod spelling`
//! - `pub struct Source<'a>`, `pub struct Written`, `pub struct Diags`
//! - `pub fn render(src: &Source, target: Dialect) -> Result<Written, Diags>`
//! - `pub fn render_kit(kit: &Kit, file: FileId, name: &str, target: Dialect) -> Result<Written, Diags>`
//! - `pub fn render_text(text: &str, d: Dialect, name: &str, target: Dialect) -> Result<Written, Diags>`
//! - `pub fn writes(target: Dialect) -> bool`

pub mod capability;
mod draws;
mod named;
mod names;
mod out;
mod read;
mod role;
pub mod spelling;
mod spice;

use crate::codegen::Dialect;
use crate::syntax::ast::Ast;
use crate::syntax::diag::{self, Diag, Kind};
use crate::syntax::resolve::{FileId, Kit, Role};
use crate::syntax::token::Tokens;
use crate::syntax::{lexicon, parse, tokenize};

/// One file to write: its tree and tokens, the dialect it was read in, the
/// name its deck is written under (the title), and the kit it was read in,
/// whose answers fill what the file alone leaves open.
pub struct Source<'a> {
    pub ast: &'a Ast<'a>,
    pub tokens: &'a Tokens<'a>,
    pub dialect: Dialect,
    pub name: &'a str,
    pub kit: Option<(&'a Kit, FileId)>,
}

/// A deck written, and the notes on its file.
#[derive(Debug)]
pub struct Written {
    pub text: String,
    /// Every diagnostic on the file; none refuses.
    pub notes: Diags,
}

/// A file's diagnostics, sorted by source position ([`diag::sort`]), and
/// their messages ([`diag::messages`], one a line, named after the deck).
#[derive(Debug)]
pub struct Diags {
    pub diags: Vec<Diag>,
    pub messages: String,
}

/// `src` written for `target`, with the file's notes; every diagnostic on
/// the file, the `W` refusals among them, when the target cannot say
/// something the deck holds.
///
/// # Panics
/// When `target` is not one [`writes`] names; when `src.kit`'s file is not
/// `src.ast`'s.
pub fn render(src: &Source, target: Dialect) -> Result<Written, Diags> {
    assert!(writes(target), "render: a target with a writer");
    let (mut rd, deck) = read::read(src.ast, src.tokens, src.dialect, src.name, src.kit);
    let (text, written) = match target {
        Dialect::Spectre | Dialect::Vacask => named::write(&mut rd, &deck, target),
        _ => spice::write(&mut rd, &deck, target),
    };
    // The earlier stages' notes: the kit's for its file (the parser's
    // copied, then its own), else the parser's; then the reader's and the
    // writer's.
    let mut diags = match src.kit {
        Some((kit, f)) => kit.file(f).diags.clone(),
        None => src.ast.diags().to_vec(),
    };
    diags.extend(deck.notes.iter().copied());
    diags.extend(written);
    let refused = diags
        .iter()
        .any(|d| diag::info(d.code).kind == Kind::Refusal);
    let text = (!refused).then_some(text);
    diag::sort(&mut diags);
    let messages = diag::messages(&diags, src.tokens, src.name);
    let diags = Diags { diags, messages };
    match text {
        Some(text) => Ok(Written { text, notes: diags }),
        None => Err(diags),
    }
}

/// File `file` of `kit` written for `target`, its deck named `name`.
///
/// # Panics
/// As [`render`]; when `file` is a Verilog-A source or unread.
pub fn render_kit(kit: &Kit, file: FileId, name: &str, target: Dialect) -> Result<Written, Diags> {
    let f = kit.file(file);
    assert!(
        matches!(f.role, Role::Deck | Role::Library),
        "render_kit: a file the kit read"
    );
    let lex = lexicon::of(kit.dialect()).expect("a kit's dialect has a reader");
    let t = tokenize(&f.text, lex);
    let ast = parse(&t, lex);
    let src = Source {
        ast: &ast,
        tokens: &t,
        dialect: kit.dialect(),
        name,
        kit: Some((kit, file)),
    };
    render(&src, target)
}

/// Deck `text` (line 1 its title), read alone in `d`, written for `target`.
///
/// # Panics
/// As [`render`]; when `d` has no reader (CDL, Verilog).
pub fn render_text(text: &str, d: Dialect, name: &str, target: Dialect) -> Result<Written, Diags> {
    let lex = lexicon::of(d).expect("a dialect with a reader");
    let t = tokenize(text, lex);
    let ast = parse(&t, lex);
    let src = Source {
        ast: &ast,
        tokens: &t,
        dialect: d,
        name,
        kit: None,
    };
    render(&src, target)
}

/// Whether [`render`] writes `target`: every target but Verilog, which
/// stays on the old layout writer (its element lines are the old IR's own
/// text).
pub fn writes(target: Dialect) -> bool {
    target != Dialect::Verilog
}
