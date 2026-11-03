//! The named-family writer (Spectre, VACASK): `name (nodes) master
//! params` statements over the reader's plan, spelled by the target's
//! [`Spelling`] row and gated by [`action`].
//!
//! This file is the Spectre spine (title, functions, globals, loads,
//! includes, parameters and `statistics`, models, subcircuits, sections,
//! the body, the testbench) and the block-level statements; `cards` writes
//! instances, waveforms and models, `bench` the testbench and analyses,
//! `checks` the deck-wide refusals (both targets'), `lift` the Monte Carlo
//! rules (Spectre's `Lift(RackVars)`, VACASK's `Lift(VacaskCalls)`: fixups
//! over `Call` nodes), `vacask` the VACASK spine and its blocks and models,
//! `vacask_cards` its instances, `vacask_bench` its `control` block. The two share the reader's plan, the checks, the
//! testbench fold and the lift's walk; their decks differ around them.
//!
//! Cites: Spectre Circuit Simulator Reference 19.1 (`[REF]`): p.494, p.493
//! (`include`), p.498 (`library`, `section`), p.506 (`parameters`),
//! pp.533-535 (`subckt`), p.491 (`if`), p.541 (`ahdl_include`), pp.175-179
//! (`statistics`); R5 pp.870-871 (`real` functions, `global 0`); Kundert,
//! *The Designer's Guide to SPICE and Spectre*, App. B (`[KUN]`: `//`,
//! `simulator lang=`, case); Spectre UG p.62 (escapes), p.108 (model
//! groups). The old writer (`codegen::spectre`) cites each statement too.
//!
//! Contents:
//! - `pub(super) fn write(rd: &mut Rd, deck: &Deck, target: Dialect) -> Result<String, Vec<Diag>>`

mod bench;
mod cards;
mod checks;
mod lift;
mod vacask;
mod vacask_bench;
mod vacask_cards;

use super::capability::{Action, Construct, Rule, action};
use super::out::{Out, Space};
use super::read::{Block, Comp, Deck, Elem, Func, Raw, Rd, Section};
use super::spelling::{Spelling, spelling};
use crate::codegen::Dialect;
use crate::syntax::ast::NONE;
use crate::syntax::diag::{Code, Diag};
use lift::Lifted;

/// The writer's state for one deck.
struct W<'a, 'r> {
    rd: &'r Rd<'a>,
    deck: &'r Deck,
    sp: &'static Spelling,
    /// The target.
    d: Dialect,
    /// The deck was read in the target's dialect: its names keep their
    /// case (`[KUN]` B.2: Spectre is case-sensitive, SPICE is not).
    keep_case: bool,
    /// The Monte Carlo lift's variables and functions.
    lifted: Lifted,
    o: Out,
    refused: Vec<Diag>,
}

/// The deck `rd` read, written for `target`; the refusals when any
/// construct has no meaning-keeping form there.
///
/// # Panics
/// For a target outside the named family (Spectre, VACASK).
pub(super) fn write(rd: &mut Rd, deck: &Deck, target: Dialect) -> Result<String, Vec<Diag>> {
    let mut refused = Vec::new();
    let lifted = match action(target, Construct::StatisticalCall) {
        Action::Lift(Rule::RackVars) => lift::lift(rd, deck, &mut refused),
        Action::Lift(Rule::VacaskCalls) => {
            lift::vacask_calls(rd, deck, &mut refused);
            Lifted::default()
        }
        _ => Lifted::default(),
    };
    let mut w = W {
        rd,
        deck,
        sp: spelling(target),
        d: target,
        keep_case: rd.d == target,
        lifted,
        o: Out::new(4096),
        refused,
    };
    w.checks();
    match target {
        Dialect::Spectre => w.spectre(),
        Dialect::Vacask => w.vacask(),
        _ => panic!("write: a named target"),
    }
    match w.refused.is_empty() {
        true => Ok(w.o.finish()),
        false => Err(w.refused),
    }
}

// ---- private ----

impl W<'_, '_> {
    /// `c`'s action for the target; a refusal is recorded at `token`.
    fn ask(&mut self, c: Construct, token: u32) -> Action {
        let a = action(self.d, c);
        if let Action::Refuse(code) = a {
            self.refuse(code, token);
        }
        a
    }

    fn refuse(&mut self, code: Code, token: u32) {
        self.refused.push(Diag { code, token });
    }

    fn line(&mut self, text: &str) {
        self.o.token(text, Space::Line);
    }

