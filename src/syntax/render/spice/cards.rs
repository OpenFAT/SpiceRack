//! One element card → its SPICE-family line: the element letter from the
//! target's device table, the nodes and values as the reader kept them,
//! numbers in the target's number format, expressions in its delimiters,
//! the waveform, and the `Spell`/`Lift`/`Refuse` answers for charge
//! capacitors, behavioural sources, RF ports, PWL repeats, noise waveforms
//! and Laplace sources.
//!
//! Cites: ngspice manual §3.3.9 (`C1 a b q='…'`), §4.1 (waveforms),
//! §4.1.11 (`portnum`), §5.1-§5.5 (controlled sources), ch.7-ch.11
//! (semiconductors), §2.4.3 (`X`); Xyce RG p.238 (`B … V={…}`); HSPICE
//! `vol=`/`cur=`; LTspice help (`Q=`).
//!
//! Contents:
//! - `W::card(&mut self, n: u32)`, `W::spelled(&self, v: &str) -> String`

use super::W;
use crate::codegen::Dialect;
use crate::codegen::laplace;
use crate::syntax::render::capability::{Action, Alt, Construct, Rule};
use crate::syntax::render::out::Space;
use crate::syntax::render::read::WaveKind;
use crate::syntax::render::read::{Card, Num, Val, Wave};

impl W<'_, '_> {
    /// Element `n`'s card, with no line end (a Laplace source is two
    /// lines). With `coupled` (an LTspice deck's top or definition, and the
    /// inductors its `K` cards couple), an uncoupled inductor with no
    /// `Rser` carries LTspice's implicit `Rser=1m`.
    pub(super) fn card(&mut self, n: u32, coupled: Option<&[String]>) {
        let tok = self.rd.main(n);
        let line = match self.rd.card_view(n) {
            Card::Rcl {
                letter,
                name,
                n1,
                n2,
                value,
                model,
                mut params,
            } => {
                if letter == 'L'
                    && let Some(c) = coupled
                    && !c.contains(&name)
                    && !params.iter().any(|(k, _)| k.eq_ignore_ascii_case("rser"))
                {
                    params.push(("Rser".into(), "1m".into()));
                }
                if model.is_some() {
                    self.ask(Construct::RclModel, tok);
                }
                let mut s = format!("{letter}{name} {n1} {n2}");
                match &value {
                    Val::Raw(t) if t.is_empty() => {}
                    v => s.push_str(&format!(" {}", self.value(v))),
                }
                if let Some(m) = model {
                    s.push_str(&format!(" {m}"));
                }
                s + &self.params(&params)
            }
            Card::Charge {
                name,
                n1,
                n2,
                charge,
                params,
            } => {
                let q = match self.ask(Construct::ChargeCapacitor, tok) {
                    Action::Spell(Alt::ChargeBraced) => format!("Q={{{charge}}}"),
                    Action::Spell(Alt::ChargeBare) => format!("Q={charge}"),
                    _ => format!("Q='{charge}'"),
                };
                format!("C{name} {n1} {n2} {q}{}", self.params(&params))
            }
            Card::Coupling { name, l1, l2, k } => {
                format!("K{name} L{l1} L{l2} {}", self.sp.decimal(k, self.x))
            }
            Card::Source {
                letter,
                name,
                np,
                nm,
                value,
                ac_mag,
                ac_phase,
                wave,
                port,
            } => {
                if let Some(w) = &wave {
                    self.wave_ok(letter, w, tok);
                }
                if port.is_some() {
                    self.ask(Construct::RfPort, tok);
                }
                let mut s = format!("{letter}{name} {np} {nm} {}", self.value(&value));
                if let Some(mag) = ac_mag {
                    s.push_str(&format!(" AC {}", self.sp.decimal(mag, self.x)));
                    if let Some(phase) = ac_phase {
                        s.push_str(&format!(" {}", self.sp.decimal(phase, self.x)));
                    }
                }
                if let Some((num, z0)) = port {
                    s.push_str(&format!(
                        " portnum {num} z0 {}",
                        self.sp.decimal(z0, self.x)
                    ));
                }
                if let Some(w) = &wave {
                    s.push(' ');
                    s.push_str(&self.wave(w));
                }
                s
            }
            Card::Behavioural {
                current,
                name,
                np,
                nm,
                expr,
            } => {
                if laplace::is_laplace(&expr) {
                    self.laplace(&name, &np, &nm, &expr, current, tok)
                } else {
                    let key = if current { 'I' } else { 'V' };
                    match self.ask(Construct::BehaviouralSource, tok) {
                        Action::Spell(Alt::VolCur) if current => {
                            format!("G{name} {np} {nm} cur='{expr}'")
                        }
                        Action::Spell(Alt::VolCur) => format!("E{name} {np} {nm} vol='{expr}'"),
                        Action::Spell(Alt::BracedBehaviour) => {
                            format!("B{name} {np} {nm} {key}={{{expr}}}")
                        }
                        _ => format!("B{name} {np} {nm} {key}={expr}"),
                    }
                }
            }
            Card::Linear {
                letter,
                name,
                nodes,
                gain,
            } => format!(
                "{letter}{name} {} {}",
                nodes.join(" "),
                self.sp.decimal(gain, self.x)
            ),
            Card::Poly {
                letter,
                name,
                np,
                nm,
                controls,
                coeffs,
            } => {
                let dims = if matches!(letter, 'E' | 'G') {
                    controls.len() / 2
                } else {
                    controls.len()
                };
                let coeffs: Vec<String> =
                    coeffs.iter().map(|&c| self.sp.number(c, self.x)).collect();
                format!(
                    "{letter}{name} {np} {nm} POLY({dims}) {} {}",
                    controls.join(" "),
                    coeffs.join(" ")
                )
            }
            Card::Semi {
                letter,
                name,
                nodes,
                model,
                params,
            } => format!(
                "{letter}{name} {} {model}{}",
                nodes.join(" "),
                self.params(&params)
            ),
            Card::Switch {
                letter,
                name,
                nodes,
                model,
            } => format!("{letter}{name} {} {model}", nodes.join(" ")),
            Card::Line {
                name,
                nodes,
                z0,
                td,
            } => format!(
                "T{name} {} Z0={} TD={}",
                nodes.join(" "),
                self.sp.decimal(z0, self.x),
                self.sp.decimal(td, self.x)
            ),
            Card::Inst {
                name,
                ports,
                master,
                params,
            } => {
                let mut s = format!("X{name}");
                for p in &ports {
                    s.push(' ');
                    s.push_str(p);
                }
                format!("{s} {master}{}", self.params(&params))
            }
        };
        self.o.token(&line, Space::None);
    }

