//! Deck: a Python circuit's blocks → its ngspice source text (the deck's
//! own text: the parser reads it, `syntax::render` writes it for the
//! target) → the netlist one backend runs, or the `RawData` of running it.
//! Verilog blocks are lowered here first, so the text holds what simulates
//! them.
//!
//! Each statement is spelled in a dialect that has it: ngspice's card, or,
//! for what only Spectre says (the analyses with no ngspice card,
//! `Analysis::spectre`), its Spectre statement in a `simulator
//! lang=spectre` region of the source (`lexicon::NGSPICE_GIVEN`). A line
//! given as one dialect's text (`extra_line`) sits in that dialect's
//! region. The renderer writes a region's text for its own dialect family
//! and refuses it (`W0001`) for any other; nothing is added to the text it
//! writes. ngspice's `ngbehavior` is an `.options ngbehavior=` card.
//! Beside the text, in `render::Source`: the top's Verilog-A and OSDI
//! loads (ngspice loads them from a `.control` command, which the reader
//! keeps no load of).
//!
//! Cites: ngspice manual ch.2 (deck order: title first, `.end` last).
//!
//! Contents:
//! - `pub(super) struct Deck`
//! - `Deck::netlist(&self, backend: &str) -> PyResult<String>`,
//!   `Deck::run(&self, backend: Option<&str>) -> PyResult<RawData>`,
//!   `Deck::run_each(&self, backend: Option<&str>) -> PyResult<Vec<Option<RawData>>>`,
//!   `Deck::to_source(&self) -> PyResult<String>`, `Deck::issues(&self, backend: &str) -> Vec<String>`,
//!   `Deck::card(body: &Body, name: &str) -> PyResult<String>`
//! - `pub(super) fn check_backend_name(name: &str) -> PyResult<()>`
//! - `pub(super) fn region_dialect(name: &str) -> Result<Dialect, String>`
//! - `pub(super) fn runtime_err(e: impl std::fmt::Display) -> PyErr`

use std::fmt::Write as _;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use super::analysis::Analysis;
use super::model::{Bench, Body, Func, ModelLibrary, dec, params};
use super::verilog::lower_verilog;
use crate::backend::{Dialect, Kind, feature};
use crate::result::RawData;
use crate::syntax::render;

/// A whole deck as Python built it: the top (a `Circuit`, or a
/// `Testbench`'s DUT), its subcircuit definitions, and the testbench.
#[derive(Clone, Debug, Default)]
pub(super) struct Deck {
    pub(super) top: Body,
    pub(super) defs: Vec<Body>,
    pub(super) bench: Option<Bench>,
    pub(super) libraries: Vec<ModelLibrary>,
    pub(super) functions: Vec<Func>,
    /// `.global` nodes.
    pub(super) globals: Vec<String>,
}

impl Deck {
    /// The deck `backend` runs: every runnable dialect, and HSPICE, which
    /// is written but never run here.
    pub(super) fn netlist(&self, backend: &str) -> PyResult<String> {
        let d = Dialect::from_backend_name(&backend.to_ascii_lowercase())
            .filter(|d| *d == Dialect::Hspice || crate::backend::RUNNABLE.contains(d))
            .ok_or_else(|| PyValueError::new_err(format!("unknown backend '{backend}'")))?;
        self.lowered()?.write(d).map_err(PyRuntimeError::new_err)
    }

    /// Runs the deck on `backend` (`None`: the best installed one for its
    /// first analysis and its features); the first plot.
    pub(super) fn run(&self, backend: Option<&str>) -> PyResult<RawData> {
        let (sim, text) = self.for_run(backend)?;
        crate::simulation::run(sim, &text).map_err(runtime_err)
    }

    /// Runs the deck once; one plot per queued analysis, in queue order.
    pub(super) fn run_each(&self, backend: Option<&str>) -> PyResult<Vec<Option<RawData>>> {
        let (sim, text) = self.for_run(backend)?;
        let kinds: Vec<Kind> = self.analyses().iter().map(Analysis::kind).collect();
        crate::simulation::run_each(sim, &text, &kinds).map_err(runtime_err)
    }

