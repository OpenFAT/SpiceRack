//! [`Ast`] (and its [`Kit`]) → deck text in any dialect: the transpiler's
//! last stage. Two
//! structural writers, because there are two statement shapes: the SPICE
//! family (`spice`: ngspice, ESPice, HSPICE, Xyce, LTspice, CDL, Verilog) and the named
//! family (`named`: Spectre, VACASK). Each reads the reader's plan
//! (`read`: which statement is written where, and its views), spells
//! through the target's [`spelling::Spelling`] row, asks
//! [`capability::action`] for everything a target may not say, and writes
//! every byte through one token choke point (`out`).
//!
//! A written deck ([`Written`]) is the old pipeline's text, byte for byte
//! (TESTING.md, "Golden gate"); the Python API's decks are written here too
//! (`python::deck`). The file's diagnostics come with it
//! ([`Diags`]): the parser's and the kit's notes (`P`, `R`) and the
//! writer's (`W`), sorted by source position, with their messages; a
//! refusal is a writer's `W` code of `diag::Kind::Refusal`.
//!
//! Cites: Zig 0.17 `lib/std/zig/Ast/Render.zig` (`zig fmt`: one renderer,
//! tokens its only output).
//!
//! Contents:
//! - `pub mod capability`, `pub mod spelling`
//! - `pub struct Source<'a>`, `pub enum Numbers`, `pub const NGBEHAVIOR: &str`, `pub struct Written`, `pub struct Diags`
//! - `pub fn render(src: &Source, target: Dialect) -> Result<Written, Diags>`
//! - `pub fn render_kit(kit: &Kit, file: FileId, name: &str, target: Dialect) -> Result<Written, Diags>`
//! - `pub fn render_text(text: &str, d: Dialect, name: &str, target: Dialect) -> Result<Written, Diags>`
//! - `pub enum Reference`, `pub fn references(src: &Source) -> Vec<Reference>`
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

use crate::syntax::Dialect;
use crate::syntax::ast::Ast;
use crate::syntax::diag::{self, Diag, Kind};
use crate::syntax::resolve::{FileId, Kit, Role};
use crate::syntax::token::Tokens;
use crate::syntax::{lexicon, parse, tokenize};

/// One file to write: its tree and tokens, the dialect it was read in, the
/// name its deck is written under (the title), the kit it was read in,
/// whose answers fill what the file alone leaves open, and the Verilog-A
/// sources and OSDI binaries its top loads beyond those it names itself.
pub struct Source<'a> {
    pub ast: &'a Ast<'a>,
    pub tokens: &'a Tokens<'a>,
    pub dialect: Dialect,
    pub name: &'a str,
    pub kit: Option<(&'a Kit, FileId)>,
    /// Loads no statement of the file holds: an ngspice deck has no load
    /// card (its `pre_osdi` is a `.control` command), so a deck built as
    /// ngspice text (the Python API's) names them here; each target writes
    /// them as it writes its own (`pre_osdi`, `.hdl`, `ahdl_include`,
    /// `load`). Empty for a file read as it is.
    pub loads: &'a [String],
    /// How an ngspice target spells a number.
    pub numbers: Numbers,
}

/// The comment line that carries a deck's ngspice compatibility mode to
/// the ngspice backend (`backend::ngspice`), the mode after it: ngspice
/// reads the mode only before parsing (no card sets it).
pub const NGBEHAVIOR: &str = "*spicerack ngbehavior=";

/// How an ngspice (or ESPice) deck spells the file's numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Numbers {
    /// Each so ngspice reads back the double the file's own reading gave
    /// (`unit::ngspice_spelling` for a file read as ngspice text, whose
    /// numbers carry ngspice's arithmetic), as the old writer did.
    #[default]
    Read,
    /// In SI form (`100u`), as given: the doubles were the caller's, not
    /// read, and the Python API has always written them so.
    Given,
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

/// A file another file names, as [`references`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    /// An `.include`, `.lib` call or `include … section=`: a netlist file.
    File(String),
    /// A Verilog-A source a load names (`ahdl_include`, `load`).
    VerilogA(String),
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
    let (mut rd, mut deck) = read::read(src.ast, src.tokens, src.dialect, src.name, src.kit);
    deck.given_loads = src.loads.to_vec();
    deck.numbers = src.numbers;
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
        loads: &[],
        numbers: Numbers::Read,
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
        loads: &[],
        numbers: Numbers::Read,
    };
    render(&src, target)
}

/// The files `src` names at its top and in each library section, in the
/// order a deck holds them (each block's includes, then its `.lib` calls,
/// then its Verilog-A sources): what a kit walk follows from this file, the
/// walk Interop's `pdk-models` and `golden` make. A subcircuit's or a
/// branch's include is not listed (`resolve::Kit::load` follows those
/// too). Paths are as written, quotes removed.
pub fn references(src: &Source) -> Vec<Reference> {
    let (rd, deck) = read::read(src.ast, src.tokens, src.dialect, src.name, src.kit);
    let blocks = std::iter::once(&deck.top).chain(deck.sections.iter().map(|s| &s.body));
    let unquoted = |p: String| p.trim_matches(['"', '\'']).to_string();
    let mut out = Vec::new();
    for b in blocks {
        out.extend(
            b.includes
                .iter()
                .map(|&n| Reference::File(unquoted(rd.include_path(n)))),
        );
        out.extend(
            b.libs
                .iter()
                .map(|&n| Reference::File(unquoted(rd.lib_view(n).0))),
        );
        let loads = b
            .loads
            .iter()
            .map(|&n| unquoted(rd.unquote(rd.fields(n)[0])));
        out.extend(
            loads
                .filter(|p| crate::veriloga::is_source(p))
                .map(Reference::VerilogA),
        );
    }
    out
}

/// Whether [`render`] writes `target`: every target, Verilog as structural
/// modules (`spice::verilog`).
pub fn writes(target: Dialect) -> bool {
    Dialect::ALL.contains(&target)
}
