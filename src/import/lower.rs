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

pub(super) fn deck(t: &Tree, src: &str, name: &str, d: Dialect, lang: &Lang) -> (CircuitIR, Report) {
    let models = (0..t.rows.len()).filter(|&i| tag(t, i) == Tag::Model).filter_map(|i| t.children(Some(i)).next()).map(|n| t.text(n).to_string()).collect();
    let mut cx = Cx { t, src, d, lang, models, report: Report::default(), tb: testbench(name), globals: vec![], functions: vec![], defs: vec![], kinds: HashMap::new(), subckts: vec![] };
    if matches!(d, Dialect::Spectre | Dialect::Vacask) {
        (cx.kinds, cx.subckts) = cx.declarations();
    }
    let mut top = sub(name);
    for card in t.children(None) {
        cx.card(card, &mut top, false);
    }
    let Cx { report, tb, globals, functions, mut defs, .. } = cx;
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
    (CircuitIR { raw_dialect, globals, functions, top, testbench: Some(tb), subcircuit_defs: defs, model_libraries: vec![] }, report)
}

fn implicit_rser(components: &mut [Component]) {
    let coupled: HashSet<String> = components
        .iter()
        .filter_map(|c| match c {
            Component::MutualInductor { inductor1, inductor2, .. } => Some([inductor1.clone(), inductor2.clone()]),
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
    Subcircuit { name: name.into(), ..Default::default() }
}

fn testbench(dut: &str) -> Testbench {
    Testbench { dut: dut.into(), ..Default::default() }
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
        self.t.children(Some(i)).filter(|&k| self.tag(k) == want).collect()
    }

    fn texts(&self, i: usize, want: Tag) -> Vec<String> {
        self.all(i, want).into_iter().map(|k| self.text(k).to_string()).collect()
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
            _ if !matches!(self.d, Dialect::Spectre | Dialect::Vacask) && text.starts_with(|c: char| c.is_ascii_alphabetic()) => here.components.push(Component::RawSpice { line: text }),
            _ => here.raw_spice.push(text),
        }
    }

    // ── Values ──

    /// A `Num` row's value: its `Mag` (one decimal, `d` an exponent letter)
    /// scaled by its scale row, parsed once so `0.1n` is exactly 1e-10.
    fn num(&self, i: usize) -> Option<f64> {
        // ngspice's own arithmetic, so a deck written back reads as the same
        // double (`0.5u` and `500n` differ in the last bit there).
        if matches!(self.d, Dialect::Ngspice | Dialect::Espice) {
            return crate::unit::ngspice_value(self.text(i));
        }
        let mag = self.kid(i, Tag::Mag)?;
        let mut m = self.text(mag).to_ascii_lowercase().replace('d', "e");
        // LTspice `6K34`: the digits after the scale are its fraction.
        if let Some(f) = self.kid(i, Tag::Frac) {
            m = format!("{m}.{}", self.text(f));
        }
        let scale = self.kids(i).into_iter().find(|&k| k != mag && self.tag(k) != Tag::Frac).map(|k| self.tag(k));
        let (base, exp) = match m.split_once('e') {
            Some((b, e)) => (b.to_string(), e.parse::<i32>().ok()?),
            None => (m.clone(), 0),
        };
        let shift = match scale {
            Some(Tag::Mil) => return Some(m.parse::<f64>().ok()? * 25.4e-6),
            Some(Tag::Peta) => 15,
            Some(Tag::Tera) => 12,
            Some(Tag::Giga) => 9,
            Some(Tag::Meg) => 6,
            Some(Tag::Kilo) => 3,
            Some(Tag::Centi) => -2,
            Some(Tag::Milli) => -3,
            Some(Tag::Micro) => -6,
            Some(Tag::Nano) => -9,
            Some(Tag::Pico) => -12,
            Some(Tag::Femto) => -15,
            Some(Tag::Atto) => -18,
            _ => 0,
        };
        format!("{base}e{}", exp + shift).parse().ok()
    }

    /// The number a value row holds when it is one: a `Num`, or a `Word`/
    /// `Val` that is all one `Num`.
    fn number(&self, i: usize) -> Option<f64> {
        match self.tag(i) {
            Tag::Num => self.num(i),
            Tag::Word | Tag::Val | Tag::MVal | Tag::Expr => {
                let k = self.kids(i);
                let t = self.text(i).trim();
                match k.as_slice() {
                    [n] if self.text(*n) == t => self.number(*n),
                    // A signed literal where the sign is an operator (Spectre,
                    // VACASK): `-1.5`.
                    [n] if t.strip_prefix('-').is_some_and(|r| r.trim_start() == self.text(*n)) => self.number(*n).map(|x| -x),
                    [n] if t.strip_prefix('+').is_some_and(|r| r.trim_start() == self.text(*n)) => self.number(*n),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn count(&self, i: usize) -> Option<u32> {
        self.number(i).filter(|v| *v >= 0.0 && v.fract() == 0.0).map(|v| v as u32)
    }

    /// A value field: a number, an expression (braced, quoted, or a bare
    /// word, which SPICE reads as a parameter), or text kept raw.
    fn value(&self, i: usize) -> IrValue {
        if let Some(value) = self.number(i) {
            return IrValue::Numeric { value };
        }
        let text = self.text(i).trim();
        match self.kids(i).as_slice() {
            [e] if matches!(self.tag(*e), Tag::Expr) => IrValue::Expression { expr: self.respell(self.text(*e).trim()) },
            [s] if self.tag(*s) == Tag::Str => IrValue::Expression { expr: trim(&text[1..text.len() - 1]) },
            _ if text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') => IrValue::Expression { expr: text.to_string() },
            _ => IrValue::Raw { text: text.to_string() },
        }
    }

    fn irnum(&self, i: usize) -> Option<IrNum> {
        match self.value(i) {
            IrValue::Numeric { value } => Some(IrNum::Num(value)),
            IrValue::Expression { expr } => Some(IrNum::Expr(expr)),
            IrValue::Raw { .. } => None,
        }
    }

    /// `k=v` rows (and bare flags, `(word, "")`) in card order.
    fn params(&self, i: usize) -> Vec<(String, String)> {
        self.kids(i)
            .into_iter()
            .filter_map(|k| match self.tag(k) {
                Tag::Pair | Tag::MPair => {
                    let kv = self.kids(k);
                    Some((self.text(kv[0]).to_string(), self.vtext(kv[1])))
                }
                Tag::Flag => Some((self.text(k).to_string(), String::new())),
                _ => None,
            })
            .collect()
    }

    /// A value's text; a vector's further numbers (`More`) joined to it by
    /// one blank (`r=0.03  0` is `0.03 0`, `ic=0, -5` is `0,-5`).
    fn vtext(&self, v: usize) -> String {
        let more = self.all(v, Tag::More);
        if more.is_empty() {
            // A Spectre/VACASK number in the IR's SPICE spelling: `1M` must
            // not reach a SPICE reader as milli.
            if matches!(self.d, Dialect::Spectre | Dialect::Vacask) && let Some(x) = self.number(v) {
                return crate::codegen::format_spice_number(x);
            }
            return self.respell(self.text(v).trim());
        }
        // A comma stays a comma; a blank is one blank in a model's list and a
        // comma in an instance's (SPICE's comma vector).
        let blank = if self.tag(v) == Tag::Val { "," } else { " " };
        let mut out = String::new();
        let mut at = self.t.rows[v].start as usize;
        for m in more {
            let r = self.t.rows[m];
            let gap = &self.src[at..r.start as usize];
            let head = gap.trim_end();
            let sep = if head.ends_with(',') { "," } else { blank };
            out.push_str(head.trim_end_matches(',').trim_end());
            out.push_str(sep);
            out.push_str(self.text(m));
            at = r.end as usize;
        }
        out
    }

    /// LTspice's `µ` is `u` (XVII help) in the text a SPICE reader gets.
    fn respell(&self, s: &str) -> String {
        if self.d == Dialect::Ltspice { s.replace('µ', "u") } else { s.to_string() }
    }

    /// A body kept whole (`B`, `.func`): its expression without delimiters.
    fn body(&self, i: usize) -> String {
        match self.kid(i, Tag::Expr) {
            Some(e) => trim(self.text(e)),
            None => trim(self.text(i)),
        }
    }

    // ── Cards ──

    fn card(&mut self, card: usize, here: &mut Subcircuit, inside: bool) {
        let named = matches!(self.d, Dialect::Spectre | Dialect::Vacask);
        if named && matches!(self.tag(card), Tag::Load | Tag::Ground | Tag::Model | Tag::Save | Tag::SpiceRegion | Tag::ControlBlock | Tag::Ctl | Tag::Ana | Tag::Inst | Tag::Embed) {
            let (kinds, subckts) = (std::mem::take(&mut self.kinds), std::mem::take(&mut self.subckts));
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
                Some(sec) => here.libs.push((self.path(card), self.text(sec).to_string())),
                None => here.includes.push(self.path(card)),
            },
            Tag::Lib => {
                let sec = self.kid(card, Tag::Section).map_or(String::new(), |s| self.text(s).to_string());
                here.libs.push((self.path(card), sec));
            }
            Tag::Global => {
                for n in self.all(card, Tag::Node).into_iter().map(|n| self.name(n)).collect::<Vec<_>>() {
                    if !self.globals.contains(&n) {
                        self.globals.push(n);
                    }
                }
            }
            Tag::Model => {
                let name = self.text(self.kids(card)[0]).to_string();
                let kind = self.text(self.kids(card)[1]).to_string();
                let mut m = ModelDef { name, kind, parameters: self.params(card) };
                if self.d == Dialect::Xyce && matches!(m.kind.to_ascii_lowercase().as_str(), "nmos" | "pmos") {
                    // Xyce's BSIM3 is level 9 (RG Table 2-36), ngspice's MOS9:
                    // the IR keeps BSIM3 as 49, which both read.
                    for (k, v) in &mut m.parameters {
                        if k.eq_ignore_ascii_case("level") && matches!(v.parse::<f64>(), Ok(8.0 | 9.0)) {
                            *v = "49".into();
                        }
                    }
                }
                here.models.push(m);
            }
            Tag::X => match self.instance(card) {
                Some(i) => here.instances.push(Instance { at: Some(here.components.len() as u32), ..i }),
                None => self.raw(card, here),
            },
            // A SPICE `.subckt`'s own `.param`s and functions are not the
            // IR's; Spectre's and VACASK's `parameters` are the definition's.
            Tag::Params | Tag::Func if inside && !(named && self.tag(card) == Tag::Params) => self.raw(card, here),
            Tag::Params => {
                for k in self.kids(card) {
                    match self.tag(k) {
                        Tag::FuncDef => {
                            let f = self.funcdef(k);
                            self.functions.push(f);
                        }
                        Tag::Assign => {
                            let kv = self.kids(k);
                            here.parameters.push(ParamDef { name: self.text(kv[0]).to_string(), default: Some(self.vtext(kv[1])) });
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
        let body = self.kid(i, Tag::Body).map_or(String::new(), |b| self.body(b));
        FuncDef { name, args: self.texts(i, Tag::Arg), body }
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
                Tag::Port => sc.ports.push(Port { name: self.name(k), direction: PortDirection::InOut }),
                Tag::Assign => {
                    let kv = self.kids(k);
                    sc.parameters.push(ParamDef { name: self.text(kv[0]).to_string(), default: Some(self.text(kv[1]).trim().to_string()) });
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
        for inst in sc.instances.iter_mut().chain(self.defs[first..].iter_mut().flat_map(|d| d.instances.iter_mut())) {
            if let Some((_, to)) = renames.iter().find(|(from, _)| from.eq_ignore_ascii_case(&inst.subcircuit)) {
                inst.subcircuit = to.clone();
            }
        }
        sc
    }

    /// A `.control` line ngspice runs as an analysis: read as its dot card.
    fn control(&mut self, cmd: usize) {
        let line = format!(".{}\n", self.text(cmd));
        let folded = line.to_ascii_lowercase();
        let Some(one) = self.lang.g.rule("one") else { return };
        let Ok(t) = self.lang.g.parse(one, &folded) else { return };
        let Some(card) = t.children(None).next() else { return };
        let mut cx = Cx { t: &t, src: &line, d: self.d, lang: self.lang, models: HashSet::new(), report: Report::default(), tb: std::mem::take(&mut self.tb), globals: vec![], functions: vec![], defs: vec![], kinds: HashMap::new(), subckts: vec![] };
        cx.testbench_card(card);
        self.tb = cx.tb;
    }

    /// A testbench card (analysis, option, output), applied; `false` if the
    /// IR cannot hold it.
    fn testbench_card(&mut self, card: usize) -> bool {
        let tag = self.tag(card);
        if let Some(a) = self.analysis(card, tag) {
            self.tb.analyses.push(a);
            return true;
        }
        match tag {
            // Xyce: `.options <pkg> k=v` (RG p.96), kept per package so writing
            // Xyce regroups them; DEVICE TEMP/TNOM are the temperatures.
            Tag::Options => match (self.d, self.kid(card, Tag::Pkg)) {
                (Dialect::Xyce, Some(pkg)) => {
                    let pkg = self.text(pkg).to_ascii_lowercase();
                    let p = self.params(card);
                    if p.is_empty() {
                        self.tb.options.backend_specific.entry("xyce".into()).or_default().push((format!("{pkg}."), String::new()));
                    }
                    let nums: Vec<Option<f64>> = self.kids(card).into_iter().filter(|&k| matches!(self.tag(k), Tag::Pair | Tag::Flag)).map(|k| match self.tag(k) {
                        Tag::Pair => self.number(self.kids(k)[1]),
                        _ => None,
                    }).collect();
                    for ((k, v), x) in p.into_iter().zip(nums) {
                        match (pkg.as_str(), k.to_ascii_lowercase().as_str(), x) {
                            ("device", "temp", Some(x)) => self.tb.temperature = Some(x),
                            ("device", "tnom", Some(x)) => self.tb.nominal_temperature = Some(x),
                            _ => self.tb.options.backend_specific.entry("xyce".into()).or_default().push((format!("{pkg}.{}", k.to_ascii_lowercase()), v)),
                        }
                    }
                }
                (Dialect::Xyce, None) => {
                    let p = self.params(card);
                    self.tb.options.backend_specific.entry("xyce".into()).or_default().extend(p);
                }
                _ => {
                    let p = self.params(card);
                    self.tb.options.portable.extend(p);
                }
            },
            Tag::Probe => {
                let outs = self.texts(card, Tag::Out);
                self.tb.saves.extend(outs);
            }
            Tag::Step => {
                let k = self.all(card, Tag::Num);
                let sweep_type = self.kid(card, Tag::StepKind).map(|s| self.text(s).to_ascii_lowercase());
                let step = StepParam { param: self.text(self.kid(card, Tag::Name).unwrap()).to_string(), start: self.num(k[0]).unwrap_or(f64::NAN), stop: self.num(k[1]).unwrap_or(f64::NAN), step: self.num(k[2]).unwrap_or(f64::NAN), sweep_type };
                if step.start.is_nan() || step.stop.is_nan() || step.step.is_nan() {
                    return false;
                }
                self.tb.step_params.push(step);
            }
            Tag::Temp => match self.kid(card, Tag::Val).and_then(|v| self.number(v)) {
                Some(t) => self.tb.temperature = Some(t),
                None => return false,
            },
            Tag::Ic | Tag::Nodeset => {
                let v: Option<Vec<(String, f64)>> = self.all(card, Tag::NodeVal).into_iter().map(|nv| {
                    let k = self.kids(nv);
                    Some((self.name(k[0]), self.number(k[1])?))
                }).collect();
                match (v, tag) {
                    (Some(v), Tag::Ic) => self.tb.initial_conditions.extend(v),
                    (Some(v), _) => self.tb.node_sets.extend(v),
                    (None, _) => return false,
                }
            }
            Tag::Save => {
                let outs = self.texts(card, Tag::Out);
                // A lone `.save all` is what no save list means.
                if !outs.iter().all(|x| x.eq_ignore_ascii_case("all")) {
                    self.tb.saves.extend(outs);
                }
            }
            Tag::Print => {
                let analysis = self.kid(card, Tag::PrintKind).map_or(String::new(), |k| self.text(k).to_ascii_lowercase());
                self.tb.prints.push(PrintCard { analysis, outputs: self.texts(card, Tag::Out) });
            }
            Tag::Meas => self.tb.measures.push(self.text(card).to_string()),
            _ => return false,
        }
        true
    }

    fn analysis(&self, card: usize, tag: Tag) -> Option<Analysis> {
        let k = self.kids(card);
        let n = |i: usize| k.get(i).and_then(|&r| self.number(r));
        let word = |i: usize| k.get(i).map(|&r| self.text(r).to_string());
        Some(match tag {
            Tag::Op => Analysis::Op,
            Tag::Dc => Analysis::Dc {
                sweeps: k.iter().map(|&s| {
                    let w = self.kids(s);
                    Some(DcSweep { source: self.text(w[0]).to_string(), start: self.number(w[1])?, stop: self.number(w[2])?, step: self.number(w[3])? })
                }).collect::<Option<_>>()?,
            },
            Tag::Ac => Analysis::Ac { variation: word(0)?.to_ascii_lowercase(), points: self.count(k[1])?, start: n(2)?, stop: n(3)? },
            Tag::Tran => {
                let uic = self.kid(card, Tag::Uic).is_some();
                let p: Vec<f64> = self.all(card, Tag::Num).into_iter().map(|r| self.num(r)).collect::<Option<_>>()?;
                // HSPICE `START=` names the start.
                let start = match self.kid(card, Tag::Pair) {
                    Some(pr) if self.text(self.kids(pr)[0]).eq_ignore_ascii_case("start") => Some(self.number(self.kids(pr)[1])?),
                    Some(_) => return None,
                    None => None,
                };
                match p.as_slice() {
                    // LTspice `.tran Tstop` (SCR .TRAN): the step is LTspice's own.
                    [stop] => Analysis::Transient { step: 0.0, stop: *stop, start: None, max_step: None, uic },
                    [step, stop] if start.is_some() => Analysis::Transient { step: *step, stop: *stop, start, max_step: None, uic },
                    [step, stop, rest @ ..] if rest.len() <= 2 => Analysis::Transient { step: *step, stop: *stop, start: rest.first().copied(), max_step: rest.get(1).copied(), uic },
                    _ => return None,
                }
            }
            Tag::Noise => {
                let nodes = self.texts(card, Tag::Node);
                let nums = self.all(card, Tag::Num);
                Analysis::Noise {
                    output: nodes.first()?.clone(),
                    reference: nodes.get(1).cloned().unwrap_or_else(|| "0".into()),
                    source: self.text(self.kid(card, Tag::Name)?).to_string(),
                    variation: self.text(self.kid(card, Tag::Word)?).to_ascii_lowercase(),
                    points: self.count(nums[0])?,
                    start: self.num(nums[1])?,
                    stop: self.num(nums[2])?,
                    points_per_summary: nums.get(3).and_then(|&r| self.text(r).parse().ok()),
                }
            }
            Tag::Tf => Analysis::Tf { output: word(0)?, source: word(1)? },
            Tag::Sens => match k.as_slice() {
                [out] => Analysis::Sensitivity { output: self.text(*out).to_string(), ac: None },
                [out, var, pts, a, b] => Analysis::Sensitivity {
                    output: self.text(*out).to_string(),
                    ac: Some(AcSweepParams { variation: self.text(*var).to_ascii_lowercase(), points: self.count(*pts)?, start: self.number(*a)?, stop: self.number(*b)? }),
                },
                _ => return None,
            },
            Tag::Pz => Analysis::PoleZero { node1: word(0)?, node2: word(1)?, node3: word(2)?, node4: word(3)?, tf_type: word(4)?.to_ascii_lowercase(), pz_type: word(5)?.to_ascii_lowercase() },
            Tag::Four => Analysis::Fourier { fundamental: n(0)?, outputs: self.texts(card, Tag::Out), num_harmonics: None },
            _ => return None,
        })
    }

    fn instance(&self, card: usize) -> Option<Instance> {
        let name = self.text(self.kid(card, Tag::Name)?).to_string();
        let subcircuit = self.text(self.kid(card, Tag::Mname)?).to_string();
        Some(Instance { name, subcircuit, port_mapping: self.texts(card, Tag::Node), parameters: self.params(card), at: None })
    }

    fn waveform(&self, w: usize) -> Option<IrWaveform> {
        let a: Vec<IrNum> = self.all(w, Tag::Val).into_iter().map(|v| self.irnum(v)).collect::<Option<_>>()?;
        let g = |i: usize| a.get(i).cloned().unwrap_or(IrNum::Num(0.0));
        Some(match self.tag(w) {
            Tag::Sin => IrWaveform::Sin { offset: g(0), amplitude: g(1), frequency: g(2), delay: g(3), damping: g(4), phase: g(5) },
            Tag::Pulse => IrWaveform::Pulse { initial: g(0), pulsed: g(1), delay: g(2), rise_time: g(3), fall_time: g(4), pulse_width: g(5), period: g(6) },
            Tag::Pwl if a.len().is_multiple_of(2) => IrWaveform::Pwl { values: a.chunks(2).map(|p| (p[0].clone(), p[1].clone())).collect() },
            Tag::Exp => IrWaveform::Exp { initial: g(0), pulsed: g(1), rise_delay: g(2), rise_tau: g(3), fall_delay: g(4), fall_tau: g(5) },
            Tag::Sffm => IrWaveform::Sffm { offset: g(0), amplitude: g(1), carrier_freq: g(2), modulation_index: g(3), signal_freq: g(4) },
            Tag::Am => IrWaveform::Am { amplitude: g(0), offset: g(1), modulating_freq: g(2), carrier_freq: g(3), delay: g(4) },
            _ => return None,
        })
    }

    /// One element card, or `None` when the IR has no faithful form for it.
    fn component(&self, card: usize) -> Option<Component> {
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
                let first_pair = self.kids(card).into_iter().position(|x| self.tag(x) == Tag::Pair);
                if self.kids(card).into_iter().position(|x| self.tag(x) == Tag::Flag).is_some_and(|f| first_pair.is_none_or(|p| f < p)) {
                    return None;
                }
                let mut params = self.params(card);
                let mut model = None;
                let value = match self.kid(card, Tag::Val) {
                    // `R1 a b rmod l=10u`: a bare word naming a `.model`, not a
                    // value; the value field is left blank.
                    Some(v) if self.kids(v).iter().all(|&w| self.tag(w) == Tag::Word && self.number(w).is_none()) && self.models.contains(&self.text(v).to_ascii_lowercase()) => {
                        model = Some(self.text(v).to_string());
                        IrValue::Raw { text: String::new() }
                    }
                    Some(v) => self.value(v),
                    // ngspice `R1 a b r=1k`
                    None => {
                        let key = format!("{tag:?}").to_ascii_lowercase();
                        let i = params.iter().position(|(k, _)| k.eq_ignore_ascii_case(&key))?;
                        let pair = self.all(card, Tag::Pair)[i];
                        params.remove(i);
                        self.value(self.kids(pair)[1])
                    }
                };
                let (n1, n2) = (node(0)?, node(1)?);
                match tag {
                    Tag::R => Component::Resistor { name, n1, n2, value, model, params },
                    Tag::C => Component::Capacitor { name, n1, n2, value, model, params },
                    _ => Component::Inductor { name, n1, n2, value, model, params },
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
                let (mut value, mut dc_given, mut ac_magnitude, mut ac_phase, mut wf) = (IrValue::Numeric { value: 0.0 }, false, None, None, None);
                for &f in &k[3..] {
                    match self.tag(f) {
                        Tag::Val => (value, dc_given) = (self.value(f), true),
                        Tag::Sdc => {
                            if let Some(v) = self.kid(f, Tag::Val) {
                                (value, dc_given) = (self.value(v), true);
                            }
                        }
                        Tag::Sac => {
                            let n = self.all(f, Tag::Num);
                            ac_magnitude = Some(n.first().and_then(|&r| self.num(r)).unwrap_or(1.0));
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
                match tag {
                    Tag::V => Component::VoltageSource { name, np, nm, value, ac_magnitude, ac_phase, waveform: wf },
                    _ => Component::CurrentSource { name, np, nm, value, ac_magnitude, ac_phase, waveform: wf },
                }
            }
            Tag::Poly => {
                let letter = self.text(card).chars().next()?.to_ascii_uppercase();
                let dims = self.count(self.kid(card, Tag::Num)?)? as usize;
                let n_controls = if matches!(letter, 'E' | 'G') { 2 * dims } else { dims };
                let rest: Vec<usize> = self.all(card, Tag::Word);
                let controls = rest.get(..n_controls)?.iter().map(|&w| self.text(w).to_string()).collect();
                let coeffs = rest[n_controls..].iter().map(|&w| self.number(w)).collect::<Option<_>>()?;
                Component::Poly { name, letter, np: node(0)?, nm: node(1)?, controls, coeffs }
            }
            Tag::E | Tag::G => {
                let (np, nm) = (node(0)?, node(1)?);
                // `value={expr}` / `vol='expr'` / `cur='expr'`
                if let Some(v) = self.kid(card, Tag::Val) {
                    let expression = self.body(v);
                    return Some(match tag {
                        Tag::E => Component::BehavioralVoltage { name, np, nm, expression },
                        _ => Component::BehavioralCurrent { name, np, nm, expression },
                    });
                }
                let (ncp, ncm, g) = (node(2)?, node(3)?, self.number(self.kid(card, Tag::Num)?)?);
                match tag {
                    Tag::E => Component::Vcvs { name, np, nm, ncp, ncm, gain: g },
                    _ => Component::Vccs { name, np, nm, ncp, ncm, transconductance: g },
                }
            }
            Tag::F | Tag::H => {
                let (np, nm, vsense, g) = (node(0)?, node(1)?, self.text(k[3]).to_string(), self.number(self.kid(card, Tag::Num)?)?);
                match tag {
                    Tag::F => Component::Cccs { name, np, nm, vsense, gain: g },
                    _ => Component::Ccvs { name, np, nm, vsense, transresistance: g },
                }
            }
            // The expression is the card's own text after `V=`/`I=`.
            Tag::B => {
                let (np, nm) = (node(0)?, node(1)?);
                let expression = self.body(self.kid(card, Tag::Body)?);
                match self.text(self.kid(card, Tag::Key)?).to_ascii_lowercase().as_str() {
                    "v" => Component::BehavioralVoltage { name, np, nm, expression },
                    "i" => Component::BehavioralCurrent { name, np, nm, expression },
                    _ => return None,
                }
            }
            Tag::D | Tag::Q | Tag::M | Tag::J | Tag::Z => {
                let (model, params) = (model?, self.params(card));
                match (tag, nodes.as_slice()) {
                    (Tag::D, [np, nm]) => Component::Diode { name, np: np.clone(), nm: nm.clone(), model, params },
                    // `Q c b e [s [t]] model`: substrate, then thermal node.
                    (Tag::Q, [nc, nb, ne, extra @ ..]) if extra.len() <= 2 => Component::Bjt { name, nc: nc.clone(), nb: nb.clone(), ne: ne.clone(), extra_nodes: extra.to_vec(), model, params },
                    // `M d g s [b [more...]] model`: 3 nodes (VDMOS) to 7 (SOI).
                    (Tag::M, [nd, ng, ns, rest @ ..]) if rest.len() <= 4 => Component::Mosfet { name, nd: nd.clone(), ng: ng.clone(), ns: ns.clone(), nb: rest.first().cloned(), extra_nodes: rest.iter().skip(1).cloned().collect(), model, params },
                    (Tag::J, [nd, ng, ns]) => Component::Jfet { name, nd: nd.clone(), ng: ng.clone(), ns: ns.clone(), model, params },
                    (Tag::Z, [nd, ng, ns]) => Component::Mesfet { name, nd: nd.clone(), ng: ng.clone(), ns: ns.clone(), model, params },
                    _ => return None,
                }
            }
            Tag::S => Component::VSwitch { name, np: node(0)?, nm: node(1)?, ncp: node(2)?, ncm: node(3)?, model: model? },
            Tag::W => Component::ISwitch { name, np: node(0)?, nm: node(1)?, vcontrol: self.text(k[3]).to_string(), model: model? },
            Tag::T => {
                let p = self.params(card);
                let get = |key: &str| self.all(card, Tag::Pair).into_iter().find(|&pr| self.text(self.kids(pr)[0]).eq_ignore_ascii_case(key)).and_then(|pr| self.number(self.kids(pr)[1]));
                if p.len() != 2 {
                    return None;
                }
                Component::TLine { name, inp: node(0)?, inm: node(1)?, outp: node(2)?, outm: node(3)?, z0: get("z0")?, td: get("td")? }
            }
            _ => return None,
        })
    }
}

fn is_element(t: Tag) -> bool {
    use Tag::*;
    matches!(t, R | C | L | K | V | I | Poly | E | G | F | H | B | D | Q | M | J | Z | S | W | T)
}

// ── Spectre and VACASK: statements led by a name, read by master ──

/// The keys one dialect's built-in masters use: the inverse of
/// `codegen::spectre` and `codegen::vacask`.
struct Keys {
    /// Keys a source's value and waveform consume; any other key is a field
    /// the IR's source cannot hold.
    source: &'static [&'static str],
    /// The sine's damping key (`damp`, `theta`).
    damp: &'static str,
    gm: &'static str,
    rm: &'static str,
    /// The controlling source of a cccs/ccvs (`probe`, `ctlinst`).
    probe: &'static str,
    /// The mutual inductor's master and coupling key.
    mutual: &'static str,
    coupling: &'static str,
}

const SPECTRE: Keys = Keys {
    source: &["dc", "mag", "phase", "type", "val0", "val1", "delay", "rise", "fall", "width", "period", "sinedc", "ampl", "freq", "damp", "sinephase", "fmmodindex", "fmmodfreq", "td1", "tau1", "td2", "tau2", "wave"],
    damp: "damp",
    gm: "gm",
    rm: "rm",
    probe: "probe",
    mutual: "mutual_inductor",
    coupling: "coupling",
};

const VACASK: Keys = Keys {
    source: &["dc", "mag", "phase", "type", "val0", "val1", "delay", "rise", "fall", "width", "period", "sinedc", "ampl", "freq", "theta", "modindex", "modfreq", "tau1", "td2", "tau2", "wave"],
    damp: "theta",
    gm: "gain",
    rm: "gain",
    probe: "ctlinst",
    mutual: "mutual",
    coupling: "k",
};

/// The module names `codegen::vacask` declares itself (`model resistor
/// sp_resistor`, `model vsource vsource`): implied by the devices.
const VACASK_BUILTINS: &[&str] = &["resistor", "capacitor", "inductor", "vsource", "isource", "vcvs", "vccs", "cccs", "ccvs", "mutual"];

/// A Spectre model master (and `type=`) -> SPICE `.model` kind and LEVEL;
/// the inverse of `codegen::spectre`'s map (ESPice `spectre.zig:433-451`).
/// A master not here is a Spectre-native model, kept as its kind.
fn spectre_kind(master: &str, ty: Option<&str>) -> (String, Option<&'static str>) {
    let ty = ty.map(str::to_ascii_lowercase);
    let mos = |l: &'static str| (if ty.as_deref() == Some("p") { "pmos" } else { "nmos" }.to_string(), Some(l));
    match master {
        "mos1" => mos("1"),
        "mos2" => mos("2"),
        "mos3" => mos("3"),
        "bsim3v3" => mos("49"),
        "bsim4" => mos("54"),
        "bjt" => (if ty.as_deref() == Some("pnp") { "pnp" } else { "npn" }.to_string(), None),
        "diode" => ("d".to_string(), None),
        other => (other.to_string(), None),
    }
}

/// `sp_<module>` (and `type=`) -> SPICE kind and LEVEL: the inverse of
/// `codegen::vacask`'s `mos_module`/`model_type`.
fn vacask_kind(module: &str, ty: Option<f64>) -> Option<(String, Option<&'static str>)> {
    let neg = ty.is_some_and(|t| t < 0.0);
    let mos = |l| Some((if neg { "pmos" } else { "nmos" }.to_string(), Some(l)));
    match module.strip_prefix("sp_")? {
        "mos1" => mos("1"),
        "mos2" => mos("2"),
        "mos3" => mos("3"),
        "mos6" => mos("6"),
        "mos9" => mos("9"),
        "bsim3v3" => mos("49"),
        "bsim4v8" => mos("54"),
        "diode" => Some(("d".into(), None)),
        "bjt" => Some((if neg { "pnp" } else { "npn" }.into(), None)),
        "jfet1" => Some((if neg { "pjf" } else { "njf" }.into(), None)),
        "mes1" => Some((if neg { "pmf" } else { "nmf" }.into(), None)),
        "vdmos" => Some(("vdmos".into(), None)),
        _ => None,
    }
}

/// The built-in a VACASK `model NAME <module>` names, when it is one or a
/// SPICE R/C/L: `vsource`, `sp_resistor` -> `resistor`.
fn vacask_builtin(module: &str) -> Option<&'static str> {
    let m = module.strip_prefix("sp_").unwrap_or(module);
    VACASK_BUILTINS.iter().find(|a| **a == m).copied()
}

/// `"text"` without its quotes.
fn unquoted(v: &str) -> Option<String> {
    Some(v.strip_prefix('"')?.strip_suffix('"')?.to_string())
}

impl Cx<'_> {
    fn keys(&self) -> &'static Keys {
        if self.d == Dialect::Vacask { &VACASK } else { &SPECTRE }
    }

    /// A name row's identifier: Spectre's `\x` escapes and VACASK's `'...'`
    /// parts are the bytes they spell.
    fn name(&self, i: usize) -> String {
        let t = self.text(i);
        match self.d {
            Dialect::Spectre => {
                let mut out = String::with_capacity(t.len());
                let mut it = t.chars();
                while let Some(c) = it.next() {
                    match c {
                        '\\' => out.extend(it.next()),
                        _ => out.push(c),
                    }
                }
                out
            }
            Dialect::Vacask => {
                let mut out = String::new();
                let mut it = t.chars().peekable();
                let mut quoted = false;
                while let Some(c) = it.next() {
                    match c {
                        '\'' if quoted && it.peek() == Some(&'\'') => {
                            it.next();
                            out.push('\'');
                        }
                        '\'' => quoted = !quoted,
                        _ => out.push(c),
                    }
                }
                out
            }
            _ => t.to_string(),
        }
    }

    /// `name`'s SPICE spelling: the letter `codegen` adds back is dropped
    /// (`r1 resistor` reads as `1`, written `r1` again; `load resistor` stays
    /// `load`). Spectre keeps case (`[KUN]` B.2); VACASK's names are lowered.
    fn strip(&self, name: &str, letter: char) -> String {
        let name = if self.d == Dialect::Vacask { name.to_lowercase() } else { name.to_string() };
        match name.chars().next() {
            Some(c) if c.eq_ignore_ascii_case(&letter) && name.len() > 1 => name[c.len_utf8()..].to_string(),
            _ => name,
        }
    }

    /// `k=v` rows of a statement, by key, values as written.
    fn pairs(&self, i: usize) -> Vec<(String, usize)> {
        self.all(i, Tag::Pair).into_iter().map(|p| (self.text(self.kids(p)[0]).to_string(), self.kids(p)[1])).collect()
    }

    fn get(&self, p: &[(String, usize)], k: &str) -> Option<usize> {
        p.iter().find(|(n, _)| n == k).map(|(_, v)| *v)
    }

    /// The deck's model kinds (`name -> kind`, or `=<builtin>` for a VACASK
    /// alias) and subcircuit names, wherever they sit: instances may come
    /// before both.
    fn declarations(&self) -> (std::collections::HashMap<String, String>, Vec<String>) {
        let (mut kinds, mut subckts) = (std::collections::HashMap::new(), Vec::new());
        for i in 0..self.t.rows.len() {
            match self.tag(i) {
                Tag::Model => {
                    let k = self.kids(i);
                    let (name, master) = (self.name(k[0]), self.name(k[1]));
                    let p = self.pairs(i);
                    let ty = self.get(&p, "type");
                    if self.d == Dialect::Vacask {
                        match vacask_kind(&master, ty.and_then(|t| self.number(t))) {
                            Some((kind, _)) => {
                                kinds.insert(name, kind);
                            }
                            None => {
                                if let Some(b) = vacask_builtin(&master) {
                                    kinds.insert(name, format!("={b}"));
                                }
                            }
                        }
                    } else {
                        kinds.insert(name, spectre_kind(&master, ty.map(|t| self.text(t))).0);
                    }
                }
                Tag::Subckt => subckts.push(self.name(self.kids(i)[0])),
                _ => {}
            }
        }
        (kinds, subckts)
    }

    /// A Spectre or VACASK `.model`; `false` if the IR holds no such model.
    fn named_model(&mut self, card: usize, here: &mut Subcircuit) -> bool {
        let k = self.kids(card);
        let (name, master) = (self.name(k[0]), self.name(k[1]));
        let p = self.pairs(card);
        let (kind, level) = if self.d == Dialect::Vacask {
            if vacask_builtin(&master).is_some() {
                return true; // implied by the devices
            }
            match vacask_kind(&master, self.get(&p, "type").and_then(|t| self.number(t))) {
                Some(kl) => kl,
                None => return false,
            }
        } else {
            spectre_kind(&master, self.get(&p, "type").map(|t| self.text(t)))
        };
        // `type=` is the master's polarity once a SPICE kind says it.
        let drop_type = self.d == Dialect::Vacask || level.is_some() || kind != master;
        let mut parameters: Vec<(String, String)> = p.iter().filter(|(k, _)| !(drop_type && k == "type")).map(|(k, v)| (k.clone(), self.vtext(*v))).collect();
        if let Some(l) = level {
            parameters.insert(0, ("level".into(), l.into()));
        }
        here.models.push(ModelDef { name, kind, parameters });
        true
    }

    fn named_waveform(&self, p: &[(String, usize)]) -> Option<Option<IrWaveform>> {
        let keys = self.keys();
        let f = |k: &str| self.get(p, k).map_or(Some(IrNum::Num(0.0)), |v| self.irnum(v));
        let ty = self.get(p, "type").map(|t| self.text(t).trim_matches('"').to_string());
        Some(Some(match ty.as_deref() {
            None | Some("dc") => return Some(None),
            Some("pulse") => IrWaveform::Pulse { initial: f("val0")?, pulsed: f("val1")?, delay: f("delay")?, rise_time: f("rise")?, fall_time: f("fall")?, pulse_width: f("width")?, period: f("period")? },
            Some("sine") if self.get(p, "fmmodindex").is_some() => IrWaveform::Sffm { offset: f("sinedc")?, amplitude: f("ampl")?, carrier_freq: f("freq")?, modulation_index: f("fmmodindex")?, signal_freq: f("fmmodfreq")? },
            Some("sine") => IrWaveform::Sin { offset: f("sinedc")?, amplitude: f("ampl")?, frequency: f("freq")?, delay: f("delay")?, damping: f(keys.damp)?, phase: f("sinephase")? },
            // VACASK's td2 counts from `delay`; SPICE's is absolute, so the
            // two delays must be numbers to add.
            Some("exp") if self.d == Dialect::Vacask => {
                let n = |k: &str| self.get(p, k).map_or(Some(0.0), |v| self.number(v));
                IrWaveform::Exp { initial: f("val0")?, pulsed: f("val1")?, rise_delay: n("delay")?.into(), rise_tau: f("tau1")?, fall_delay: (n("delay")? + n("td2")?).into(), fall_tau: f("tau2")? }
            }
            Some("exp") => IrWaveform::Exp { initial: f("val0")?, pulsed: f("val1")?, rise_delay: f("td1")?, rise_tau: f("tau1")?, fall_delay: f("td2")?, fall_tau: f("tau2")? },
            Some("fm") => IrWaveform::Sffm { offset: f("sinedc")?, amplitude: f("ampl")?, carrier_freq: f("freq")?, modulation_index: f("modindex")?, signal_freq: f("modfreq")? },
            Some("am") => IrWaveform::Am { offset: f("sinedc")?, amplitude: f("ampl")?, carrier_freq: f("freq")?, modulating_freq: f("modfreq")?, delay: f("delay")? },
            Some("pwl") if self.d == Dialect::Spectre => {
                let w = self.get(p, "wave")?;
                let xs: Vec<IrNum> = self.velems(w).into_iter().map(|x| self.irnum(x)).collect::<Option<_>>()?;
                if !xs.len().is_multiple_of(2) {
                    return None;
                }
                IrWaveform::Pwl { values: xs.chunks(2).map(|c| (c[0].clone(), c[1].clone())).collect() }
            }
            Some(_) => return None,
        }))
    }

    /// A vector value's elements: the rows directly under its `Val`.
    fn velems(&self, v: usize) -> Vec<usize> {
        self.kids(v)
    }

    /// One instance statement as a component or subcircuit instance; `None`
    /// when the IR has no form for it.
    fn inst(&self, card: usize, kinds: &std::collections::HashMap<String, String>, subckts: &[String]) -> Option<Result<Component, Instance>> {
        let keys = self.keys();
        let name = self.name(self.kid(card, Tag::Name)?);
        let ns: Vec<String> = self.all(card, Tag::Node).into_iter().map(|n| self.name(n)).collect();
        let mut master = self.kid(card, Tag::Mname).map(|m| self.name(m))?;
        // `model v vsource` then `v1 (a 0) v ...`: an alias of a built-in.
        if let Some(b) = kinds.get(&master).and_then(|k| k.strip_prefix('=')) {
            master = b.to_string();
        }
        let p = self.pairs(card);
        let kept = |used: &[&str]| -> Vec<(String, String)> { p.iter().filter(|(k, _)| !used.contains(&k.as_str())).map(|(k, v)| (k.clone(), self.vtext(*v))).collect() };
        let two = || match ns.as_slice() { [a, b] => Some((a.clone(), b.clone())), _ => None };
        let num = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        // VACASK names a controlling instance by a string (`ctlinst="v1"`).
        let ident = |k: &str| self.get(&p, k).map(|v| { let t = self.text(v); unquoted(t).unwrap_or_else(|| t.to_string()) });
        Some(Ok(match master.as_str() {
            "resistor" | "capacitor" | "inductor" => {
                let (n1, n2) = two()?;
                let (letter, key) = match master.as_str() { "resistor" => ('r', "r"), "capacitor" => ('c', "c"), _ => ('l', "l") };
                let (name, value, params) = (self.strip(&name, letter), self.value(self.get(&p, key)?), kept(&[key]));
                match letter {
                    'r' => Component::Resistor { name, n1, n2, value, model: None, params },
                    'c' => Component::Capacitor { name, n1, n2, value, model: None, params },
                    _ => Component::Inductor { name, n1, n2, value, model: None, params },
                }
            }
            "vsource" | "isource" => {
                let (np, nm) = two()?;
                if p.iter().any(|(k, _)| !keys.source.contains(&k.as_str())) {
                    return None;
                }
                let waveform = self.named_waveform(&p)?;
                let value = self.get(&p, "dc").map_or(IrValue::Numeric { value: 0.0 }, |v| self.value(v));
                let (ac_magnitude, ac_phase) = (num("mag"), num("phase"));
                if master == "vsource" {
                    Component::VoltageSource { name: self.strip(&name, 'v'), np, nm, value, ac_magnitude, ac_phase, waveform }
                } else {
                    Component::CurrentSource { name: self.strip(&name, 'i'), np, nm, value, ac_magnitude, ac_phase, waveform }
                }
            }
            "vcvs" | "vccs" => {
                let [np, nm, ncp, ncm] = ns.as_slice() else { return None };
                let (np, nm, ncp, ncm) = (np.clone(), nm.clone(), ncp.clone(), ncm.clone());
                if master == "vcvs" {
                    Component::Vcvs { name: self.strip(&name, 'e'), np, nm, ncp, ncm, gain: num("gain")? }
                } else {
                    Component::Vccs { name: self.strip(&name, 'g'), np, nm, ncp, ncm, transconductance: num(keys.gm)? }
                }
            }
            "cccs" | "ccvs" => {
                let (np, nm) = two()?;
                let vsense = ident(keys.probe)?;
                if master == "cccs" {
                    Component::Cccs { name: self.strip(&name, 'f'), np, nm, vsense, gain: num("gain")? }
                } else {
                    Component::Ccvs { name: self.strip(&name, 'h'), np, nm, vsense, transresistance: num(keys.rm)? }
                }
            }
            m if m == keys.mutual && (self.d == Dialect::Spectre || ns.is_empty()) => Component::MutualInductor {
                name: self.strip(&name, 'k'),
                inductor1: self.strip(&ident("ind1")?, 'l'),
                inductor2: self.strip(&ident("ind2")?, 'l'),
                coupling: num(keys.coupling)?,
            },
            "pvcvs" | "pvccs" | "pcccs" | "pccvs" if self.d == Dialect::Spectre => {
                let letter = match master.as_str() { "pvcvs" => 'E', "pvccs" => 'G', "pcccs" => 'F', _ => 'H' };
                let list = |k: &str| self.get(&p, k).map(|v| self.velems(v));
                let coeffs: Vec<f64> = list("coeffs")?.into_iter().map(|c| self.number(c)).collect::<Option<_>>()?;
                let (np, nm) = (ns.first()?.clone(), ns.get(1)?.clone());
                let controls = if matches!(letter, 'E' | 'G') { ns[2..].to_vec() } else { list("probes")?.into_iter().map(|c| self.name(c)).collect() };
                Component::Poly { name: self.strip(&name, letter.to_ascii_lowercase()), letter, np, nm, controls, coeffs }
            }
            m if subckts.iter().any(|s| s == m) => {
                return Some(Err(Instance { name: self.strip(&name, 'x'), subcircuit: m.to_string(), port_mapping: ns, parameters: kept(&[]), at: None }));
            }
            m => {
                let kind = kinds.get(m)?.as_str();
                let (model, params) = (m.to_string(), kept(&[]));
                match (kind, ns.as_slice()) {
                    ("d", [np, nm]) => Component::Diode { name: self.strip(&name, 'd'), np: np.clone(), nm: nm.clone(), model, params },
                    ("npn" | "pnp", [nc, nb, ne, extra @ ..]) if extra.len() <= 2 => Component::Bjt { name: self.strip(&name, 'q'), nc: nc.clone(), nb: nb.clone(), ne: ne.clone(), extra_nodes: extra.to_vec(), model, params },
                    ("nmos" | "pmos", [nd, ng, s, nb]) => Component::Mosfet { name: self.strip(&name, 'm'), nd: nd.clone(), ng: ng.clone(), ns: s.clone(), nb: Some(nb.clone()), extra_nodes: vec![], model, params },
                    ("njf" | "pjf", [nd, ng, s]) if self.d == Dialect::Vacask => Component::Jfet { name: self.strip(&name, 'j'), nd: nd.clone(), ng: ng.clone(), ns: s.clone(), model, params },
                    ("nmf" | "pmf", [nd, ng, s]) if self.d == Dialect::Vacask => Component::Mesfet { name: self.strip(&name, 'z'), nd: nd.clone(), ng: ng.clone(), ns: s.clone(), model, params },
                    _ => return None,
                }
            }
        }))
    }

    /// `name [(p n)] kind k=v`: the analyses `codegen::spectre` writes.
    fn spectre_analysis(&self, card: usize) -> Option<Analysis> {
        let kind = self.text(self.kid(card, Tag::Word)?);
        let ns: Vec<String> = self.all(card, Tag::Node).into_iter().map(|n| self.name(n)).collect();
        let p = self.pairs(card);
        let n = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let s = |k: &str| self.get(&p, k).map(|v| self.text(v).to_string());
        let sweep = || ["dec", "lin", "log"].iter().find_map(|v| n(v).map(|x| (v.to_string(), x as u32)));
        Some(match (kind, ns.as_slice()) {
            ("dc", []) if p.is_empty() => Analysis::Op,
            ("dc", []) => {
                let source = match (s("dev"), s("param")) {
                    (Some(dev), Some(param)) if param == "dc" => dev,
                    (None, Some(param)) => param,
                    _ => return None,
                };
                Analysis::Dc { sweeps: vec![DcSweep { source, start: n("start")?, stop: n("stop")?, step: n("step")? }] }
            }
            ("ac", []) => {
                let (variation, points) = sweep()?;
                Analysis::Ac { variation, points, start: n("start")?, stop: n("stop")? }
            }
            ("tran", []) => Analysis::Transient { step: n("step")?, stop: n("stop")?, start: n("start"), max_step: n("maxstep"), uic: s("skipdc").as_deref() == Some("yes") },
            ("noise", [out, reference]) => {
                let (variation, points) = sweep()?;
                Analysis::Noise { output: out.clone(), reference: reference.clone(), source: s("iprobe")?, variation, points, start: n("start")?, stop: n("stop")?, points_per_summary: None }
            }
            _ => return None,
        })
    }

    /// `name options k=v`: options, `temp`/`tnom`, and the inverse of
    /// `codegen::spectre::map_option_name`.
    fn spectre_options(&mut self, card: usize) {
        for (k, v) in self.pairs(card) {
            match (k.as_str(), self.number(v)) {
                ("temp", Some(x)) => self.tb.temperature = Some(x),
                ("tnom", Some(x)) => self.tb.nominal_temperature = Some(x),
                _ => {
                    let k = match k.as_str() { "iabstol" => "abstol", "vabstol" => "vntol", "dcmaxiters" => "max_iterations", other => other };
                    let v = self.vtext(v);
                    self.tb.options.portable.push((k.to_string(), v));
                }
            }
        }
    }

    /// `ic=["n"; v; ...]` (or `{"n", v, ...}`) -> node values.
    fn node_list(&self, v: usize) -> Option<Vec<(String, f64)>> {
        let mut items = self.kids(v);
        // The `Val` holds one `Expr`, whose rows are the list's elements.
        while let [one] = items.as_slice() {
            items = self.kids(*one);
        }
        items.chunks(2).map(|c| Some((unquoted(self.text(*c.first()?))?, self.number(*c.get(1)?)?))).collect()
    }

    /// One VACASK `control` block statement the IR models; `false` if not.
    fn vacask_control(&mut self, st: usize) -> bool {
        let p = self.pairs(st);
        let n = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let s = |k: &str| self.get(&p, k).and_then(|v| unquoted(self.text(v)));
        match self.tag(st) {
            Tag::Ctl if self.kid(st, Tag::Word).is_some_and(|w| self.text(w) == "options") => {
                for (k, v) in &p {
                    match (k.as_str(), self.number(*v)) {
                        ("rawfile", _) => {}
                        ("temp", Some(x)) => self.tb.temperature = Some(x),
                        ("tnom", Some(x)) => self.tb.nominal_temperature = Some(x),
                        // VACASK's own option names stay VACASK's, written back
                        // as they read (the portable names are SPICE's).
                        _ => {
                            let v = self.text(*v).to_string();
                            self.tb.options.backend_specific.entry("vacask".into()).or_default().push((k.clone(), v));
                        }
                    }
                }
                true
            }
            Tag::Ana => {
                // `sweep NAME instance="v1" parameter="dc" from= to= mode="lin" points=n`
                let sweeps: Option<Vec<DcSweep>> = self.all(st, Tag::Sweep).into_iter().rev().map(|sw| {
                    let p = self.pairs(sw);
                    let n = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
                    let s = |k: &str| self.get(&p, k).and_then(|v| unquoted(self.text(v)));
                    let (src, from, to, pts) = (s("instance")?, n("from")?, n("to")?, n("points")?);
                    (s("parameter").as_deref() == Some("dc")).then(|| DcSweep { source: src, start: from, stop: to, step: (to - from) / pts })
                }).collect();
                let Some(sweeps) = sweeps else { return false };
                let kind = self.kid(st, Tag::Word).map(|w| self.text(w).to_string()).unwrap_or_default();
                let freq = || -> Option<(String, u32, f64, f64)> {
                    let mode = s("mode")?;
                    let pts = n("points")? as u32 + u32::from(mode == "lin");
                    Some((mode, pts, n("from")?, n("to")?))
                };
                let a = match kind.as_str() {
                    "op" if !sweeps.is_empty() => Some(Analysis::Dc { sweeps }),
                    _ if !sweeps.is_empty() => None,
                    "op" => Some(Analysis::Op),
                    "ac" => freq().map(|(variation, points, start, stop)| Analysis::Ac { variation, points, start, stop }),
                    "tran" => (|| {
                        let ic = match self.get(&p, "ic") { Some(v) => self.node_list(v)?, None => vec![] };
                        let ns = match self.get(&p, "nodeset") { Some(v) => self.node_list(v)?, None => vec![] };
                        let a = Analysis::Transient { step: n("step")?, stop: n("stop")?, start: None, max_step: n("maxstep"), uic: s("icmode").as_deref() == Some("uic") };
                        Some((a, ic, ns))
                    })().map(|(a, ic, ns)| {
                        self.tb.initial_conditions.extend(ic);
                        self.tb.node_sets.extend(ns);
                        a
                    }),
                    "noise" => freq().and_then(|(variation, points, start, stop)| Some(Analysis::Noise { output: s("out")?, reference: "0".into(), source: s("in")?, variation, points, start, stop, points_per_summary: None })),
                    "dcxf" => s("out").map(|o| Analysis::Tf { output: o, source: String::new() }),
                    _ => None,
                };
                match a {
                    Some(a) => {
                        self.tb.analyses.push(a);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// A Spectre or VACASK statement; `false` when the IR holds no such one.
    fn named_card(&mut self, card: usize, here: &mut Subcircuit, inside: bool, kinds: &std::collections::HashMap<String, String>, subckts: &[String]) -> bool {
        match self.tag(card) {
            Tag::Load => {
                let path = self.path(card);
                // `spice/<m>.osdi` is what codegen::vacask loads for its devices.
                if !(self.d == Dialect::Vacask && path.starts_with("spice/")) {
                    here.osdi_loads.push(path);
                }
            }
            Tag::Ground => return self.kid(card, Tag::Node).is_none(),
            Tag::Model => return self.named_model(card, here),
            Tag::Save => {
                let outs: Vec<String> = self.all(card, Tag::Out).into_iter().map(|o| {
                    let s = self.name(o);
                    match s.split_once(':') {
                        Some((dev, "p")) => format!("i({dev})"),
                        _ => s,
                    }
                }).collect();
                self.tb.saves.extend(outs);
            }
            Tag::SpiceRegion => {
                for k in self.kids(card) {
                    match self.tag(k) {
                        Tag::Meas => self.tb.measures.push(self.text(k).to_string()),
                        Tag::Raw => {
                            self.report.raw += 1;
                            here.raw_spice.push(self.text(k).to_string());
                        }
                        _ => {}
                    }
                }
            }
            Tag::ControlBlock => {
                for st in self.kids(card) {
                    if !self.vacask_control(st) {
                        let text = self.text(st).to_string();
                        self.tb.extra_lines.extend(text.split('\n').map(str::to_string));
                    }
                }
            }
            Tag::Ctl if self.kid(card, Tag::Word).is_some_and(|w| self.text(w) == "options") => self.spectre_options(card),
            Tag::Ana if !inside => match self.spectre_analysis(card) {
                Some(a) => self.tb.analyses.push(a),
                None => return false,
            },
            Tag::Inst => match self.inst(card, kinds, subckts) {
                Some(Ok(c)) => here.components.push(c),
                Some(Err(i)) => here.instances.push(Instance { at: Some(here.components.len() as u32), ..i }),
                None => return false,
            },
            _ => return false,
        }
        true
    }
}
