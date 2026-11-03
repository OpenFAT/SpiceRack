//! SPICE element cards -> `Component`s and `Instance`s, with the source
//! waveforms they carry; `None` when the IR has no faithful form.
//!
//! Cites: ngspice manual §4.1.4 (PWL `r=`/`td=`), §4.1.7, §4.1.8 (TRNOISE,
//! TRRANDOM), §4.1.11 (RF port `portnum`/`z0`).
//!
//! Contents:
//! - `Cx::instance(&self, card: usize) -> Option<Instance>`
//! - `Cx::component(&self, card: usize) -> Option<Component>`

use super::Cx;
use crate::import::Tag;
use crate::ir::*;

impl Cx<'_> {
    pub(super) fn instance(&self, card: usize) -> Option<Instance> {
        let name = self.text(self.kid(card, Tag::Name)?).to_string();
        let subcircuit = self.text(self.kid(card, Tag::Mname)?).to_string();
        Some(Instance {
            name,
            subcircuit,
            port_mapping: self.texts(card, Tag::Node),
            parameters: self.params(card),
            at: None,
        })
    }

    /// One element card, or `None` when the IR has no faithful form for it.
    pub(super) fn component(&self, card: usize) -> Option<Component> {
        let tag = self.tag(card);
        let k = self.kids(card);
        let name = self.text(k[0]).to_string();
        let nodes = self.texts(card, Tag::Node);
        let node = |i: usize| nodes.get(i).cloned();
        let model = self.kid(card, Tag::Mname).map(|m| self.text(m).to_string());
        Some(match tag {
            Tag::R | Tag::C | Tag::L => {
                // A flag before the parameters is a second positional field
                // the IR's R/C/L has no place for; after them it is a flag.
                let first_pair = self
                    .kids(card)
                    .into_iter()
                    .position(|x| self.tag(x) == Tag::Pair);
                if self
                    .kids(card)
                    .into_iter()
                    .position(|x| self.tag(x) == Tag::Flag)
                    .is_some_and(|f| first_pair.is_none_or(|p| f < p))
                {
                    return None;
                }
                let mut params = self.params(card);
                let mut model = None;
                let value = match self.kid(card, Tag::Val) {
                    // `R1 a b rmod l=10u`: a bare word naming a `.model`, not a
                    // value; the value field is left blank.
                    Some(v)
                        if self
                            .kids(v)
                            .iter()
                            .all(|&w| self.tag(w) == Tag::Word && self.number(w).is_none())
                            && self.models.contains(&self.text(v).to_ascii_lowercase()) =>
                    {
                        model = Some(self.text(v).to_string());
                        IrValue::Raw {
                            text: String::new(),
                        }
                    }
                    Some(v) => self.value(v),
                    // ngspice `R1 a b r=1k`
                    None => {
                        let key = format!("{tag:?}").to_ascii_lowercase();
                        let i = params
                            .iter()
                            .position(|(k, _)| k.eq_ignore_ascii_case(&key))?;
                        let pair = self.all(card, Tag::Pair)[i];
                        params.remove(i);
                        self.value(self.kids(pair)[1])
                    }
                };
                let (n1, n2) = (node(0)?, node(1)?);
                match tag {
                    Tag::R => Component::Resistor {
                        name,
                        n1,
                        n2,
                        value,
                        model,
                        params,
                    },
                    Tag::C => Component::Capacitor {
                        name,
                        n1,
                        n2,
                        value,
                        model,
                        params,
                    },
                    _ => Component::Inductor {
                        name,
                        n1,
                        n2,
                        value,
                        model,
                        params,
                    },
                }
            }
            Tag::K => Component::MutualInductor {
                name,
                inductor1: node(0)?.get(1..)?.to_string(),
                inductor2: node(1)?.get(1..)?.to_string(),
                coupling: self.number(self.kid(card, Tag::Num)?)?,
            },
            Tag::V | Tag::I => {
                let (np, nm) = (node(0)?, node(1)?);
                let (mut value, mut dc_given, mut ac_magnitude, mut ac_phase, mut wf) =
                    (IrValue::Numeric { value: 0.0 }, false, None, None, None);
                // ngspice's named fields (§4.1.4, §4.1.11), applied once the
                // whole card is read: they may come before the PWL.
                let (mut repeat, mut delay, mut portnum, mut z0) = (None, None, None, None);
                for &f in &k[3..] {
                    match self.tag(f) {
                        Tag::Pair => {
                            let [key, val] = self.kids(f)[..] else {
                                return None;
                            };
                            let v = self.irnum(val)?;
                            match self.text(key).to_ascii_lowercase().as_str() {
                                "r" => repeat = Some(v),
                                "td" => delay = Some(v),
                                "portnum" => portnum = Some(self.count(val)?),
                                "z0" => z0 = Some(v.num()?),
                                _ => return None,
                            }
                        }
                        Tag::Val => (value, dc_given) = (self.value(f), true),
                        Tag::Sdc => {
                            if let Some(v) = self.kid(f, Tag::Val) {
                                (value, dc_given) = (self.value(v), true);
                            }
                        }
                        Tag::Sac => {
                            let n = self.all(f, Tag::Num);
                            ac_magnitude =
                                Some(n.first().and_then(|&r| self.num(r)).unwrap_or(1.0));
                            if let Some(&p) = n.get(1) {
                                ac_phase = Some(self.num(p)?);
                            }
                        }
                        _ => wf = Some(self.waveform(f)?),
                    }
                }
                // No DC value: SPICE takes the waveform's t=0 value for the
                // operating point, so an explicit 0 would move it.
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
                // A z0 with no port, or a port on an I source, has no IR field.
                let port = match (portnum, z0) {
                    (Some(num), z0) => Some(RfPort {
                        num,
                        z0: z0.unwrap_or(50.0),
                    }),
                    (None, None) => None,
                    (None, Some(_)) => return None,
                };
                match tag {
                    Tag::V => Component::VoltageSource {
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
                }
            }
            Tag::Poly => {
                let letter = self.text(card).chars().next()?.to_ascii_uppercase();
                let dims = self.count(self.kid(card, Tag::Num)?)? as usize;
                let n_controls = if matches!(letter, 'E' | 'G') {
                    2 * dims
                } else {
                    dims
                };
                let rest: Vec<usize> = self.all(card, Tag::Word);
                let controls = rest
                    .get(..n_controls)?
                    .iter()
                    .map(|&w| self.text(w).to_string())
                    .collect();
                let coeffs = rest[n_controls..]
                    .iter()
                    .map(|&w| self.number(w))
                    .collect::<Option<_>>()?;
                Component::Poly {
                    name,
                    letter,
                    np: node(0)?,
                    nm: node(1)?,
                    controls,
                    coeffs,
                }
            }
            Tag::E | Tag::G => {
                let (np, nm) = (node(0)?, node(1)?);
                // `value={expr}` / `vol='expr'` / `cur='expr'`
                if let Some(v) = self.kid(card, Tag::Val) {
                    let expression = self.body(v);
                    return Some(match tag {
                        Tag::E => Component::BehavioralVoltage {
                            name,
                            np,
                            nm,
                            expression,
                        },
                        _ => Component::BehavioralCurrent {
                            name,
                            np,
                            nm,
                            expression,
                        },
                    });
                }
                let (ncp, ncm, g) = (node(2)?, node(3)?, self.number(self.kid(card, Tag::Num)?)?);
                match tag {
                    Tag::E => Component::Vcvs {
                        name,
                        np,
                        nm,
                        ncp,
                        ncm,
                        gain: g,
                    },
                    _ => Component::Vccs {
                        name,
                        np,
                        nm,
                        ncp,
                        ncm,
                        transconductance: g,
                    },
                }
            }
            Tag::F | Tag::H => {
                let (np, nm, vsense, g) = (
                    node(0)?,
                    node(1)?,
                    self.text(k[3]).to_string(),
                    self.number(self.kid(card, Tag::Num)?)?,
                );
                match tag {
                    Tag::F => Component::Cccs {
                        name,
                        np,
                        nm,
                        vsense,
                        gain: g,
                    },
                    _ => Component::Ccvs {
                        name,
                        np,
                        nm,
                        vsense,
                        transresistance: g,
                    },
                }
            }
            // The expression is the card's own text after `V=`/`I=`.
            Tag::B => {
                let (np, nm) = (node(0)?, node(1)?);
                let expression = self.body(self.kid(card, Tag::Body)?);
                match self
                    .text(self.kid(card, Tag::Key)?)
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "v" => Component::BehavioralVoltage {
                        name,
                        np,
                        nm,
                        expression,
                    },
                    "i" => Component::BehavioralCurrent {
                        name,
                        np,
                        nm,
                        expression,
                    },
                    _ => return None,
                }
            }
            Tag::D | Tag::Q | Tag::M | Tag::J | Tag::Z => {
                let (model, params) = (model?, self.params(card));
                match (tag, nodes.as_slice()) {
                    (Tag::D, [np, nm]) => Component::Diode {
                        name,
                        np: np.clone(),
                        nm: nm.clone(),
                        model,
                        params,
                    },
                    // `Q c b e [s [t]] model`: substrate, then thermal node.
                    (Tag::Q, [nc, nb, ne, extra @ ..]) if extra.len() <= 2 => Component::Bjt {
                        name,
                        nc: nc.clone(),
                        nb: nb.clone(),
                        ne: ne.clone(),
                        extra_nodes: extra.to_vec(),
                        model,
                        params,
                    },
                    // `M d g s [b [more...]] model`: 3 nodes (VDMOS) to 7 (SOI).
                    (Tag::M, [nd, ng, ns, rest @ ..]) if rest.len() <= 4 => Component::Mosfet {
                        name,
                        nd: nd.clone(),
                        ng: ng.clone(),
                        ns: ns.clone(),
                        nb: rest.first().cloned(),
                        extra_nodes: rest.iter().skip(1).cloned().collect(),
                        model,
                        params,
                    },
                    (Tag::J, [nd, ng, ns]) => Component::Jfet {
                        name,
                        nd: nd.clone(),
                        ng: ng.clone(),
                        ns: ns.clone(),
                        model,
                        params,
                    },
                    (Tag::Z, [nd, ng, ns]) => Component::Mesfet {
                        name,
                        nd: nd.clone(),
                        ng: ng.clone(),
                        ns: ns.clone(),
                        model,
                        params,
                    },
                    _ => return None,
                }
            }
            Tag::S => Component::VSwitch {
                name,
                np: node(0)?,
                nm: node(1)?,
                ncp: node(2)?,
                ncm: node(3)?,
                model: model?,
            },
            Tag::W => Component::ISwitch {
                name,
                np: node(0)?,
                nm: node(1)?,
                vcontrol: self.text(k[3]).to_string(),
                model: model?,
            },
            Tag::T => {
                let p = self.params(card);
                let get = |key: &str| {
                    self.all(card, Tag::Pair)
                        .into_iter()
                        .find(|&pr| self.text(self.kids(pr)[0]).eq_ignore_ascii_case(key))
                        .and_then(|pr| self.number(self.kids(pr)[1]))
                };
                if p.len() != 2 {
                    return None;
                }
                Component::TLine {
                    name,
                    inp: node(0)?,
                    inm: node(1)?,
                    outp: node(2)?,
                    outm: node(3)?,
                    z0: get("z0")?,
                    td: get("td")?,
                }
            }
            _ => return None,
        })
    }
}

