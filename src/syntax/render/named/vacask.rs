//! The VACASK spine of the named writer: the same `name (nodes) master
//! k=v` statements as Spectre's, with VACASK's own deck around them. VACASK
//! has no anonymous instances, so every device module the deck uses is
//! loaded and named by a `model` line (`load "spice/resistor.osdi"`, `model
//! resistor sp_resistor`); names are quoted where they are no identifier and
//! lowercased for a SPICE-origin deck (VACASK is case-sensitive); numbers
//! carry no suffix (`M` is mega there, milli in SPICE); the testbench is one
//! `control` block (`vacask_cards` writes the instances, `vacask_bench`
//! the block).
//!
//! Every shape was probed against the binary by the old writer
//! (`codegen::vacask`, which cites `share/doc/vacask/{demo,test}/*.sim`);
//! VACASK hard-errors on an unknown parameter, so a name passed through is
//! a loud failure, never a substitution.
//!
//! Cites: VACASK `docs/input-identifiers.md` (quoting),
//! `lib/dfllexer.l` L536-L557 (`section`), P462-P509 (`@if`), P848-855
//! (`parameters`), `docs/cmd-analysis-mc.md`.
//!
//! Contents:
//! - `W::vacask(&mut self)`, `W::expr(&self, s: &str) -> String`, `W::vval(&self, v: &Val) -> String`,
//!   `W::vacask_num(&self, x: &Num) -> String`, `W::inst_params(&self, params: &[(String, String)]) -> String`
//! - `fn num(v: f64) -> String`, `fn respell(s: &str) -> String`, `fn q(name: &str) -> String`

use std::collections::BTreeSet;

use super::W;
use crate::codegen::Dialect;
use crate::syntax::ast::NONE;
use crate::syntax::diag::Code;
use crate::syntax::render::capability::Construct;
use crate::syntax::render::read::Func;
use crate::syntax::render::read::{Block, Card, Comp, Elem, Model, Num, Raw, Val};