    /// The whole Spectre deck, with no line end after its last line.
    fn spectre(&mut self) {
        let deck = self.deck;
        let top = &deck.top;
        // `[KUN]` B.2: `//` is a comment; a netlist begins with its
        // language.
        self.line(&format!("{} {}", self.sp.comment, top.name));
        self.line("");
        self.line("simulator lang=spectre");
        self.line("");
        self.funcs(&deck.funcs, true);
        // `global 0 a b`: Spectre takes the first global as ground.
        if !deck.globals.is_empty() {
            let mut s = String::from("global 0");
            for g in &deck.globals {
                s.push(' ');
                s.push_str(&self.node(g));
            }
            self.line(&s);
        }
        for &n in &top.loads {
            let p = self.rd.unquote(self.rd.fields(n)[0]);
            self.line(&format!("ahdl_include \"{p}\""));
        }
        self.file_cards(top);
        let params = self.param_defs(top, Some(None));
        if !params.is_empty() {
            self.line(&format!("parameters{params}"));
        }
        self.statistics(top, None);
        self.models(top);
        for sc in &deck.defs {
            self.line("");
            self.subckt(sc);
        }
        self.sections();
        self.line("");
        self.elements(top);
        self.bench();
        // Lines were each written with their end; the deck has none after
        // the last.
        self.o.pop();
    }

    /// `real f(real x) {` / `return body;` / `}`, the deck's functions
    /// (and, with `rack`, the lift's `rack_*` functions after them). A body
    /// reading a node or a current has no Spectre function form.
    fn funcs(&mut self, funcs: &[Func], rack: bool) {
        let mut all: Vec<(String, Vec<String>, String, u32)> = funcs
            .iter()
            .map(|&f| {
                let n = match f {
                    Func::Param(a) | Func::Card(a) | Func::Real(a) => a,
                };
                let (name, args, body) = self.rd.func_view(f);
                (name, args, body, self.rd.main(n))
            })
            .collect();
        if rack {
            all.extend(self.lifted.rack().map(|(name, args, body)| {
                let args = args.iter().filter(|a| !a.is_empty());
                let args = args.map(|a| a.to_string()).collect();
                (name.to_string(), args, body.to_string(), 0)
            }));
        }
        for (name, args, body, tok) in all {
            let probe = checks::calls(&body)
                .any(|c| c.eq_ignore_ascii_case("v") || c.eq_ignore_ascii_case("i"));
            if probe {
                self.refuse(Code::W0106, tok);
                continue;
            }
            let args: Vec<String> = args.iter().map(|a| format!("real {a}")).collect();
            self.line(&format!("real {name}({}) {{", args.join(", ")));
            self.o.indent();
            self.o.indent();
            self.line(&format!("return {body};"));
            self.o.dedent();
            self.o.dedent();
            self.line("}");
        }
    }

    /// A block's `include` and `include … section=` statements.
    fn file_cards(&mut self, b: &Block) {
        for &inc in &b.includes {
            let p = self.rd.include_path(inc);
            self.line(&format!("include \"{p}\""));
        }
        for &lib in &b.libs {
            let (p, section) = self.rd.lib_view(lib);
            if section.is_empty() {
                self.ask(Construct::LibWholeFile, self.rd.main(lib));
            }
            self.line(&format!("include \"{p}\" section={section}"));
        }
    }

    /// ` name=value …`: block `b`'s parameter declarations (a subcircuit
    /// head's defaults first), then the lift's variables declared in the
    /// top or section `section` (`Some(None)` the deck's top).
    fn param_defs(&self, b: &Block, lifted: Option<Option<usize>>) -> String {
        let mut s = String::new();
        if b.node != NONE {
            let head = self
                .rd
                .subckt_head(b.node)
                .expect("the reader read the head");
            for (k, v) in &head.params {
                s.push_str(&format!(" {k}={}", self.value_text(v)));
            }
        }
        for &a in &b.params {
            let (k, v) = self.rd.param_view(a);
            s.push_str(&format!(" {k}={}", self.value_text(&v)));
        }
        if let Some(at) = lifted {
            for v in self.lifted.vars_at(at) {
                s.push_str(&format!(" {}=0", v.name));
            }
        }
        s
    }

    /// `statistics { … }`: block `b`'s, then the lift's for the top or
    /// section `at` (`[REF]` pp.175-179), one statement a line.
    fn statistics(&mut self, b: &Block, at: Option<usize>) {
        for &st in &b.statistics {
            let s = self.rd.statistics(st);
            self.stat_block(&s);
        }
        if let Some(s) = self.lifted.block_at(at) {
            self.stat_block(&s);
        }
    }

