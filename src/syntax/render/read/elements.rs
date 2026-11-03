//! SPICE element `Instance` nodes → their place in the block (a typed
//! card, an `X` card, or a raw line) and the [`Card`] view the writer
//! spells, each card first checked against the shape its letter's rule in
//! the old SPICE grammar reads (a card that fails it was `Raw_element`,
//! kept raw; one with no field after its name was `Unparsed`). In a kit, an
//! R/C/L card's model word is the one `syntax::resolve` finds among its
//! values. Two-terminal and linear controlled elements here; POLY, `B`,
//! semiconductors and `X` in `devices`, V and I in `sources`.
//!
//! Cites: the old lowering's element rules (as `syntax::adapter::elements`
//! ports them); the old SPICE grammar's §3.1 element rules (`R` … `X`,
//! `inst`, `nodes`, `spec`, `wargs`, `Poly`, `behave`, `B`).
//!
//! Contents:
//! - `pub(in crate::syntax::render) enum Card`
//! - `Rd::element(&mut self, n: u32, here: &mut Block)`
//! - `Rd::card_view(&self, n: u32) -> Card`, `Rd::letter(&self, n: u32) -> char`

use super::{Block, Comp, Rd, Val, Wave};
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::device::Primitive as Element;

/// An element card as the writer spells it: what the old reader kept of
/// it, its words as written. `name` is the name after the element letter.
#[derive(Clone, Debug)]
pub(in crate::syntax::render) enum Card {
    /// `R`, `C`, `L`: the value (an empty `Raw` when the word was a model),
    /// the model, the parameters.
    Rcl {
        letter: char,
        name: String,
        n1: String,
        n2: String,
        value: Val,
        model: Option<String>,
        params: Vec<(String, String)>,
    },
    /// `C1 a b q=expr`.
    Charge {
        name: String,
        n1: String,
        n2: String,
        charge: String,
        params: Vec<(String, String)>,
    },
    /// `K1 L1 L2 k`: the inductor names after their letter.
    Coupling {
        name: String,
        l1: String,
        l2: String,
        k: f64,
    },
    /// `V`, `I`.
    Source {
        letter: char,
        name: String,
        np: String,
        nm: String,
        value: Val,
        ac_mag: Option<f64>,
        ac_phase: Option<f64>,
        wave: Option<Wave>,
        /// ngspice `portnum n z0 Z`.
        port: Option<(u32, f64)>,
    },
    /// `B … V=` / `I=`, `E … value=`, `G … cur=`: the expression's text,
    /// and its tree (the value node).
    Behavioural {
        current: bool,
        name: String,
        np: String,
        nm: String,
        expr: String,
        value: u32,
    },
    /// `E`/`G` (two control nodes), `F`/`H` (a controlling source).
    Linear {
        letter: char,
        name: String,
        nodes: Vec<String>,
        gain: f64,
    },
    /// `E o 0 POLY(n) controls… coeffs…`.
    Poly {
        letter: char,
        name: String,
        np: String,
        nm: String,
        controls: Vec<String>,
        coeffs: Vec<f64>,
    },
    /// `D`, `Q`, `M`, `J`, `Z`: nodes, model, parameters.
    Semi {
        letter: char,
        name: String,
        nodes: Vec<String>,
        model: String,
        params: Vec<(String, String)>,
    },
    /// `S` (four nodes), `W` (two nodes and a controlling source): nodes
    /// then the model.
    Switch {
        letter: char,
        name: String,
        nodes: Vec<String>,
        model: String,
    },
    /// `T a b c d Z0= TD=`.
    Line {
        name: String,
        nodes: Vec<String>,
        z0: f64,
        td: f64,
    },
    /// `X`: ports, master (after the name map), parameters.
    Inst {
        name: String,
        ports: Vec<String>,
        master: String,
        params: Vec<(String, String)>,
    },
}

impl<'a> Rd<'a> {
    /// An element card onto `here`: a typed card, a subcircuit instance, or
    /// a raw line.
    pub(super) fn element(&mut self, n: u32, here: &mut Block) {
        if !self
            .t
            .text(self.main(n))
            .starts_with(|c: char| c.is_ascii_alphabetic())
        {
            // quirk: the old SPICE grammar's `element_any <- &[a-z] Name`:
            // an element's name starts with a letter; any other line is
            // `Unparsed`.
            return self.unparsed(n, here);
        }
        let has_fields = self.fields_ok(n);
        match self.try_card(n) {
            Some(Card::Inst { .. }) => here.insts.push((here.comps.len() as u32, n)),
            Some(_) => here.comps.push(Comp::Card(n)),
            None if has_fields => self.raw(n, here, false),
            None => self.unparsed(n, here),
        }
    }

    /// The view of an element [`Rd::element`] placed as a typed card or an
    /// instance.
    ///
    /// # Panics
    /// When `n` is no such element.
    pub(in crate::syntax::render) fn card_view(&self, n: u32) -> Card {
        self.try_card(n).expect("an element the reader typed")
    }

