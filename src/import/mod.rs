//! Deck text in any dialect -> [`CircuitIR`], in three stages:
//!
//! 1. lines: the dialect grammar's `File` rule splits physical lines into
//!    the title and logical statements (continuations joined, comments gone);
//! 2. cards: its `Deck` rule parses the logical lines into the AST, rows
//!    tagged with [`Tag`] (every dialect captures into this one schema);
//! 3. lowering (`lower.rs`): AST -> IR, shared by every dialect.
//!
//! Syntax is the grammar's alone (`grammar/*.peg`): a card the grammar
//! refuses is [`Report::unparsed`]; one the IR has no field for is
//! [`Report::raw`]. Both stay verbatim lines, so they reach the deck's own
//! dialect unchanged. `tools/spice2rack.rs` is the CLI.

use std::sync::OnceLock;

use crate::codegen::Dialect;
use crate::ir::*;
use crate::peg::{Grammar, Tree};

mod lower;

macro_rules! tags {
    ($($t:ident),* $(,)?) => {
        /// The AST's node kinds: a grammar's captured rule `Foo` (or
        /// `Foo_anything`, a second spelling of it) is a `Tag::Foo` row.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(u8)]
        pub(crate) enum Tag { $($t),* }
        const TAGS: &[(&str, Tag)] = &[$((stringify!($t), Tag::$t)),*];
    };
}

tags! {
    // lines stage
    Title, Line, Piece, Glue, EndRun,
    // blocks and kept text
    Subckt, Port, Ends, Block, Control, CtlCmd, Raw, Unparsed, SpiceRegion, ControlBlock,
    // dot cards
    Include, Lib, Path, Section, Model, MPair, MVal, More, Flag, Params, Assign, FuncDef, Func, Arg, Body,
    Load, Embed, Ground, Options, Pkg, Temp, Ic, Nodeset, NodeVal, Global, Save, Out, Print, PrintKind, Meas, Probe, Step, StepKind,
    // analyses
    Op, Dc, Sweep, Ac, Tran, Uic, Noise, Tf, Sens, Pz, Four,
    // elements and their fields
    R, C, L, K, V, I, Sdc, Sac, Pulse, Sin, Pwl, Exp, Sffm, Am, Poly, E, G, F, H, B, D, Q, M, J, Z, S, W, T, X,
    Mname, Inst, Ana, Ctl,
    // values
    Pair, Val, Name, Node, Key, Word, Str, Expr, Num, Mag, Frac,
    Meg, Mil, Peta, Tera, Giga, Kilo, Unit, Centi, Milli, Micro, Nano, Pico, Femto, Atto,
}

impl Tag {
    fn of(rule: &str) -> Option<u8> {
        let base = rule.split('_').next().unwrap_or(rule);
        TAGS.iter().find(|(n, _)| *n == base).map(|(_, t)| *t as u8)
    }

    fn from_u8(t: u8) -> Tag {
        TAGS[t as usize].1
    }
}

/// One dialect's grammar: `spice.peg` under the dialect's file.
pub(crate) struct Lang {
    g: Grammar,
    file: u32,
    deck: u32,
    /// The cards stage reads lowercased text (SPICE is case-blind).
    fold: bool,
}

macro_rules! peg {
    ($f:literal) => {
        include_str!(concat!("../../grammar/", $f))
    };
}

