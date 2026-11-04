//! SPICE V and I cards → [`Card::Source`] views with their [`Wave`]s (as
//! the old lowering typed them), each card's fields read by
//! the old SPICE grammar's `spec` rules: `dc`, `ac`, a waveform with or
//! without parentheses, ngspice's named fields.
//!
//! Cites: the old lowering; the old SPICE grammar's `V`, `I`, `spec`,
//! `Sdc`, `Sac`, `wave`, `wargs`; the old ngspice grammar's `Pair_src`, `Trnoise`,
//! `Trrandom`; ngspice manual §4.1.4-§4.1.11.
//!
//! Contents:
//! - `pub(in crate::syntax::render) enum Num`, `struct Wave`, `enum WaveKind`
//! - `Num::num(&self) -> Option<f64>`, `Wave::at_zero(&self) -> Val`
//! - `Rd::source(&self, n: u32, e: Element, name: String) -> Option<Card>`

use super::{Card, Rd, Val};
use crate::syntax::Dialect;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::device::Primitive as Element;

/// A waveform value: a number or an expression (without delimiters).
#[derive(Clone, PartialEq, Debug)]
pub(in crate::syntax::render) enum Num {
    Num(f64),
    Expr(String),
}

/// A source waveform: its kind and its values in the card's order, each
/// past the card's last read as the kind's default (`0`; TRRANDOM's `p1`
/// is `1`).
#[derive(Clone, PartialEq, Debug)]
pub(in crate::syntax::render) struct Wave {
    pub(in crate::syntax::render) kind: WaveKind,
    /// The values: SIN 6, PULSE 7, EXP 6, SFFM 5, AM 5, TRNOISE 7,
    /// TRRANDOM 5 (the type first), PWL its pairs flat.
    pub(in crate::syntax::render) args: Vec<Num>,
    /// PWL `r=`, `td=`.
    pub(in crate::syntax::render) repeat: Option<Num>,
    pub(in crate::syntax::render) delay: Option<Num>,
    /// The statement gave more values than `args` holds (ngspice's PULSE
    /// count NP): the rest are not written.
    pub(in crate::syntax::render) cut: bool,
}

/// A waveform's kind, by its keyword.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::syntax::render) enum WaveKind {
    Pulse,
    Sin,
    Pwl,
    Exp,
    Sffm,
    Am,
    Trnoise,
    Trrandom,
}

/// One `spec` of a V or I card, the items it holds.
enum Spec {
    /// `dc [v]`.
    Dc(Option<u32>),
    /// `ac [mag [phase]]`.
    Ac(Vec<u32>),
    /// A waveform and its values.
    Wave(WaveKind, Vec<u32>),
    /// ngspice `r=`, `td=`, `portnum n`, `z0 x`: key and value.
    Pair(String, u32),
}

