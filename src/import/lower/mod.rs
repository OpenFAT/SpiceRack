//! AST rows -> [`CircuitIR`]: the one lowering every dialect's grammar feeds.
//!
//! The grammar has settled the syntax; what is left is meaning: which bare
//! word names a `.model`, which numbers a waveform's fields are, what a card
//! means inside a `.subckt`. A card that parsed but has no IR form is kept
//! raw ([`Report::raw`]).

use std::collections::{HashMap, HashSet};

use super::{Lang, Report, Tag};
use crate::codegen::Dialect;
use crate::ir::*;
use crate::peg::Tree;

mod analyses;
mod elements;
mod named;
mod named_analyses;
mod named_elements;
mod values;

struct Cx<'a> {
    t: &'a Tree<'a>,
    /// The logical text in its own case; rows index it as they index the
    /// folded text the grammar read.
    src: &'a str,
    d: Dialect,
    lang: &'a Lang,
    /// The deck's `.model` names, lowercased.
    models: HashSet<String>,
    report: Report,
    tb: Testbench,
    globals: Vec<String>,
    functions: Vec<FuncDef>,
    defs: Vec<Subcircuit>,
    /// Spectre/VACASK: model kinds by name, and subcircuit names.
    kinds: HashMap<String, String>,
    subckts: Vec<String>,
}

pub(super) fn deck(
    t: &Tree,
    src: &str,
    name: &str,
    d: Dialect,
    lang: &Lang,
) -> (CircuitIR, Report) {
    let models = (0..t.rows.len())
        .filter(|&i| tag(t, i) == Tag::Model)
        .filter_map(|i| t.children(Some(i)).next())
        .map(|n| t.text(n).to_string())
        .collect();
    let mut cx = Cx {
        t,
        src,
        d,
        lang,
        models,
        report: Report::default(),
        tb: Testbench::default(),
        globals: vec![],
        functions: vec![],
        defs: vec![],
        kinds: HashMap::new(),
        subckts: vec![],
    };
    if matches!(d, Dialect::Spectre | Dialect::Vacask) {
        (cx.kinds, cx.subckts) = cx.declarations();
    }
    let mut top = sub(name);
    for card in t.children(None) {
        cx.card(card, &mut top, false);
    }
    let Cx {
        report,
        tb,
        globals,
        functions,
        mut defs,
        ..
    } = cx;
    // LTspice inductors carry an implicit Rser=1m unless given or K-coupled
    // (ADI ltspice-reference CIRCUIT-ELEMENTS-REFERENCE); make it explicit so
    // other dialects don't silently lose it.
    if d == Dialect::Ltspice {
        for sc in std::iter::once(&mut top).chain(defs.iter_mut()) {
            implicit_rser(&mut sc.components);
        }
    }
    // VACASK loads are order-free; codegen::vacask writes them sorted.
    if d == Dialect::Vacask {
        top.osdi_loads.sort();
    }
    let raw_dialect = Some(d.family());
    (
        CircuitIR {
            raw_dialect,
            globals,
            functions,
            top,
            testbench: Some(tb),
            subcircuit_defs: defs,
            model_libraries: vec![],
        },
        report,
    )
}

fn implicit_rser(components: &mut [Component]) {
    let coupled: HashSet<String> = components
        .iter()
        .filter_map(|c| match c {
            Component::MutualInductor {
                inductor1,
                inductor2,
                ..
            } => Some([inductor1.clone(), inductor2.clone()]),
            _ => None,
        })
        .flatten()
        .collect();
    for c in components.iter_mut() {
        if let Component::Inductor { name, params, .. } = c
            && !coupled.contains(name)
            && !params.iter().any(|(k, _)| k.eq_ignore_ascii_case("rser"))
        {
            params.push(("Rser".to_string(), "1m".to_string()));
        }
    }
}

fn tag(t: &Tree, i: usize) -> Tag {
    Tag::from_u8(t.tag(i))
}

fn sub(name: &str) -> Subcircuit {
    Subcircuit {
        name: name.into(),
        ..Default::default()
    }
}

/// `{expr}`-style delimiters off a body the grammar kept whole.
fn trim(s: &str) -> String {
    s.trim().to_string()
}