    /// A value kept as SPICE text, as the target reads it: as written, but
    /// for HSPICE, which reads braces as name bytes and an unquoted value
    /// as one word starting with a letter, or a number: any other
    /// expression is quoted (`{a*2}` is `'a*2'`, `{a} + b` is `'(a) + b'`);
    /// a run of numbers (a matrix) is not.
    pub(super) fn spelled(&self, v: &str) -> String {
        if self.d == Dialect::Hspice {
            hspice_value(v)
        } else {
            v.to_string()
        }
    }
}

// ---- private ----

impl W<'_, '_> {
    /// ` k=v` each (a flag bare), values [`W::spelled`].
    fn params(&self, params: &[(String, String)]) -> String {
        let mut s = String::new();
        for (k, v) in params {
            if v.is_empty() {
                s.push_str(&format!(" {k}"));
            } else {
                s.push_str(&format!(" {k}={}", self.spelled(v)));
            }
        }
        s
    }

    /// A value field: a number in the target's format, an expression in its
    /// delimiters, raw text as written.
    fn value(&self, v: &Val) -> String {
        match v {
            Val::Num(x) => self.sp.number(*x, self.x),
            Val::Expr(e) => format!("{}{e}{}", self.sp.expr.0, self.sp.expr.1),
            Val::Raw(t) => t.clone(),
        }
    }

    /// A waveform value: a number as plain decimal, an expression
    /// delimited.
    fn wave_num(&self, n: &Num) -> String {
        match n {
            Num::Num(v) => self.sp.decimal(*v, self.x),
            Num::Expr(e) => self.value(&Val::Expr(e.clone())),
        }
    }

    /// Records a refusal for a waveform the target lacks.
    fn wave_ok(&mut self, letter: char, w: &Wave, tok: u32) {
        match w.kind {
            WaveKind::Pwl if w.repeat.is_some() || w.delay.is_some() => {
                let c = match letter {
                    'V' => Construct::PwlRepeatV,
                    _ => Construct::PwlRepeatI,
                };
                self.ask(c, tok);
            }
            WaveKind::Trnoise | WaveKind::Trrandom => {
                self.ask(Construct::NoiseWave, tok);
            }
            _ => {}
        }
    }

