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
//! A refusal is a list of `W` diagnostics ([`Diag`]); a written deck is the
//! old pipeline's text, byte for byte (TESTING.md, "Golden gate"): the
//! Python API's `codegen::netlist(&CircuitIR, d)` stays until phase 6.
//!
//! Cites: `transpiler-architecture.md` §2.5, §2.6, §5 phase 4;
//! `compiler-architecture-reference.md` §4 (Zig `Ast/Render.zig`), §12.4,
//! §12.5.
//!
//! Contents:
//! - `pub mod capability`, `pub mod spelling`
//! - `pub struct Source<'a>`
//! - `pub fn render(src: &Source, target: Dialect) -> Result<String, Vec<Diag>>`
//! - `pub fn render_kit(kit: &Kit, file: FileId, name: &str, target: Dialect) -> Result<String, Vec<Diag>>`
//! - `pub fn render_text(text: &str, d: Dialect, name: &str, target: Dialect) -> Result<String, Vec<Diag>>`
//! - `pub fn writes(target: Dialect) -> bool`

pub mod capability;
mod named;
mod out;
mod read;
mod role;
pub mod spelling;
mod spice;

use crate::codegen::Dialect;
use crate::syntax::ast::Ast;
use crate::syntax::diag::Diag;
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

/// `src` written for `target`; the `W` refusals when the target cannot say
/// something the deck holds.
///
/// # Panics
/// When `target` is not one [`writes`] names; when `src.kit`'s file is not
/// `src.ast`'s.
pub fn render(src: &Source, target: Dialect) -> Result<String, Vec<Diag>> {
    assert!(writes(target), "render: a target with a writer");
    let (mut rd, deck) = read::read(src.ast, src.tokens, src.dialect, src.name, src.kit);
    match target {
        Dialect::Spectre | Dialect::Vacask => named::write(&mut rd, &deck, target),
        _ => spice::write(&mut rd, &deck, target),
    }
}

/// File `file` of `kit` written for `target`, its deck named `name`.
///
/// # Panics
/// As [`render`]; when `file` is a Verilog-A source or unread.
pub fn render_kit(
    kit: &Kit,
    file: FileId,
    name: &str,
    target: Dialect,
) -> Result<String, Vec<Diag>> {
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
pub fn render_text(
    text: &str,
    d: Dialect,
    name: &str,
    target: Dialect,
) -> Result<String, Vec<Diag>> {
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
