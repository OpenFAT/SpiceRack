//! SPICE element `Instance`s → `Component`s and `Instance`s, as
//! `import/lower/elements.rs` builds them, each card first checked against
//! the shape its letter's rule in `grammar/spice.peg` reads (a card that
//! fails it was `Raw_element`, kept raw; one with no field after its name
//! was `Unparsed`). Two-terminal and linear controlled elements here; POLY,
//! `B`, semiconductors and `X` in `devices`, V and I in `sources`.
//!
//! Cites: `import/lower/elements.rs` (ported); spice.peg §3.1 element rules
//! (`R` … `X`, `inst`, `nodes`, `spec`, `wargs`, `Poly`, `behave`, `B`);
//! ngspice.peg `Pair_src`, `Trnoise`, `Trrandom`; ngspice manual §4.1.4,
//! §4.1.7, §4.1.8, §4.1.11.
//!
//! Contents:
//! - `Cx::element(&mut self, n: u32, here: &mut Subcircuit)`

use super::Cx;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, PRIMITIVE, Tag};
use crate::syntax::lexicon::Element;

const ELEMENTS: [Element; 20] = [
    Element::Resistor,
    Element::Capacitor,
    Element::Inductor,
    Element::Coupling,
    Element::VoltageSource,
    Element::CurrentSource,
    Element::Vcvs,
    Element::Vccs,
    Element::Cccs,
    Element::Ccvs,
    Element::Behavioural,
    Element::Diode,
    Element::Bjt,
    Element::Mosfet,
    Element::Jfet,
    Element::Mesfet,
    Element::VoltageSwitch,
    Element::CurrentSwitch,
    Element::TransmissionLine,
    Element::Instance,
];

impl<'a> Cx<'a> {
    /// An element card onto `here`: a component, a subcircuit instance, or
    /// a raw line.
    pub(super) fn element(&mut self, n: u32, here: &mut Subcircuit) {
        let words = self.ast.rhs(Node(n));
        let master = self.ast.extra(words + 1);
        if !self
            .t
            .text(self.main(n))
            .starts_with(|c: char| c.is_ascii_alphabetic())
        {
            // quirk: spice.peg `element_any <- &[a-z] Name`.
            return self.unparsed(n, here);
        }
        let has_fields = self.fields_ok(n);
        let got = if master == NONE {
            None
        } else if master & PRIMITIVE != 0 {
            let e = ELEMENTS
                .iter()
                .find(|&&e| e as u32 == master & !PRIMITIVE)
                .copied()
                .expect("an Element discriminant");
            self.component(n, e).map(Ok)
        } else {
            self.instance(n).map(Err)
        };
        match got {
            Some(Ok(c)) => here.components.push(c),
            Some(Err(i)) => here.instances.push(Instance {
                at: Some(here.components.len() as u32),
                ..i
            }),
            None if has_fields => self.raw(n, here, false),
            None => self.unparsed(n, here),
        }
    }
}

// ---- private ----

