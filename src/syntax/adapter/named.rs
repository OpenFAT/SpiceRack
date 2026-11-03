//! Spectre and VACASK statements → the IR, as `import/lower/mod.rs`'s
//! `card` (the `named` branches), `named.rs`, `statistics.rs`,
//! `sections.rs` and `conditionals.rs` lower them: the deck's declarations,
//! the dispatch over statement kinds, models and model groups, `statistics`
//! blocks, libraries and sections, `real` functions, `if` chains, `simulator
//! lang=spice` regions and VACASK `control` blocks.
//!
//! Cites: `import/lower/named.rs`, `statistics.rs`, `sections.rs`,
//! `conditionals.rs` (ported); `grammar/spectre.peg` (`stmt`, `named`, `Ana`,
//! `Ctl`, `Block`, `SpiceRegion`), `grammar/vacask.peg` (`stmt`,
//! `ControlBlock`, `Ana`); ESPice `spectre.zig:433-451` (the master map);
//! Spectre Reference 19.1 pp.175-179.
//!
//! Contents:
//! - `Cx::declarations(&self, stmts: &[u32]) -> (HashMap<String, String>, Vec<String>)`
//! - `Cx::named_card_spine(&mut self, n: u32, here: &mut Subcircuit, scope: Scope)`
//! - `Cx::pairs(&self, n: u32) -> Vec<(String, u32)>`, `Cx::get(&self, p: &[(String, u32)], k: &str) -> Option<u32>`
//! - `Cx::analysis_kind(&self, n: u32) -> Option<&'static str>`

use std::collections::HashMap;

use super::{Cx, Scope};
use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::token::Tag as T;

/// spectre.peg `Word_kind`: the masters that make a statement an analysis.
const WORD_KIND: &[&str] = &[
    "montecarlo",
    "acmatch",
    "dcmatch",
    "hbnoise",
    "qpnoise",
    "fourier",
    "pdisto",
    "pnoise",
    "hbstb",
    "envlp",
    "hbsp",
    "hbac",
    "hbxf",
    "noise",
    "qpac",
    "qpsp",
    "qpss",
    "qpxf",
    "sweep",
    "pac",
    "psp",
    "pss",
    "pstb",
    "pxf",
    "sens",
    "tran",
    "tdr",
    "stb",
    "ac",
    "dc",
    "hb",
    "pz",
    "sp",
    "xf",
];

/// spectre.peg `Word_ctl`: the masters that make it a control statement.
const WORD_CTL: &[&str] = &[
    "altergroup",
    "checklimit",
    "paramtest",
    "paramset",
    "options",
    "assert",
    "alter",
    "check",
    "shell",
    "info",
    "set",
];

/// The module names `codegen::vacask` declares itself (lower/named.rs).
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