// ---- private ----

impl Cx<'_> {
    fn waveform(&self, w: usize) -> Option<IrWaveform> {
        let a: Vec<IrNum> = self
            .all(w, Tag::Val)
            .into_iter()
            .map(|v| self.irnum(v))
            .collect::<Option<_>>()?;
        let g = |i: usize| a.get(i).cloned().unwrap_or(IrNum::Num(0.0));
        Some(match self.tag(w) {
            Tag::Sin => IrWaveform::Sin {
                offset: g(0),
                amplitude: g(1),
                frequency: g(2),
                delay: g(3),
                damping: g(4),
                phase: g(5),
            },
            Tag::Pulse => IrWaveform::Pulse {
                initial: g(0),
                pulsed: g(1),
                delay: g(2),
                rise_time: g(3),
                fall_time: g(4),
                pulse_width: g(5),
                period: g(6),
            },
            Tag::Pwl if a.len().is_multiple_of(2) => IrWaveform::Pwl {
                values: a.chunks(2).map(|p| (p[0].clone(), p[1].clone())).collect(),
                repeat: None,
                delay: None,
            },
            Tag::Exp => IrWaveform::Exp {
                initial: g(0),
                pulsed: g(1),
                rise_delay: g(2),
                rise_tau: g(3),
                fall_delay: g(4),
                fall_tau: g(5),
            },
            Tag::Sffm => IrWaveform::Sffm {
                offset: g(0),
                amplitude: g(1),
                carrier_freq: g(2),
                modulation_index: g(3),
                signal_freq: g(4),
            },
            Tag::Am => IrWaveform::Am {
                amplitude: g(0),
                offset: g(1),
                modulating_freq: g(2),
                carrier_freq: g(3),
                delay: g(4),
            },
            Tag::Trnoise => IrWaveform::TrNoise {
                na: g(0),
                nt: g(1),
                nalpha: g(2),
                namp: g(3),
                rtsam: g(4),
                rtscapt: g(5),
                rtsemt: g(6),
            },
            // TYPE must be a number ngspice defines; PARAM1 defaults to 1
            // (manual §4.1.8).
            Tag::Trrandom => {
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
            _ => return None,
        })
    }
}
