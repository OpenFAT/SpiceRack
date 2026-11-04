//! Spectre and VACASK instance statements → [`Card`] views (by built-in
//! master or model kind, or a subcircuit instance), with their source
//! waveforms, as the old lowering typed them; a master another file of the
//! kit declares is read as `syntax::resolve` binds it.
//!
//! Cites: the old lowering's named elements;
//! Spectre Circuit Simulator Reference 5.0 `[CMP]` (`vsource` #4, #17, #18,
//! p.686); the old Spectre and VACASK writers (the key names, inverted).
//!
//! Contents:
//! - `fn unquoted(v: &str) -> Option<String>`
//! - `Rd::named_inst(&self, n: u32) -> Option<Card>`

use super::named::{spectre_kind, vacask_builtin, vacask_kind};
use super::{Card, Num, Rd, Val, Wave, WaveKind};
use crate::syntax::Dialect;
use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::resolve::{Binding, Def, DefKind, Polarity};

/// The keys one dialect's built-in masters use (the old lowering).
struct Keys {
    source: &'static [&'static str],
    damp: &'static str,
    gm: &'static str,
    rm: &'static str,
    probe: &'static str,
    mutual: &'static str,
    coupling: &'static str,
}

const SPECTRE: Keys = Keys {
    source: &[
        "dc",
        "mag",
        "phase",
        "type",
        "val0",
        "val1",
        "delay",
        "rise",
        "fall",
        "width",
        "period",
        "sinedc",
        "ampl",
        "freq",
        "damp",
        "sinephase",
        "fmmodindex",
        "fmmodfreq",
        "td1",
        "tau1",
        "td2",
        "tau2",
        "wave",
        "pwlperiod",
    ],
    damp: "damp",
    gm: "gm",
    rm: "rm",
    probe: "probe",
    mutual: "mutual_inductor",
    coupling: "coupling",
};

const VACASK: Keys = Keys {
    source: &[
        "dc", "mag", "phase", "type", "val0", "val1", "delay", "rise", "fall", "width", "period",
        "sinedc", "ampl", "freq", "theta", "modindex", "modfreq", "tau1", "td2", "tau2", "wave",
    ],
    damp: "theta",
    gm: "gain",
    rm: "gain",
    probe: "ctlinst",
    mutual: "mutual",
    coupling: "k",
};

/// What the kit declares a master as.
enum Declared {
    Subckt,
    /// A model, by the kind `Rd::declarations` would give it.
    Model(String),
}

/// `"text"` without its quotes.
pub(in crate::syntax::render) fn unquoted(v: &str) -> Option<String> {
    Some(v.strip_prefix('"')?.strip_suffix('"')?.to_string())
}

