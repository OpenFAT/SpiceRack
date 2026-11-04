//! The SPICE-family writer (ngspice, ESPice, HSPICE, Xyce, LTspice, and
//! CDL in the xschem layout): one deck spine over the reader's plan, spelled by the target's
//! [`Spelling`] row and gated by [`action`]. The targets differ only in
//! table rows and in the `Spell`/`Lift`/`Refuse` answers.
//!
//! This file is the spine (title, loads, globals, functions, includes,
//! parameters and models, subcircuits, sections, the body, the testbench,
//! `.end`; the frame of title, `.end` and control blocks decided once, by
//! the file's role) and the block-level cards; `cards` writes elements and
//! waveforms, `bench` the testbench and analyses, `checks` the deck-wide
//! refusals (another family's text, `^`, statistical calls), `laplace` the
//! transfer function a Laplace lift reads,
//! `names` the element names with their letters, `cdl` CDL's layout; the
//! `statistics` lift is `render::draws`, which VACASK's writer shares.
//!
//! Cites: ngspice manual ch.2 (the deck), §2.4.3 (`.subckt`), §2.6, §2.7
//! (`.include`, `.lib`), §2.15 (`.if`); HSPICE `.END` (a library ends
//! without one: the old HSPICE grammar's L9); Xyce RG Table 2-36 (BSIM3
//! level 49).
//!
//! Contents:
//! - `pub(super) fn write(rd: &mut Rd, deck: &Deck, target: Dialect) -> Result<String, Vec<Diag>>`

mod bench;
mod cards;
mod cdl;
mod checks;
mod laplace;
mod names;

use super::capability::{Action, Alt, Construct, Rule, action};
use super::draws::{self, Layers};
use super::out::{Out, Space};
use super::read::{Block, Comp, Deck, Elem, Func, Raw, Rd, Section};
use super::role::{self, Frame};
use super::spelling::{FuncForm, Spelling, spelling};
use crate::codegen::Dialect;
use crate::syntax::diag::{Code, Diag};
use crate::syntax::resolve::Role;

/// The writer's state for one deck.
struct W<'a, 'r> {
    rd: &'r Rd<'a>,
    deck: &'r Deck,
    sp: &'static Spelling,
    /// The target.
    d: Dialect,
    /// The deck's numbers were read with ngspice's arithmetic.
    x: bool,
    o: Out,
    refused: Vec<Diag>,
    /// The `statistics` lift's fixups.
    draws: draws::Draws,
    /// The element names, letters added.
    names: names::Names,
    /// The subcircuit being written (its node; `NONE` at the top): the
    /// scope a reference is looked up in.
    scope: u32,
    /// The deck runs its `.four` cards as `fourier` commands in its
    /// `.control` block, so the cards are left out.
    drop_four: bool,
    /// A section's definitions are being written.
    in_section: bool,
}

/// The deck `rd` read, written for `target`; the refusals when any
/// construct has no meaning-keeping form there. The `Lift` rules record
/// their fixups in `rd` before anything is written.
pub(super) fn write(rd: &mut Rd, deck: &Deck, target: Dialect) -> Result<String, Vec<Diag>> {
    let sp = spelling(target);
    let mut refused = Vec::new();
    let draws = match action(target, Construct::Statistics) {
        Action::Lift(Rule::StatisticsDraws) => {
            draws::lift(rd, deck, Layers::Two).unwrap_or_else(|d| {
                refused.push(d);
                draws::Draws::default()
            })
        }
        _ => draws::Draws::default(),
    };
    let rd: &Rd = rd;
    let mut w = W {
        rd,
        deck,
        sp,
        d: target,
        // CDL's cards are the old layout's `codegen::card(c, Ngspice)`,
        // which spells numbers without ngspice's arithmetic.
        x: rd.d == Dialect::Ngspice && target != Dialect::Cdl,
        o: Out::new(4096),
        refused,
        draws,
        names: names::Names::new(rd, deck),
        scope: crate::syntax::ast::NONE,
        drop_four: false,
        in_section: false,
    };
    w.checks();
    if target == Dialect::Cdl {
        w.cdl();
        return match w.refused.is_empty() {
            true => Ok(w.o.finish()),
            false => Err(w.refused),
        };
    }
    let frame = w.frame();
    let control = if frame.control { w.control_run() } else { None };
    w.drop_four = control.is_some();
    if frame.title {
        w.o.token(&format!("{} {}", sp.comment, deck.top.name), Space::Line);
    }
    w.loads(frame.control);
    w.globals();
    w.funcs(&deck.funcs);
    w.file_cards(&deck.top);
    w.params_and_models(&deck.top);
    for sc in &deck.defs {
        w.o.token("", Space::Line);
        w.subckt(sc);
    }
    for s in &deck.sections {
        w.o.token("", Space::Line);
        w.section(s);
    }
    w.body(&deck.top, true);
    w.bench(control.as_deref());
    match frame.end {
        true => w.o.token(".end", Space::None),
        // The deck ends on its last card's line end.
        false => w.o.pop(),
    }
    match w.refused.is_empty() {
        true => Ok(w.o.finish()),
        false => Err(w.refused),
    }
}