/// A device module the deck needs: VACASK's build of the SPICE device set
/// (`load "spice/<m>.osdi"` and `model <m> sp_<m>`) or a built-in (a
/// `model` line only). The model is named after the module, as upstream's
/// own decks do (`model resistor resistor`, `test/test_op.sim`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Need {
    Sp(&'static str),
    Builtin(&'static str),
}

impl W<'_, '_> {
    /// The whole VACASK deck, ending in a line end.
    pub(super) fn vacask(&mut self) {
        let deck = self.deck;
        let blocks = self.vacask_blocks();
        let mut funcs = deck
            .funcs
            .iter()
            .chain(deck.sections.iter().flat_map(|s| &s.funcs));
        if let Some(&(Func::Param(n) | Func::Card(n) | Func::Real(n))) = funcs.next() {
            self.ask(Construct::Func, self.rd.main(n));
        }
        let mut needs: BTreeSet<Need> = BTreeSet::new();
        for b in &blocks {
            for &c in &b.comps {
                if let Comp::Card(n) = c
                    && let Some(need) = need(&self.rd.card_view(n))
                {
                    needs.insert(need);
                }
            }
        }
        // Models first: they add loads too. A subcircuit's and a section's
        // models stay in it, where VACASK scopes them.
        let mut loads: BTreeSet<String> = BTreeSet::new();
        let top_models = self.model_lines(&deck.top, &mut loads);
        let def_models: Vec<Vec<String>> = deck
            .defs
            .iter()
            .map(|sc| self.model_lines(sc, &mut loads))
            .collect();
        // A branch's models load their modules too.
        for b in &blocks {
            for (_, c) in &b.conds {
                for (_, body) in &c.branches {
                    self.model_lines(body, &mut loads);
                }
            }
        }
        let section_models: Vec<Vec<Vec<String>>> = deck
            .sections
            .iter()
            .map(|s| {
                std::iter::once(&s.body)
                    .chain(&s.defs)
                    .map(|b| self.model_lines(b, &mut loads))
                    .collect()
            })
            .collect();
        for n in &needs {
            if let Need::Sp(m) = n {
                loads.insert(format!("spice/{m}.osdi"));
            }
        }
        // A VACASK deck's own `load`s are VACASK's (it compiles a `.va`
        // itself); a SPICE-origin Verilog-A load goes through OpenVAF.
        let paths: Vec<String> = deck
            .top
            .loads
            .iter()
            .map(|&n| self.rd.unquote(self.rd.fields(n)[0]))
            .collect();
        self.unwritten_loads();
        match self.rd.d == Dialect::Vacask {
            true => loads.extend(paths),
            false => match crate::veriloga::osdi_loads(&paths) {
                Ok(l) => loads.extend(l),
                Err(_) => self.refuse(Code::W0004, self.rd.main(deck.top.loads[0])),
            },
        }

        let title = deck.top.name.replace(['\n', '\r'], " ");
        let title = match title.trim().is_empty() {
            true => "circuit".to_string(),
            false => title,
        };
        let frame = self.frame();
        if frame.title {
            self.line(&title);
            self.line("");
        }
        if frame.ground {
            self.line("ground 0");
            self.line("");
        }
        if !deck.globals.is_empty() {
            let gs: Vec<String> = deck.globals.iter().map(|g| q(g)).collect();
            self.line(&format!("global {}", gs.join(" ")));
        }
        for l in &loads {
            self.line(&format!("load \"{l}\""));
        }
        self.vacask_files(&deck.top, "");
        self.line("");
        for n in &needs {
            // A user model wearing a synthesised name would be a second
            // definition; VACASK takes the first and runs the wrong device.
            let auto = n.auto();
            let clash = blocks.iter().find_map(|b| {
                b.models.iter().find_map(|&(m, bin)| {
                    let name = self.rd.model_view(m, bin).name;
                    name.eq_ignore_ascii_case(auto).then(|| self.rd.main(m))
                })
            });
            if let Some(tok) = clash {
                self.refuse(Code::W0104, tok);
            }
            let line = match n {
                Need::Sp(m) => format!("model {auto} sp_{m}"),
                Need::Builtin(m) => format!("model {auto} {m}"),
            };
            self.line(&line);
        }
        for l in &top_models {
            self.line(l);
        }
        self.line("");
        // `.step param` names become control variables, so they are no
        // netlist parameters.
        let tb = self.fold();
        let stepped: Vec<String> = tb.steps.iter().map(|s| s.1.to_ascii_lowercase()).collect();
        let params = self.param_list(&deck.top.params, &stepped);
        if !params.is_empty() {
            self.line(&format!("parameters{params}"));
            self.line("");
        }
        for (sc, models) in deck.defs.iter().zip(&def_models) {
            self.vacask_subckt(sc, models);
            self.line("");
        }
        for (s, models) in deck.sections.iter().zip(&section_models) {
            self.line(&format!("section {}", q(&s.body.name)));
            self.vacask_files(&s.body, "  ");
            self.vacask_body(&s.body, &models[0]);
            for (sc, m) in s.defs.iter().zip(&models[1..]) {
                self.vacask_subckt(sc, m);
            }
            self.line("endsection");
            self.line("");
        }
        self.vacask_elements(&deck.top, "");
        if frame.control {
            self.control(&tb);
        }
    }
}

impl W<'_, '_> {
    /// An expression, lowercased for a SPICE-origin deck (SPICE names are
    /// case-blind, VACASK's are not) unless it is a number, then
    /// [`respell`]ed; a SPICE `'e'` is an expression, a VACASK one a quoted
    /// name, kept.
    pub(super) fn expr(&self, s: &str) -> String {
        if self.keep_case {
            return respell(s);
        }
        let s = super::undelimited(s);
        match super::spice_num(s.trim()) {
            Some(_) => respell(s),
            None => respell(&s.to_ascii_lowercase()),
        }
    }

    /// A value: a number through [`num`], an expression through
    /// [`W::expr`], raw text [`respell`]ed.
    pub(super) fn vval(&self, v: &Val) -> String {
        match v {
            Val::Num(x) => num(*x),
            Val::Expr(e) => self.expr(e),
            Val::Raw(t) => respell(t),
        }
    }

    /// A waveform value: [`num`], or a [`W::expr`].
    pub(super) fn vacask_num(&self, x: &Num) -> String {
        match x {
            Num::Num(v) => num(*v),
            Num::Expr(e) => self.expr(e),
        }
    }

    /// Instance parameters: `m`, SPICE's device multiplier, is the
    /// Verilog-A builtin `$mfactor`.
    pub(super) fn inst_params(&self, params: &[(String, String)]) -> String {
        let mut s = String::new();
        for (k, v) in params {
            let k = k.to_ascii_lowercase();
            let k = if k == "m" { "$mfactor" } else { &k };
            s.push_str(&format!(" {k}={}", self.expr(v)));
        }
        s
    }
}

/// A plain number VACASK cannot misread: no suffix (SPICE's are
/// case-blind, VACASK's not: `M` is milli in one and mega in the other).
pub(super) fn num(v: f64) -> String {
    match v == 0.0 {
        true => "0".to_string(),
        false => format!("{v:e}"),
    }
}

/// A value as text: a number respelled in VACASK's suffixes, anything else
/// (a name, an expression) without ngspice's `{}`.
pub(super) fn respell(s: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .unwrap_or(t)
        .trim();
    match super::spice_num(t) {
        Some((_, spelled)) => spelled,
        None => t.to_string(),
    }
}

/// A name as a VACASK identifier (`docs/input-identifiers.md`):
/// `[a-zA-Z_$][0-9a-zA-Z_$]*` or a number without a leading zero, else
/// quoted, `''` for a quote; lowercased.
pub(super) fn q(name: &str) -> String {
    let ident = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    let lead = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$');
    let number =
        name.chars().all(|c| c.is_ascii_digit()) && (name == "0" || !name.starts_with('0'));
    let quoted = !(ident && (lead || number)) && !name.is_empty();
    let mut out = String::with_capacity(name.len() + 2);
    if quoted {
        out.push('\'');
    }
    for c in name.chars() {
        if quoted && c == '\'' {
            out.push('\'');
        }
        out.push(c.to_ascii_lowercase());
    }
    if quoted {
        out.push('\'');
    }
    out
}

// ---- private ----

impl<'r> W<'_, 'r> {
    /// Every block a body is written from, branches included.
    fn vacask_blocks(&self) -> Vec<&'r Block> {
        let deck = self.deck;
        let sections = deck
            .sections
            .iter()
            .flat_map(|s| std::iter::once(&s.body).chain(&s.defs));
        let mut work: Vec<&Block> = std::iter::once(&deck.top)
            .chain(&deck.defs)
            .chain(sections)
            .collect();
        work.reverse();
        let mut out = Vec::new();
        while let Some(b) = work.pop() {
            out.push(b);
            let branches = b.conds.iter().flat_map(|(_, c)| &c.branches);
            let mut inner: Vec<&Block> = branches.map(|(_, body)| body).collect();
            inner.reverse();
            work.extend(inner);
        }
        out
    }
}