    /// The deck's stored source (`store::with_loads`): its ngspice text,
    /// a model library at its ngspice path.
    pub(super) fn to_source(&self) -> PyResult<String> {
        let deck = self.lowered()?;
        let text = deck
            .source(Dialect::Ngspice)
            .map_err(PyRuntimeError::new_err)?;
        Ok(super::store::with_loads(&text, &deck.top.osdi))
    }

    /// `backend::check` as `"Severity: message"` lines.
    pub(super) fn issues(&self, backend: &str) -> Vec<String> {
        let elements = self.bodies().map(|b| b.elements.len() + b.instances.len());
        crate::backend::check(self.needs(), elements.sum(), backend)
            .into_iter()
            .map(|issue| format!("{:?}: {}", issue.severity, issue.message))
            .collect()
    }

    /// The ngspice card of `body`'s first element or instance called
    /// `name` (without its letter), as a deck holding it alone writes it.
    pub(super) fn card(body: &Body, name: &str) -> PyResult<String> {
        let c = body.elements.iter().chain(&body.instances);
        let Some(c) = c.into_iter().find(|c| c.name == name) else {
            return Err(pyo3::exceptions::PyKeyError::new_err(format!(
                "Element '{name}' not found"
            )));
        };
        let one = Deck {
            top: Body {
                elements: vec![c.clone()],
                ..Default::default()
            },
            ..Default::default()
        };
        let deck = one
            .write(Dialect::Ngspice)
            .map_err(PyRuntimeError::new_err)?;
        // Drop the title line and `.end`.
        let lines: Vec<&str> = deck.lines().collect();
        Ok(lines[1..lines.len() - 1].join("\n"))
    }
}

/// Refuse a backend name at selection time rather than at the first run.
pub(super) fn check_backend_name(name: &str) -> PyResult<()> {
    if crate::backend::parse(name).is_some() {
        return Ok(());
    }
    let names = crate::backend::RUNNABLE
        .map(Dialect::name)
        .into_iter()
        .chain(Dialect::ALIASES.map(|a| a.0));
    Err(PyValueError::new_err(format!(
        "unknown backend {name:?}; expected one of {}",
        names.collect::<Vec<_>>().join(", ")
    )))
}

/// The dialect family a line given as `name`'s text is kept in: one the
/// deck's source has a `simulator lang=` region for.
///
/// # Errors
/// A name no dialect has; a dialect with no region (CDL, Verilog).
pub(super) fn region_dialect(name: &str) -> Result<Dialect, String> {
    let d = name.parse::<Dialect>()?.family();
    let regions = crate::syntax::lexicon::NGSPICE_GIVEN.regions;
    match regions.iter().any(|(n, _)| *n == d.name()) {
        true => Ok(d),
        false => {
            let names: Vec<&str> = regions.iter().map(|r| r.0).collect();
            Err(format!(
                "no {} text in a deck; one of {}",
                d.name(),
                names.join(", ")
            ))
        }
    }
}