impl<'a> Rd<'a> {
    /// A V or I card (the old SPICE grammar's `V`, `spec`; the old lowering).
    pub(in crate::syntax::render) fn source(
        &self,
        n: u32,
        e: Element,
        name: String,
    ) -> Option<Card> {
        let items = &self.items(n)[..];
        if items.len() < 2 || !self.is_word(items[0]) || !self.is_word(items[1]) {
            return None;
        }
        let (np, nm) = (self.text(items[0]), self.text(items[1]));
        let mut j = 2;
        let mut lead = None;
        if let Some(&v) = items.get(2)
            && self.spec(items, 2).is_none()
            && self.tag(v) != Tag::Assign
        {
            if !self.val_ok(v) {
                return None;
            }
            lead = Some(v);
            j = 3;
        }
        let mut specs = Vec::new();
        while j < items.len() {
            let (s, next) = self.spec(items, j)?;
            specs.push(s);
            j = next;
        }
        let (mut value, mut dc_given, mut ac_magnitude, mut ac_phase, mut wf) =
            (Val::Num(0.0), false, None, None, None);
        let (mut repeat, mut delay, mut portnum, mut z0) = (None, None, None, None);
        if let Some(v) = lead {
            (value, dc_given) = (self.value(v), true);
        }
        for s in specs {
            match s {
                Spec::Pair(key, val) => {
                    let v = self.irnum(val)?;
                    match key.as_str() {
                        "r" => repeat = Some(v),
                        "td" => delay = Some(v),
                        "portnum" => portnum = Some(self.count(val)?),
                        "z0" => z0 = Some(v.num()?),
                        _ => return None,
                    }
                }
                Spec::Dc(v) => {
                    if let Some(v) = v {
                        (value, dc_given) = (self.value(v), true);
                    }
                }
                Spec::Ac(nums) => {
                    ac_magnitude = Some(nums.first().and_then(|&r| self.number(r)).unwrap_or(1.0));
                    if let Some(&p) = nums.get(1) {
                        ac_phase = Some(self.number(p)?);
                    }
                }
                Spec::Wave(w, vals) => wf = Some(self.waveform(w, &vals)?),
            }
        }
        if !dc_given && let Some(w) = &wf {
            value = w.at_zero();
        }
        if repeat.is_some() || delay.is_some() {
            let Some(w) = &mut wf else { return None };
            if w.kind != WaveKind::Pwl {
                return None;
            }
            (w.repeat, w.delay) = (repeat, delay);
        }
        let port = match (portnum, z0) {
            (Some(num), z0) => Some((num, z0.unwrap_or(50.0))),
            (None, None) => None,
            (None, Some(_)) => return None,
        };
        if e != Element::VoltageSource && port.is_some() {
            return None;
        }
        Some(Card::Source {
            letter: if e == Element::VoltageSource {
                'V'
            } else {
                'I'
            },
            name,
            np,
            nm,
            value,
            ac_mag: ac_magnitude,
            ac_phase,
            wave: wf,
            port,
        })
    }
}

impl Num {
    /// The number, when this is one.
    pub(in crate::syntax::render) fn num(&self) -> Option<f64> {
        match self {
            Num::Num(v) => Some(*v),
            Num::Expr(_) => None,
        }
    }
}

