//! Spectre and VACASK statements → the IR, as the old lowering's
//! `card` (the `named` branches), `named.rs`, `statistics.rs`,
//! `sections.rs` and `conditionals.rs` lower them: the deck's declarations,
//! the dispatch over statement kinds, models and model groups, `statistics`
//! blocks, libraries and sections, `real` functions, `if` chains, `simulator
//! lang=spice` regions and VACASK `control` blocks.
//!
//! Cites: the old lowering (its named, statistics, sections and
//! conditionals files, ported); the old Spectre grammar (`stmt`, `named`, `Ana`,
//! `Ctl`, `Block`, `SpiceRegion`), the old VACASK grammar (`stmt`,
//! `ControlBlock`, `Ana`); ESPice `spectre.zig:433-451` (the master map);
//! Spectre Reference 19.1 pp.175-179.
//!
//! Contents:
//! - `fn spectre_kind(master: &str, ty: Option<&str>) -> (String, Option<&'static str>)`
//! - `fn vacask_kind(module: &str, ty: Option<f64>) -> Option<(String, Option<&'static str>)>`
//! - `fn vacask_builtin(module: &str) -> Option<&'static str>`
//! - `Cx::declarations(&self, stmts: &[u32]) -> (HashMap<String, String>, Vec<String>)`
//! - `Cx::named_card_spine(&mut self, n: u32, here: &mut Subcircuit, scope: Scope)`
//! - `Cx::pairs(&self, n: u32) -> Vec<(String, u32)>`
//! - `Cx::get(&self, p: &[(String, u32)], k: &str) -> Option<u32>`
//! - `Cx::analysis_kind(&self, n: u32) -> bool`

use std::collections::HashMap;

use super::{Cx, Scope};
use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{Header, NONE, Node, Tag};
use crate::syntax::statement::Statement;
use crate::syntax::token::Tag as T;

/// The module names `codegen::vacask` declares itself (the old lowering).
const VACASK_BUILTINS: &[&str] = &[
    "resistor",
    "capacitor",
    "inductor",
    "vsource",
    "isource",
    "vcvs",
    "vccs",
    "cccs",
    "ccvs",
    "mutual",
];

/// A Spectre model master (and `type=`) → SPICE `.model` kind and LEVEL
/// (the old lowering's `spectre_kind`).
pub(super) fn spectre_kind(master: &str, ty: Option<&str>) -> (String, Option<&'static str>) {
    let ty = ty.map(str::to_ascii_lowercase);
    let mos = |l: &'static str| {
        let k = if ty.as_deref() == Some("p") {
            "pmos"
        } else {
            "nmos"
        };
        (k.to_string(), Some(l))
    };
    match master {
        "mos1" => mos("1"),
        "mos2" => mos("2"),
        "mos3" => mos("3"),
        "bsim3v3" => mos("49"),
        "bsim4" => mos("54"),
        "bjt" => {
            let k = if ty.as_deref() == Some("pnp") {
                "pnp"
            } else {
                "npn"
            };
            (k.to_string(), None)
        }
        "diode" => ("d".to_string(), None),
        other => (other.to_string(), None),
    }
}

/// `sp_<module>` (and `type=`) → SPICE kind and LEVEL (the old lowering
/// `vacask_kind`).
pub(super) fn vacask_kind(module: &str, ty: Option<f64>) -> Option<(String, Option<&'static str>)> {
    let neg = ty.is_some_and(|t| t < 0.0);
    let mos = |l| Some((if neg { "pmos" } else { "nmos" }.to_string(), Some(l)));
    match module.strip_prefix("sp_")? {
        "mos1" => mos("1"),
        "mos2" => mos("2"),
        "mos3" => mos("3"),
        "mos6" => mos("6"),
        "mos9" => mos("9"),
        "bsim3v3" => mos("49"),
        "bsim4v8" => mos("54"),
        "diode" => Some(("d".into(), None)),
        "bjt" => Some((if neg { "pnp" } else { "npn" }.into(), None)),
        "jfet1" => Some((if neg { "pjf" } else { "njf" }.into(), None)),
        "mes1" => Some((if neg { "pmf" } else { "nmf" }.into(), None)),
        "vdmos" => Some(("vdmos".into(), None)),
        _ => None,
    }
}

