//! Spectre and VACASK blocks → the IR: `model` statements and model groups
//! (one model per bin), `statistics` blocks, `real` functions, `if` chains
//! and `@if` blocks, VACASK `control` blocks (options and analyses, the
//! rest kept as lines).
//!
//! Cites: the old lowering (`named_model`, `one_model`,
//! `ControlBlock`), `statistics.rs`, `conditionals.rs` (ported); Spectre
//! Reference 19.1 pp.175-179, p.491; Spectre UG p.92, p.108.
//!
//! Contents:
//! - `Cx::named_model(&mut self, n: u32, here: &mut Subcircuit) -> bool`
//! - `Cx::statistics(&self, n: u32) -> Statistics`
//! - `Cx::real_func(&self, n: u32) -> FuncDef`
//! - `Cx::named_conditional(&mut self, n: u32, scope: Scope) -> Conditional`
//! - `Cx::vacask_control_block(&mut self, n: u32)`

use super::named::{spectre_kind, vacask_builtin, vacask_kind};
use super::{Cx, Scope};
use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{Node, Tag};
use crate::syntax::token::Tag as T;

impl<'a> Cx<'a> {
    /// A `model` statement, a model group one model per bin (the old lowering
    /// `named_model`).
    pub(super) fn named_model(&mut self, n: u32, here: &mut Subcircuit) -> bool {
        let f = self.fields(n);
        let (Some(&name), Some(&master)) = (f.first(), f.get(1)) else {
            return false;
        };
        let (name, master) = (self.name_of(name), self.name_of(master));
        let p = self.pairs(n);
        let bins = self.body(n).to_vec();
        if bins.is_empty() {
            return self.one_model(name, &master, p, here);
        }
        bins.into_iter().all(|b| {
            let bin = self.t.text(self.main(b)).to_string();
            let mut p = p.clone();
            p.extend(self.pairs(b));
            self.one_model(format!("{name}.{bin}"), &master, p, here)
        })
    }

    /// A Spectre `statistics` block (the old lowering).
    pub(super) fn statistics(&self, n: u32) -> Statistics {
        let mut st = Statistics::default();
        for &k in self.body(n) {
            let head = self.ttag(self.main(k));
            match (self.tag(k), head) {
                (Tag::Process, _) => {
                    let scope = if head == T::KwProcess { 1 } else { 2 };
                    for &v in self.body(k) {
                        match self.ttag(self.main(v)) {
                            T::KwVary => {
                                let vary = self.vary(v);
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
                    st.correlations.push(Correlate {
                        params: list("param"),
                        devices: list("dev"),
                        cc: self.get(&p, "cc").map_or(String::new(), |v| self.vtext(v)),
                    });
                }
                (_, T::KwTruncate) => st.truncate[0] = self.tr(k),
                _ => {}
            }
        }
        st
    }

    /// `real f(real a, real b) { return e }` → a function: the name, then
    /// each argument after its `real`.
    pub(super) fn real_func(&self, n: u32) -> FuncDef {
        let f = self.fields(n);
        let args = f[1..]
            .iter()
            .skip(1)
            .step_by(2)
            .map(|&k| self.name_of(k))
            .collect();
        let ret = self
            .body(n)
            .iter()
            .copied()
            .find(|&k| self.ttag(self.main(k)) == T::KwReturn);
        let body = ret.map_or(String::new(), |r| {
            self.text(self.fields(r)[0]).trim().to_string()
        });
        FuncDef {
            name: self.name_of(f[0]),
            args,
            body,
        }
    }

    /// A Spectre `if` chain or a VACASK `@if` block (the old lowering).
    pub(super) fn named_conditional(&mut self, n: u32, scope: Scope) -> Conditional {
        let mut c = Conditional::default();
        let mut branch = Some(Node(n));
        while let Some(b) = branch {
            let condition = match self.tag(b.0) {
                Tag::If => Some(super::unwrapped(&self.text(self.fields(b.0)[0]))),
                _ => None,
            };
            let mut body = Subcircuit::default();
            let stmts = self.body(b.0).to_vec();
            self.cards(&stmts, &mut body, scope);
            c.branches.push(Branch { condition, body });
            branch = self.ast.next(b);
        }
        c
    }

    /// A VACASK `control` block: its options and analyses, the rest kept as
    /// lines (the old lowering's `ControlBlock`).
    pub(super) fn vacask_control_block(&mut self, n: u32) {
        let body = self.body(n).to_vec();
        let mut sweeps: Vec<u32> = Vec::new();
        for &st in &body {
            let head = self.ttag(self.main(st));
            if head == T::KwSweep {
                sweeps.push(st);
                continue;
            }
            if head == T::KwAnalysis {
                let done = self.vacask_analysis(&sweeps, st);
                if !done {
                    for &s in sweeps.iter().chain([&st]) {
                        let t = self.line(s);
                        self.tb.extra_lines.push(t);
                    }
                }
                sweeps.clear();
                continue;
            }
            for &s in &sweeps {
                let t = self.line(s);
                self.tb.extra_lines.push(t);
            }
            sweeps.clear();
            if !self.vacask_options(st) {
                let t = self.line(st);
                self.tb.extra_lines.push(t);
            }
        }
        for &s in &sweeps {
            let t = self.line(s);
            self.tb.extra_lines.push(t);
        }
    }
}

// ---- private ----

impl<'a> Cx<'a> {
    /// The model `name` of `master` with the parameters `p` (the old lowering
    /// `one_model`).
    fn one_model(
        &mut self,
        name: String,
        master: &str,
        p: Vec<(String, u32)>,
        here: &mut Subcircuit,
    ) -> bool {
        let (kind, level) = if self.d == Dialect::Vacask {
            if vacask_builtin(master).is_some() {
                return true;
            }
            match vacask_kind(master, self.get(&p, "type").and_then(|t| self.number(t))) {
                Some(kl) => kl,
                None => return false,
            }
        } else {
            let ty = self.get(&p, "type").map(|t| self.text(t));
            spectre_kind(master, ty.as_deref())
        };
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

    /// `vary <param> dist=<d> std=<s> | N=<n> [percent=yes]`.
    fn vary(&self, v: u32) -> Vary {
        let p = self.pairs(v);
        let text = |k: &str| self.get(&p, k).map(|x| self.text(x).trim().to_string());
        let dist = match text("dist").as_deref() {
            Some("lnorm") => Dist::Lnorm,
            Some("unif") => Dist::Unif,
            _ => Dist::Gauss,
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