impl<'a> Cx<'a> {
    fn component(&self, n: u32, e: Element) -> Option<Component> {
        let items = self.items(n);
        let name = self.elem_name(n);
        // quirk: every element rule reads `s` between its fields; only an
        // `inst` tail and a waveform's values take commas.
        let commas = matches!(
            e,
            Element::Resistor
                | Element::Capacitor
                | Element::Inductor
                | Element::Diode
                | Element::Bjt
                | Element::Mosfet
                | Element::Jfet
                | Element::Mesfet
                | Element::VoltageSource
                | Element::CurrentSource
        );
        let head = items.len().min(2);
        // A `B` card's body after `key =` is text to the card's end.
        let spaced = match e {
            Element::Behavioural => items.len().min(3),
            _ => items.len(),
        };
        if !self.spaced_items(self.name_last(n), &items[..head], false)
            || !self.spaced_items(
                items
                    .get(head.wrapping_sub(1))
                    .map_or(self.name_last(n), |&k| self.last(k)),
                &items[head..spaced],
                commas,
            )
        {
            return None;
        }
        let node = |i: usize| {
            items
                .get(i)
                .copied()
                .filter(|&k| self.is_word(k))
                .map(|k| self.text(k))
        };
        Some(match e {
            Element::Resistor | Element::Capacitor | Element::Inductor => {
                let (n1, n2) = (node(0)?, node(1)?);
                let rest = &items[2..];
                let (val, inst) = match rest.first() {
                    Some(&v) if self.tag(v) != Tag::Assign => (Some(v), &rest[1..]),
                    _ => (None, rest),
                };
                if val.is_some_and(|v| !self.val_ok(v) || self.tag(v) == Tag::Seq)
                    || !self.inst_ok(inst)
                {
                    return None;
                }
                // A flag before the parameters (lower/elements.rs:38-51).
                let first_pair = inst.iter().position(|&x| self.tag(x) == Tag::Assign);
                if inst
                    .iter()
                    .position(|&x| self.is_word(x))
                    .is_some_and(|f| first_pair.is_none_or(|p| f < p))
                {
                    return None;
                }
                let mut params = self.params_of(inst, false);
                let pairs: Vec<u32> = inst
                    .iter()
                    .copied()
                    .filter(|&x| self.tag(x) == Tag::Assign)
                    .collect();
                let mut model = None;
                let value = match val {
                    Some(v)
                        if self.is_word(v)
                            && self.number(v).is_none()
                            && self.models.contains(&self.text(v).to_ascii_lowercase()) =>
                    {
                        model = Some(self.text(v));
                        IrValue::Raw {
                            text: String::new(),
                        }
                    }
                    Some(v) => self.value(v),
                    None if e == Element::Capacitor
                        && !params.iter().any(|(k, _)| k.eq_ignore_ascii_case("c"))
                        && params.iter().any(|(k, _)| k.eq_ignore_ascii_case("q")) =>
                    {
                        let i = params
                            .iter()
                            .position(|(k, _)| k.eq_ignore_ascii_case("q"))?;
                        let pair = pairs[i];
                        params.remove(i);
                        return Some(Component::ChargeCapacitor {
                            name,
                            n1,
                            n2,
                            charge: self.body_text(self.ast.rhs(Node(pair))),
                            params,
                        });
                    }
                    None => {
                        let key = match e {
                            Element::Resistor => "r",
                            Element::Capacitor => "c",
                            _ => "l",
                        };
                        let i = params
                            .iter()
                            .position(|(k, _)| k.eq_ignore_ascii_case(key))?;
                        let pair = pairs[i];
                        params.remove(i);
                        self.value(self.ast.rhs(Node(pair)))
                    }
                };
                match e {
                    Element::Resistor => Component::Resistor {
                        name,
                        n1,
                        n2,
                        value,
                        model,
                        params,
                    },
                    Element::Capacitor => Component::Capacitor {
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
            Element::Coupling => {
                let [a, b, k] = items else { return None };
                if !self.is_word(*a) || !self.is_word(*b) || !self.is_word(*k) {
                    return None;
                }
                Component::MutualInductor {
                    name,
                    inductor1: self.text(*a).get(1..)?.to_string(),
                    inductor2: self.text(*b).get(1..)?.to_string(),
                    coupling: self.number(*k)?,
                }
            }
            Element::VoltageSource | Element::CurrentSource => self.source(n, e, name)?,
            Element::Vcvs | Element::Vccs | Element::Cccs | Element::Ccvs => {
                if let Some(p) = self.poly(n, name.clone()) {
                    return p;
                }
                let (np, nm) = (node(0)?, node(1)?);
                match e {
                    Element::Vcvs | Element::Vccs => {
                        if let [_, _, a] = items
                            && self.tag(*a) == Tag::Assign
                        {
                            let key = self.key(*a).to_ascii_lowercase();
                            let v = self.ast.rhs(Node(*a));
                            if !matches!(key.as_str(), "value" | "vol" | "cur") || !self.val_ok(v) {
                                return None;
                            }
                            let expression = self.body_text(v);
                            return Some(match e {
                                Element::Vcvs => Component::BehavioralVoltage {
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
                        let [_, _, c, d, g] = items else { return None };
                        if ![c, d, g].iter().all(|&&k| self.is_word(k)) {
                            return None;
                        }
                        let (ncp, ncm, g) = (self.text(*c), self.text(*d), self.number(*g)?);
                        match e {
                            Element::Vcvs => Component::Vcvs {
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
                    _ => {
                        let [_, _, v, g] = items else { return None };
                        if !self.is_word(*v) || !self.is_word(*g) {
                            return None;
                        }
                        let (vsense, g) = (self.text(*v), self.number(*g)?);
                        match e {
                            Element::Cccs => Component::Cccs {
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
                }
            }
            Element::Behavioural => self.behavioural(n, name)?,
            Element::Diode | Element::Bjt | Element::Mosfet | Element::Jfet | Element::Mesfet => {
                self.semiconductor(n, e, name)?
            }
            Element::VoltageSwitch => {
                let [a, b, c, d, m] = items else { return None };
                if ![a, b, c, d, m].iter().all(|&&k| self.is_word(k)) {
                    return None;
                }
                Component::VSwitch {
                    name,
                    np: self.text(*a),
                    nm: self.text(*b),
                    ncp: self.text(*c),
                    ncm: self.text(*d),
                    model: self.text(*m),
                }
            }
            Element::CurrentSwitch => {
                let [a, b, c, m] = items else { return None };
                if ![a, b, c, m].iter().all(|&&k| self.is_word(k)) {
                    return None;
                }
                Component::ISwitch {
                    name,
                    np: self.text(*a),
                    nm: self.text(*b),
                    vcontrol: self.text(*c),
                    model: self.text(*m),
                }
            }
            Element::TransmissionLine => {
                if items.len() < 4 || !items[..4].iter().all(|&k| self.is_word(k)) {
                    return None;
                }
                let pairs = &items[4..];
                if !pairs.iter().all(|&p| self.pair_ok(p)) {
                    return None;
                }
                let get = |key: &str| {
                    pairs
                        .iter()
                        .find(|&&p| self.key(p).eq_ignore_ascii_case(key))
                        .and_then(|&p| self.number(self.ast.rhs(Node(p))))
                };
                if pairs.len() != 2 {
                    return None;
                }
                Component::TLine {
                    name,
                    inp: self.text(items[0]),
                    inm: self.text(items[1]),
                    outp: self.text(items[2]),
                    outm: self.text(items[3]),
                    z0: get("z0")?,
                    td: get("td")?,
                }
            }
            Element::Instance => return None,
        })
    }
}