    /// The card's element letter, uppercased (as the old IR kept it).
    pub(super) fn letter(&self, n: u32) -> char {
        let first = self.t.text(self.main(n)).chars().next();
        first.expect("an element name").to_ascii_uppercase()
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// What the old reader typed element `n` as; `None` for a raw one.
    fn try_card(&self, n: u32) -> Option<Card> {
        if self.named() {
            return self.named_inst(n);
        }
        let h = self.ast.header(Node(n));
        match Element::from_u32(h.code) {
            None => None,
            Some(Element::Subckt) if h.master == NONE => None,
            Some(Element::Subckt) => self.instance(n),
            Some(e) => self.component(n, e),
        }
    }

    fn component(&self, n: u32, e: Element) -> Option<Card> {
        let items = &self.items(n)[..];
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
                self.rcl(n, e, name, node(0)?, node(1)?, &items[2..])?
            }
            Element::Coupling => {
                let [a, b, k] = items else { return None };
                if !self.is_word(*a) || !self.is_word(*b) || !self.is_word(*k) {
                    return None;
                }
                Card::Coupling {
                    name,
                    l1: self.text(*a).get(1..)?.to_string(),
                    l2: self.text(*b).get(1..)?.to_string(),
                    k: self.number(*k)?,
                }
            }
            Element::VoltageSource | Element::CurrentSource => self.source(n, e, name)?,
            Element::Vcvs | Element::Vccs | Element::Cccs | Element::Ccvs => {
                if let Some(p) = self.poly(n, name.clone()) {
                    return p;
                }
                let (np, nm) = (node(0)?, node(1)?);
                let letter = self.letter(n);
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
                            return Some(Card::Behavioural {
                                current: e == Element::Vccs,
                                name,
                                np,
                                nm,
                                expr: self.body_text(v),
                                value: v,
                            });
                        }
                        let [_, _, c, d, g] = items else { return None };
                        if ![c, d, g].iter().all(|&&k| self.is_word(k)) {
                            return None;
                        }
                        Card::Linear {
                            letter,
                            name,
                            nodes: vec![np, nm, self.text(*c), self.text(*d)],
                            gain: self.number(*g)?,
                        }
                    }
                    _ => {
                        let [_, _, v, g] = items else { return None };
                        if !self.is_word(*v) || !self.is_word(*g) {
                            return None;
                        }
                        Card::Linear {
                            letter,
                            name,
                            nodes: vec![np, nm, self.text(*v)],
                            gain: self.number(*g)?,
                        }
                    }
                }
            }
            Element::Behavioural => self.behavioural(n, name)?,
            Element::Diode | Element::Bjt | Element::Mosfet | Element::Jfet | Element::Mesfet => {
                self.semiconductor(n, e, name)?
            }
            Element::VoltageSwitch | Element::CurrentSwitch => {
                let want = if e == Element::VoltageSwitch { 5 } else { 4 };
                if items.len() != want || !items.iter().all(|&k| self.is_word(k)) {
                    return None;
                }
                let (nodes, m) = items.split_at(want - 1);
                Card::Switch {
                    letter: self.letter(n),
                    name,
                    nodes: nodes.iter().map(|&k| self.text(k)).collect(),
                    model: self.text(m[0]),
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
                Card::Line {
                    name,
                    nodes: items[..4].iter().map(|&k| self.text(k)).collect(),
                    z0: get("z0")?,
                    td: get("td")?,
                }
            }
            // else: the ngspice letters the old reader had no rule for.
            Element::Subckt
            | Element::LossyLine
            | Element::RcLine
            | Element::Txl
            | Element::CoupledLines
            | Element::CodeModel
            | Element::Osdi => return None,
        })
    }

    /// An R, C or L card's value, model and parameters.
    fn rcl(
        &self,
        n: u32,
        e: Element,
        name: String,
        n1: String,
        n2: String,
        rest: &[u32],
    ) -> Option<Card> {
        let (val, inst) = match rest.first() {
            Some(&v) if self.tag(v) != Tag::Assign => (Some(v), &rest[1..]),
            _ => (None, rest),
        };
        // The kit's model word: the value, or a word after it.
        let kit_model = self.kit_split(n).map(|s| s.master.0);
        let late = kit_model.filter(|m| inst.contains(m));
        let inst: Vec<u32> = inst.iter().copied().filter(|&k| Some(k) != late).collect();
        let inst = &inst[..];
        if val.is_some_and(|v| !self.val_ok(v) || self.tag(v) == Tag::Seq) || !self.inst_ok(inst) {
            return None;
        }
        // A flag before the parameters (the old lowering).
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
            Some(v) if late.is_some() => {
                model = late.map(|m| self.text(m));
                self.value(v)
            }
            Some(v)
                if self.is_word(v)
                    && self.number(v).is_none()
                    && (self.models.contains(&self.text(v).to_ascii_lowercase())
                        || kit_model == Some(v)) =>
            {
                model = Some(self.text(v));
                Val::Raw(String::new())
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
                return Some(Card::Charge {
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
        let letter = match e {
            Element::Resistor => 'R',
            Element::Capacitor => 'C',
            _ => 'L',
        };
        Some(Card::Rcl {
            letter,
            name,
            n1,
            n2,
            value,
            model,
            params,
        })
    }
}