impl W<'_, '_> {
    /// Block `b`'s `include` and `include … section=` statements, each
    /// after `pad`.
    fn vacask_files(&mut self, b: &Block, pad: &str) {
        for &inc in &b.includes {
            let p = self.rd.include_path(inc);
            self.line(&format!("{pad}include \"{p}\""));
        }
        for &lib in &b.libs {
            let (p, section) = self.rd.lib_view(lib);
            if section.is_empty() {
                self.ask(Construct::LibWholeFile, self.rd.main(lib));
            }
            self.line(&format!("{pad}include \"{p}\" section={section}"));
        }
    }

    /// `subckt name (ports)` … `ends`.
    fn vacask_subckt(&mut self, sc: &Block, models: &[String]) {
        let head = self
            .rd
            .subckt_head(sc.node)
            .expect("the reader read the head");
        let ports: Vec<String> = head.ports.iter().map(|p| q(p)).collect();
        self.line(&format!("subckt {} ({})", q(&sc.name), ports.join(" ")));
        self.vacask_body(sc, models);
        self.line("ends");
    }

    /// A block's models, `parameters` (its own, then a SPICE definition's
    /// `.param`s: one reading another cannot be overridden, P848-855),
    /// elements and text kept: a `subckt`'s or a `section`'s body.
    fn vacask_body(&mut self, b: &Block, models: &[String]) {
        for m in models {
            self.line(&format!("  {m}"));
        }
        let mut params = String::new();
        if b.node != NONE {
            let head = self
                .rd
                .subckt_head(b.node)
                .expect("the reader read the head");
            for (k, v) in &head.params {
                params.push_str(&format!(" {}={}", k.to_ascii_lowercase(), self.expr(v)));
            }
        }
        params.push_str(&self.param_list(&b.params, &[]));
        if !params.is_empty() {
            self.line(&format!("parameters{params}"));
        }
        // A subcircuit's mismatch draws come first among its locals.
        let draws = match b.node {
            NONE => &[][..],
            n => self.draws.locals(n),
        };
        let mut locals: String = draws
            .iter()
            .map(|(k, v)| format!(" {}={}", k.to_ascii_lowercase(), self.expr(v)))
            .collect();
        locals.push_str(&self.param_list(&b.locals, &[]));
        if !locals.is_empty() {
            self.line(&format!("parameters{locals}"));
        }
        self.vacask_elements(b, "  ");
    }

