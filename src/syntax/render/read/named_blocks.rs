//! Spectre and VACASK blocks → the plan and their views: `model`
//! statements and model groups (one model per bin), `real` functions, and
//! a Spectre `statistics` block's view.
//!
//! Cites: the old lowering's `named_model`, `one_model` and its statistics
//! reading (as `syntax::adapter::named_blocks` ports them); Spectre
//! Reference 19.1 pp.175-179, p.481; Spectre UG p.92, p.108.
//!
//! Contents:
//! - `pub(in crate::syntax::render) struct Stats`, `struct Vary`
//! - `Rd::named_model_ok(&mut self, n: u32, here: &mut Block) -> bool`
//! - `Rd::named_model_view(&self, n: u32, bin: u32) -> Model`
//! - `Rd::real_func(&self, n: u32) -> (String, Vec<String>, String)`, `Rd::real_return(&self, n: u32) -> Option<u32>`
//! - `Rd::statistics(&self, n: u32) -> Stats`

use super::cards::Model;
use super::named::{spectre_kind, vacask_builtin, vacask_kind};
use super::{Block, Rd};
use crate::codegen::Dialect;
use crate::syntax::ast::{NONE, Tag};
use crate::syntax::token::Tag as T;

/// A Spectre `statistics` block as the old reader kept it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::syntax::render) struct Stats {
    pub(in crate::syntax::render) process: Vec<Vary>,
    pub(in crate::syntax::render) mismatch: Vec<Vary>,
    /// Each `correlate`: its parameters, its devices, its `cc` text.
    pub(in crate::syntax::render) correlations: Vec<(Vec<String>, Vec<String>, String)>,
    /// `truncate tr=` at the block's top, in `process`, in `mismatch`.
    pub(in crate::syntax::render) truncate: [Option<String>; 3],
}

/// `vary <param> dist=<d> std=<s> | N=<n> [percent=yes]`.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::syntax::render) struct Vary {
    pub(in crate::syntax::render) param: String,
    /// `gauss`, `unif` or `lnorm`.
    pub(in crate::syntax::render) dist: &'static str,
    pub(in crate::syntax::render) spread: String,
    pub(in crate::syntax::render) percent: bool,
}

impl<'a> Rd<'a> {
    /// A `model` statement placed in `here`, a model group one model per
    /// bin, as far as the bins read (the old lowering's `named_model`);
    /// `false` when one does not.
    pub(super) fn named_model_ok(&mut self, n: u32, here: &mut Block) -> bool {
        let f = self.fields(n);
        if f.len() < 2 {
            return false;
        }
        let bins = self.body(n);
        if bins.is_empty() {
            return self.one_model(n, NONE, here);
        }
        bins.iter().all(|&b| self.one_model(n, b, here))
    }

    /// Model statement `n`'s view, or its bin `bin`'s (`NONE`: no bin).
    ///
    /// # Panics
    /// When the reader did not place it.
    pub(in crate::syntax::render) fn named_model_view(&self, n: u32, bin: u32) -> Model {
        self.model_of(n, bin).expect("a model the reader placed")
    }

    /// `real f(real a, real b) { return e }` → its name, arguments (each
    /// after its `real`) and body.
    pub(in crate::syntax::render) fn real_func(&self, n: u32) -> (String, Vec<String>, String) {
        let f = self.fields(n);
        let args = f[1..]
            .iter()
            .skip(1)
            .step_by(2)
            .map(|&k| self.name_of(k))
            .collect();
        let body = self.real_return(n).map_or(String::new(), |r| {
            self.text(self.fields(r)[0]).trim().to_string()
        });
        (self.name_of(f[0]), args, body)
    }

    /// A `real` function's `return` statement, when it has one.
    pub(in crate::syntax::render) fn real_return(&self, n: u32) -> Option<u32> {
        self.body(n)
            .iter()
            .copied()
            .find(|&k| self.ttag(self.main(k)) == T::KwReturn)
    }

