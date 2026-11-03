//! The types a caller fills to lay a deck out the xschem way: [`Deck`] and
//! its [`Lines`] and [`SetupLine`]s.
//!
//! Contents:
//! - `pub struct Lines`, `Lines::push(&mut self, line: &str)`, `Lines::end_line(&mut self)`,
//!   `Lines::len(&self) -> usize`, `Lines::is_empty(&self) -> bool`,
//!   `Lines::whole(&self) -> bool` (`pub(super)`), `Lines::get(&self, i: usize) -> &str`
//! - `pub enum SetupLine`
//! - `pub struct Deck<'a>`, `Deck::new(ir: &'a CircuitIR) -> Deck<'a>`

use crate::ir::*;

/// Element lines packed in one buffer: line `i` is `text[ends[i - 1]..ends[i]]`. A
/// line may end in a newline (a multi-line element) and is written with one if it
/// does not. One allocation for any number of lines, against one `String` (and one
/// 200-byte [`Component`]) per `RawSpice` line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    pub text: String,
    /// End offset of each line in `text`, ascending.
    pub ends: Vec<u32>,
    /// The producer vouches that the lines are in element-name order (it read every
    /// name already, e.g. for a name map), so [`write`](super::write) compares only the run's first
    /// and last names with its neighbours instead of scanning every line. A wrong
    /// `true` writes the run unsorted (checked by a debug assert); `false` is always
    /// correct.
    pub sorted: bool,
}

/// One setup line of a [`Deck`]. Paths and values are written as given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupLine {
    /// A section of a model library (`.lib <path> <section>`).
    Lib { path: String, section: String },
    /// A file included whole (`.include <path>`).
    Include { path: String },
    /// The circuit temperature, as text (`.temp <value>`).
    Temp { value: String },
}

/// A [`CircuitIR`] and what an xschem deck carries beside it.
#[derive(Clone, Debug)]
pub struct Deck<'a> {
    pub ir: &'a CircuitIR,
    /// The first, comment line; `None` writes the top's name.
    pub title: Option<String>,
    /// Nets that are one net at every level (`.global`), in the order written. `0`
    /// is ground and is written only where a dialect lists it (Spectre).
    pub globals: Vec<String>,
    /// Model and corner setup in the order written (a corner's `.lib`, `.include`
    /// and `.temp` interleave, so they are one list).
    pub setup: Vec<SetupLine>,
    /// Text written once after the definitions and models: model cards, library
    /// subcircuits, Verilog modules. Each block is verbatim in the dialect's syntax
    /// (SPICE for Spectre, which switches language around it) and ends in a newline.
    pub bodies: Vec<String>,
    /// Per block (the top, then each of `ir.subcircuit_defs`): raw element lines, in
    /// one or more runs (a caller that renders a block in parallel hands each part's
    /// run over as it is, uncopied). Shorter than the block list means no lines for
    /// the rest.
    pub lines: Vec<Vec<Lines>>,
    /// Per block, as `lines`: its internal nets, for the dialects that declare them
    /// (Verilog `wire`s), in the order written. Ports are not listed.
    pub nets: Vec<Vec<String>>,
}

impl Lines {
    /// Appends `line`.
    pub fn push(&mut self, line: &str) {
        self.text.push_str(line);
        self.end_line();
    }

    /// Ends the line written into `text` since the last end.
    pub fn end_line(&mut self) {
        let end = u32::try_from(self.text.len()).expect("lines below 4 GiB");
        assert!(self.ends.last().is_none_or(|&e| e <= end));
        self.ends.push(end);
    }

    /// The number of lines (ended ones; text after the last end is not one).
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    /// Whether no line has been ended.
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Whether every line is non-empty and ends in a newline, so the run is written as
    /// one copy of `text`.
    pub(super) fn whole(&self) -> bool {
        let b = self.text.as_bytes();
        let mut start = 0;
        self.ends.iter().all(|&e| {
            let e = e as usize;
            let ok = e > start + 1 && b.get(e - 1) == Some(&b'\n');
            start = e;
            ok
        })
    }

    /// Line `i`.
    pub fn get(&self, i: usize) -> &str {
        let a = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        &self.text[a..self.ends[i] as usize]
    }
}

impl<'a> Deck<'a> {
    /// `ir` with nothing beside it.
    pub fn new(ir: &'a CircuitIR) -> Deck<'a> {
        Deck {
            ir,
            title: None,
            globals: Vec::new(),
            setup: Vec::new(),
            bodies: Vec::new(),
            lines: Vec::new(),
            nets: Vec::new(),
        }
    }
}