// ---- private ----

impl W<'_, '_> {
    /// What the deck is written in: what its role says ([`role::frame`]),
    /// but for the old writer's rules where they give other bytes, which
    /// the golden gate holds until phase 7.
    fn frame(&self) -> Frame {
        let role = self.rd.role();
        let mut f = role::frame(role);
        if role == Role::Library {
            // quirk: the old writer titled and ended a library, and loaded
            // its Verilog-A in a `.control` block, as any deck; phase 7
            // writes it bare.
            f = Frame {
                title: true,
                end: true,
                control: true,
                ground: true,
            };
        }
        if self.d == Dialect::Hspice {
            // quirk: the old HSPICE writer's guess at the role: `.END` ends
            // the run even in an included file, so a file with no analysis
            // (a library, or a deck of sources alone) ends without one;
            // phase 7 asks the role.
            f.end = self.runs();
        }
        f
    }

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

    /// Model loads that must precede any card instantiating them:
    /// ngspice's `pre_osdi` control block (when the frame has `control`),
    /// ESPice's and HSPICE's `.hdl` (they compile Verilog-A themselves and
    /// load no OSDI binaries).
    fn loads(&mut self, control: bool) {
        let deck = self.deck;
        let path = |n: u32| self.rd.unquote(self.rd.fields(n)[0]);
        let top: Vec<String> = deck.top.loads.iter().map(|&n| path(n)).collect();
        let tok = deck.top.loads.first().map_or(0, |&n| self.rd.main(n));
        let source = crate::veriloga::is_source;
        match self.d {
            Dialect::Ngspice if control && !top.is_empty() => {
                match crate::veriloga::osdi_loads(&top) {
                    Ok(loads) => {
                        self.o.token(".control", Space::Line);
                        for p in &loads {
                            self.o.token(&format!("pre_osdi {p}"), Space::Line);
                        }
                        self.o.token(".endc", Space::Line);
                    }
                    Err(_) => self.refuse(Code::W0004, tok),
                }
            }
            Dialect::Espice => {
                let all = deck
                    .top
                    .loads
                    .iter()
                    .chain(deck.defs.iter().flat_map(|s| &s.loads));
                let mut seen: Vec<String> = Vec::new();
                for &n in all {
                    let l = path(n);
                    if !source(&l) {
                        self.ask(Construct::OsdiLoad, self.rd.main(n));
                    } else if !seen.contains(&l) {
                        self.o.token(&format!(".hdl \"{l}\""), Space::Line);
                        seen.push(l);
                    }
                }
            }
            Dialect::Xyce if !top.is_empty() => {
                self.ask(Construct::VerilogALoad, tok);
            }
            Dialect::Hspice => {
                for l in &top {
                    if !source(l) {
                        self.ask(Construct::OsdiLoad, tok);
                    } else {
                        self.o.token(&format!(".hdl \"{l}\""), Space::Line);
                    }
                }
            }
            _ => {}
        }
    }

    fn globals(&mut self) {
        if !self.deck.globals.is_empty() {
            let line = format!(".global {}", self.deck.globals.join(" "));
            self.o.token(&line, Space::Line);
        }
    }

    /// The functions, one card each (HSPICE spells one as a parameter).
    fn funcs(&mut self, funcs: &[Func]) {
        for &f in funcs {
            let (name, args, body) = self.rd.func_view(f);
            let args = args.join(", ");
            let tok = match f {
                Func::Param(a) | Func::Card(a) | Func::Real(a) => self.rd.main(a),
            };
            let line = match self.ask(Construct::Func, tok) {
                Action::Spell(Alt::FuncParam) => {
                    debug_assert_eq!(self.sp.func, FuncForm::Param);
                    format!(".param {name}({args})='{body}'")
                }
                _ => format!(".func {name}({args}) {{{body}}}"),
            };
            self.o.token(&line, Space::Line);
        }
    }

