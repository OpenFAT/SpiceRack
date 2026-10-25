//! Decks laid out the way xschem writes them.
//!
//! [`write`] lays a [`Deck`] (a [`CircuitIR`] plus what an xschem deck carries beside
//! it) out in the order and spelling of xschem's netlister, so a schematic tool keeps
//! byte-for-byte parity with xschem's golden netlists while SpiceRack writes the text:
//!
//! ```text
//! title           * <title>   (// for Spectre and Verilog; Spectre adds `simulator lang=spectre`)
//! globals         .global a b            (Spectre `global 0 a b`; CDL `.GLOBAL`)
//! parameters      .param k=v             (Spectre one `parameters` line; not CDL or Verilog)
//! setup           .lib / .include / .temp in deck order (not CDL or Verilog)
//! top block       the top's elements; wrapped as a .SUBCKT for CDL, a module for Verilog
//! bench text      the testbench's extra_lines, verbatim
//! definitions     subcircuit_defs in IR order, each its elements then its raw_spice
//! Verilog-A       one load per source, then the top's models, then the bodies
//! end             .end (CDL .END; none for Spectre and Verilog)
//! ```
//!
//! A block's elements are its packed [`Lines`], its components and its instances,
//! sorted bytewise by element name (the first token, the second for Verilog's
//! `<module> <instance> (…);`). The sort is stable and lines come first, then
//! components, then instances. A multi-line element counts as one unit.
//!
//! Dialects: ngspice, ESPice, Xyce, HSPICE, LTspice, CDL, Spectre, Verilog (VACASK
//! reads its own language and has no xschem layout). A component a dialect cannot
//! spell becomes a comment naming it. Only the testbench's `extra_lines` are
//! written; an xschem deck carries its analyses as text.

use std::collections::BTreeMap;

use super::{CodeGenError, Dialect, spectre, spice3};
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

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Line `i`.
    pub fn get(&self, i: usize) -> &str {
        let a = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        &self.text[a..self.ends[i] as usize]
    }
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

/// The dialect word (`ngspice`, …).
pub fn word(d: Dialect) -> &'static str {
    d.name()
}

/// SPICE syntax: verbatim names, `*` comments, `.subckt` blocks.
pub fn is_spice(d: Dialect) -> bool {
    !matches!(d, Dialect::Spectre | Dialect::Vacask | Dialect::Verilog)
}

/// The comment leader.
pub fn comment(d: Dialect) -> &'static str {
    if is_spice(d) { "*" } else { "//" }
}

/// Appends `name` as `d` spells an identifier: SPICE dialects verbatim; Spectre
/// escapes every character outside `[A-Za-z0-9_]` with `\`; Verilog writes a name
/// outside `[A-Za-z_][A-Za-z0-9_$]*` as an escaped identifier (`\d[3] `).
#[inline]
pub fn push_ident(out: &mut String, name: &str, d: Dialect) {
    match d {
        Dialect::Spectre | Dialect::Vacask => {
            for c in name.chars() {
                if !(c.is_ascii_alphanumeric() || c == '_') {
                    out.push('\\');
                }
                out.push(c);
            }
        }
        Dialect::Verilog => {
            let b = name.as_bytes();
            let simple = !b.is_empty()
                && (b[0].is_ascii_alphabetic() || b[0] == b'_')
                && b.iter()
                    .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'$');
            if simple {
                out.push_str(name);
            } else {
                out.push('\\');
                out.push_str(name);
                out.push(' ');
            }
        }
        _ => out.push_str(name),
    }
}

/// Appends a net: `?` (an unconnected pin) as is, any other name as an identifier.
#[inline]
fn push_net(out: &mut String, net: &str, d: Dialect) {
    if net == "?" {
        out.push('?')
    } else {
        push_ident(out, net, d)
    }
}

/// The element name of a written element line, its sort key: the first token, the
/// second for Verilog (`<module> <instance> (…);`); empty for a blank line.
pub fn element_name(line: &str, d: Dialect) -> &str {
    let skip = usize::from(d == Dialect::Verilog);
    line.split_ascii_whitespace().nth(skip).unwrap_or("")
}