/// The built-in a VACASK `model NAME <module>` names (the old lowering
/// `vacask_builtin`).
pub(super) fn vacask_builtin(module: &str) -> Option<&'static str> {
    let m = module.strip_prefix("sp_").unwrap_or(module);
    VACASK_BUILTINS.iter().find(|a| **a == m).copied()
}

impl<'a> Cx<'a> {
    /// The deck's model kinds (`name -> kind`, `=<builtin>` for a VACASK
    /// alias) and subcircuit names, wherever they sit (the old lowering
    /// `declarations`).
    pub(super) fn declarations(&self, stmts: &[u32]) -> (HashMap<String, String>, Vec<String>) {
        let (mut kinds, mut subckts) = (HashMap::new(), Vec::new());
        let mut work: Vec<u32> = stmts.iter().rev().copied().collect();
        while let Some(s) = work.pop() {
            match self.tag(s) {
                Tag::Model => {
                    let f = self.fields(s);
                    let (Some(&name), Some(&master)) = (f.first(), f.get(1)) else {
                        continue;
                    };
                    let (name, master) = (self.name_of(name), self.name_of(master));
                    let mut p = self.pairs(s);
                    // A model group's polarity is its bins'.
                    if let Some(&bin) = self.body(s).first() {
                        p.extend(self.pairs(bin));
                    }
                    let ty = self.get(&p, "type");
                    if self.d == Dialect::Vacask {
                        match vacask_kind(&master, ty.and_then(|t| self.number(t))) {
                            Some((kind, _)) => {
                                kinds.insert(name, kind);
                            }
                            None => {
                                if let Some(b) = vacask_builtin(&master) {
                                    kinds.insert(name, format!("={b}"));
                                }
                            }
                        }
                    } else {
                        let ty = ty.map(|t| self.text(t));
                        kinds.insert(name, spectre_kind(&master, ty.as_deref()).0);
                    }
                }
                Tag::Subckt => {
                    if let Some(&name) = self.fields(s).first() {
                        subckts.push(self.name_of(name));
                    }
                    work.extend(self.body(s).iter().rev());
                }
                Tag::Section | Tag::Library | Tag::If | Tag::Else => {
                    if let Some(nx) = self.ast.next(Node(s)) {
                        work.push(nx.0);
                    }
                    work.extend(self.body(s).iter().rev());
                }
                _ => {}
            }
        }
        (kinds, subckts)
    }