    /// A Spectre `statistics` block (the old lowering).
    pub(in crate::syntax::render) fn statistics(&self, n: u32) -> Stats {
        let mut st = Stats::default();
        for &k in self.body(n) {
            let head = self.ttag(self.main(k));
            match (self.tag(k), head) {
                (Tag::Process, _) => {
                    let scope = if head == T::KwProcess { 1 } else { 2 };
                    for &v in self.body(k) {
                        match self.ttag(self.main(v)) {
                            T::KwVary => {
                                let vary = self.vary(v);
                                if self.unlifted_var(&vary.param) {
                                    continue;
                                }
                                if scope == 1 {
                                    st.process.push(vary)
                                } else {
                                    st.mismatch.push(vary)
                                }
                            }
                            T::KwTruncate => st.truncate[scope] = self.tr(v),
                            _ => {}
                        }
                    }
                }
                (_, T::KwCorrelate) => {
                    let p = self.pairs(k);
                    let list = |key: &str| {
                        self.get(&p, key).map_or(vec![], |v| {
                            let t = self.text(v);
                            t.trim_matches(|c| c == '[' || c == ']')
                                .split([' ', ',', '\t'])
                                .filter(|w| !w.is_empty())
                                .map(str::to_string)
                                .collect()
                        })
                    };
                    let cc = self.get(&p, "cc").map_or(String::new(), |v| self.vtext(v));
                    st.correlations.push((list("param"), list("dev"), cc));
                }
                (_, T::KwTruncate) => st.truncate[0] = self.tr(k),
                _ => {}
            }
        }
        st
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// Places model `n` (its bin `bin`) in `here` (the old lowering's
    /// `one_model`): `false` when the IR had no kind for it; a VACASK
    /// built-in is declared by the writer, not placed.
    fn one_model(&mut self, n: u32, bin: u32, here: &mut Block) -> bool {
        if self.d == Dialect::Vacask && vacask_builtin(&self.name_of(self.fields(n)[1])).is_some() {
            return true;
        }
        match self.model_of(n, bin) {
            Some(_) => {
                here.models.push((n, bin));
                true
            }
            None => false,
        }
    }

    /// Model `n` (bin `bin`) as SPICE reads it; `None` for a VACASK module
    /// with no SPICE kind.
    fn model_of(&self, n: u32, bin: u32) -> Option<Model> {
        let f = self.fields(n);
        let (name, master) = (self.name_of(f[0]), self.name_of(f[1]));
        let mut p = self.pairs(n);
        let name = match bin {
            NONE => name,
            b => {
                p.extend(self.pairs(b));
                format!("{name}.{}", self.t.text(self.main(b)))
            }
        };
        let (kind, level) = if self.d == Dialect::Vacask {
            vacask_kind(&master, self.get(&p, "type").and_then(|t| self.number(t)))?
        } else {
            let ty = self.get(&p, "type").map(|t| self.text(t));
            spectre_kind(&master, ty.as_deref())
        };
        let drop_type = self.d == Dialect::Vacask || level.is_some() || kind != master;
        let mut params: Vec<(String, String)> = p
            .iter()
            .filter(|(k, _)| !(drop_type && k == "type"))
            .map(|(k, v)| (k.clone(), self.vtext(*v)))
            .collect();
        if let Some(l) = level {
            params.insert(0, ("level".into(), l.into()));
        }
        Some(Model { name, kind, params })
    }

    /// `vary <param> dist=<d> std=<s> | N=<n> [percent=yes]`.
    fn vary(&self, v: u32) -> Vary {
        let p = self.pairs(v);
        let text = |k: &str| self.get(&p, k).map(|x| self.text(x).trim().to_string());
        let dist = match text("dist").as_deref() {
            Some("lnorm") => "lnorm",
            Some("unif") => "unif",
            _ => "gauss",
        };
        let spread = self
            .get(&p, "std")
            .or_else(|| self.get(&p, "N"))
            .map_or(String::new(), |x| self.vtext(x));
        Vary {
            param: self.name_of(self.fields(v)[0]),
            dist,
            spread,
            percent: text("percent").as_deref() == Some("yes"),
        }
    }

    /// `truncate tr=<t>`.
    fn tr(&self, t: u32) -> Option<String> {
        let p = self.pairs(t);
        self.get(&p, "tr").map(|v| self.vtext(v))
    }
}
