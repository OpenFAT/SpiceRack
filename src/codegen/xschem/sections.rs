//! A [`Deck`]'s sections other than its blocks -> xschem text: title, globals,
//! parameters, setup, Verilog-A loads.

use std::collections::BTreeMap;

use super::deck::{Deck, SetupLine};
use super::spell::{comment, push_ident, word};
use crate::codegen::Dialect;
use crate::ir::*;

pub(super) struct Writer<'d, 'a> {
    pub(super) d: Dialect,
    pub(super) deck: &'d Deck<'a>,
    /// Verilog: each definition's ports, for named port connections.
    pub(super) ports: BTreeMap<&'a str, &'a [Port]>,
}

impl Writer<'_, '_> {
    pub(super) fn header(&self, out: &mut String) {
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
    pub(super) fn globals(&self, out: &mut String) {
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
    pub(super) fn params(&self, out: &mut String) {
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

    pub(super) fn setup(&self, out: &mut String) {
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

    /// One load per Verilog-A source (`<stem>.va`) of the top: ngspice the OSDI the
    /// caller compiled (`pre_osdi <stem>.osdi` in a `.control` block), ESPice and HSPICE
    /// the source (`.hdl`), Spectre `ahdl_include`; a comment where none loads it.
    pub(super) fn loads(&self, out: &mut String) {
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
