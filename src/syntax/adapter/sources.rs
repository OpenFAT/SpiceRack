//! SPICE V and I cards → `VoltageSource`/`CurrentSource` components with
//! their waveforms (`import/lower/elements.rs`), each card's fields read by
//! `grammar/spice.peg`'s `spec` rules: `dc`, `ac`, a waveform with or
//! without parentheses, ngspice's named fields.
//!
//! Cites: `import/lower/elements.rs` (ported); spice.peg `V`, `I`, `spec`,
//! `Sdc`, `Sac`, `wave`, `wargs`; ngspice.peg `Pair_src`, `Trnoise`,
//! `Trrandom`; ngspice manual §4.1.4-§4.1.11.
//!
//! Contents:
//! - `Cx::source(&self, n: u32, e: Element, name: String) -> Option<Component>`

use super::Cx;
use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::device::Primitive as Element;

/// A waveform's kind, by its keyword.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wave {
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
    Wave(Wave, Vec<u32>),
    /// ngspice `r=`, `td=`, `portnum n`, `z0 x`: key and value.
    Pair(String, u32),
}

impl<'a> Cx<'a> {
    /// A V or I card (spice.peg `V`, `spec`; lower/elements.rs:133-219).
    pub(super) fn source(&self, n: u32, e: Element, name: String) -> Option<Component> {
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
            (IrValue::Numeric { value: 0.0 }, false, None, None, None);
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
            let Some(IrWaveform::Pwl {
                repeat: r,
                delay: d,
                ..
            }) = &mut wf
            else {
                return None;
            };
            (*r, *d) = (repeat, delay);
        }
        let port = match (portnum, z0) {
            (Some(num), z0) => Some(RfPort {
                num,
                z0: z0.unwrap_or(50.0),
            }),
            (None, None) => None,
            (None, Some(_)) => return None,
        };
        Some(match e {
            Element::VoltageSource => Component::VoltageSource {
                name,
                np,
                nm,
                value,
                ac_magnitude,
                ac_phase,
                waveform: wf,
                port,
            },
            _ if port.is_some() => return None,
            _ => Component::CurrentSource {
                name,
                np,
                nm,
                value,
                ac_magnitude,
                ac_phase,
                waveform: wf,
            },
        })
    }
}

// ---- private ----

impl<'a> Cx<'a> {
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
    fn wave(&self, w: u32) -> Option<Wave> {
        let ngspice = self.d == Dialect::Ngspice;
        let t = self.word_text(w)?.to_ascii_lowercase();
        Some(match t.as_str() {
            "pulse" => Wave::Pulse,
            "sin" | "sine" => Wave::Sin,
            "pwl" => Wave::Pwl,
            "exp" => Wave::Exp,
            "sffm" => Wave::Sffm,
            "am" => Wave::Am,
            "trnoise" if ngspice => Wave::Trnoise,
            "trrandom" if ngspice => Wave::Trrandom,
            _ => return None,
        })
    }

    /// A waveform's values → its IR form (lower/elements.rs `waveform`).
    fn waveform(&self, w: Wave, vals: &[u32]) -> Option<IrWaveform> {
        let a: Vec<IrNum> = vals.iter().map(|&v| self.irnum(v)).collect::<Option<_>>()?;
        let g = |i: usize| a.get(i).cloned().unwrap_or(IrNum::Num(0.0));
        if matches!(w, Wave::Sffm | Wave::Am)
            && a[a.len().min(5)..].iter().any(|x| *x != IrNum::Num(0.0))
        {
            return None;
        }
        Some(match w {
            Wave::Sin => IrWaveform::Sin {
                offset: g(0),
                amplitude: g(1),
                frequency: g(2),
                delay: g(3),
                damping: g(4),
                phase: g(5),
            },
            Wave::Pulse => IrWaveform::Pulse {
                initial: g(0),
                pulsed: g(1),
                delay: g(2),
                rise_time: g(3),
                fall_time: g(4),
                pulse_width: g(5),
                period: g(6),
            },
            Wave::Pwl if a.len().is_multiple_of(2) => IrWaveform::Pwl {
                values: a.chunks(2).map(|p| (p[0].clone(), p[1].clone())).collect(),
                repeat: None,
                delay: None,
            },
            Wave::Pwl => return None,
            Wave::Exp => IrWaveform::Exp {
                initial: g(0),
                pulsed: g(1),
                rise_delay: g(2),
                rise_tau: g(3),
                fall_delay: g(4),
                fall_tau: g(5),
            },
            Wave::Sffm => IrWaveform::Sffm {
                offset: g(0),
                amplitude: g(1),
                carrier_freq: g(2),
                modulation_index: g(3),
                signal_freq: g(4),
            },
            Wave::Am => IrWaveform::Am {
                amplitude: g(0),
                offset: g(1),
                modulating_freq: g(2),
                carrier_freq: g(3),
                delay: g(4),
            },
            Wave::Trnoise => IrWaveform::TrNoise {
                na: g(0),
                nt: g(1),
                nalpha: g(2),
                namp: g(3),
                rtsam: g(4),
                rtscapt: g(5),
                rtsemt: g(6),
            },
            Wave::Trrandom => {
                let kind = a
                    .first()?
                    .num()
                    .filter(|t| t.fract() == 0.0 && (1.0..=4.0).contains(t))?
                    as u8;
                let p1 = a.get(3).cloned().unwrap_or(IrNum::Num(1.0));
                IrWaveform::TrRandom {
                    kind: RandomKind::try_from(kind).ok()?,
                    ts: a.get(1)?.clone(),
                    td: g(2),
                    p1,
                    p2: g(4),
                }
            }
        })
    }
}