    /// A block's `.include` and `.lib` cards, in the block: SPICE scopes a
    /// `.lib` inside a `.subckt` to that subcircuit, so hoisting one
    /// changes which models it sees.
    fn file_cards(&mut self, b: &Block) {
        let q = self.sp.path_quote;
        for &inc in &b.includes {
            let p = self.rd.include_path(inc);
            self.o.token(&format!(".include {q}{p}{q}"), Space::Line);
        }
        for &lib in &b.libs {
            let (p, section) = self.rd.lib_view(lib);
            if section.is_empty() {
                if self.ask(Construct::LibWholeFile, self.rd.main(lib)) == Action::Native {
                    self.o.token(&format!(".lib {q}{p}{q}"), Space::Line);
                }
                continue;
            }
            self.o
                .token(&format!(".lib {q}{p}{q} {section}"), Space::Line);
        }
    }

    /// A block's `.param` cards and `.model` cards, as a deck's top and a
    /// library section hold them.
    fn params_and_models(&mut self, b: &Block) {
        self.param_cards(&b.params);
        self.models(b);
    }

    /// `.param name=value`, one card each, in order, with the draws the
    /// `statistics` lift puts before each and the value it gives one.
    fn param_cards(&mut self, params: &[u32]) {
        for &a in params {
            let draws = self.draws.before(a).to_vec();
            self.param_lines(&draws);
            let (name, mut v) = self.rd.param_view(a);
            if let Some(d) = self.draws.value(a) {
                v = d.to_string();
            }
            let v = self.spelled(&v);
            self.o.token(&format!(".param {name}={v}"), Space::Line);
        }
    }

    /// `.param name=value` for parameters a `Lift` rule adds.
    fn param_lines(&mut self, params: &[(String, String)]) {
        for (name, v) in params {
            let v = self.spelled(v);
            self.o.token(&format!(".param {name}={v}"), Space::Line);
        }
    }

    fn models(&mut self, b: &Block) {
        for &(m, bin) in &b.models {
            self.model(m, bin);
            self.o.token("", Space::Line);
        }
    }

    /// `.model name kind (k=v …)`. Xyce rejects BSIM3 as level 8 and reads
    /// 49. The parameter list is separated from the type by a space:
    /// ngspice-45 drops the parameters of an OSDI model written
    /// `vres(r=3k)`.
    fn model(&mut self, n: u32, bin: u32) {
        let m = self.rd.model_view(n, bin);
        let mos = m.kind.eq_ignore_ascii_case("nmos") || m.kind.eq_ignore_ascii_case("pmos");
        let mut line = format!(".model {} {}", m.name, m.kind);
        for (i, (k, v)) in m.params.iter().enumerate() {
            let v = if self.d == Dialect::Xyce
                && mos
                && k.eq_ignore_ascii_case("level")
                && v.trim() == "8"
            {
                "49"
            } else {
                v
            };
            let open = if i == 0 { " (" } else { " " };
            if v.is_empty() {
                line.push_str(&format!("{open}{k}"));
            } else {
                line.push_str(&format!("{open}{k}={}", self.spelled(v)));
            }
        }
        if !m.params.is_empty() {
            line.push(')');
        }
        self.o.token(&line, Space::None);
    }

    fn subckt(&mut self, sc: &Block) {
        self.scope = sc.node;
        let head = self
            .rd
            .subckt_head(sc.node)
            .expect("the reader read the head");
        let mut line = format!(".subckt {}", sc.name);
        for p in &head.ports {
            line.push(' ');
            line.push_str(p);
        }
        let params = !head.params.is_empty() || !sc.params.is_empty();
        if let (true, Some(word)) = (params, self.sp.subckt_params) {
            line.push(' ');
            line.push_str(word);
        }
        for (k, v) in &head.params {
            line.push_str(&format!(" {k}={}", self.spelled(v)));
        }
        // A Spectre or VACASK subcircuit's `parameters` are its defaults.
        for &a in &sc.params {
            let (k, v) = self.rd.param_view(a);
            line.push_str(&format!(" {k}={}", self.spelled(&v)));
        }
        self.o.token(&line, Space::Line);
        self.file_cards(sc);
        let draws = self.draws.locals(sc.node).to_vec();
        self.param_lines(&draws);
        self.param_cards(&sc.locals);
        self.models(sc);
        self.body(sc, self.rd.d == Dialect::Ltspice && !self.in_section);
        self.o.token(&format!(".ends {}", sc.name), Space::Line);
        self.scope = crate::syntax::ast::NONE;
    }

