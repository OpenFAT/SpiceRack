//! One element → its VACASK instance statement (`r1 (a b) resistor
//! r=1e3`), named by the model its module's `model` line declares, and its
//! source waveform; the refusals for what VACASK has no element or
//! waveform for.
//!
//! Cites: VACASK `share/doc/vacask/{demo,test}/*.sim` (as the old writer,
//! `codegen::vacask::components`, probed them), `docs/input-identifiers.md`.
//!
//! Contents:
//! - `W::vacask_card(&mut self, n: u32) -> Option<String>`

use super::W;
use super::vacask::{num, q};
use crate::syntax::diag::Code;
use crate::syntax::render::capability::Construct;
use crate::syntax::render::read::{Card, Num, Wave, WaveKind};

impl W<'_, '_> {
    /// Element `n`'s statement, with no line end; `None` when it is
    /// refused.
    pub(super) fn vacask_card(&mut self, n: u32) -> Option<String> {
        let tok = self.rd.main(n);
        Some(match self.rd.card_view(n) {
            Card::Rcl {
                letter,
                name,
                n1,
                n2,
                value,
                model,
                params,
            } => {
                if model.is_some() {
                    self.ask(Construct::RclModel, tok);
                    return None;
                }
                let (l, m) = match letter {
                    'R' => ('r', "resistor"),
                    'C' => ('c', "capacitor"),
                    _ => ('l', "inductor"),
                };
                format!(
                    "{l}{} ({} {}) {m} {l}={}{}",
                    q(&name),
                    q(&n1),
                    q(&n2),
                    self.vval(&value),
                    self.inst_params(&params)
                )
            }
            Card::Coupling {
                name, l1, l2, k, ..
            } => format!(
                "k{} () mutual k={} ind1=\"l{}\" ind2=\"l{}\"",
                q(&name),
                num(k),
                q(&l1),
                q(&l2)
            ),
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
                if port.is_some() {
                    self.ask(Construct::RfPort, tok);
                    return None;
                }
                let (l, m) = match letter {
                    'V' => ('v', "vsource"),
                    _ => ('i', "isource"),
                };
                let wave = match &wave {
                    Some(w) => Some(self.vacask_wave(w, tok)?),
                    None => None,
                };
                let mut s = format!(
                    "{l}{} ({} {}) {m} dc={}",
                    q(&name),
                    q(&np),
                    q(&nm),
                    self.vval(&value)
                );
                if let Some(v) = ac_mag {
                    s.push_str(&format!(" mag={}", num(v)));
                }
                if let Some(v) = ac_phase {
                    s.push_str(&format!(" phase={}", num(v)));
                }
                if let Some(w) = wave {
                    s.push(' ');
                    s.push_str(&w);
                }
                s
            }
            Card::Linear {
                letter,
                name,
                nodes,
                gain,
            } => {
                let ns: Vec<String> = nodes.iter().map(|x| q(x)).collect();
                let name = q(&name);
                let g = num(gain);
                match letter {
                    'E' => format!("e{name} ({}) vcvs gain={g}", ns.join(" ")),
                    'G' => format!("g{name} ({}) vccs gain={g}", ns.join(" ")),
                    'F' => format!(
                        "f{name} ({} {}) cccs ctlinst=\"{}\" gain={g}",
                        ns[0], ns[1], ns[2]
                    ),
                    _ => format!(
                        "h{name} ({} {}) ccvs ctlinst=\"{}\" gain={g}",
                        ns[0], ns[1], ns[2]
                    ),
                }
            }
            Card::Semi {
                letter,
                name,
                nodes,
                model,
                params,
            } => {
                if letter == 'M' && nodes.len() != 4 {
                    self.refuse(Code::W0105, tok);
                    return None;
                }
                let ns: Vec<String> = nodes.iter().map(|x| q(x)).collect();
                format!(
                    "{}{} ({}) {}{}",
                    letter.to_ascii_lowercase(),
                    q(&name),
                    ns.join(" "),
                    q(&model),
                    self.inst_params(&params)
                )
            }
            Card::Inst {
                name,
                ports,
                master,
                params,
            } => {
                let ps: Vec<String> = ports.iter().map(|p| q(p)).collect();
                format!(
                    "x{} ({}) {}{}",
                    q(&name),
                    ps.join(" "),
                    q(&master),
                    self.inst_params(&params)
                )
            }
            Card::Charge { .. } => {
                self.ask(Construct::ChargeCapacitor, tok);
                return None;
            }
            Card::Behavioural { .. } => {
                self.ask(Construct::BehaviouralSource, tok);
                return None;
            }
            // No polynomial builtin, no switch, no lossless line.
            Card::Poly { .. } | Card::Switch { .. } | Card::Line { .. } => {
                self.refuse(Code::W0105, tok);
                return None;
            }
        })
    }

    /// `w` as `type="…"` parameters; `None` when it is refused.
    fn vacask_wave(&mut self, w: &Wave, tok: u32) -> Option<String> {
        let a: Vec<String> = w.args.iter().map(|x| self.vacask_num(x)).collect();
        Some(match w.kind {
            WaveKind::Sin => {
                // VACASK accepts a phase and ignores it.
                if w.args[5] != Num::Num(0.0) {
                    self.refuse(Code::W0108, tok);
                    return None;
                }
                format!(
                    "type=\"sine\" sinedc={} ampl={} freq={} delay={} theta={}",
                    a[0], a[1], a[2], a[3], a[4]
                )
            }
            WaveKind::Pulse => {
                // A period no longer than the pulse is no repeat (period 0).
                let nums: Option<Vec<f64>> =
                    [6, 3, 4, 5].iter().map(|&i| w.args[i].num()).collect();
                let period = match nums.as_deref() {
                    Some(&[p, r, f, wd]) if p > 0.0 && p <= r + f + wd => num(0.0),
                    _ => a[6].clone(),
                };
                format!(
                    "type=\"pulse\" val0={} val1={} delay={} rise={} fall={} width={} period={period}",
                    a[0], a[1], a[2], a[3], a[4], a[5]
                )
            }
            WaveKind::Exp => {
                // VACASK counts td2 from delay.
                let (Some(rd), Some(fd)) = (w.args[2].num(), w.args[4].num()) else {
                    self.refuse(Code::W0108, tok);
                    return None;
                };
                if fd < rd {
                    self.refuse(Code::W0108, tok);
                    return None;
                }
                format!(
                    "type=\"exp\" val0={} val1={} delay={} tau1={} td2={} tau2={}",
                    a[0],
                    a[1],
                    a[2],
                    a[3],
                    num(fd - rd),
                    a[5]
                )
            }
            WaveKind::Sffm => format!(
                "type=\"fm\" sinedc={} ampl={} freq={} modindex={} modfreq={}",
                a[0], a[1], a[2], a[3], a[4]
            ),
            WaveKind::Am => format!(
                "type=\"am\" sinedc={} ampl={} freq={} modfreq={} delay={}",
                a[1], a[0], a[3], a[2], a[4]
            ),
            // PWL comes back identically zero in VACASK's transient.
            WaveKind::Pwl => {
                self.refuse(Code::W0108, tok);
                return None;
            }
            WaveKind::Trnoise | WaveKind::Trrandom => {
                self.ask(Construct::NoiseWave, tok);
                return None;
            }
        })
    }
}
