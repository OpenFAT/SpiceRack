//! Spectre and VACASK statements (led by a name, read by master) -> the IR:
//! the dispatch over statement kinds, the deck's model and subcircuit
//! declarations, and `model` statements as `.model`s.
//!
//! Cites: ESPice `spectre.zig:433-451` (the Spectre master map, inverted);
//! `codegen::vacask`'s `mos_module`/`model_type` (inverted).
//!
//! Contents:
//! - `Cx::declarations(&self) -> (HashMap<String, String>, Vec<String>)`
//! - `Cx::named_card(&mut self, card: usize, here: &mut Subcircuit, top: bool,
//!   kinds: &HashMap<String, String>, subckts: &[String]) -> bool`

use super::Cx;
use crate::codegen::Dialect;
use crate::import::Tag;
use crate::ir::*;

/// The module names `codegen::vacask` declares itself (`model resistor
/// sp_resistor`, `model vsource vsource`): implied by the devices.
const VACASK_BUILTINS: &[&str] = &[
    "resistor",
    "capacitor",
    "inductor",
    "vsource",
    "isource",
    "vcvs",
    "vccs",
    "cccs",
    "ccvs",
    "mutual",
];

impl Cx<'_> {
    /// The deck's model kinds (`name -> kind`, or `=<builtin>` for a VACASK
    /// alias) and subcircuit names, wherever they sit: instances may come
    /// before both.
    pub(super) fn declarations(&self) -> (std::collections::HashMap<String, String>, Vec<String>) {
        let (mut kinds, mut subckts) = (std::collections::HashMap::new(), Vec::new());
        for i in 0..self.t.rows.len() {
            match self.tag(i) {
                Tag::Model => {
                    let k = self.kids(i);
                    let (name, master) = (self.name(k[0]), self.name(k[1]));
                    let mut p = self.pairs(i);
                    // A model group's polarity is its bins'.
                    p.extend(
                        self.kid(i, Tag::Bin)
                            .map(|b| self.pairs(b))
                            .unwrap_or_default(),
                    );
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

    /// A Spectre or VACASK statement; `false` when the IR holds no such one.
    pub(super) fn named_card(
        &mut self,
        card: usize,
        here: &mut Subcircuit,
        top: bool,
        kinds: &std::collections::HashMap<String, String>,
        subckts: &[String],
    ) -> bool {
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
                let outs: Vec<String> = self
                    .all(card, Tag::Out)
                    .into_iter()
                    .map(|o| {
                        let s = self.name(o);
                        match s.split_once(':') {
                            Some((dev, "p")) => format!("i({dev})"),
                            _ => s,
                        }
                    })
                    .collect();
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
                        self.tb
                            .extra_lines
                            .extend(text.split('\n').map(str::to_string));
                    }
                }
            }
            Tag::Ctl
                if self
                    .kid(card, Tag::Word)
                    .is_some_and(|w| self.text(w) == "options") =>
            {
                self.spectre_options(card)
            }
            Tag::Ana if top => match self.spectre_analysis(card) {
                Some(a) => self.tb.analyses.push(a),
                None => return false,
            },
            Tag::Inst => match self.inst(card, kinds, subckts) {
                Some(Ok(c)) => here.components.push(c),
                Some(Err(i)) => here.instances.push(Instance {
                    at: Some(here.components.len() as u32),
                    ..i
                }),
                None => return false,
            },
            _ => return false,
        }
        true
    }
}

// ---- private ----

impl Cx<'_> {
    /// A Spectre or VACASK `.model`, a Spectre model group one model per
    /// bin (`nch.1`, `nch.2`: SPICE's binning), its own parameters after the
    /// group's; `false` if the IR holds no such model.
    fn named_model(&mut self, card: usize, here: &mut Subcircuit) -> bool {
        let k = self.kids(card);
        let (name, master) = (self.name(k[0]), self.name(k[1]));
        let p = self.pairs(card);
        let bins = self.all(card, Tag::Bin);
        if bins.is_empty() {
            return self.one_model(name, &master, p, here);
        }
        bins.into_iter().all(|b| {
            let bin = self
                .text(b)
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            let mut p = p.clone();
            p.extend(self.pairs(b));
            self.one_model(format!("{name}.{bin}"), &master, p, here)
        })
    }

    /// The model `name` of `master` with the parameters `p`, onto `here`.
    fn one_model(
        &mut self,
        name: String,
        master: &str,
        p: Vec<(String, usize)>,
        here: &mut Subcircuit,
    ) -> bool {
        let (kind, level) = if self.d == Dialect::Vacask {
            if vacask_builtin(master).is_some() {
                return true; // implied by the devices
            }
            match vacask_kind(master, self.get(&p, "type").and_then(|t| self.number(t))) {
                Some(kl) => kl,
                None => return false,
            }
        } else {
            spectre_kind(master, self.get(&p, "type").map(|t| self.text(t)))
        };
        // `type=` is the master's polarity once a SPICE kind says it.
        let drop_type = self.d == Dialect::Vacask || level.is_some() || kind != master;
        let mut parameters: Vec<(String, String)> = p
            .iter()
            .filter(|(k, _)| !(drop_type && k == "type"))
            .map(|(k, v)| (k.clone(), self.vtext(*v)))
            .collect();
        if let Some(l) = level {
            parameters.insert(0, ("level".into(), l.into()));
        }
        here.models.push(ModelDef {
            name,
            kind,
            parameters,
        });
        true
    }
}

/// A Spectre model master (and `type=`) -> SPICE `.model` kind and LEVEL;
/// the inverse of `codegen::spectre`'s map (ESPice `spectre.zig:433-451`).
/// A master not here is a Spectre-native model, kept as its kind.
fn spectre_kind(master: &str, ty: Option<&str>) -> (String, Option<&'static str>) {
    let ty = ty.map(str::to_ascii_lowercase);
    let mos = |l: &'static str| {
        (
            if ty.as_deref() == Some("p") {
                "pmos"
            } else {
                "nmos"
            }
            .to_string(),
            Some(l),
        )
    };
    match master {
        "mos1" => mos("1"),
        "mos2" => mos("2"),
        "mos3" => mos("3"),
        "bsim3v3" => mos("49"),
        "bsim4" => mos("54"),
        "bjt" => (
            if ty.as_deref() == Some("pnp") {
                "pnp"
            } else {
                "npn"
            }
            .to_string(),
            None,
        ),
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