    /// ` name=value …` for `params` (the names in `skip` left out), each
    /// after the draws the `statistics` lift puts before it and with the
    /// value it gives it (one layer: `p = agauss(…)`).
    fn param_list(&self, params: &[u32], skip: &[String]) -> String {
        let mut s = String::new();
        let mut put = |k: &str, v: &str| {
            if !skip.contains(&k.to_ascii_lowercase()) {
                s.push_str(&format!(" {}={}", k.to_ascii_lowercase(), self.expr(v)));
            }
        };
        for &a in params {
            for (k, v) in self.draws.before(a) {
                put(k, v);
            }
            let (k, v) = self.rd.param_view(a);
            put(&k, self.draws.value(a).unwrap_or(&v));
        }
        s
    }

    /// Elements, instances and conditionals in place, then the text kept,
    /// each after `pad` (a conditional's first line only).
    fn vacask_elements(&mut self, b: &Block, pad: &str) {
        for e in b.elements() {
            match e {
                Elem::Comp(Comp::Card(n)) | Elem::Inst(n) => {
                    if let Some(line) = self.vacask_card(n) {
                        self.line(&format!("{pad}{line}"));
                    }
                }
                Elem::Comp(Comp::Raw(n)) => {
                    self.ask(Construct::ForeignText, self.rd.main(n));
                }
                Elem::Cond(k) => self.vacask_conditional(&b.conds[k].1, pad),
            }
        }
        for &r in &b.raw {
            let lines = match r {
                Raw::Line(n) => vec![self.rd.line(n)],
                Raw::Lines(n) => self.rd.lines(n),
                Raw::Joined(n) => vec![self.rd.lines(n).join("\n")],
            };
            for l in lines {
                self.line(&format!("{pad}{l}"));
            }
        }
    }

    /// `@if c` … `@elseif c` … `@else` … `@end` (P462-P509): a branch holds
    /// instances and models only; its models' loads were collected with
    /// every other model's.
    fn vacask_conditional(&mut self, c: &crate::syntax::render::read::Cond, pad: &str) {
        for (i, (b, body)) in c.branches.iter().enumerate() {
            if !body.params.is_empty() || !body.locals.is_empty() {
                self.ask(Construct::BranchParam, self.rd.main(*b));
            }
            let head = match (self.rd.condition(*b), i) {
                (Some(c), 0) => format!("{pad}@if {}", self.expr(&c)),
                (Some(c), _) => format!("@elseif {}", self.expr(&c)),
                (None, _) => "@else".to_string(),
            };
            self.line(&head);
            let models = self.model_lines(body, &mut BTreeSet::new());
            self.vacask_body(body, &models);
        }
        self.line("@end");
    }

    /// Block `b`'s `model` lines, the OSDI files their modules need added
    /// to `loads`; a model with no VACASK module is refused.
    fn model_lines(&mut self, b: &Block, loads: &mut BTreeSet<String>) -> Vec<String> {
        let mut out = Vec::new();
        for &(m, bin) in &b.models {
            let view = self.rd.model_view(m, bin);
            match self.vacask_model(&view, loads) {
                Some(l) => out.push(l),
                None => self.refuse(Code::W0104, self.rd.main(m)),
            }
        }
        out
    }

