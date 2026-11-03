//! Spectre `statistics { }` rows -> [`Statistics`]: each scope's `vary`s,
//! the `correlate`s and the `truncate`s (R19 pp.175-179); values in the
//! IR's SPICE spelling, as every other value.
//!
//! Cites: Spectre Circuit Simulator Reference 19.1 pp.175-179.
//!
//! Contents:
//! - `Cx::statistics(&self, card: usize) -> Statistics`

use super::Cx;
use crate::import::Tag;
use crate::ir::*;

impl Cx<'_> {
    pub(super) fn statistics(&self, card: usize) -> Statistics {
        let mut st = Statistics::default();
        for k in self.kids(card) {
            match self.tag(k) {
                Tag::Process | Tag::Mismatch => {
                    let scope = if self.tag(k) == Tag::Process { 1 } else { 2 };
                    for v in self.kids(k) {
                        match self.tag(v) {
                            Tag::Vary => {
                                let vary = self.vary(v);
                                if scope == 1 {
                                    st.process.push(vary)
                                } else {
                                    st.mismatch.push(vary)
                                }
                            }
                            Tag::Truncate => st.truncate[scope] = self.tr(v),
                            _ => {}
                        }
                    }
                }
                Tag::Correlate => {
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
                Tag::Truncate => st.truncate[0] = self.tr(k),
                _ => {}
            }
        }
        st
    }
}

// ---- private ----

impl Cx<'_> {
    /// `vary <param> dist=<d> std=<s> | N=<n> [percent=yes]`.
    fn vary(&self, v: usize) -> Vary {
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
            param: self.name(self.kids(v)[0]),
            dist,
            spread,
            percent: text("percent").as_deref() == Some("yes"),
        }
    }

    /// `truncate tr=<t>`.
    fn tr(&self, t: usize) -> Option<String> {
        let p = self.pairs(t);
        self.get(&p, "tr").map(|v| self.vtext(v))
    }
}