    /// One Spectre or VACASK statement (the old lowering's `card`, named).
    pub(super) fn named_card_spine(&mut self, n: u32, here: &mut Subcircuit, scope: Scope) {
        let tag = self.tag(n);
        if tag.has_header() && self.ast.rhs(Node(n)) != NONE {
            // quirk: the old Spectre grammar's `Block`: a `name master … {`
            // statement is kept whole, its body unread.
            return self.raw(n, here, true);
        }
        if scope == Scope::Section && self.named_leaks(n) {
            return self.raw(n, here, matches!(tag, Tag::Region | Tag::Control));
        }
        match tag {
            Tag::Title | Tag::End => {}
            Tag::Verbatim | Tag::Line => self.unparsed(n, here),
            Tag::Card if self.ttag(self.main(n)) == T::KwSimulator => {}
            Tag::Instance if !self.inst_peg_ok(n) => self.unparsed(n, here),
            Tag::Analysis | Tag::Command | Tag::Options
                if self.d == Dialect::Spectre && !self.inst_peg_ok(n) =>
            {
                self.unparsed(n, here)
            }
            // quirk: the old VACASK grammar's `stmt`: `save`, `analysis`,
            // `sweep`, `options` are statements only inside `control`.
            Tag::Save | Tag::Card | Tag::Analysis | Tag::Options
                if self.d == Dialect::Vacask && !self.vacask_top_ok(n) =>
            {
                self.unparsed(n, here)
            }
            Tag::Card
            | Tag::Model
            | Tag::Save
            | Tag::Region
            | Tag::Control
            | Tag::Instance
            | Tag::Analysis
            | Tag::Command
            | Tag::Options => {
                let ok = self.named_card(n, here, scope == Scope::Top);
                if !ok {
                    let block =
                        matches!(tag, Tag::Card | Tag::Model) && self.ast.rhs(Node(n)) != NONE;
                    self.raw(n, here, block || matches!(tag, Tag::Region | Tag::Control));
                }
            }
            Tag::If => {
                let c = Conditional {
                    at: Some(here.components.len() as u32),
                    ..self.named_conditional(n, scope)
                };
                here.conditionals.push(c);
            }
            Tag::Statistics => {
                let st = self.statistics(n);
                here.statistics.push(st);
            }
            Tag::Library | Tag::Section if scope != Scope::Top => self.raw(n, here, true),
            Tag::Library => {
                let lib = self.name_of(self.fields(n)[0]);
                for &k in self.body(n).to_vec().iter() {
                    match self.tag(k) {
                        Tag::Section => {
                            let s = self.section(k, Some(lib.clone()));
                            self.sections.push(s);
                        }
                        _ => self.card(k, here, Scope::Top),
                    }
                }
            }
            Tag::Section => {
                let s = self.section(n, None);
                self.sections.push(s);
            }
            Tag::Subckt => {
                let head = self
                    .subckt_head(n)
                    .expect("a named subckt head is its fields");
                let sc = self.subckt(n, head);
                self.defs.push(sc);
            }
            Tag::Include => {
                let f = self.fields(n);
                let path = self.unquote(f[0]);
                match self.get(&self.pairs(n), "section") {
                    Some(sec) => here.libs.push((path, self.text(sec))),
                    None => here.includes.push(path),
                }
            }
            Tag::Global => {
                for &k in self.fields(n) {
                    if self.d == Dialect::Spectre && self.text(k) == "0" {
                        continue;
                    }
                    let g = self.name_of(k);
                    if !self.globals.contains(&g) {
                        self.globals.push(g);
                    }
                }
            }
            Tag::Param => {
                for &k in self.fields(n) {
                    if self.tag(k) == Tag::Assign {
                        here.parameters.push(ParamDef {
                            name: self.key(k),
                            default: Some(self.vtext(self.ast.rhs(Node(k)))),
                        });
                    }
                }
            }
            Tag::Func if scope == Scope::Subckt => self.raw(n, here, true),
            Tag::Func => {
                let f = self.real_func(n);
                self.functions.push(f);
            }
            _ if scope != Scope::Top => self.raw(n, here, false),
            _ => {
                if !self.named_testbench(n) {
                    self.raw(n, here, false);
                }
            }
        }
    }

    /// `k=v` items of a statement, by key, values as written.
    pub(super) fn pairs(&self, n: u32) -> Vec<(String, u32)> {
        self.fields(n)
            .iter()
            .filter(|&&k| self.tag(k) == Tag::Assign)
            .map(|&k| (self.key(k), self.ast.rhs(Node(k))))
            .collect()
    }

    pub(super) fn get(&self, p: &[(String, u32)], k: &str) -> Option<u32> {
        p.iter().find(|(n, _)| n == k).map(|(_, v)| *v)
    }

    /// quirk: the old lowering read the old Spectre grammar's `Word_kind`
    /// for VACASK too: a VACASK instance whose master is a Spectre analysis
    /// word is an analysis (Spectre's are `Analysis` nodes already).
    pub(super) fn analysis_kind(&self, n: u32) -> bool {
        self.tag(n) == Tag::Instance
            && self.master(n).is_some_and(|m| {
                matches!(
                    crate::syntax::lexicon::SPECTRE.master(&m),
                    Some(Statement::Analysis(_))
                )
            })
    }
}

// ---- private ----

impl<'a> Cx<'a> {
    /// A header statement's master word's text.
    fn master(&self, n: u32) -> Option<String> {
        if !self.tag(n).has_header() {
            return None;
        }
        let m = self.ast.header(Node(n)).master;
        (m != NONE).then(|| self.text(m))
    }

    /// quirk: the old Spectre grammar's `Inst`/`Ana`/`Ctl` and the old
    /// VACASK grammar's `Inst`: a name, its nodes (in parentheses for
    /// VACASK), a master (VACASK: optional), then only `key=value`
    /// parameters; else the line is `Unparsed`.
    fn inst_peg_ok(&self, n: u32) -> bool {
        let Header { name, master, .. } = self.ast.header(Node(n));
        let parens = self.last(name) < self.line_last(self.main(n))
            && self.ttag(self.last(name) + 1) == T::LParen;
        if self.d == Dialect::Vacask && !parens {
            return false;
        }
        if self.d == Dialect::Spectre && master == NONE {
            return false;
        }
        self.fields(n).iter().all(|&k| {
            let v = self.ast.rhs(Node(k));
            self.tag(k) == Tag::Assign && v != NONE && self.tag(v) != Tag::Bad
        })
    }