    fn stat_block(&mut self, st: &super::read::Stats) {
        self.line("statistics {");
        self.o.indent();
        self.o.indent();
        let scopes = [
            ("process", &st.process, &st.truncate[1]),
            ("mismatch", &st.mismatch, &st.truncate[2]),
        ];
        for (name, varies, t) in scopes {
            if varies.is_empty() && t.is_none() {
                continue;
            }
            self.line(&format!("{name} {{"));
            self.o.indent();
            self.o.indent();
            for x in varies {
                let key = if x.dist == "unif" { "N" } else { "std" };
                let pct = if x.percent { " percent=yes" } else { "" };
                let spread = self.value_text(&x.spread);
                self.line(&format!(
                    "vary {} dist={} {key}={spread}{pct}",
                    x.param, x.dist
                ));
            }
            if let Some(t) = t {
                self.line(&format!("truncate tr={}", self.value_text(t)));
            }
            self.o.dedent();
            self.o.dedent();
            self.line("}");
        }
        for (params, devices, cc) in &st.correlations {
            let mut s = String::from("correlate");
            if !devices.is_empty() {
                s.push_str(&format!(" dev=[{}]", devices.join(" ")));
            }
            if !params.is_empty() {
                s.push_str(&format!(" param=[{}]", params.join(" ")));
            }
            s.push_str(&format!(" cc={}", self.value_text(cc)));
            self.line(&s);
        }
        if let Some(t) = &st.truncate[0] {
            self.line(&format!("truncate tr={}", self.value_text(t)));
        }
        self.o.dedent();
        self.o.dedent();
        self.line("}");
    }

    /// `subckt name (ports)` … `ends name` (`[REF]` pp.533-535), an empty
    /// body one blank line.
    fn subckt(&mut self, sc: &Block) {
        let head = self
            .rd
            .subckt_head(sc.node)
            .expect("the reader read the head");
        let ports: Vec<String> = head.ports.iter().map(|p| self.node(p)).collect();
        let name = self.node(&sc.name);
        self.line(&format!("subckt {name} ({})", ports.join(" ")));
        if empty(sc) && head.params.is_empty() {
            self.line("");
        }
        self.body(sc, None);
        self.line(&format!("ends {name}"));
    }

    /// A block's statements, as a subcircuit's body holds them:
    /// `parameters` (its own, then a SPICE definition's `.param`s: Spectre
    /// has no parameter an instance cannot set), `statistics`, models,
    /// elements, the text kept. `lifted` is the top or section whose lifted
    /// variables it declares.
    fn body(&mut self, b: &Block, lifted: Option<Option<usize>>) {
        let params = self.param_defs(b, lifted);
        if !params.is_empty() {
            self.line(&format!("parameters{params}"));
        }
        if let Some(at) = lifted {
            self.statistics(b, at);
        } else {
            for &st in &b.statistics {
                let s = self.rd.statistics(st);
                self.stat_block(&s);
            }
        }
        if !b.locals.is_empty() {
            let mut s = String::from("parameters");
            for &a in &b.locals {
                let (k, v) = self.rd.param_view(a);
                s.push_str(&format!(" {k}={}", self.value_text(&v)));
            }
            self.line(&s);
        }
        self.models(b);
        self.elements(b);
    }

    /// Elements, instances and conditionals in place, then the text kept.
    fn elements(&mut self, b: &Block) {
        for e in b.elements() {
            match e {
                Elem::Comp(Comp::Card(n)) | Elem::Inst(n) => {
                    if let Some(line) = self.card(n) {
                        self.line(&line);
                    }
                }
                Elem::Comp(Comp::Raw(n)) => {
                    // A SPICE element line has no Spectre spelling (`checks`
                    // refuses it).
                    self.ask(Construct::ForeignText, self.rd.main(n));
                }
                Elem::Cond(k) => self.conditional(&b.conds[k].1),
            }
        }
        for &r in &b.raw {
            match r {
                Raw::Line(n) => {
                    let l = self.rd.line(n);
                    self.line(&l);
                }
                Raw::Lines(n) => {
                    for l in self.rd.lines(n) {
                        self.line(&l);
                    }
                }
                Raw::Joined(n) => {
                    let l = self.rd.lines(n).join("\n");
                    self.line(&l);
                }
            }
        }
    }

    /// `if (c) {` … `} else if (c) {` … `} else {` … `}` (`[REF]` p.491),
    /// each branch a body.
    fn conditional(&mut self, c: &super::read::Cond) {
        for (i, (b, body)) in c.branches.iter().enumerate() {
            let head = match (self.rd.condition(*b), i) {
                (Some(c), 0) => format!("if ({}) {{", self.value_text(&c)),
                (Some(c), _) => format!("}} else if ({}) {{", self.value_text(&c)),
                (None, _) => "} else {".to_string(),
            };
            self.line(&head);
            self.body(body, None);
        }
        self.line("}");
    }

    /// `library <lib>` / `section <name>` … `endsection <name>` /
    /// `endlibrary <lib>` (`[REF]` p.498), one `library` per run of
    /// sections of one library; a section of none is written bare.
    fn sections(&mut self) {
        let sections = &self.deck.sections;
        for (i, s) in sections.iter().enumerate() {
            let lib = s.library.as_ref();
            let first = i == 0 || sections[i - 1].library.as_ref() != lib;
            if let Some(name) = lib.filter(|_| first) {
                self.line("");
                self.line(&format!("library {}", self.node(name)));
            }
            self.section(s, i);
            let last = sections
                .get(i + 1)
                .is_none_or(|next| next.library.as_ref() != lib);
            if let Some(name) = lib.filter(|_| last) {
                self.line(&format!("endlibrary {}", self.node(name)));
            }
        }
    }