/// Appends the element name instance `name` is written with: SPICE dialects add the
/// subcircuit letter `X` unless the name starts with it (ASCII case-insensitively),
/// so `X1` stays `X1` and `M1` is `XM1`; Spectre and Verilog escape the name. A
/// caller's name map keys on this, so it cannot drift from the deck.
pub fn push_element_name(out: &mut String, name: &str, d: Dialect) {
    if !is_spice(d) {
        push_ident(out, name, d);
        return;
    }
    if !name
        .as_bytes()
        .first()
        .is_some_and(|b| b.eq_ignore_ascii_case(&b'X'))
    {
        out.push('X');
    }
    out.push_str(name);
}

/// The deck, `\n` line ends, ending in a newline.
///
/// # Errors
///
/// VACASK, which has no xschem layout.
pub fn write(deck: &Deck<'_>, d: Dialect) -> Result<String, CodeGenError> {
    if d == Dialect::Vacask {
        return Err(CodeGenError::Other("VACASK has no xschem layout".into()));
    }
    let ir = deck.ir;
    let blocks = 1 + ir.subcircuit_defs.len();
    assert!(
        deck.lines.len() <= blocks && deck.nets.len() <= blocks,
        "one entry per block"
    );
    let text: usize = deck
        .lines
        .iter()
        .flatten()
        .map(|l| l.text.len() + l.len())
        .sum();
    let raw: usize = std::iter::once(&ir.top)
        .chain(&ir.subcircuit_defs)
        .map(|s| 48 * (s.components.len() + s.instances.len()))
        .sum();
    let bodies: usize = deck.bodies.iter().map(String::len).sum();
    let mut out = String::with_capacity(4096 + text + raw + bodies);
    let ports: BTreeMap<&str, &[Port]> = match d {
        Dialect::Verilog => ir
            .subcircuit_defs
            .iter()
            .map(|s| (s.name.as_str(), s.ports.as_slice()))
            .collect(),
        _ => BTreeMap::new(),
    };
    let w = Writer { d, deck, ports };
    w.header(&mut out);
    w.globals(&mut out);
    w.params(&mut out);
    w.setup(&mut out);
    w.block(
        &mut out,
        0,
        &ir.top,
        matches!(d, Dialect::Cdl | Dialect::Verilog),
    );
    for l in ir.testbench.iter().flat_map(|tb| &tb.extra_lines) {
        out.push_str(l);
        out.push('\n');
    }
    for (k, sc) in ir.subcircuit_defs.iter().enumerate() {
        w.block(&mut out, k + 1, sc, true);
    }
    w.loads(&mut out);
    for m in &ir.top.models {
        out.push_str(if is_spice(d) { ".model " } else { "model " });
        out.push_str(&m.name);
        out.push(' ');
        out.push_str(&m.kind);
        for (k, v) in &m.parameters {
            out.push(' ');
            out.push_str(k);
            out.push('=');
            out.push_str(v);
        }
        out.push('\n');
    }
    for b in &deck.bodies {
        debug_assert!(
            b.is_empty() || b.ends_with('\n'),
            "a body ends in a newline"
        );
        if d == Dialect::Spectre {
            out.push_str("simulator lang=spice\n");
        }
        out.push_str(b);
        if d == Dialect::Spectre {
            out.push_str("simulator lang=spectre\n");
        }
    }
    out.push_str(match d {
        Dialect::Cdl => ".END\n",
        Dialect::Spectre | Dialect::Verilog => "",
        _ => ".end\n",
    });
    debug_assert!(out.ends_with('\n'));
    Ok(out)
}

struct Writer<'d, 'a> {
    d: Dialect,
    deck: &'d Deck<'a>,
    /// Verilog: each definition's ports, for named port connections.
    ports: BTreeMap<&'a str, &'a [Port]>,
}

impl Writer<'_, '_> {
    fn header(&self, out: &mut String) {
        assert!(out.is_empty(), "the title is the first line");
        out.push_str(comment(self.d));
        out.push(' ');
        out.push_str(self.deck.title.as_deref().unwrap_or(&self.deck.ir.top.name));
        out.push('\n');
        match self.d {
            Dialect::Spectre => out.push_str("simulator lang=spectre\n"),
            // LTspice compresses waveforms (lossy) and stores them in single precision
            // unless told otherwise; a deck written for reading back keeps every point.
            Dialect::Ltspice => out.push_str(".options plotwinsize=0 numdgt=15\n"),
            _ => {}
        }
    }