    /// `.lib <name>` … `.endl <name>`, the body written as a deck's top is.
    fn section(&mut self, s: &Section) {
        let name = &s.body.name;
        if self.ask(Construct::LibSection, 0) != Action::Native {
            return;
        }
        self.o.token(&format!(".lib {name}"), Space::Line);
        self.file_cards(&s.body);
        self.funcs(&s.funcs);
        self.params_and_models(&s.body);
        self.in_section = true;
        for sc in &s.defs {
            self.subckt(sc);
        }
        self.in_section = false;
        self.body(&s.body, false);
        self.o.token(&format!(".endl {name}"), Space::Line);
    }

    /// Elements, instances and conditionals in place, then the text kept.
    /// With `rser` (an LTspice deck's top or definition), an inductor
    /// carries LTspice's implicit `Rser=1m` unless a `K` couples it.
    fn body(&mut self, b: &Block, rser: bool) {
        let coupled = match rser && self.rd.d == Dialect::Ltspice {
            true => Some(self.coupled(b)),
            false => None,
        };
        for e in b.elements() {
            match e {
                Elem::Comp(Comp::Card(n)) => self.card(n, coupled.as_deref()),
                Elem::Comp(Comp::Raw(n)) => {
                    let line = self.rd.line(n);
                    self.o.token(&line, Space::None);
                }
                Elem::Inst(n) => self.card(n, None),
                Elem::Cond(k) => {
                    self.conditional(&b.conds[k].1);
                    continue;
                }
            }
            self.o.token("", Space::Line);
        }
        let lines = self.raw_lines(&b.raw);
        self.write_raw(&lines);
    }

    /// Each branch of `.if` … `.endif`: its models, then its elements.
    fn conditional(&mut self, c: &super::read::Cond) {
        let first = c.branches[0].0;
        if self.ask(Construct::Conditional, self.rd.main(first)) != Action::Native {
            return;
        }
        for (i, (b, body)) in c.branches.iter().enumerate() {
            if !body.params.is_empty() || !body.locals.is_empty() {
                self.ask(Construct::BranchParam, self.rd.main(*b));
            }
            let head = match (self.rd.condition(*b), i) {
                (Some(c), 0) => format!(".if ({c})"),
                (Some(c), _) => format!(".elseif ({c})"),
                (None, _) => ".else".to_string(),
            };
            self.o.token(&head, Space::Line);
            self.file_cards(body);
            self.models(body);
            self.body(body, false);
        }
        self.o.token(".endif", Space::Line);
    }

    /// The inductors a `K` card of `b` couples, by name after the letter.
    fn coupled(&self, b: &Block) -> Vec<String> {
        let mut out = Vec::new();
        for &c in &b.comps {
            if let Comp::Card(n) = c
                && let super::read::Card::Coupling { l1, l2, .. } = self.rd.card_view(n)
            {
                out.extend([l1, l2]);
            }
        }
        out
    }

    /// The text of `raw`, one string per line.
    fn raw_lines(&self, raw: &[Raw]) -> Vec<String> {
        let mut out = Vec::new();
        for &r in raw {
            match r {
                Raw::Line(n) => out.push(self.rd.line(n)),
                Raw::Lines(n) => out.extend(self.rd.lines(n)),
                Raw::Joined(n) => out.push(self.rd.lines(n).join("\n")),
            }
        }
        out
    }

    /// Raw lines, one each; with `drop_four`, every `.four` card and its
    /// `+` continuation lines are left out (the deck runs them as
    /// `fourier` commands in its `.control` block instead).
    fn write_raw(&mut self, lines: &[String]) {
        let mut four = false;
        for l in lines {
            if !bench::is_continuation(l) {
                four = self.drop_four && bench::is_four(l);
            }
            if !four {
                self.o.token(l, Space::Line);
            }
        }
    }
}