    /// Model `m`'s line: LEVEL picks the module (a type or level with none
    /// is `None`); a kind no SPICE type names is a module the deck loads
    /// itself, passed through ("Model not found" beats a substitute).
    fn vacask_model(&self, m: &Model, loads: &mut BTreeSet<String>) -> Option<String> {
        let kind = m.kind.to_ascii_lowercase();
        let name = q(&m.name);
        let mut level = 1u32;
        for (k, v) in &m.params {
            if k.eq_ignore_ascii_case("level") {
                let t = v
                    .trim()
                    .trim_start_matches('{')
                    .trim_end_matches('}')
                    .trim();
                level = super::spice_num(t)?.0 as u32;
            }
        }
        let (module, sign) = match kind.as_str() {
            "nmos" | "pmos" => (mos_module(level)?, if kind == "nmos" { 1 } else { -1 }),
            // `.model x NPN (LEVEL=4)` is VBIC, `LEVEL=8` HICUM: other
            // equations under the same keyword.
            k if model_type(k).is_some() => {
                if level != 1 {
                    return None;
                }
                model_type(k)?
            }
            // `sp_resistor` renames the card's `r`/`tc1` to `model_r`…: a
            // name-for-name card would set the instance's. No switches.
            "r" | "c" | "l" | "res" | "cap" | "ind" | "sw" | "csw" => return None,
            _ => {
                return Some(format!(
                    "model {name} {kind}{}",
                    self.vacask_params(m, None)
                ));
            }
        };
        loads.insert(format!("spice/{module}.osdi"));
        // `type=` is the polarity, and keeps an otherwise empty list
        // non-empty: `model x sp_diode ( )` is a syntax error.
        let lead = (sign != 0).then_some(sign);
        Some(format!(
            "model {name} sp_{module}{}",
            self.vacask_params(m, lead)
        ))
    }

    /// ` (type=1 k=v …)`, or nothing.
    fn vacask_params(&self, m: &Model, sign: Option<i8>) -> String {
        let mut s = String::new();
        if let Some(t) = sign {
            s.push_str(&format!(" type={t}"));
        }
        for (k, v) in m
            .params
            .iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("level"))
        {
            s.push_str(&format!(" {}={}", k.to_ascii_lowercase(), self.expr(v)));
        }
        if !s.is_empty() {
            s.replace_range(..1, " (");
            s.push(')');
        }
        s
    }
}

impl Need {
    /// The synthesised model's name, the module's.
    fn auto(&self) -> &'static str {
        match self {
            Need::Sp(m) | Need::Builtin(m) => m,
        }
    }
}

/// The module element `c` needs, if any.
fn need(c: &Card) -> Option<Need> {
    Some(match c {
        Card::Rcl { letter: 'R', .. } => Need::Sp("resistor"),
        Card::Rcl { letter: 'C', .. } => Need::Sp("capacitor"),
        Card::Rcl { .. } => Need::Sp("inductor"),
        Card::Coupling { .. } => Need::Builtin("mutual"),
        Card::Source { letter: 'V', .. } => Need::Builtin("vsource"),
        Card::Source { .. } => Need::Builtin("isource"),
        Card::Linear { letter: 'E', .. } => Need::Builtin("vcvs"),
        Card::Linear { letter: 'G', .. } => Need::Builtin("vccs"),
        Card::Linear { letter: 'F', .. } => Need::Builtin("cccs"),
        Card::Linear { .. } => Need::Builtin("ccvs"),
        _ => return None,
    })
}

/// SPICE MOSFET LEVEL → VACASK module; a level not here has none ("the
/// nearest model" is other equations).
fn mos_module(level: u32) -> Option<&'static str> {
    Some(match level {
        1 => "mos1",
        2 => "mos2",
        3 => "mos3",
        6 => "mos6",
        9 => "mos9",
        49 | 8 => "bsim3v3",
        54 | 14 => "bsim4v8",
        _ => return None,
    })
}

/// `.model` TYPE → (module, polarity), for the types whose parameters
/// transcribe name for name at LEVEL 1.
fn model_type(kind: &str) -> Option<(&'static str, i8)> {
    Some(match kind {
        "d" => ("diode", 0),
        "npn" => ("bjt", 1),
        "pnp" => ("bjt", -1),
        "njf" => ("jfet1", 1),
        "pjf" => ("jfet1", -1),
        "nmf" => ("mes1", 1),
        "pmf" => ("mes1", -1),
        "vdmos" => ("vdmos", 1),
        _ => return None,
    })
}