    /// One `.global` line of every global but `0` (Spectre lists `0` too); Verilog has
    /// no global nets.
    fn globals(&self, out: &mut String) {
        let d = self.d;
        let word = match d {
            Dialect::Cdl => ".GLOBAL",
            Dialect::Spectre => "global",
            Dialect::Verilog => return,
            _ => ".global",
        };
        let mut names = self
            .deck
            .globals
            .iter()
            .filter(|g| d == Dialect::Spectre || *g != "0")
            .peekable();
        if names.peek().is_none() {
            return;
        }
        out.push_str(word);
        for g in names {
            out.push(' ');
            push_ident(out, g, d);
        }
        out.push('\n');
    }

    /// The top's parameters: `.param k=v` lines, one Spectre `parameters` line; CDL
    /// writes them on the top's `.SUBCKT` line instead, Verilog not at all.
    fn params(&self, out: &mut String) {
        let d = self.d;
        if matches!(d, Dialect::Cdl | Dialect::Verilog) {
            return;
        }
        let mut first = true;
        for p in &self.deck.ir.top.parameters {
            let Some(v) = &p.default else { continue };
            match d {
                Dialect::Spectre if first => out.push_str("parameters "),
                Dialect::Spectre => out.push(' '),
                _ => out.push_str(".param "),
            }
            first = false;
            out.push_str(&p.name);
            out.push('=');
            out.push_str(v);
            if d != Dialect::Spectre {
                out.push('\n');
            }
        }
        if d == Dialect::Spectre && !first {
            out.push('\n');
        }
    }

    fn setup(&self, out: &mut String) {
        let d = self.d;
        if matches!(d, Dialect::Cdl | Dialect::Verilog) {
            return;
        }
        // HSPICE quotes paths: `.lib 'file' section`.
        let q = if d == Dialect::Hspice { "'" } else { "" };
        for l in &self.deck.setup {
            match (l, d) {
                (SetupLine::Lib { path, section }, Dialect::Spectre) => {
                    out.push_str("include \"");
                    out.push_str(path);
                    out.push_str("\" section=");
                    out.push_str(section);
                }
                (SetupLine::Include { path }, Dialect::Spectre) => {
                    out.push_str("include \"");
                    out.push_str(path);
                    out.push('"');
                }
                (SetupLine::Temp { value }, Dialect::Spectre) => {
                    out.push_str("corner_temp options temp=");
                    out.push_str(value);
                }
                (SetupLine::Lib { path, section }, _) => {
                    out.push_str(".lib ");
                    out.push_str(q);
                    out.push_str(path);
                    out.push_str(q);
                    out.push(' ');
                    out.push_str(section);
                }
                (SetupLine::Include { path }, _) => {
                    out.push_str(".include ");
                    out.push_str(q);
                    out.push_str(path);
                    out.push_str(q);
                }
                (SetupLine::Temp { value }, Dialect::Xyce) => {
                    out.push_str(".options device temp=");
                    out.push_str(value);
                }
                (SetupLine::Temp { value }, _) => {
                    out.push_str(".temp ");
                    out.push_str(value);
                }
            }
            out.push('\n');
        }
    }

    /// Block `k` (0 the top): its opening (when `wrap`), its elements sorted by element
    /// name, its `raw_spice` lines verbatim, its closing.
    fn block(&self, out: &mut String, k: usize, sc: &Subcircuit, wrap: bool) {
        let d = self.d;
        if wrap {
            self.open(out, k, sc);
        }
        let indent = if d == Dialect::Verilog { "  " } else { "" };
        let runs = self.deck.lines.get(k).map_or(&[][..], Vec::as_slice);
        let n =
            runs.iter().map(Lines::len).sum::<usize>() + sc.components.len() + sc.instances.len();
        let seg = out.len();
        let mut spans: Vec<(usize, usize)> = Vec::with_capacity(n);
        let mut seen = Seen {
            sorted: true,
            prev: (0, 0),
        };
        // Records the element written into `out[start..]` (nothing written: dropped).
        let end =
            |out: &mut String, start: usize, spans: &mut Vec<(usize, usize)>, seen: &mut Seen| {
                if out.len() == start + indent.len() {
                    out.truncate(start); // nothing to write
                    return;
                }
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                seen.note(out, start, out.len(), spans, d);
            };
        for run in runs {
            // A run whose every line is non-empty and ends in a newline is copied whole;
            // its lines are then only scanned for their names.
            let whole = indent.is_empty()
                && (0..run.len()).all(|i| run.get(i).len() > 1 && run.get(i).ends_with('\n'));
            if whole {
                let base = out.len();
                out.push_str(&run.text);
                let mut a = base;
                for &e in &run.ends {
                    let e = base + e as usize;
                    seen.note(out, a, e, &mut spans, d);
                    a = e;
                }
                assert_eq!(a, out.len());
                continue;
            }
            for i in 0..run.len() {
                let start = out.len();
                out.push_str(indent);
                out.push_str(run.get(i));
                end(out, start, &mut spans, &mut seen);
            }
        }
        for c in &sc.components {
            let start = out.len();
            out.push_str(indent);
            self.component(out, c);
            end(out, start, &mut spans, &mut seen);
        }
        for i in &sc.instances {
            let start = out.len();
            out.push_str(indent);
            self.instance(out, i);
            end(out, start, &mut spans, &mut seen);
        }
        if !seen.sorted {
            sort_spans(out, seg, &spans, d);
        }
        for l in &sc.raw_spice {
            out.push_str(l);
            out.push('\n');
        }
        if wrap {
            out.push_str(match d {
                Dialect::Cdl => ".ENDS ",
                Dialect::Spectre => "ends ",
                Dialect::Verilog => "endmodule",
                _ => ".ends ",
            });
            if d != Dialect::Verilog {
                push_ident(out, &sc.name, d);
            }
            out.push('\n');
        }
    }