impl Cx<'_> {
    fn text(&self, i: usize) -> &str {
        let r = self.t.rows[i];
        &self.src[r.start as usize..r.end as usize]
    }

    fn tag(&self, i: usize) -> Tag {
        tag(self.t, i)
    }

    fn kids(&self, i: usize) -> Vec<usize> {
        self.t.children(Some(i)).collect()
    }

    fn kid(&self, i: usize, want: Tag) -> Option<usize> {
        self.t.children(Some(i)).find(|&k| self.tag(k) == want)
    }

    fn all(&self, i: usize, want: Tag) -> Vec<usize> {
        self.t
            .children(Some(i))
            .filter(|&k| self.tag(k) == want)
            .collect()
    }

    fn texts(&self, i: usize, want: Tag) -> Vec<String> {
        self.all(i, want)
            .into_iter()
            .map(|k| self.text(k).to_string())
            .collect()
    }

    /// A card kept verbatim; a block, one raw line per logical line. A
    /// SPICE element card stays among the components, in its place: the
    /// simulator's matrix and timestep order follow the card order (LTRA,
    /// TRNOISE), so moving it to the end would be a different run.
    fn raw(&mut self, card: usize, here: &mut Subcircuit) {
        self.report.raw += 1;
        let text = self.text(card).to_string();
        match self.tag(card) {
            Tag::Block => here.raw_spice.extend(text.split('\n').map(str::to_string)),
            _ if !matches!(self.d, Dialect::Spectre | Dialect::Vacask)
                && text.starts_with(|c: char| c.is_ascii_alphabetic()) =>
            {
                here.components.push(Component::RawSpice { line: text })
            }
            _ => here.raw_spice.push(text),
        }
    }

    // ── Cards ──

    fn card(&mut self, card: usize, here: &mut Subcircuit, inside: bool) {
        let named = matches!(self.d, Dialect::Spectre | Dialect::Vacask);
        if named
            && matches!(
                self.tag(card),
                Tag::Load
                    | Tag::Ground
                    | Tag::Model
                    | Tag::Save
                    | Tag::SpiceRegion
                    | Tag::ControlBlock
                    | Tag::Ctl
                    | Tag::Ana
                    | Tag::Inst
                    | Tag::Embed
            )
        {
            let (kinds, subckts) = (
                std::mem::take(&mut self.kinds),
                std::mem::take(&mut self.subckts),
            );
            let ok = self.named_card(card, here, inside, &kinds, &subckts);
            (self.kinds, self.subckts) = (kinds, subckts);
            if !ok {
                self.raw(card, here);
            }
            return;
        }
        match self.tag(card) {
            Tag::Unparsed => {
                let line = self.text(card).to_string();
                self.report.unparsed.push(line.clone());
                here.raw_spice.push(line);
            }
            Tag::Raw | Tag::Block => self.raw(card, here),
            Tag::Subckt => {
                let sc = self.subckt(card);
                self.defs.push(sc);
            }
            Tag::Control => {
                for cmd in self.all(card, Tag::CtlCmd) {
                    self.control(cmd);
                }
            }
            // Spectre `include "f" section=s` is a library section.
            Tag::Include => match self.kid(card, Tag::Section) {
                Some(sec) => here
                    .libs
                    .push((self.path(card), self.text(sec).to_string())),
                None => here.includes.push(self.path(card)),
            },
            Tag::Lib => {
                let sec = self
                    .kid(card, Tag::Section)
                    .map_or(String::new(), |s| self.text(s).to_string());
                here.libs.push((self.path(card), sec));
            }
            Tag::Global => {
                for n in self
                    .all(card, Tag::Node)
                    .into_iter()
                    .map(|n| self.name(n))
                    .collect::<Vec<_>>()
                {
                    if !self.globals.contains(&n) {
                        self.globals.push(n);
                    }
                }
            }
            Tag::Model => {
                let name = self.text(self.kids(card)[0]).to_string();
                let kind = self.text(self.kids(card)[1]).to_string();
                let mut m = ModelDef {
                    name,
                    kind,
                    parameters: self.params(card),
                };
                if self.d == Dialect::Xyce
                    && matches!(m.kind.to_ascii_lowercase().as_str(), "nmos" | "pmos")
                {
                    // Xyce's BSIM3 is level 9 (RG Table 2-36), ngspice's MOS9:
                    // the IR keeps BSIM3 as 49, which both read.
                    for (k, v) in &mut m.parameters {
                        if k.eq_ignore_ascii_case("level")
                            && matches!(v.parse::<f64>(), Ok(8.0 | 9.0))
                        {
                            *v = "49".into();
                        }
                    }
                }
                here.models.push(m);
            }
            Tag::X => match self.instance(card) {
                Some(i) => here.instances.push(Instance {
                    at: Some(here.components.len() as u32),
                    ..i
                }),
                None => self.raw(card, here),
            },
            // A SPICE `.subckt`'s own `.param`s and functions are not the
            // IR's; Spectre's and VACASK's `parameters` are the definition's.
            Tag::Params | Tag::Func if inside && !(named && self.tag(card) == Tag::Params) => {
                self.raw(card, here)
            }
            Tag::Params => {
                for k in self.kids(card) {
                    match self.tag(k) {
                        Tag::FuncDef => {
                            let f = self.funcdef(k);
                            self.functions.push(f);
                        }
                        Tag::Assign => {
                            let kv = self.kids(k);
                            here.parameters.push(ParamDef {
                                name: self.text(kv[0]).to_string(),
                                default: Some(self.vtext(kv[1])),
                            });
                        }
                        _ => {}
                    }
                }
            }
            Tag::Func => {
                let f = self.funcdef(card);
                self.functions.push(f);
            }
            t if is_element(t) => match self.component(card) {
                Some(c) => here.components.push(c),
                None => self.raw(card, here),
            },
            _ if inside => self.raw(card, here),
            _ => {
                if !self.testbench_card(card) {
                    self.raw(card, here);
                }
            }
        }
    }

    fn path(&self, card: usize) -> String {
        let p = self.text(self.kid(card, Tag::Path).unwrap_or(card));
        let p = p.trim();
        match p.as_bytes() {
            [b'"', .., b'"'] | [b'\'', .., b'\''] => p[1..p.len() - 1].to_string(),
            _ => p.to_string(),
        }
    }

    fn funcdef(&self, i: usize) -> FuncDef {
        let name = self.text(self.kids(i)[0]).to_string();
        let body = self
            .kid(i, Tag::Body)
            .map_or(String::new(), |b| self.body(b));
        FuncDef {
            name,
            args: self.texts(i, Tag::Arg),
            body,
        }
    }

    /// `.subckt` ... `.ends`. A definition nested in another is local to it
    /// in SPICE, while IR subcircuit names are global: it is hoisted as
    /// `<parent>__<name>` and the parent's references to it renamed, so the
    /// two `sub`s of `sub1` and `sub2` stay two.
    fn subckt(&mut self, card: usize) -> Subcircuit {
        let kids = self.kids(card);
        let mut sc = sub(&self.name(kids[0]));
        let first = self.defs.len();
        let mut renames: Vec<(String, String)> = Vec::new();
        for &k in &kids[1..] {
            match self.tag(k) {
                Tag::Port => sc.ports.push(Port {
                    name: self.name(k),
                    direction: PortDirection::InOut,
                }),
                Tag::Assign => {
                    let kv = self.kids(k);
                    sc.parameters.push(ParamDef {
                        name: self.text(kv[0]).to_string(),
                        default: Some(self.text(kv[1]).trim().to_string()),
                    });
                }
                Tag::Ends => {}
                Tag::Subckt => {
                    let mut inner = self.subckt(k);
                    let scoped = format!("{}__{}", sc.name, inner.name);
                    renames.push((std::mem::replace(&mut inner.name, scoped.clone()), scoped));
                    self.defs.push(inner);
                }
                _ => self.card(k, &mut sc, true),
            }
        }
        if self.kid(card, Tag::Ends).is_none() {
            eprintln!("warning: .subckt {} has no .ends", sc.name);
        }
        for inst in sc.instances.iter_mut().chain(
            self.defs[first..]
                .iter_mut()
                .flat_map(|d| d.instances.iter_mut()),
        ) {
            if let Some((_, to)) = renames
                .iter()
                .find(|(from, _)| from.eq_ignore_ascii_case(&inst.subcircuit))
            {
                inst.subcircuit = to.clone();
            }
        }
        sc
    }
}

fn is_element(t: Tag) -> bool {
    use Tag::*;
    matches!(
        t,
        R | C | L | K | V | I | Poly | E | G | F | H | B | D | Q | M | J | Z | S | W | T
    )
}
