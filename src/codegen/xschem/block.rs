//! One block of a [`Deck`] -> its xschem text: opening, elements sorted by
//! element name, raw lines, closing.

use super::deck::Lines;
use super::sections::Writer;
use super::spell::{comment, element_name, push_ident, push_instance};
use crate::codegen::{CodeGenError, Dialect, spectre};
use crate::ir::*;

/// Whether a block's elements came in element-name order so far, and the last name.
struct Seen {
    sorted: bool,
    /// The previous element's name, as offsets into the output.
    prev: (usize, usize),
}

/// A written element (`run == u32::MAX`), or a whole sorted run of [`Lines`] (`run`
/// its index in the block's runs), as offsets into the output.
struct Span {
    start: usize,
    end: usize,
    run: u32,
}

impl Writer<'_, '_> {
    /// Block `k` (0 the top): its opening (when `wrap`), its elements sorted by element
    /// name, its `raw_spice` lines verbatim, its closing.
    pub(super) fn block(&self, out: &mut String, k: usize, sc: &Subcircuit, wrap: bool) {
        let d = self.d;
        if wrap {
            self.open(out, k, sc);
        }
        let indent = if d == Dialect::Verilog { "  " } else { "" };
        let runs = self.deck.lines.get(k).map_or(&[][..], Vec::as_slice);
        let n =
            runs.iter().map(Lines::len).sum::<usize>() + sc.components.len() + sc.instances.len();
        let seg = out.len();
        let mut spans: Vec<Span> = Vec::with_capacity(n);
        let mut seen = Seen {
            sorted: true,
            prev: (0, 0),
        };
        // Records the element written into `out[start..]` (nothing written: dropped).
        let end = |out: &mut String, start: usize, spans: &mut Vec<Span>, seen: &mut Seen| {
            if out.len() == start + indent.len() {
                out.truncate(start); // nothing to write
                return;
            }
            if !out.ends_with('\n') {
                out.push('\n');
            }
            seen.note(out, start, out.len(), spans, d);
        };
        for (r, run) in runs.iter().enumerate() {
            // A run whose every line is non-empty and ends in a newline is copied whole;
            // its lines are then only scanned for their names, or, when the producer
            // vouches for their order, only its first and last.
            let whole = indent.is_empty() && run.whole();
            if whole && run.sorted && !run.is_empty() {
                debug_assert!(
                    (1..run.len())
                        .all(|i| element_name(run.get(i - 1), d) <= element_name(run.get(i), d)),
                    "a run marked sorted is in element-name order"
                );
                assert_eq!(run.ends[run.len() - 1] as usize, run.text.len());
                let base = out.len();
                out.push_str(&run.text);
                let last = if run.len() > 1 {
                    run.ends[run.len() - 2] as usize
                } else {
                    0
                };
                seen.name(out, base, base + run.ends[0] as usize, spans.is_empty(), d);
                seen.name(out, base + last, out.len(), false, d);
                spans.push(Span {
                    start: base,
                    end: out.len(),
                    run: r as u32,
                });
                continue;
            }
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
            sort_spans(out, seg, &spans, runs, d);
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
}

// ---- private ----

impl Writer<'_, '_> {
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
            Dialect::Spectre => {
                let mut s = String::new();
                spectre::emit_component(&mut s, c, spectre::Names::VERBATIM).map(|()| s)
            }
            Dialect::Verilog => Err(CodeGenError::UnsupportedComponent {
                backend: "verilog".into(),
                component: format!("{c:?}"),
            }),
            Dialect::Hspice | Dialect::Ltspice => crate::codegen::card(c, d),
            _ => crate::codegen::card(c, Dialect::Ngspice),
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

    /// A subcircuit instance ([`push_instance`]); Verilog pairs the definition's ports.
    fn instance(&self, out: &mut String, i: &Instance) {
        let ports = match self.d {
            Dialect::Verilog => self
                .ports
                .get(i.subcircuit.as_str())
                .copied()
                .unwrap_or(&[]),
            _ => &[],
        };
        push_instance(
            out,
            &i.name,
            &i.subcircuit,
            i.port_mapping.iter().map(String::as_str),
            ports.iter().map(|p| p.name.as_str()),
            i.parameters.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            self.d,
        );
    }
}

impl Seen {
    /// Records the element `out[start..end]` (its newline included) in `spans`.
    fn note(&mut self, out: &str, start: usize, end: usize, spans: &mut Vec<Span>, d: Dialect) {
        self.name(out, start, end, spans.is_empty(), d);
        spans.push(Span {
            start,
            end,
            run: u32::MAX,
        });
    }

    /// Takes the name of the element `out[start..end]` as the latest; `first` for the
    /// block's first element.
    fn name(&mut self, out: &str, start: usize, end: usize, first: bool, d: Dialect) {
        let name = element_name(&out[start..end], d);
        // `name` borrows `out`, so its address gives its offset.
        let a = name.as_ptr() as usize - out.as_ptr() as usize;
        let name = (a, a + name.len());
        self.sorted &= first || out[self.prev.0..self.prev.1] <= out[name.0..name.1];
        self.prev = name;
    }
}

/// Reorders the elements written since `seg` by element name (stable). `spans` are
/// contiguous from `seg` to the end; a run's span stands for each of its lines.
fn sort_spans(out: &mut String, seg: usize, spans: &[Span], runs: &[Lines], d: Dialect) {
    assert!(spans.first().is_some_and(|s| s.start == seg));
    assert!(spans.last().is_some_and(|s| s.end == out.len()));
    // ponytail: the out-of-order case copies the block's text once; fine while
    // templates that reorder names are rare.
    let text = out[seg..].to_owned();
    let mut lines: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for s in spans {
        let (a, b) = (s.start - seg, s.end - seg);
        match runs.get(s.run as usize) {
            None => lines.push((a, b)),
            Some(run) => {
                assert_eq!(b - a, run.text.len());
                let mut start = a;
                for &e in &run.ends {
                    lines.push((start, a + e as usize));
                    start = a + e as usize;
                }
            }
        }
    }
    let mut order: Vec<(&str, u32)> = (0..lines.len())
        .map(|i| (element_name(&text[lines[i].0..lines[i].1], d), i as u32))
        .collect();
    order.sort_unstable();
    out.truncate(seg);
    for i in order.into_iter().map(|o| o.1 as usize) {
        out.push_str(&text[lines[i].0..lines[i].1]);
    }
    assert_eq!(out.len(), seg + text.len());
}