impl Wave {
    /// The source's value at time zero when the card gives no DC value.
    pub(in crate::syntax::render) fn at_zero(&self) -> Val {
        let v = |n: &Num| match n {
            Num::Num(v) => Val::Num(*v),
            Num::Expr(e) => Val::Expr(e.clone()),
        };
        match self.kind {
            WaveKind::Sin | WaveKind::Sffm | WaveKind::Pulse | WaveKind::Exp => v(&self.args[0]),
            WaveKind::Pwl => self.args.get(1).map_or(Val::Num(0.0), v),
            // TRNOISE and TRRANDOM add to the card's DC value instead of
            // replacing it, so a card with none has DC 0.
            WaveKind::Am | WaveKind::Trnoise | WaveKind::Trrandom => Val::Num(0.0),
        }
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// The `spec` starting at item `j`, and the item after it.
    fn spec(&self, items: &[u32], j: usize) -> Option<(Spec, usize)> {
        let it = items[j];
        let next = items.get(j + 1).copied();
        let ngspice = self.d == Dialect::Ngspice;
        let numword = |k: u32| self.number(k).is_some();
        if self.word_is(it, "dc") {
            let v = next.filter(|&v| {
                self.tag(v) != Tag::Assign && self.spec(items, j + 1).is_none() && self.val_ok(v)
            });
            return Some((Spec::Dc(v), j + 1 + usize::from(v.is_some())));
        }
        if self.tag(it) == Tag::Assign && self.key(it).eq_ignore_ascii_case("dc") {
            let v = self.ast.rhs(Node(it));
            return self.val_ok(v).then_some((Spec::Dc(Some(v)), j + 1));
        }
        if self.word_is(it, "ac") {
            let mut nums = Vec::new();
            let mut k = j + 1;
            while nums.len() < 2 && items.get(k).is_some_and(|&x| numword(x)) {
                nums.push(items[k]);
                k += 1;
            }
            return Some((Spec::Ac(nums), k));
        }
        if self.tag(it) == Tag::Assign && self.key(it).eq_ignore_ascii_case("ac") {
            let v = self.ast.rhs(Node(it));
            if v == NONE || !numword(v) {
                return None;
            }
            let mut nums = vec![v];
            if let Some(x) = next.filter(|&x| numword(x)) {
                nums.push(x);
                return Some((Spec::Ac(nums), j + 2));
            }
            return Some((Spec::Ac(nums), j + 1));
        }
        let wave_name = match self.tag(it) {
            Tag::Word => Some(it),
            Tag::Group => Some(self.ast.lhs(Node(it))).filter(|&w| w != NONE),
            _ => None,
        };
        if let Some(w) = wave_name.and_then(|w| self.wave(w)) {
            if self.tag(it) == Tag::Group {
                if !self.group_closed(it) {
                    return None;
                }
                let vals = self.group_items(it).to_vec();
                if !vals.iter().all(|&v| self.is_word(v) || self.expr_ok(v)) {
                    return None;
                }
                return Some((Spec::Wave(w, vals), j + 1));
            }
            let mut vals = Vec::new();
            let mut k = j + 1;
            while let Some(&x) = items.get(k)
                && self.is_word(x)
                && self
                    .text(x)
                    .starts_with(|c: char| c.is_ascii_digit() || "+-.".contains(c))
            {
                vals.push(x);
                k += 1;
            }
            return Some((Spec::Wave(w, vals), k));
        }
        if ngspice {
            let keys = ["portnum", "z0", "td", "r"];
            if self.tag(it) == Tag::Assign {
                let key = self.key(it).to_ascii_lowercase();
                let v = self.ast.rhs(Node(it));
                if keys.contains(&key.as_str())
                    && self.is_word(self.ast.lhs(Node(it)))
                    && self.val_ok(v)
                {
                    return Some((Spec::Pair(key, v), j + 1));
                }
            }
            if let Some(k) = keys.iter().find(|k| self.word_is(it, k))
                && let Some(v) = next.filter(|&v| self.tag(v) != Tag::Assign && self.val_ok(v))
            {
                return Some((Spec::Pair(k.to_string(), v), j + 2));
            }
        }
        None
    }

    /// The waveform a word names.
    fn wave(&self, w: u32) -> Option<WaveKind> {
        let ngspice = self.d == Dialect::Ngspice;
        let t = self.word_text(w)?.to_ascii_lowercase();
        Some(match t.as_str() {
            "pulse" => WaveKind::Pulse,
            "sin" | "sine" => WaveKind::Sin,
            "pwl" => WaveKind::Pwl,
            "exp" => WaveKind::Exp,
            "sffm" => WaveKind::Sffm,
            "am" => WaveKind::Am,
            "trnoise" if ngspice => WaveKind::Trnoise,
            "trrandom" if ngspice => WaveKind::Trrandom,
            _ => return None,
        })
    }

    /// A waveform's values → its view (the old lowering's `waveform`).
    fn waveform(&self, w: WaveKind, vals: &[u32]) -> Option<Wave> {
        let a: Vec<Num> = vals.iter().map(|&v| self.irnum(v)).collect::<Option<_>>()?;
        if matches!(w, WaveKind::Sffm | WaveKind::Am)
            && a[a.len().min(5)..].iter().any(|x| *x != Num::Num(0.0))
        {
            return None;
        }
        let padded = |k: usize| -> Vec<Num> {
            (0..k)
                .map(|i| a.get(i).cloned().unwrap_or(Num::Num(0.0)))
                .collect()
        };
        let held = match w {
            WaveKind::Sin | WaveKind::Exp => 6,
            WaveKind::Pulse | WaveKind::Trnoise => 7,
            WaveKind::Sffm | WaveKind::Am | WaveKind::Trrandom => 5,
            WaveKind::Pwl => a.len(),
        };
        let args = match w {
            WaveKind::Sin | WaveKind::Exp => padded(6),
            WaveKind::Pulse | WaveKind::Trnoise => padded(7),
            WaveKind::Sffm | WaveKind::Am => padded(5),
            WaveKind::Pwl if a.len().is_multiple_of(2) => a.clone(),
            WaveKind::Pwl => return None,
            WaveKind::Trrandom => {
                let kind = a
                    .first()?
                    .num()
                    .filter(|t| t.fract() == 0.0 && (1.0..=4.0).contains(t))?;
                let mut args = padded(5);
                args[0] = Num::Num(kind);
                args[1] = a.get(1)?.clone();
                args[3] = a.get(3).cloned().unwrap_or(Num::Num(1.0));
                args
            }
        };
        Some(Wave {
            kind: w,
            args,
            repeat: None,
            delay: None,
            cut: a.len() > held,
        })
    }
}