impl<'a> Rd<'a> {
    /// One instance statement as a typed card or a subcircuit instance;
    /// `None` when the IR had no form for it (the old lowering's `inst`).
    pub(in crate::syntax::render) fn named_inst(&self, n: u32) -> Option<Card> {
        let (kinds, subckts) = (&self.kinds, &self.subckts);
        let keys = self.keys();
        let h = self.ast.header(Node(n));
        let name = self.name_of(h.name);
        let ns: Vec<String> = h.nodes.iter().map(|&k| self.name_of(k)).collect();
        let m = h.master;
        if m == NONE {
            return None;
        }
        let mut master = self.name_of(m);
        // A master this file does not declare: the kit's declaration.
        let local = kinds.contains_key(&master) || subckts.contains(&master);
        let kit = if local { None } else { self.kit_master(n) };
        let kind_of = match &kit {
            Some(Declared::Model(k)) => Some(k.clone()),
            Some(Declared::Subckt) | None => kinds.get(&master).cloned(),
        };
        if let Some(b) = kind_of.as_deref().and_then(|k| k.strip_prefix('=')) {
            master = b.to_string();
        }
        let p = self.pairs(n);
        let kept = |used: &[&str]| -> Vec<(String, String)> {
            p.iter()
                .filter(|(k, _)| !used.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), self.vtext(*v)))
                .collect()
        };
        let two = || match ns.as_slice() {
            [a, b] => Some((a.clone(), b.clone())),
            _ => None,
        };
        let num = |k: &str| self.get(&p, k).and_then(|v| self.number(v));
        let ident = |k: &str| {
            self.get(&p, k).map(|v| {
                let t = self.text(v);
                unquoted(&t).unwrap_or(t)
            })
        };
        Some(match master.as_str() {
            "resistor" | "capacitor" | "inductor" => {
                let (n1, n2) = two()?;
                let (letter, key) = match master.as_str() {
                    "resistor" => ('r', "r"),
                    "capacitor" => ('c', "c"),
                    _ => ('l', "l"),
                };
                Card::Rcl {
                    letter: letter.to_ascii_uppercase(),
                    name: self.strip(&name, letter),
                    n1,
                    n2,
                    value: self.value(self.get(&p, key)?),
                    model: None,
                    params: kept(&[key]),
                }
            }
            "vsource" | "isource" => {
                let (np, nm) = two()?;
                if p.iter().any(|(k, _)| !keys.source.contains(&k.as_str())) {
                    return None;
                }
                let wave = self.named_waveform(&p)?;
                let value = self.get(&p, "dc").map_or(Val::Num(0.0), |v| self.value(v));
                let letter = if master == "vsource" { 'v' } else { 'i' };
                Card::Source {
                    letter: letter.to_ascii_uppercase(),
                    name: self.strip(&name, letter),
                    np,
                    nm,
                    value,
                    ac_mag: num("mag"),
                    ac_phase: num("phase"),
                    wave,
                    port: None,
                }
            }
            "vcvs" | "vccs" => {
                let [np, nm, ncp, ncm] = ns.as_slice() else {
                    return None;
                };
                let (letter, gain) = match master.as_str() {
                    "vcvs" => ('e', num("gain")?),
                    _ => ('g', num(keys.gm)?),
                };
                Card::Linear {
                    letter: letter.to_ascii_uppercase(),
                    name: self.strip(&name, letter),
                    nodes: vec![np.clone(), nm.clone(), ncp.clone(), ncm.clone()],
                    gain,
                }
            }
            "cccs" | "ccvs" => {
                let (np, nm) = two()?;
                // quirk: the controlling source as written, not as the name
                // map spells its declaration (`in1` is `Vin1`, `v1` is `V1`);
                // phase 7 spells the reference through the map.
                let vsense = ident(keys.probe)?;
                let (letter, gain) = match master.as_str() {
                    "cccs" => ('f', num("gain")?),
                    _ => ('h', num(keys.rm)?),
                };
                Card::Linear {
                    letter: letter.to_ascii_uppercase(),
                    name: self.strip(&name, letter),
                    nodes: vec![np, nm, vsense],
                    gain,
                }
            }
            m if m == keys.mutual && (self.d == Dialect::Spectre || ns.is_empty()) => {
                let refs = [ident("ind1")?, ident("ind2")?];
                Card::Coupling {
                    name: self.strip(&name, 'k'),
                    l1: self.strip(&refs[0], 'l'),
                    l2: self.strip(&refs[1], 'l'),
                    refs,
                    k: num(keys.coupling)?,
                }
            }
            "pvcvs" | "pvccs" | "pcccs" | "pccvs" if self.d == Dialect::Spectre => {
                let letter = match master.as_str() {
                    "pvcvs" => 'E',
                    "pvccs" => 'G',
                    "pcccs" => 'F',
                    _ => 'H',
                };
                let list = |k: &str| self.get(&p, k).map(|v| self.velems(v));
                let coeffs: Vec<f64> = list("coeffs")?
                    .into_iter()
                    .map(|c| self.number(c))
                    .collect::<Option<_>>()?;
                let (np, nm) = (ns.first()?.clone(), ns.get(1)?.clone());
                let controls = if matches!(letter, 'E' | 'G') {
                    ns[2..].to_vec()
                } else {
                    list("probes")?
                        .into_iter()
                        .map(|c| self.name_of(c))
                        .collect()
                };
                Card::Poly {
                    name: self.strip(&name, letter.to_ascii_lowercase()),
                    letter,
                    np,
                    nm,
                    controls,
                    coeffs,
                }
            }
            m if subckts.iter().any(|s| s == m) || matches!(kit, Some(Declared::Subckt)) => {
                return Some(Card::Inst {
                    name: self.strip(&name, 'x'),
                    ports: ns,
                    master: self
                        .masters
                        .get(&n)
                        .cloned()
                        .unwrap_or_else(|| m.to_string()),
                    params: kept(&[]),
                });
            }
            m => {
                let kind = kind_of.as_deref()?;
                let (model, params) = (m.to_string(), kept(&[]));
                let letter = match (kind, ns.len()) {
                    ("d", 2) => 'd',
                    ("npn" | "pnp", 3..=5) => 'q',
                    ("nmos" | "pmos", 4) => 'm',
                    ("njf" | "pjf", 3) if self.d == Dialect::Vacask => 'j',
                    ("nmf" | "pmf", 3) if self.d == Dialect::Vacask => 'z',
                    _ => return None,
                };
                Card::Semi {
                    letter: letter.to_ascii_uppercase(),
                    name: self.strip(&name, letter),
                    nodes: ns,
                    model,
                    params,
                }
            }
        })
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// What instance `n`'s master is declared as in the kit, its kind
    /// named as `Rd::declarations` names a model's (the old lowering's
    /// `spectre_kind`, `vacask_kind`, `vacask_builtin`).
    fn kit_master(&self, n: u32) -> Option<Declared> {
        match &self.kit_def(n)?.kind {
            DefKind::Subckt { .. } => Some(Declared::Subckt),
            DefKind::Model { word, polarity, .. } => {
                let p = *polarity == Some(Polarity::P);
                if self.d == Dialect::Vacask {
                    let ty = polarity.map(|_| if p { -1.0 } else { 1.0 });
                    return match vacask_kind(word, ty) {
                        Some((kind, _)) => Some(Declared::Model(kind)),
                        None => vacask_builtin(word).map(|b| Declared::Model(format!("={b}"))),
                    };
                }
                let ty = p.then_some(if word == "bjt" { "pnp" } else { "p" });
                Some(Declared::Model(spectre_kind(word, ty).0))
            }
            DefKind::Param | DefKind::Func | DefKind::Section => None,
        }
    }

    /// The declaration instance `n`'s master binds to in the kit.
    fn kit_def(&self, n: u32) -> Option<&'a Def> {
        let (kit, f) = self.kit?;
        match kit.binding(f, Node(n))? {
            Binding::Def(id) => Some(kit.definition(id)),
            Binding::Primitive(_) | Binding::Unknown => None,
        }
    }

    fn keys(&self) -> &'static Keys {
        if self.d == Dialect::Vacask {
            &VACASK
        } else {
            &SPECTRE
        }
    }

    /// `name`'s SPICE spelling: the letter a writer adds back is dropped
    /// (the old lowering's `strip`).
    fn strip(&self, name: &str, letter: char) -> String {
        let name = if self.d == Dialect::Vacask {
            name.to_lowercase()
        } else {
            name.to_string()
        };
        match name.chars().next() {
            Some(c) if c.eq_ignore_ascii_case(&letter) && name.len() > 1 => {
                name[c.len_utf8()..].to_string()
            }
            _ => name,
        }
    }

    /// the old lowering's `named_waveform`.
    fn named_waveform(&self, p: &[(String, u32)]) -> Option<Option<Wave>> {
        let keys = self.keys();
        let f = |k: &str| {
            self.get(p, k)
                .map_or(Some(Num::Num(0.0)), |v| self.irnum(v))
        };
        let ty = self
            .get(p, "type")
            .map(|t| self.text(t).trim_matches('"').to_string());
        let fm = ty.as_deref() == Some("fm")
            || (ty.as_deref() == Some("sine") && self.get(p, "fmmodindex").is_some());
        let spectre_exp = self.d == Dialect::Spectre && ty.as_deref() == Some("exp");
        if (fm || spectre_exp)
            && self
                .get(p, "delay")
                .is_some_and(|v| self.number(v) != Some(0.0))
        {
            return None;
        }
        if ty.as_deref() != Some("pwl") && self.get(p, "pwlperiod").is_some() {
            return None;
        }
        Some(Some(match ty.as_deref() {
            None | Some("dc") => return Some(None),
            Some("pulse") => Wave {
                kind: WaveKind::Pulse,
                args: vec![
                    f("val0")?,
                    f("val1")?,
                    f("delay")?,
                    f("rise")?,
                    f("fall")?,
                    f("width")?,
                    f("period")?,
                ],
                repeat: None,
                delay: None,
                cut: false,
            },
            Some("sine") if self.get(p, "fmmodindex").is_some() => Wave {
                kind: WaveKind::Sffm,
                args: vec![
                    f("sinedc")?,
                    f("ampl")?,
                    f("freq")?,
                    f("fmmodindex")?,
                    f("fmmodfreq")?,
                ],
                repeat: None,
                delay: None,
                cut: false,
            },
            Some("sine") => Wave {
                kind: WaveKind::Sin,
                args: vec![
                    f("sinedc")?,
                    f("ampl")?,
                    f("freq")?,
                    f("delay")?,
                    f(keys.damp)?,
                    f("sinephase")?,
                ],
                repeat: None,
                delay: None,
                cut: false,
            },
            Some("exp") if self.d == Dialect::Vacask => {
                let n = |k: &str| self.get(p, k).map_or(Some(0.0), |v| self.number(v));
                Wave {
                    kind: WaveKind::Exp,
                    args: vec![
                        f("val0")?,
                        f("val1")?,
                        Num::Num(n("delay")?),
                        f("tau1")?,
                        Num::Num(n("delay")? + n("td2")?),
                        f("tau2")?,
                    ],
                    repeat: None,
                    delay: None,
                    cut: false,
                }
            }
            Some("exp") => Wave {
                kind: WaveKind::Exp,
                args: vec![
                    f("val0")?,
                    f("val1")?,
                    f("td1")?,
                    f("tau1")?,
                    f("td2")?,
                    f("tau2")?,
                ],
                repeat: None,
                delay: None,
                cut: false,
            },
            Some("fm") => Wave {
                kind: WaveKind::Sffm,
                args: vec![
                    f("sinedc")?,
                    f("ampl")?,
                    f("freq")?,
                    f("modindex")?,
                    f("modfreq")?,
                ],
                repeat: None,
                delay: None,
                cut: false,
            },
            Some("am") => Wave {
                kind: WaveKind::Am,
                args: vec![
                    f("ampl")?,
                    f("sinedc")?,
                    f("modfreq")?,
                    f("freq")?,
                    f("delay")?,
                ],
                repeat: None,
                delay: None,
                cut: false,
            },
            Some("pwl") if self.d == Dialect::Spectre => {
                let w = self.get(p, "wave")?;
                let xs: Vec<Num> = self
                    .velems(w)
                    .into_iter()
                    .map(|x| self.irnum(x))
                    .collect::<Option<_>>()?;
                if !xs.len().is_multiple_of(2) {
                    return None;
                }
                let repeat = match self.get(p, "pwlperiod") {
                    Some(v) if pwl_period(&xs)? == self.number(v)? => Some(Num::Num(0.0)),
                    Some(_) => return None,
                    None => None,
                };
                let delay = match self.get(p, "delay") {
                    Some(v) => Some(self.irnum(v)?).filter(|d| *d != Num::Num(0.0)),
                    None => None,
                };
                Wave {
                    kind: WaveKind::Pwl,
                    args: xs,
                    repeat,
                    delay,
                    cut: false,
                }
            }
            Some(_) => return None,
        }))
    }

    /// quirk: the old lowering's `velems`: a vector's element rows; the old
    /// Spectre grammar's `s_unary2` captures no sign, so `-1` reads as `1`.
    fn velems(&self, v: u32) -> Vec<u32> {
        if self.tag(v) != Tag::Vector {
            return vec![v];
        }
        self.ast
            .list(self.ast.rhs(Node(v)) + 1)
            .iter()
            .map(|&k| match self.tag(k) {
                Tag::Unary => self.ast.lhs(Node(k)),
                _ => k,
            })
            .collect()
    }
}

/// The Spectre `pwlperiod` that repeats PWL points `xs` (time, value flat)
/// exactly as ngspice's `r=0` does, when one does: numeric points from
/// t = 0 whose last value equals the first, the last segment at least the
/// default `twidth` (period/1000) long (Spectre Reference 5.0, `vsource`
/// #17, #18, p.686). `None` otherwise.
fn pwl_period(xs: &[Num]) -> Option<f64> {
    let n = xs.len();
    if n < 4 {
        return None;
    }
    let (t0, v0) = (xs[0].num()?, xs[1].num()?);
    let (tp, tn, vn) = (xs[n - 4].num()?, xs[n - 2].num()?, xs[n - 1].num()?);
    (t0 == 0.0 && vn == v0 && tn > 0.0 && tn - tp >= tn / 1000.0).then_some(tn)
}