/// Any error as a Python `RuntimeError` carrying its `Display` text.
pub(super) fn runtime_err(e: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

// ---- private ----

impl Deck {
    /// The simulator a run picks and the deck written for it.
    fn for_run(&self, backend: Option<&str>) -> PyResult<(Dialect, String)> {
        let deck = self.lowered()?;
        let first = deck.analyses().first().map_or(Kind::Op, Analysis::kind);
        let sim = crate::backend::select(first, backend, deck.needs()).map_err(runtime_err)?;
        let text = deck.write(sim).map_err(runtime_err)?;
        Ok((sim, text))
    }

    /// A copy with every Verilog block expanded into what simulates it.
    fn lowered(&self) -> PyResult<Deck> {
        let mut deck = self.clone();
        for b in std::iter::once(&mut deck.top).chain(&mut deck.defs) {
            lower_verilog(b).map_err(runtime_err)?;
        }
        Ok(deck)
    }

    fn analyses(&self) -> &[Analysis] {
        self.bench.as_ref().map_or(&[], |b| &b.analyses)
    }

    fn bodies(&self) -> impl Iterator<Item = &Body> {
        std::iter::once(&self.top).chain(&self.defs)
    }

    /// What the deck needs from a simulator (`feature` bits).
    fn needs(&self) -> u8 {
        let mut needs = 0;
        for b in self.bodies() {
            if !b.osdi.is_empty() {
                needs |= feature::OSDI;
            }
            if b.verilog
                .iter()
                .any(|v| matches!(v.mode, crate::veriloga::VerilogMode::Simulate))
            {
                needs |= feature::VERILOG_COSIM;
            }
            let control = |l: &String| {
                let t = l.trim().to_lowercase();
                t == ".control" || t.starts_with(".control ")
            };
            if b.raw.iter().any(control) {
                needs |= feature::CONTROL_BLOCKS;
            }
            needs |= b.elements.iter().fold(0, |n, c| n | c.needs);
        }
        if let Some(tb) = &self.bench {
            if !tb.measures.is_empty() {
                needs |= feature::MEASURES;
            }
            if !tb.steps.is_empty() {
                needs |= feature::STEP_PARAMS;
            }
        }
        needs
    }

    /// The deck written for `d`: its ngspice source rendered there.
    fn write(&self, d: Dialect) -> Result<String, String> {
        let name = self.top.name.as_str();
        let source = self.source(d)?;
        let lex = &crate::syntax::lexicon::NGSPICE_GIVEN;
        let tokens = crate::syntax::tokenize(&source, lex);
        let ast = crate::syntax::parse(&tokens, lex);
        let src = render::Source {
            ast: &ast,
            tokens: &tokens,
            dialect: Dialect::Ngspice,
            name,
            kit: None,
            loads: &self.top.osdi,
            numbers: render::Numbers::Given,
        };
        match render::render(&src, d) {
            Ok(w) => Ok(w.text),
            Err(diags) => Err(diags.messages.trim_end().to_string()),
        }
    }

    /// The deck as ngspice text, for target `d`: a model library's path
    /// is the one for `d`'s family, the only part that depends on `d`.
    /// `Err` names an analysis no dialect here has a statement for.
    fn source(&self, d: Dialect) -> Result<String, String> {
        let mut out = String::with_capacity(4096);
        let line = |out: &mut String, l: &str| {
            out.push_str(l);
            out.push('\n');
        };
        line(&mut out, &self.top.name);
        if !self.globals.is_empty() {
            line(&mut out, &format!(".global {}", self.globals.join(" ")));
        }
        for f in &self.functions {
            let args = f.args.join(", ");
            line(
                &mut out,
                &format!(".func {}({args}) {{{}}}", f.name, f.body),
            );
        }
        file_cards(&mut out, &self.top);
        for lib in &self.libraries {
            for s in &lib.setup_includes {
                line(&mut out, &format!(".include {s}"));
            }
            let path = lib
                .backend_paths
                .get(d.family().name())
                .unwrap_or(&lib.path);
            match &lib.corner {
                Some(corner) => line(&mut out, &format!(".lib {path} {corner}")),
                None => line(&mut out, &format!(".include {path}")),
            }
        }
        for (k, v) in &self.top.params {
            line(&mut out, &format!(".param {k}={v}"));
        }
        for m in &self.top.models {
            line(&mut out, m);
        }
        for sc in &self.defs {
            out.push('\n');
            let ps = match sc.params.is_empty() {
                true => String::new(),
                false => format!(" PARAMS:{}", params(&sc.params)),
            };
            line(
                &mut out,
                &format!(".subckt {} {}{ps}", sc.name, sc.ports.join(" ")),
            );
            file_cards(&mut out, sc);
            for m in &sc.models {
                line(&mut out, m);
            }
            body(&mut out, sc);
            line(&mut out, &format!(".ends {}", sc.name));
        }
        body(&mut out, &self.top);
        if let Some(tb) = &self.bench {
            bench(&mut out, tb)?;
        }
        line(&mut out, ".end");
        Ok(out)
    }
}

/// A block's `.include` and `.lib` cards.
fn file_cards(out: &mut String, b: &Body) {
    for inc in &b.includes {
        let _ = writeln!(out, ".include {inc}");
    }
    for (p, section) in &b.libs {
        let _ = writeln!(out, ".lib {p} {section}");
    }
}

/// A block's elements, then its instances, then its raw lines.
fn body(out: &mut String, b: &Body) {
    for c in b.elements.iter().chain(&b.instances) {
        let _ = writeln!(out, "{}", c.text);
    }
    for l in &b.raw {
        let _ = writeln!(out, "{l}");
    }
}

/// The testbench's cards: stimulus, options, initial conditions, outputs,
/// measures, steps, the analyses, and the regions of other dialects' text.
fn bench(out: &mut String, tb: &Bench) -> Result<(), String> {
    for c in &tb.stimulus {
        let _ = writeln!(out, "{}", c.text);
    }
    if !tb.options.is_empty() {
        let opts: Vec<String> = tb
            .options
            .iter()
            .map(|(k, v)| match v.is_empty() {
                true => k.clone(),
                false => format!("{k}={v}"),
            })
            .collect();
        let _ = writeln!(out, ".options {}", opts.join(" "));
    }
    if let Some(mode) = &tb.ngbehavior {
        let _ = writeln!(out, ".options ngbehavior={mode}");
    }
    if let Some(t) = tb.temperature {
        let _ = writeln!(out, ".temp {}", dec(t));
    }
    if let Some(t) = tb.nominal_temperature {
        let _ = writeln!(out, ".options tnom={}", dec(t));
    }
    for (n, v) in &tb.ics {
        let _ = writeln!(out, ".ic V({n})={}", dec(*v));
    }
    for (n, v) in &tb.nodesets {
        let _ = writeln!(out, ".nodeset V({n})={}", dec(*v));
    }
    if !tb.saves.is_empty() {
        let _ = writeln!(out, ".save {}", tb.saves.join(" "));
    }
    for (kind, outs) in &tb.prints {
        let sep = if kind.is_empty() { "" } else { " " };
        let _ = writeln!(out, ".print{sep}{kind} {}", outs.join(" "));
    }
    for m in &tb.measures {
        let card = m.get(..5).is_some_and(|p| p.eq_ignore_ascii_case(".meas"));
        let _ = writeln!(out, "{}{m}", if card { "" } else { ".meas " });
    }
    for s in &tb.steps {
        let sweep = s
            .sweep
            .map(|w| format!("{} ", w.name()))
            .unwrap_or_default();
        let (a, b, c) = (dec(s.start), dec(s.stop), dec(s.step));
        let _ = writeln!(out, ".step {sweep}{} {a} {b} {c}", s.param);
    }
    let mut spectre = Vec::new();
    for a in &tb.analyses {
        match a.card() {
            Some(card) => {
                let _ = writeln!(out, "{card}");
            }
            None => spectre.push(a.spectre().map_err(|why| unsupported(&why))?),
        }
    }
    region(out, "spectre", &spectre);
    for (l, d) in &tb.extra_lines {
        region(out, d.name(), std::slice::from_ref(l));
    }
    Ok(())
}

/// `lines` inside a `simulator lang=<lang>` … `simulator lang=spice`
/// region; nothing for none.
fn region(out: &mut String, lang: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let _ = writeln!(out, "simulator lang={lang}");
    for l in lines {
        let _ = writeln!(out, "{l}");
    }
    let _ = writeln!(out, "simulator lang=spice");
}

/// The refusal of an analysis no dialect here has a statement for.
fn unsupported(what: &str) -> String {
    format!("Unsupported analysis: {what}")
}