    fn open(&self, out: &mut String, k: usize, sc: &Subcircuit) {
        let d = self.d;
        let (open, sep, close) = match d {
            Dialect::Cdl => (".SUBCKT ", " ", ""),
            Dialect::Spectre => ("subckt ", " ", ")"),
            Dialect::Verilog => ("module ", ", ", ");"),
            _ => (".subckt ", " ", ""),
        };
        out.push_str(open);
        push_ident(out, &sc.name, d);
        if !close.is_empty() {
            out.push_str(" (");
        }
        for (i, p) in sc.ports.iter().enumerate() {
            if i > 0 || close.is_empty() {
                out.push_str(sep);
            }
            push_ident(out, &p.name, d);
        }
        // CDL has no `.param`: a definition's parameters are its `.SUBCKT` line's.
        if d == Dialect::Cdl {
            for p in &sc.parameters {
                out.push(' ');
                out.push_str(&p.name);
                if let Some(v) = &p.default {
                    out.push('=');
                    out.push_str(v);
                }
            }
        }
        out.push_str(close);
        out.push('\n');
        let dir = |p: &Port| match p.direction {
            PortDirection::Input => 0,
            PortDirection::Output => 1,
            PortDirection::InOut => 2,
        };
        match d {
            Dialect::Cdl if !sc.ports.is_empty() => {
                out.push_str("*.PININFO");
                for p in &sc.ports {
                    out.push(' ');
                    out.push_str(&p.name);
                    out.push_str([":I", ":O", ":B"][dir(p)]);
                }
                out.push('\n');
            }
            Dialect::Verilog => {
                for p in &sc.ports {
                    out.push_str(["  input ", "  output ", "  inout "][dir(p)]);
                    push_ident(out, &p.name, d);
                    out.push_str(";\n");
                }
                for n in self.deck.nets.get(k).into_iter().flatten() {
                    out.push_str("  wire ");
                    push_ident(out, n, d);
                    out.push_str(";\n");
                }
            }
            _ => {}
        }
    }

    /// A component's line: `RawSpice` verbatim, any other through SpiceRack's codegen
    /// of the dialect's family (a comment where it has no spelling).
    fn component(&self, out: &mut String, c: &Component) {
        let d = self.d;
        if let Component::RawSpice { line } = c {
            out.push_str(line);
            return;
        }
        let line = match d {
            Dialect::Spectre => spectre::emit_component(c),
            Dialect::Verilog => Err(CodeGenError::UnsupportedComponent {
                backend: "verilog".into(),
                component: format!("{c:?}"),
            }),
            Dialect::Hspice | Dialect::Ltspice => spice3::component(d, false, c),
            _ => spice3::component(Dialect::Ngspice, false, c),
        };
        match line {
            Ok(l) => out.push_str(&l),
            Err(e) => {
                out.push_str(comment(d));
                out.push_str(" error: ");
                out.push_str(&e.to_string().replace('\n', " "));
            }
        }
    }