impl Cx<'_> {
    /// The deck's model kinds (`name -> kind`, `=<builtin>` for a VACASK
    /// alias) and subcircuit names, wherever they sit (lower/named.rs
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

    /// One Spectre or VACASK statement (lower/mod.rs `card`, named).
    pub(super) fn named_card_spine(&mut self, n: u32, here: &mut Subcircuit, scope: Scope) {
        let tag = self.tag(n);
        if scope == Scope::Section && self.named_leaks(n) {
            return self.raw(n, here, matches!(tag, Tag::Region | Tag::Control));
        }
        match tag {
            Tag::Title | Tag::End => {}
            Tag::Verbatim | Tag::Line => self.unparsed(n, here),
            Tag::Card if self.ttag(self.main(n)) == T::KwSimulator => {}
            Tag::Instance if !self.inst_peg_ok(n) => self.unparsed(n, here),
            // quirk: vacask.peg `stmt`: `save`, `analysis`, `sweep` are
            // statements only inside `control`.
            Tag::Save | Tag::Card if self.d == Dialect::Vacask && !self.vacask_top_ok(n) => {
                self.unparsed(n, here)
            }
            Tag::Card | Tag::Model | Tag::Save | Tag::Region | Tag::Control | Tag::Instance => {
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

    /// The analysis master a Spectre instance statement names
    /// (spectre.peg `Ana`).
    pub(super) fn analysis_kind(&self, n: u32) -> Option<&'static str> {
        let m = self.master(n)?;
        WORD_KIND.iter().find(|&&k| k == m).copied()
    }
}

// ---- private ----

impl Cx<'_> {
    /// An instance statement's master word's text.
    fn master(&self, n: u32) -> Option<String> {
        if self.tag(n) != Tag::Instance {
            return None;
        }
        let m = self.ast.extra(self.ast.rhs(Node(n)) + 1);
        (m != NONE).then(|| self.text(m))
    }

    /// quirk: spectre.peg `Inst`/`Ana`/`Ctl`, vacask.peg `Inst`: a name, its
    /// nodes (in parentheses for VACASK), a master (VACASK: optional), then
    /// only `key=value` parameters; else the line is `Unparsed`.
    fn inst_peg_ok(&self, n: u32) -> bool {
        let words = self.ast.rhs(Node(n));
        let name = self.ast.extra(words);
        let master = self.ast.extra(words + 1);
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

    /// Whether a VACASK keyword statement is one `vacask.peg`'s `stmt` reads
    /// outside `control`.
    fn vacask_top_ok(&self, n: u32) -> bool {
        matches!(
            self.ttag(self.main(n)),
            T::KwLoad | T::KwEmbed | T::KwGround
        )
    }

    /// Whether the statement is a Spectre control statement (`Ctl`).
    fn is_ctl(&self, n: u32) -> bool {
        let nodes = self.ast.list(self.ast.rhs(Node(n)) + 2);
        self.d == Dialect::Spectre
            && nodes.is_empty()
            && self
                .master(n)
                .is_some_and(|m| WORD_CTL.contains(&m.as_str()))
    }

    /// lower/sections.rs `leaks`, named: control, output and analysis
    /// statements, and `global`.
    fn named_leaks(&self, n: u32) -> bool {
        match self.tag(n) {
            Tag::Global | Tag::Save | Tag::Region | Tag::Control => true,
            Tag::Instance => self.analysis_kind(n).is_some() || self.is_ctl(n),
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

    /// lower/named.rs `named_card`; `false` when the IR holds no such
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
            Tag::Instance if self.is_ctl(n) && self.master(n).as_deref() == Some("options") => {
                self.spectre_options(n);
                true
            }
            Tag::Instance if self.analysis_kind(n).is_some() || self.is_ctl(n) => {
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

    /// A Spectre `ic`/`nodeset` at the top (lower/analyses.rs).
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

    /// A `model` statement, a model group one model per bin (lower/named.rs
    /// `named_model`).
    fn named_model(&mut self, n: u32, here: &mut Subcircuit) -> bool {
        let f = self.fields(n);
        let (Some(&name), Some(&master)) = (f.first(), f.get(1)) else {
            return false;
        };
        let (name, master) = (self.name_of(name), self.name_of(master));
        let p = self.pairs(n);
        let bins = self.body(n).to_vec();
        if bins.is_empty() {
            return self.one_model(name, &master, p, here);
        }
        bins.into_iter().all(|b| {
            let bin = self.t.text(self.main(b)).to_string();
            let mut p = p.clone();
            p.extend(self.pairs(b));
            self.one_model(format!("{name}.{bin}"), &master, p, here)
        })
    }

    /// The model `name` of `master` with the parameters `p` (lower/named.rs
    /// `one_model`).
    fn one_model(
        &mut self,
        name: String,
        master: &str,
        p: Vec<(String, u32)>,
        here: &mut Subcircuit,
    ) -> bool {
        let (kind, level) = if self.d == Dialect::Vacask {
            if vacask_builtin(master).is_some() {
                return true;
            }
            match vacask_kind(master, self.get(&p, "type").and_then(|t| self.number(t))) {
                Some(kl) => kl,
                None => return false,
            }
        } else {
            let ty = self.get(&p, "type").map(|t| self.text(t));
            spectre_kind(master, ty.as_deref())
        };
        let drop_type = self.d == Dialect::Vacask || level.is_some() || kind != master;
        let mut parameters: Vec<(String, String)> = p
            .iter()
            .filter(|(k, _)| !(drop_type && k == "type"))
            .map(|(k, v)| (k.clone(), self.vtext(*v)))
            .collect();
        if let Some(l) = level {
            parameters.insert(0, ("level".into(), l.into()));
        }
        here.models.push(ModelDef {
            name,
            kind,
            parameters,
        });
        true
    }

    /// A Spectre `statistics` block (lower/statistics.rs).
    fn statistics(&self, n: u32) -> Statistics {
        let mut st = Statistics::default();
        for &k in self.body(n) {
            let head = self.ttag(self.main(k));
            match (self.tag(k), head) {
                (Tag::Process, _) => {
                    let scope = if head == T::KwProcess { 1 } else { 2 };
                    for &v in self.body(k) {
                        match self.ttag(self.main(v)) {
                            T::KwVary => {
                                let vary = self.vary(v);
                                if scope == 1 {
                                    st.process.push(vary)
                                } else {
                                    st.mismatch.push(vary)
                                }
                            }
                            T::KwTruncate => st.truncate[scope] = self.tr(v),
                            _ => {}
                        }
                    }
                }
                (_, T::KwCorrelate) => {
                    let p = self.pairs(k);
                    let list = |key: &str| {
                        self.get(&p, key).map_or(vec![], |v| {
                            let t = self.text(v);
                            t.trim_matches(|c| c == '[' || c == ']')
                                .split([' ', ',', '\t'])
                                .filter(|w| !w.is_empty())
                                .map(str::to_string)
                                .collect()
                        })
                    };
                    st.correlations.push(Correlate {
                        params: list("param"),
                        devices: list("dev"),
                        cc: self.get(&p, "cc").map_or(String::new(), |v| self.vtext(v)),
                    });
                }
                (_, T::KwTruncate) => st.truncate[0] = self.tr(k),
                _ => {}
            }
        }
        st
    }

    /// `vary <param> dist=<d> std=<s> | N=<n> [percent=yes]`.
    fn vary(&self, v: u32) -> Vary {
        let p = self.pairs(v);
        let text = |k: &str| self.get(&p, k).map(|x| self.text(x).trim().to_string());
        let dist = match text("dist").as_deref() {
            Some("lnorm") => Dist::Lnorm,
            Some("unif") => Dist::Unif,
            _ => Dist::Gauss,
        };
        let spread = self
            .get(&p, "std")
            .or_else(|| self.get(&p, "N"))
            .map_or(String::new(), |x| self.vtext(x));
        Vary {
            param: self.name_of(self.fields(v)[0]),
            dist,
            spread,
            percent: text("percent").as_deref() == Some("yes"),
        }
    }

    /// `truncate tr=<t>`.
    fn tr(&self, t: u32) -> Option<String> {
        let p = self.pairs(t);
        self.get(&p, "tr").map(|v| self.vtext(v))
    }

    /// `real f(real a, real b) { return e }` → a function: the name, then
    /// each argument after its `real`.
    fn real_func(&self, n: u32) -> FuncDef {
        let f = self.fields(n);
        let args = f[1..]
            .iter()
            .skip(1)
            .step_by(2)
            .map(|&k| self.name_of(k))
            .collect();
        let ret = self
            .body(n)
            .iter()
            .copied()
            .find(|&k| self.ttag(self.main(k)) == T::KwReturn);
        let body = ret.map_or(String::new(), |r| {
            self.text(self.fields(r)[0]).trim().to_string()
        });
        FuncDef {
            name: self.name_of(f[0]),
            args,
            body,
        }
    }

    /// A Spectre `if` chain or a VACASK `@if` block (lower/conditionals.rs).
    fn named_conditional(&mut self, n: u32, scope: Scope) -> Conditional {
        let mut c = Conditional::default();
        let mut branch = Some(Node(n));
        while let Some(b) = branch {
            let condition = match self.tag(b.0) {
                Tag::If => Some(super::unwrapped(&self.text(self.fields(b.0)[0]))),
                _ => None,
            };
            let mut body = Subcircuit::default();
            let stmts = self.body(b.0).to_vec();
            self.cards(&stmts, &mut body, scope);
            c.branches.push(Branch { condition, body });
            branch = self.ast.next(b);
        }
        c
    }

    /// A VACASK `control` block: its options and analyses, the rest kept as
    /// lines (lower/named.rs `ControlBlock`).
    fn vacask_control_block(&mut self, n: u32) {
        let body = self.body(n).to_vec();
        let mut sweeps: Vec<u32> = Vec::new();
        for &st in &body {
            let head = self.ttag(self.main(st));
            if head == T::KwSweep {
                sweeps.push(st);
                continue;
            }
            if head == T::KwAnalysis {
                let done = self.vacask_analysis(&sweeps, st);
                if !done {
                    for &s in sweeps.iter().chain([&st]) {
                        let t = self.line(s);
                        self.tb.extra_lines.push(t);
                    }
                }
                sweeps.clear();
                continue;
            }
            for &s in &sweeps {
                let t = self.line(s);
                self.tb.extra_lines.push(t);
            }
            sweeps.clear();
            if !self.vacask_options(st) {
                let t = self.line(st);
                self.tb.extra_lines.push(t);
            }
        }
        for &s in &sweeps {
            let t = self.line(s);
            self.tb.extra_lines.push(t);
        }
    }
}

/// A Spectre model master (and `type=`) → SPICE `.model` kind and LEVEL
/// (lower/named.rs `spectre_kind`).
fn spectre_kind(master: &str, ty: Option<&str>) -> (String, Option<&'static str>) {
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

/// `sp_<module>` (and `type=`) → SPICE kind and LEVEL (lower/named.rs
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

/// The built-in a VACASK `model NAME <module>` names (lower/named.rs
/// `vacask_builtin`).
fn vacask_builtin(module: &str) -> Option<&'static str> {
    let m = module.strip_prefix("sp_").unwrap_or(module);
    VACASK_BUILTINS.iter().find(|a| **a == m).copied()
}