    /// One section: its functions and includes, then its body as a
    /// subcircuit's, then its definitions.
    fn section(&mut self, s: &Section, k: usize) {
        let name = self.node(&s.body.name);
        self.line(&format!("section {name}"));
        self.funcs(&s.funcs, false);
        self.file_cards(&s.body);
        self.body(&s.body, Some(Some(k)));
        for sc in &s.defs {
            self.subckt(sc);
        }
        self.line(&format!("endsection {name}"));
    }

    /// A node, model or subcircuit name in Spectre's spelling.
    fn node(&self, s: &str) -> String {
        escaped(s, false)
    }

    /// A device or instance name, or a reference to one: folded to
    /// lowercase for a SPICE-origin deck, whose device names are
    /// case-blind (an F card may sense `Vin` as `vin`).
    fn dev(&self, s: &str) -> String {
        escaped(s, !self.keep_case)
    }

    /// A reference to another instance (a probe, a swept device): lowered
    /// as [`W::dev`] lowers, unescaped.
    fn inst_ref(&self, s: &str) -> String {
        match self.keep_case {
            true => s.to_string(),
            false => s.to_lowercase(),
        }
    }

    /// A number in Spectre's suffixes (`[KUN]` Table B.1: `M` is mega).
    fn num(&self, v: f64) -> String {
        self.sp.format(v)
    }

    /// A value kept as SPICE text, in Spectre's spelling: without its
    /// SPICE delimiters (`'w*2'` is `w*2`), a SPICE number respelled
    /// (`IKF=30M` is milli: `30m`), anything else as written.
    fn value_text(&self, v: &str) -> String {
        let v = undelimited(v);
        match spice_num(v) {
            Some((x, _)) => self.num(x),
            None => v.to_string(),
        }
    }

    /// ` k=v` each (a flag bare), values [`W::value_text`].
    fn params(&self, params: &[(String, String)]) -> String {
        let mut s = String::new();
        for (k, v) in params {
            match v.is_empty() {
                true => s.push_str(&format!(" {k}")),
                false => s.push_str(&format!(" {k}={}", self.value_text(v))),
            }
        }
        s
    }
}

/// Whether block `b` writes no statement in a body.
fn empty(b: &Block) -> bool {
    b.params.is_empty()
        && b.statistics.is_empty()
        && b.locals.is_empty()
        && b.models.is_empty()
        && b.comps.is_empty()
        && b.insts.is_empty()
        && b.conds.is_empty()
        && b.raw.is_empty()
}

/// `s` as a Spectre identifier (UG p.62): a byte outside `[A-Za-z0-9_!]`
/// escaped with `\`, a leading digit too when any is (a name starting with
/// a digit reads as digits only); lowercased first with `fold`.
fn escaped(s: &str, fold: bool) -> String {
    let lower = fold && !s.bytes().all(|b| b.is_ascii() && !b.is_ascii_uppercase());
    let name = match lower {
        true => s.to_lowercase(),
        false => s.to_string(),
    };
    let plain = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '!';
    let mut out = String::with_capacity(name.len());
    if !name.chars().all(plain) && name.starts_with(|c: char| c.is_ascii_digit()) {
        out.push('\\');
    }
    for c in name.chars() {
        if !plain(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A SPICE value without its expression delimiters (`{e}`, `'e'`).
fn undelimited(v: &str) -> &str {
    let t = v.trim();
    let inner = t.strip_prefix('{').and_then(|r| r.strip_suffix('}'));
    inner
        .or_else(|| t.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
        .map_or(t, str::trim)
}

/// The value of `t` when it is one signed SPICE number with a scale and
/// a unit tail (`5V`, `1meg`), nothing else (`1k*(1+x)` is an expression),
/// and its VACASK spelling: the unit dropped, tera and giga in capitals
/// (`t` is no VACASK scale).
// ponytail: the view holds the value as SPICE text, so its number is read
// again here (the old writer's `parse_spice_num`); a view carrying the
// token's decoded value retires it.
fn spice_num(t: &str) -> Option<(f64, String)> {
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    let (v, len, scale) = crate::unit::parse_spice_number(body)?;
    let lead = t.len() - body.len() + len;
    if !t[lead..].bytes().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let scale = match scale {
        "t" => "T",
        "g" => "G",
        s => s,
    };
    let v = if t.starts_with('-') { -v } else { v };
    Some((v, format!("{}{scale}", &t[..lead])))
}