    /// `w`'s spelling: `KIND(v …)`, PWL's `r=`/`td=` after it.
    fn wave(&self, w: &Wave) -> String {
        let word = match w.kind {
            WaveKind::Sin => "SIN",
            WaveKind::Pulse => "PULSE",
            WaveKind::Pwl => "PWL",
            WaveKind::Exp => "EXP",
            WaveKind::Sffm => "SFFM",
            WaveKind::Am => "AM",
            WaveKind::Trnoise => "TRNOISE",
            WaveKind::Trrandom => "TRRANDOM",
        };
        let mut args: Vec<String> = w.args.iter().map(|a| self.wave_num(a)).collect();
        match w.kind {
            // The RTS fields are left off when all zero, as the manual's
            // white and 1/f examples write the card.
            WaveKind::Trnoise if w.args[4..].iter().all(|a| a.num() == Some(0.0)) => {
                args.truncate(4)
            }
            WaveKind::Trrandom => {
                let kind = w.args[0].num().expect("a TRRANDOM type");
                args[0] = format!("{}", kind as u8);
            }
            _ => {}
        }
        let mut s = format!("{word}({})", args.join(" "));
        if let Some(r) = &w.repeat {
            s.push_str(&format!(" r={}", self.wave_num(r)));
        }
        if let Some(td) = &w.delay {
            s.push_str(&format!(" td={}", self.wave_num(td)));
        }
        s
    }

    /// A `Laplace(...)` behavioural source, rewritten into a card the
    /// target parses (`Lift`): ngspice's XSPICE `s_xfer` (instance and
    /// `.model`), HSPICE's and ESPice's `LAPLACE` table, LTspice's
    /// `Laplace=`; a constant `H(s)` is a plain controlled source anywhere.
    fn laplace(
        &mut self,
        name: &str,
        np: &str,
        nm: &str,
        expr: &str,
        current: bool,
        tok: u32,
    ) -> String {
        let letter = if current { 'G' } else { 'E' };
        let parsed = match laplace::parse(expr) {
            Ok(p) => p,
            Err(_) => {
                self.refuse(crate::syntax::diag::Code::W0010, tok);
                return String::new();
            }
        };
        let (ip, im) = parsed.input().clone();
        let (num, den) = match parsed {
            laplace::Laplace::Gain { gain, .. } => {
                return format!(
                    "{letter}B{name} {np} {nm} {ip} {im} {}",
                    self.sp.number(gain, self.x)
                );
            }
            laplace::Laplace::Xfer { num, den, .. } => (num, den),
        };
        match self.ask(Construct::Laplace, tok) {
            Action::Lift(Rule::LaplaceAttr) => format!(
                "{letter}B{name} {np} {nm} {ip} {im} Laplace={}/{}",
                laplace::horner(&num),
                laplace::horner(&den),
            ),
            Action::Lift(Rule::LaplaceTable) => {
                let asc =
                    |c: &[f64]| laplace::coeff_array(&c.iter().rev().copied().collect::<Vec<_>>());
                format!(
                    "{letter}B{name} {np} {nm} LAPLACE {ip} {im} {} / {}",
                    asc(&num),
                    asc(&den)
                )
            }
            Action::Lift(Rule::LaplaceXfer) => {
                let port = if current { "id" } else { "vd" };
                // One integrator stage per denominator order.
                let ic = vec!["0"; den.len() - 1].join(" ");
                format!(
                    "AB{name} %vd({ip} {im}) %{port}({np} {nm}) B{name}_xfer\n\
                     .model B{name}_xfer s_xfer(num_coeff=[{}] den_coeff=[{}] int_ic=[{ic}])",
                    laplace::coeff_array(&num),
                    laplace::coeff_array(&den),
                )
            }
            _ => String::new(),
        }
    }
}

/// [`W::spelled`] for HSPICE.
fn hspice_value(v: &str) -> String {
    let t = v.trim();
    let number = |s: &str| {
        let m = s.strip_prefix(['-', '+']).unwrap_or(s);
        crate::unit::parse_spice_number(m)
            .is_some_and(|(_, end, _)| m[end..].bytes().all(|c| c.is_ascii_alphabetic()))
    };
    let word = |s: &str| s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
    let quoted = (t.starts_with('\'') || t.starts_with('"')) && t.len() > 1;
    // Words split at blanks outside parentheses (`unif(1k, 0.1)` is one); a
    // run of numbers is a vector (a CPL model's matrix), not an expression.
    let mut depth = 0i32;
    let words: Vec<&str> = t
        .split(|c: char| {
            depth += i32::from(c == '(') - i32::from(c == ')');
            depth == 0 && c.is_whitespace()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let plain = match words[..] {
        [w] => number(w) || word(w),
        _ => words.iter().all(|w| number(w)),
    };
    if !t.contains('{') && (quoted || plain) {
        return v.to_string();
    }
    let t = t
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
        .unwrap_or(t);
    let mut depth = 0i32;
    let closes = t.char_indices().find(|&(_, c)| {
        depth += i32::from(c == '{') - i32::from(c == '}');
        depth == 0
    });
    let single = t.starts_with('{') && closes.is_some_and(|(i, _)| i + 1 == t.len());
    let e = if single { &t[1..t.len() - 1] } else { t };
    format!("'{}'", e.trim().replace('{', "(").replace('}', ")"))
}