    /// A subcircuit instance: `X<name> <nets> <subckt>` (SPICE), `<name> (<nets>)
    /// <subckt>` (Spectre), `<module> <name> (.<port>(<net>), …);` (Verilog, ports by
    /// the definition's port names); parameters as ` k=v`.
    fn instance(&self, out: &mut String, i: &Instance) {
        let d = self.d;
        match d {
            Dialect::Spectre => {
                push_element_name(out, &i.name, d);
                out.push_str(" (");
                for (k, n) in i.port_mapping.iter().enumerate() {
                    if k > 0 {
                        out.push(' ');
                    }
                    push_net(out, n, d);
                }
                out.push_str(") ");
                push_ident(out, &i.subcircuit, d);
            }
            Dialect::Verilog => {
                push_ident(out, &i.subcircuit, d);
                out.push(' ');
                push_element_name(out, &i.name, d);
                out.push_str(" (");
                let names = self
                    .ports
                    .get(i.subcircuit.as_str())
                    .copied()
                    .unwrap_or(&[]);
                for (k, (p, n)) in names.iter().zip(&i.port_mapping).enumerate() {
                    if k > 0 {
                        out.push_str(", ");
                    }
                    out.push('.');
                    push_ident(out, &p.name, d);
                    out.push('(');
                    push_net(out, n, d);
                    out.push(')');
                }
                out.push_str(");");
                return;
            }
            _ => {
                push_element_name(out, &i.name, d);
                for n in &i.port_mapping {
                    out.push(' ');
                    push_net(out, n, d);
                }
                out.push(' ');
                push_ident(out, &i.subcircuit, d);
            }
        }
        for (k, v) in &i.parameters {
            out.push(' ');
            out.push_str(k);
            out.push('=');
            out.push_str(v);
        }
    }

    /// One load per Verilog-A source (`<stem>.va`) of the top: ngspice the OSDI the
    /// caller compiled (`pre_osdi <stem>.osdi` in a `.control` block), ESPice and HSPICE
    /// the source (`.hdl`), Spectre `ahdl_include`; a comment where none loads it.
    fn loads(&self, out: &mut String) {
        let d = self.d;
        for l in &self.deck.ir.top.osdi_loads {
            let stem = l.rsplit_once('.').map_or(l.as_str(), |(s, _)| s);
            match d {
                Dialect::Ngspice => {
                    out.push_str(".control\npre_osdi ");
                    out.push_str(stem);
                    out.push_str(".osdi\n.endc\n");
                }
                Dialect::Espice | Dialect::Hspice => {
                    out.push_str(".hdl \"");
                    out.push_str(l);
                    out.push_str("\"\n");
                }
                Dialect::Spectre => {
                    out.push_str("ahdl_include \"");
                    out.push_str(l);
                    out.push_str("\"\n");
                }
                _ => {
                    out.push_str(comment(d));
                    out.push_str(" error: ");
                    out.push_str(word(d));
                    out.push_str(" loads no Verilog-A (");
                    out.push_str(l);
                    out.push_str(")\n");
                }
            }
        }
    }
}

/// Whether a block's elements came in element-name order so far, and the last name.
struct Seen {
    sorted: bool,
    /// The previous element's name, as offsets into the output.
    prev: (usize, usize),
}

impl Seen {
    /// Records the element `out[start..end]` (its newline included) in `spans`.
    fn note(
        &mut self,
        out: &str,
        start: usize,
        end: usize,
        spans: &mut Vec<(usize, usize)>,
        d: Dialect,
    ) {
        let name = element_name(&out[start..end], d);
        // `name` borrows `out`, so its address gives its offset.
        let a = name.as_ptr() as usize - out.as_ptr() as usize;
        let name = (a, a + name.len());
        self.sorted &= spans.is_empty() || out[self.prev.0..self.prev.1] <= out[name.0..name.1];
        self.prev = name;
        spans.push((start, end));
    }
}