fn lang(d: Dialect) -> Result<&'static Lang, String> {
    static NG: OnceLock<Lang> = OnceLock::new();
    static HS: OnceLock<Lang> = OnceLock::new();
    static LT: OnceLock<Lang> = OnceLock::new();
    static XY: OnceLock<Lang> = OnceLock::new();
    static SP: OnceLock<Lang> = OnceLock::new();
    static VA: OnceLock<Lang> = OnceLock::new();
    // Spectre and VACASK are whole files of their own; SPICE dialects layer
    // over spice.peg.
    let (cell, texts): (_, &[&str]) = match d {
        Dialect::Spectre => (&SP, &[peg!("spectre.peg")]),
        Dialect::Vacask => (&VA, &[peg!("vacask.peg")]),
        _ => (&XY, &[]),
    };
    if !texts.is_empty() {
        return Ok(cell.get_or_init(|| {
            let g = Grammar::compile(texts, Tag::of).unwrap_or_else(|e| panic!("{d:?} grammar: {e}"));
            Lang { file: g.rule("file").unwrap(), deck: g.rule("deck").unwrap(), fold: false, g }
        }));
    }
    let (cell, text) = match d {
        Dialect::Ngspice | Dialect::Espice => (&NG, peg!("ngspice.peg")),
        Dialect::Hspice => (&HS, peg!("hspice.peg")),
        Dialect::Ltspice => (&LT, peg!("ltspice.peg")),
        Dialect::Xyce => (&XY, peg!("xyce.peg")),
        other => return Err(format!("no {other:?} reader yet")),
    };
    Ok(cell.get_or_init(|| {
        let g = Grammar::compile(&[peg!("spice.peg"), text], Tag::of).unwrap_or_else(|e| panic!("{d:?} grammar: {e}"));
        Lang { file: g.rule("file").unwrap(), deck: g.rule("deck").unwrap(), fold: true, g }
    }))
}

/// What a deck lost on the way to IR.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    /// Cards the dialect grammar refused, kept as raw lines.
    pub unparsed: Vec<String>,
    /// Cards the grammar read that no IR field holds, kept as raw lines.
    pub raw: usize,
}

/// Deck bytes as text: UTF-16LE with a BOM (LTspice), UTF-8, else Latin-1
/// (LTspice's older netlists, SSR "Encoding"), so a `µ` survives.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = body.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// The logical lines, each ended by `\n`: a `Line`'s parts joined, a
/// `Piece` after a blank, a `Glue` (HSPICE's `\\`) with nothing.
fn logical(t: &Tree) -> String {
    let mut out = String::new();
    for row in t.children(None).filter(|&r| Tag::from_u8(t.tag(r)) == Tag::Line) {
        let mut line = String::new();
        for part in t.children(Some(row)) {
            let text = t.text(part).trim();
            if !line.is_empty() && !text.is_empty() && Tag::from_u8(t.tag(part)) == Tag::Piece {
                line.push(' ');
            }
            line.push_str(text);
        }
        if !line.is_empty() {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// Parses deck text in dialect `d` (line 1 is the title) into IR named `name`.
pub fn parse(src: &str, name: &str, d: Dialect) -> Result<(CircuitIR, Report), String> {
    let lang = lang(d)?;
    let lines = lang.g.parse(lang.file, src).map_err(|at| format!("line grammar stops at byte {at}"))?;
    let text = logical(&lines);
    let folded = if lang.fold { text.to_ascii_lowercase() } else { text.clone() };
    let deck = lang.g.parse(lang.deck, &folded).map_err(|at| format!("card grammar stops at byte {at}"))?;
    Ok(lower::deck(&deck, &text, name, d, lang))
}

#[cfg(test)]
mod tests;

/// The cards stage's AST, one row per line, indented by depth: what the
/// grammar made of `src` (debugging a grammar).
pub fn dump(src: &str, d: Dialect) -> Result<String, String> {
    let lang = lang(d)?;
    let lines = lang.g.parse(lang.file, src).map_err(|at| format!("line grammar stops at byte {at}"))?;
    let text = logical(&lines);
    let folded = if lang.fold { text.to_ascii_lowercase() } else { text.clone() };
    let t = lang.g.parse(lang.deck, &folded).map_err(|at| format!("card grammar stops at byte {at}"))?;
    let mut out = String::new();
    let mut open: Vec<u32> = Vec::new(); // ends of the rows enclosing this one
    for (i, r) in t.rows.iter().enumerate() {
        while open.last().is_some_and(|&e| e as usize <= i) {
            open.pop();
        }
        let s = &text[r.start as usize..r.end as usize];
        out.push_str(&format!("{}{:?} {:?}\n", "  ".repeat(open.len()), Tag::from_u8(r.tag), s));
        open.push(r.next);
    }
    Ok(out)
}