    /// Whether a VACASK keyword statement is one the old VACASK grammar's `stmt` reads
    /// outside `control`.
    fn vacask_top_ok(&self, n: u32) -> bool {
        matches!(
            self.ttag(self.main(n)),
            T::KwLoad | T::KwEmbed | T::KwGround
        )
    }

    /// the old lowering's `leaks`, named: control, output and analysis
    /// statements, and `global`.
    fn named_leaks(&self, n: u32) -> bool {
        match self.tag(n) {
            Tag::Global | Tag::Save | Tag::Region | Tag::Control => true,
            Tag::Analysis | Tag::Command | Tag::Options => self.d == Dialect::Spectre,
            Tag::Instance => self.analysis_kind(n),
            _ => false,
        }
    }

    /// A path string without its quotes.
    fn unquote(&self, p: u32) -> String {
        let t = self.text(p);
        let t = t.trim();
        match t.as_bytes() {
            [b'"', .., b'"'] => t[1..t.len() - 1].to_string(),
            _ => t.to_string(),
        }
    }

    /// the old lowering's `named_card`; `false` when the IR holds no such
    /// statement.
    fn named_card(&mut self, n: u32, here: &mut Subcircuit, top: bool) -> bool {
        let head = self.ttag(self.main(n));
        match self.tag(n) {
            Tag::Card if matches!(head, T::KwLoad | T::KwAhdlInclude) => {
                let path = self.unquote(self.fields(n)[0]);
                if !(self.d == Dialect::Vacask && path.starts_with("spice/")) {
                    here.osdi_loads.push(path);
                }
                true
            }
            Tag::Card if head == T::KwGround => {
                // `ground 0` is the default; `ground (a b)` is no IR field.
                matches!(self.fields(n), [z] if self.text(*z) == "0")
            }
            Tag::Card => false,
            Tag::Model => self.named_model(n, here),
            Tag::Save => {
                let outs: Vec<String> = self
                    .fields(n)
                    .iter()
                    .map(|&o| {
                        let s = self.name_of(o);
                        match s.split_once(':') {
                            Some((dev, "p")) => format!("i({dev})"),
                            _ => s,
                        }
                    })
                    .collect();
                self.tb.saves.extend(outs);
                true
            }
            Tag::Region => {
                for &k in self.body(n) {
                    let text = self.line(k);
                    if text.starts_with(".meas") {
                        self.tb.measures.push(text);
                    } else {
                        self.report.raw += 1;
                        here.raw_spice.push(text);
                    }
                }
                true
            }
            Tag::Control => {
                self.vacask_control_block(n);
                true
            }
            Tag::Options => {
                self.spectre_options(n);
                true
            }
            Tag::Analysis | Tag::Command | Tag::Instance
                if self.tag(n) != Tag::Instance || self.analysis_kind(n) =>
            {
                top && match self.spectre_analysis(n) {
                    Some(a) => {
                        self.tb.analyses.push(a);
                        true
                    }
                    None => false,
                }
            }
            Tag::Instance => {
                let (kinds, subckts) = (
                    std::mem::take(&mut self.kinds),
                    std::mem::take(&mut self.subckts),
                );
                let got = self.inst(n, &kinds, &subckts);
                (self.kinds, self.subckts) = (kinds, subckts);
                match got {
                    Some(Ok(c)) => here.components.push(c),
                    Some(Err(i)) => here.instances.push(Instance {
                        at: Some(here.components.len() as u32),
                        ..i
                    }),
                    None => return false,
                }
                true
            }
            _ => false,
        }
    }

    /// A Spectre `ic`/`nodeset` at the top (the old lowering).
    fn named_testbench(&mut self, n: u32) -> bool {
        if !matches!(self.tag(n), Tag::Ic | Tag::Nodeset) {
            return false;
        }
        let v: Option<Vec<(String, f64)>> = self
            .fields(n)
            .iter()
            .map(|&k| {
                (self.tag(k) == Tag::Assign).then_some(())?;
                Some((self.key(k), self.number(self.ast.rhs(Node(k)))?))
            })
            .collect();
        match (v, self.tag(n)) {
            (Some(v), Tag::Ic) => self.tb.initial_conditions.extend(v),
            (Some(v), _) => self.tb.node_sets.extend(v),
            (None, _) => return false,
        }
        true
    }
}
