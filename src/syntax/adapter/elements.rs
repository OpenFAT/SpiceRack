//! SPICE element `Instance`s → `Component`s and `Instance`s, as
//! `import/lower/elements.rs` builds them, each card first checked against
//! the shape its letter's rule in `grammar/spice.peg` reads (a card that
//! fails it was `Raw_element`, kept raw; one with no field after its name
//! was `Unparsed`).
//!
//! Cites: `import/lower/elements.rs` (ported); spice.peg §3.1 element rules
//! (`R` … `X`, `inst`, `nodes`, `spec`, `wargs`, `Poly`, `behave`, `B`);
//! ngspice.peg `Pair_src`, `Trnoise`, `Trrandom`; ngspice manual §4.1.4,
//! §4.1.7, §4.1.8, §4.1.11.
//!
//! Contents:
//! - `Cx::element(&mut self, n: u32, here: &mut Subcircuit)`

use super::Cx;
use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{NONE, Node, PRIMITIVE, Tag};
use crate::syntax::lexicon::Element;
use crate::syntax::token::Tag as T;

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

impl Cx<'_> {
    /// An element card onto `here`: a component, a subcircuit instance, or
    /// a raw line.
    pub(super) fn element(&mut self, n: u32, here: &mut Subcircuit) {
        let words = self.ast.rhs(Node(n));
        let master = self.ast.extra(words + 1);
        let name_last = self.last(self.ast.extra(words));
        let has_fields = name_last < self.line_last(self.main(n));
        if !self
            .t
            .text(self.main(n))
            .starts_with(|c: char| c.is_ascii_alphabetic())
        {
            // quirk: spice.peg `element_any <- &[a-z] Name`.
            return self.unparsed(n, here);
        }
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

impl Cx<'_> {
    fn items(&self, n: u32) -> &[u32] {
        self.fields(n)
    }

    /// The element's name after its letter (spice.peg `Name_e`).
    fn elem_name(&self, n: u32) -> String {
        let w = self.ast.extra(self.ast.rhs(Node(n)));
        let t = self.text(w);
        t[1..].to_string()
    }

    fn is_word(&self, n: u32) -> bool {
        self.tag(n) == Tag::Word
    }

    fn word_is(&self, n: u32, w: &str) -> bool {
        self.is_word(n) && self.text(n).eq_ignore_ascii_case(w)
    }

    /// A value the grammar's `Val` reads: a word, a string, a readable
    /// delimited expression, or an expression that does not start with a
    /// delimiter.
    pub(super) fn val_ok(&self, v: u32) -> bool {
        if v == NONE {
            return false;
        }
        match self.tag(v) {
            Tag::Word | Tag::Str => true,
            Tag::Delimited => {
                self.expr_ok(v) || (self.d == Dialect::Xyce && self.ttag(self.main(v)) == T::LBrace)
            }
            Tag::Seq | Tag::Bad | Tag::Group | Tag::Assign => false,
            _ => !self.nests_delimiter(v),
        }
    }

    /// An `Assign` the grammar's `Pair` reads (`Key = Val`).
    fn pair_ok(&self, a: u32) -> bool {
        self.tag(a) == Tag::Assign
            && self.is_word(self.ast.lhs(Node(a)))
            && self.val_ok(self.ast.rhs(Node(a)))
    }

    /// As [`Cx::pair_ok`], a value that may run on through numbers
    /// (`Pair_i`).
    fn pair_i_ok(&self, a: u32) -> bool {
        if self.tag(a) != Tag::Assign || !self.is_word(self.ast.lhs(Node(a))) {
            return false;
        }
        let v = self.ast.rhs(Node(a));
        match v {
            NONE => false,
            _ if self.tag(v) == Tag::Seq => {
                let parts = self.ast.list(self.ast.lhs(Node(v)));
                self.val_ok(parts[0]) && parts[1..].iter().all(|&p| self.is_word(p))
            }
            _ => self.val_ok(v),
        }
    }

    /// spice.peg `inst`: flags and `Pair_i`s; `None` when another item
    /// stands among them.
    fn inst_ok(&self, rest: &[u32]) -> bool {
        rest.iter().all(|&k| self.is_word(k) || self.pair_i_ok(k))
    }

    fn component(&self, n: u32, e: Element) -> Option<Component> {
        let items = self.items(n);
        let name = self.elem_name(n);
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

    /// `E o 0 POLY(n) controls... coeffs...` (and G, F, H); `None` when the
    /// card is no POLY card, `Some(None)` when it is one the IR cannot hold.
    fn poly(&self, n: u32, name: String) -> Option<Option<Component>> {
        let items = self.items(n);
        let [a, b, p, rest @ ..] = items else {
            return None;
        };
        if !self.is_word(*a) || !self.is_word(*b) || self.tag(*p) != Tag::Group {
            return None;
        }
        let gname = self.ast.lhs(Node(*p));
        if gname == NONE || !self.word_is(gname, "poly") {
            return None;
        }
        let inside = self.group_items(*p);
        let [dims] = inside[..] else { return None };
        if !self.is_word(dims)
            || self.number(dims).is_none()
            || rest.is_empty()
            || !rest.iter().all(|&w| self.is_word(w))
        {
            return None;
        }
        let card = (|| {
            let letter = self
                .t
                .text(self.main(n))
                .chars()
                .next()?
                .to_ascii_uppercase();
            let dims = self.count(dims)? as usize;
            let n_controls = if matches!(letter, 'E' | 'G') {
                2 * dims
            } else {
                dims
            };
            let controls = rest
                .get(..n_controls)?
                .iter()
                .map(|&w| self.text(w))
                .collect();
            let coeffs = rest[n_controls..]
                .iter()
                .map(|&w| self.number(w))
                .collect::<Option<_>>()?;
            Some(Component::Poly {
                name,
                letter,
                np: self.text(*a),
                nm: self.text(*b),
                controls,
                coeffs,
            })
        })();
        Some(card)
    }

    /// A group's items.
    pub(super) fn group_items(&self, g: u32) -> &[u32] {
        let at = self.ast.rhs(Node(g));
        self.ast.list(at + 1)
    }

    /// `B1 n p V=expr`: the expression to the card's end (spice.peg `B`,
    /// `Body`).
    fn behavioural(&self, n: u32, name: String) -> Option<Component> {
        let items = self.items(n);
        let [a, b, kv, ..] = items else { return None };
        if !self.is_word(*a)
            || !self.is_word(*b)
            || self.tag(*kv) != Tag::Assign
            || !self.is_word(self.ast.lhs(Node(*kv)))
        {
            return None;
        }
        let expression = self.body_to_end(*kv)?;
        let (np, nm) = (self.text(*a), self.text(*b));
        match self.key(*kv).to_ascii_lowercase().as_str() {
            "v" => Some(Component::BehavioralVoltage {
                name,
                np,
                nm,
                expression,
            }),
            "i" => Some(Component::BehavioralCurrent {
                name,
                np,
                nm,
                expression,
            }),
            _ => None,
        }
    }

    /// quirk: spice.peg `Body`: the text after `key =` to the card's end,
    /// unless a ` key=` follows; braces (or a readable expression) around
    /// all of it are dropped.
    pub(super) fn body_to_end(&self, kv: u32) -> Option<String> {
        let eq = self.last(self.ast.lhs(Node(kv))) + 1;
        let first = eq + 1;
        let last = self.line_last(self.main(kv));
        if first > last {
            return None;
        }
        for k in first + 1..=last {
            let gap = self.t.end(k - 1) != self.t.start(k);
            let key = self.ttag(k) == T::Ident
                && self
                    .t
                    .text(k)
                    .starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
            if gap && key && k < last && self.ttag(k + 1) == T::Equal {
                return None;
            }
        }
        let v = self.ast.rhs(Node(kv));
        let whole = v != NONE && self.main(v) == first && self.last(v) == last;
        if whole
            && self.tag(v) == Tag::Delimited
            && (self.expr_ok(v) || self.ttag(first) == T::LBrace)
        {
            return Some(self.inner(v).trim().to_string());
        }
        if self.ttag(first) == T::LBrace && self.brace_close(first) == Some(last) {
            return Some(self.span(first + 1, last - 1).trim().to_string());
        }
        Some(self.span(first, last).trim().to_string())
    }

    /// The `}` matching the `{` at `open`, on its statement.
    fn brace_close(&self, open: u32) -> Option<u32> {
        let last = self.line_last(open);
        let mut depth = 0i32;
        for k in open..=last {
            match self.ttag(k) {
                T::LBrace => depth += 1,
                T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(k);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `D/Q/M/J/Z name nodes... model [flags] [k=v...]` (spice.peg `nodes`,
    /// `inst`).
    fn semiconductor(&self, n: u32, e: Element, name: String) -> Option<Component> {
        let items = self.items(n);
        let iflag = |k: u32| ["thermal", "off", "on"].iter().any(|f| self.word_is(k, f));
        // Nodes while another word (no flag, no pair) follows; then the model.
        let mut m = 0;
        while m < items.len() && self.is_word(items[m]) {
            let next = items.get(m + 1);
            if next.is_some_and(|&x| self.is_word(x) && !iflag(x)) {
                m += 1;
            } else {
                break;
            }
        }
        if m >= items.len() || !self.is_word(items[m]) || !self.inst_ok(&items[m + 1..]) {
            return None;
        }
        let nodes: Vec<String> = items[..m].iter().map(|&k| self.text(k)).collect();
        let (model, params) = (self.text(items[m]), self.params_of(&items[m + 1..], false));
        Some(match (e, nodes.as_slice()) {
            (Element::Diode, [np, nm]) => Component::Diode {
                name,
                np: np.clone(),
                nm: nm.clone(),
                model,
                params,
            },
            (Element::Bjt, [nc, nb, ne, extra @ ..]) if extra.len() <= 2 => Component::Bjt {
                name,
                nc: nc.clone(),
                nb: nb.clone(),
                ne: ne.clone(),
                extra_nodes: extra.to_vec(),
                model,
                params,
            },
            (Element::Mosfet, [nd, ng, ns, rest @ ..]) if rest.len() <= 4 => Component::Mosfet {
                name,
                nd: nd.clone(),
                ng: ng.clone(),
                ns: ns.clone(),
                nb: rest.first().cloned(),
                extra_nodes: rest.iter().skip(1).cloned().collect(),
                model,
                params,
            },
            (Element::Jfet, [nd, ng, ns]) => Component::Jfet {
                name,
                nd: nd.clone(),
                ng: ng.clone(),
                ns: ns.clone(),
                model,
                params,
            },
            (Element::Mesfet, [nd, ng, ns]) => Component::Mesfet {
                name,
                nd: nd.clone(),
                ng: ng.clone(),
                ns: ns.clone(),
                model,
                params,
            },
            _ => return None,
        })
    }

    /// An `X` card (spice.peg `X`, lower/elements.rs `instance`).
    fn instance(&self, n: u32) -> Option<Instance> {
        let words = self.ast.rhs(Node(n));
        let master = self.ast.extra(words + 1);
        let nodes = self.ast.list(words + 2);
        let items = self.items(n);
        let pairs: Vec<u32> = items
            .iter()
            .copied()
            .filter(|&k| self.tag(k) == Tag::Assign)
            .collect();
        if !pairs.iter().all(|&p| self.pair_ok(p)) {
            return None;
        }
        Some(Instance {
            name: self.elem_name(n),
            subcircuit: self.text(master),
            port_mapping: nodes.iter().map(|&k| self.text(k)).collect(),
            parameters: self.params_of(&pairs, false),
            at: None,
        })
    }

    /// A V or I card (spice.peg `V`, `spec`; lower/elements.rs:133-219).
    fn source(&self, n: u32, e: Element, name: String) -> Option<Component> {
        let items = self.items(n);
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