/// Reorders the elements written since `seg` by element name (stable). `spans` are
/// `(start, end)` offsets into `out`, contiguous from `seg` to the end.
fn sort_spans(out: &mut String, seg: usize, spans: &[(usize, usize)], d: Dialect) {
    assert!(spans.first().is_some_and(|l| l.0 == seg));
    assert!(spans.last().is_some_and(|l| l.1 == out.len()));
    // ponytail: the out-of-order case copies the block's text once; fine while
    // templates that reorder names are rare.
    let text = out[seg..].to_owned();
    let mut order: Vec<(&str, u32)> = (0..spans.len())
        .map(|i| {
            (
                element_name(&text[spans[i].0 - seg..spans[i].1 - seg], d),
                i as u32,
            )
        })
        .collect();
    order.sort_unstable();
    out.truncate(seg);
    for i in order.into_iter().map(|o| o.1 as usize) {
        out.push_str(&text[spans[i].0 - seg..spans[i].1 - seg]);
    }
    assert_eq!(out.len(), seg + text.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(line: &str) -> Component {
        Component::RawSpice { line: line.into() }
    }

    fn port(name: &str, direction: PortDirection) -> Port {
        Port {
            name: name.into(),
            direction,
        }
    }

    fn ir() -> CircuitIR {
        CircuitIR {
            top: Subcircuit {
                name: "tb".into(),
                parameters: vec![ParamDef {
                    name: "vdd".into(),
                    default: Some("1.8".into()),
                }],
                components: vec![raw("V1 vdd! 0 1.8")],
                instances: vec![Instance {
                    name: "1".into(),
                    subcircuit: "inv".into(),
                    port_mapping: vec!["in".into(), "out[3]".into()], ..Default::default() }],
                osdi_loads: vec!["dev.va".into()],
                models: vec![ModelDef {
                    name: "m1".into(),
                    kind: "dev".into(), ..Default::default() }],
                ..Subcircuit::default() },
            testbench: Some(Testbench {
                extra_lines: vec![".op".into()],
                ..Testbench::default() }),
            subcircuit_defs: vec![Subcircuit {
                name: "inv".into(),
                ports: vec![
                    port("a", PortDirection::Input),
                    port("y", PortDirection::Output),
                ],
                components: vec![raw("M2 y a 0 0 n"), raw("M1 y a vdd! vdd! p")],
                raw_spice: vec!["* own code".into()],
                ..Subcircuit::default()
            }],
            ..CircuitIR::default()
        } }

    fn deck(ir: &CircuitIR) -> Deck<'_> {
        let mut top = Lines::default();
        top.push("C1 out 0 1e-12");
        Deck {
            title: Some("tb (test)".into()),
            globals: vec!["0".into(), "vdd!".into()],
            setup: vec![
                SetupLine::Lib {
                    path: "m.lib".into(),
                    section: "tt".into(),
                },
                SetupLine::Temp { value: "27".into() },
                SetupLine::Include {
                    path: "x.inc".into(),
                },
            ],
            bodies: vec![".model d1 d\n".into()],
            lines: vec![vec![top]],
            nets: vec![vec![], vec!["mid".into()]],
            ..Deck::new(ir)
        }
    }

    fn emit(d: Dialect) -> String {
        write(&deck(&ir()), d).unwrap()
    }

    #[test]
    fn ngspice_layout() {
        assert_eq!(
            emit(Dialect::Ngspice),
            "* tb (test)\n.global vdd!\n.param vdd=1.8\n.lib m.lib tt\n.temp 27\n.include x.inc\n\
             C1 out 0 1e-12\nV1 vdd! 0 1.8\nX1 in out[3] inv\n.op\n\
             .subckt inv a y\nM1 y a vdd! vdd! p\nM2 y a 0 0 n\n* own code\n.ends inv\n\
             .control\npre_osdi dev.osdi\n.endc\n.model m1 dev\n.model d1 d\n.end\n"
        );
    }

    #[test]
    fn dialect_spellings() {
        let x = emit(Dialect::Xyce);
        assert!(x.contains(".options device temp=27\n"), "{x}");
        let e = emit(Dialect::Espice);
        assert!(
            e.contains(".hdl \"dev.va\"\n") && !e.contains("pre_osdi"),
            "{e}"
        );
        let h = emit(Dialect::Hspice);
        assert!(
            h.contains(".lib 'm.lib' tt\n.temp 27\n.include 'x.inc'\n")
                && h.contains(".hdl \"dev.va\"\n"),
            "{h}"
        );
        let l = emit(Dialect::Ltspice);
        assert!(l.starts_with("* tb (test)\n.options plotwinsize=0 numdgt=15\n"), "{l}");
        assert!(
            l.contains("* error: ltspice loads no Verilog-A (dev.va)\n"),
            "{l}"
        );
        let c = emit(Dialect::Cdl);
        assert!(
            c.starts_with("* tb (test)\n.GLOBAL vdd!\n.SUBCKT tb vdd=1.8\n"),
            "{c}"
        );
        assert!(
            c.contains(".SUBCKT inv a y\n*.PININFO a:I y:O\n") && c.ends_with(".END\n"),
            "{c}"
        );
        assert!(!c.contains(".lib") && !c.contains(".param"), "{c}");
        let s = emit(Dialect::Spectre);
        assert!(
            s.starts_with(
                "// tb (test)\nsimulator lang=spectre\nglobal 0 vdd\\!\nparameters vdd=1.8\n"
            ),
            "{s}"
        );
        assert!(
            s.contains("include \"m.lib\" section=tt\ncorner_temp options temp=27\n"),
            "{s}"
        );
        assert!(
            s.contains("1 (in out\\[3\\]) inv\n") && s.contains("subckt inv (a y)\n"),
            "{s}"
        );
        assert!(
            s.contains("simulator lang=spice\n.model d1 d\nsimulator lang=spectre\n"),
            "{s}"
        );
        let v = emit(Dialect::Verilog);
        assert!(
            v.contains("module tb ();\n  inv \\1  (.a(in), .y(\\out[3] ));\n  C1 out"),
            "{v}"
        );
        assert!(
            v.contains("module inv (a, y);\n  input a;\n  output y;\n  wire mid;\n"),
            "{v}"
        );
        assert!(write(&deck(&ir()), Dialect::Vacask).is_err());
        // The plain codegen entry writes the dialects only this layout has.
        assert!(
            super::super::netlist(&ir(), Dialect::Cdl)
                .unwrap()
                .starts_with("* tb\n")
        );
    }

    #[test]
    fn element_names() {
        let mut s = String::new();
        for n in ["X1", "x2", "M1"] {
            push_element_name(&mut s, n, Dialect::Ngspice);
            s.push(' ');
        }
        assert_eq!(s, "X1 x2 XM1 ");
        assert_eq!(element_name("XM1 a b", Dialect::Cdl), "XM1");
        assert_eq!(element_name("  inv u1 (.a(x));", Dialect::Verilog), "u1");
        assert_eq!(element_name("", Dialect::Ngspice), "");
        for (name, d, want) in [
            ("a[0]", Dialect::Cdl, "a[0]"),
            ("a[0]", Dialect::Spectre, r"a\[0\]"),
            ("_x$1", Dialect::Verilog, "_x$1"),
            ("1a", Dialect::Verilog, r"\1a "),
            ("", Dialect::Verilog, r"\ "),
        ] {
            let mut out = String::from(">");
            push_ident(&mut out, name, d);
            assert_eq!(out, format!(">{want}"), "{name} {d:?}");
        }
    }

    #[test]
    fn lines_pack() {
        let mut a = Lines::default();
        a.push("R1 a b 1");
        a.push("R2 c d 2\n+ tc=1\n");
        a.push("");
        assert_eq!(
            (a.len(), a.get(0), a.get(1), a.get(2)),
            (3, "R1 a b 1", "R2 c d 2\n+ tc=1\n", "")
        );
        // Runs of one block are its lines in order.
        let ir = ir();
        let mut d = deck(&ir);
        let mut second = Lines::default();
        second.push("B1 x 0 v=1");
        d.lines[0].push(second);
        let n = write(&d, Dialect::Ngspice).unwrap();
        assert!(n.contains("B1 x 0 v=1\nC1 out 0 1e-12\nV1"), "{n}");
    }

    /// Determinism: one IR gives one deck, a clone included (every map on the emit
    /// path is ordered), and elements out of name order are sorted stably.
    #[test]
    fn two_emits_are_byte_identical_and_sorted() {
        for d in [
            Dialect::Ngspice,
            Dialect::Espice,
            Dialect::Xyce,
            Dialect::Hspice,
            Dialect::Ltspice,
            Dialect::Cdl,
            Dialect::Spectre,
            Dialect::Verilog,
        ] {
            let a = emit(d);
            let ir2 = ir().clone();
            assert_eq!(a, write(&deck(&ir2), d).unwrap(), "{d:?}");
        }
        let mut ir = ir();
        ir.top.components = vec![raw("R2 a 0 1"), raw("R1 a 0 1"), raw("R2 b 0 2")];
        ir.top.instances.clear();
        let n = write(&Deck::new(&ir), Dialect::Ngspice).unwrap();
        assert!(n.contains("R1 a 0 1\nR2 a 0 1\nR2 b 0 2\n"), "{n}");
    }
}
