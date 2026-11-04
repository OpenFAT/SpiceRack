//! Spectre `statistics` blocks → SPICE and VACASK statistical draws: the
//! `Lift(StatisticsDraws)` rule, shared by both writers. A `process` vary is the
//! parameter drawn once per run, as the two-layer idiom (`.param
//! p__mc={agauss(v, s, 1)}`, `.param p=p__mc`); a `mismatch` vary is a
//! two-layer draw in every subcircuit that reads the parameter, its
//! references there renamed `p__mm`; correlated gauss process parameters
//! are a Cholesky combination of normalized two-layer draws. SPICE draws a
//! one-layer `.param` at every reference; a VACASK `parameters` call is
//! already one value, so VACASK's draws are one layer ([`Layers::One`]). The rule records fixups, never touching the
//! tree: parameters written before a parameter, a parameter's new value,
//! locals written first in a subcircuit, and each renamed reference's token
//! ([`Rd::respell_token`]).
//!
//! Refused (`W0018`): `correlate dev=`, an n>2 `correlate`
//! with an expression `cc`, a `correlate` over a parameter that is no gauss
//! `process` vary, `truncate` below 4 sigma (or not a number), `lnorm` with
//! `percent`; and a `statistics` block inside a subcircuit or a branch,
//! which SPICE has no place for (the old writer dropped it unwritten). A
//! `vary` of a parameter no `parameters` declares is `W0019`; a correlation
//! matrix that is not positive definite `W0020`.
//!
//! Cites: ngspice `inpcom.c` `inp_fix_agauss_in_param` (the two-layer idiom:
//! one layer redraws per reference, measured on ngspice-44.2); Spectre Reference 19.1
//! pp.175-179; the old writer's `codegen::statistics::lower`, whose output
//! this is byte for byte.
//!
//! Contents:
//! - `pub(super) struct Draws`, `pub(super) enum Layers`
//! - `fn lift(rd: &mut Rd, deck: &Deck, layers: Layers) -> Result<Draws, Diag>`
//! - `Draws::before(&self, a: u32) -> &[(String, String)]`, `Draws::value(&self, a: u32) -> Option<&str>`,
//!   `Draws::locals(&self, sc: u32) -> &[(String, String)]`, `Draws::texts(&self) -> impl Iterator<Item = &str>`

use std::collections::HashMap;

use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::diag::{Code, Diag};
use crate::syntax::render::read::{Block, Deck, Rd, Stats, Vary};
use crate::syntax::token::Tag as T;

/// The rule's fixups, keyed by `.param` assignment and by subcircuit node.
#[derive(Default)]
pub(super) struct Draws {
    /// Parameters written before an assignment.
    before: HashMap<u32, Vec<(String, String)>>,
    /// An assignment's value, replaced.
    value: HashMap<u32, String>,
    /// Locals written first in a subcircuit, by its `Subckt` node.
    locals: HashMap<u32, Vec<(String, String)>>,
}

/// How many `.param` layers a draw takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Layers {
    /// SPICE: a one-layer `.param` is drawn at every reference, so a draw
    /// is `p__mc = draw`, `p = p__mc`.
    Two,
    /// VACASK: `p = draw`.
    One,
}

/// One parameter of a body while the rule rewrites it: `node` is its
/// assignment, `NONE` for one the rule adds.
struct P {
    name: String,
    value: String,
    node: u32,
}

/// The rule's fixups for `deck`; the first refusal when a block has no SPICE
/// form. Each renamed reference is recorded in `rd`.
pub(super) fn lift(rd: &mut Rd, deck: &Deck, layers: Layers) -> Result<Draws, Diag> {
    let mut out = Draws::default();
    let mut mismatch: Vec<(String, String)> = Vec::new(); // (param, draw around it)
    let bodies = std::iter::once(&deck.top).chain(deck.sections.iter().map(|s| &s.body));
    for body in bodies {
        if body.statistics.is_empty() {
            continue;
        }
        let mut ps: Vec<P> = body
            .params
            .iter()
            .map(|&a| {
                let (name, value) = rd.param_view(a);
                P {
                    name,
                    value,
                    node: a,
                }
            })
            .collect();
        for &st in &body.statistics {
            let tok = rd.main(st);
            let s = rd.statistics(st);
            truncation(&s).map_err(|c| diag(c, tok))?;
            let correlated = correlate(&mut ps, &s, layers).map_err(|c| diag(c, tok))?;
            for v in s.process.iter().filter(|v| !correlated.contains(&v.param)) {
                let i = declared(&ps, &v.param).map_err(|c| diag(c, tok))?;
                let draw = draw(v, &ps[i].value).map_err(|c| diag(c, tok))?;
                match layers {
                    Layers::One => ps[i].value = braced(&draw),
                    Layers::Two => {
                        let r = format!("{}__mc", v.param);
                        ps[i].value = r.clone();
                        ps.insert(i, new(r, braced(&draw)));
                    }
                }
            }
            for v in &s.mismatch {
                let draw = draw(v, &v.param).map_err(|c| diag(c, tok))?;
                mismatch.push((v.param.clone(), draw));
            }
        }
        out.record(rd, ps);
    }
    for b in placeless(deck) {
        if let Some(&st) = b.statistics.first() {
            return Err(diag(Code::W0018, rd.main(st)));
        }
    }
    for (p, draw) in &mismatch {
        let to = format!("{p}__mm");
        let subckts = deck
            .defs
            .iter()
            .chain(deck.sections.iter().flat_map(|s| &s.defs));
        for sc in subckts {
            let refs = references(rd, sc, p);
            if refs.is_empty() {
                continue;
            }
            for k in refs {
                rd.respell_token(k, to.clone());
            }
            let mut locals = match layers {
                Layers::One => vec![(to.clone(), braced(draw))],
                Layers::Two => {
                    let r = format!("{to}_r");
                    vec![(r.clone(), braced(draw)), (to.clone(), r)]
                }
            };
            locals.extend(out.locals.remove(&sc.node).unwrap_or_default());
            out.locals.insert(sc.node, locals);
        }
    }
    Ok(out)
}

impl Draws {
    /// The parameters written before assignment `a`.
    pub(super) fn before(&self, a: u32) -> &[(String, String)] {
        self.before.get(&a).map_or(&[], Vec::as_slice)
    }

    /// Assignment `a`'s value, when the rule replaced it.
    pub(super) fn value(&self, a: u32) -> Option<&str> {
        self.value.get(&a).map(String::as_str)
    }

    /// The locals written first in the subcircuit whose node is `sc`.
    pub(super) fn locals(&self, sc: u32) -> &[(String, String)] {
        self.locals.get(&sc).map_or(&[], Vec::as_slice)
    }

    /// Every value the rule writes, for the deck-wide checks.
    pub(super) fn texts(&self) -> impl Iterator<Item = &str> {
        let added = self.before.values().chain(self.locals.values()).flatten();
        added
            .map(|(_, v)| v.as_str())
            .chain(self.value.values().map(String::as_str))
    }
}

// ---- private ----

impl Draws {
    /// A body's rewritten parameters as fixups on its assignments: the
    /// added ones before the assignment that follows them, a changed value
    /// on its assignment.
    fn record(&mut self, rd: &Rd, ps: Vec<P>) {
        let mut added = Vec::new();
        for p in ps {
            if p.node == NONE {
                added.push((p.name, p.value));
                continue;
            }
            if !added.is_empty() {
                self.before.insert(p.node, std::mem::take(&mut added));
            }
            if rd.param_view(p.node).1 != p.value {
                self.value.insert(p.node, p.value);
            }
        }
        // Every draw is inserted before a declared parameter.
        assert!(added.is_empty(), "no parameter added after the last");
    }
}

fn diag(code: Code, token: u32) -> Diag {
    Diag { code, token }
}

fn new(name: String, value: String) -> P {
    P {
        name,
        value,
        node: NONE,
    }
}

/// SPICE does not truncate (Spectre does at 4 sigma by default, Spectre
/// Reference 19.1 p.179): 4 sigma or more costs under 0.06% of a
/// std and is dropped; less changes the distribution and is refused.
fn truncation(st: &Stats) -> Result<(), Code> {
    for t in st.truncate.iter().flatten() {
        match crate::unit::parse_spice_number(t.trim()) {
            Some((x, end, s)) if end + s.len() == t.trim().len() && x >= 4.0 => {}
            _ => return Err(Code::W0018),
        }
    }
    Ok(())
}

/// The index of `p`'s declaration among `ps`.
fn declared(ps: &[P], p: &str) -> Result<usize, Code> {
    ps.iter().position(|d| d.name == p).ok_or(Code::W0019)
}

/// One draw of `v` around the mean `nom`.
fn draw(v: &Vary, nom: &str) -> Result<String, Code> {
    let s = spread(v, nom);
    Ok(match v.dist {
        "gauss" => format!("agauss({nom}, {s}, 1)"),
        "unif" => format!("aunif({nom}, {s})"),
        _ if v.percent => return Err(Code::W0018),
        _ => format!("exp(agauss(log({nom}), {s}, 1))"),
    })
}

/// `v`'s spread around `nom`, scaled when it is a percentage.
fn spread(v: &Vary, nom: &str) -> String {
    if v.percent {
        format!("({nom})*({})/100", v.spread)
    } else {
        v.spread.clone()
    }
}

/// `e` as one braced expression, unless it is already one.
fn braced(e: &str) -> String {
    let t = e.trim();
    if t.starts_with('{') && t.ends_with('}') {
        t.to_string()
    } else {
        format!("{{{t}}}")
    }
}

/// Each correlated set of gauss process parameters: normalized two-layer
/// draws `p__z`, then `p = v + s*sum(L*z)` with `L` the Cholesky factor of
/// the correlation matrix (unlisted pairs 0). Returns the parameters
/// handled.
fn correlate(ps: &mut Vec<P>, st: &Stats, layers: Layers) -> Result<Vec<String>, Code> {
    let mut names: Vec<String> = Vec::new();
    for (params, devices, _) in &st.correlations {
        if !devices.is_empty() {
            return Err(Code::W0018);
        }
        for p in params {
            if !names.contains(p) {
                names.push(p.clone());
            }
        }
    }
    if names.is_empty() {
        return Ok(names);
    }
    let n = names.len();
    let mut cc = vec![vec![String::new(); n]; n];
    for (i, row) in cc.iter_mut().enumerate() {
        row[i] = "1".into();
    }
    for (params, _, c) in &st.correlations {
        for a in params {
            for b in params {
                let at = |x: &String| names.iter().position(|y| y == x).expect("a named param");
                let (i, j) = (at(a), at(b));
                if i != j {
                    cc[i][j] = c.clone();
                }
            }
        }
    }
    let vary: Vec<&Vary> = names
        .iter()
        .map(|p| {
            let v = st
                .process
                .iter()
                .find(|v| &v.param == p && v.dist == "gauss");
            v.ok_or(Code::W0018)
        })
        .collect::<Result<_, _>>()?;
    let flat: Option<Vec<f64>> = cc.iter().flatten().map(|s| number(s)).collect();
    let l: Vec<Vec<String>> = match flat {
        Some(flat) => cholesky(&flat, n)?
            .iter()
            .map(|r| r.iter().map(|x| format!("{x:e}")).collect())
            .collect(),
        None if n == 2 => {
            let c = format!("({})", cc[1][0]);
            vec![
                vec!["1".into(), "0".into()],
                vec![c.clone(), format!("sqrt(1-{c}*{c})")],
            ]
        }
        None => return Err(Code::W0018),
    };
    for p in &names {
        let i = declared(ps, p)?;
        let z = format!("{p}__z");
        let decl = match layers {
            Layers::One => vec![new(z, "{agauss(0, 1, 1)}".into())],
            Layers::Two => {
                let r = format!("{z}__r");
                vec![new(r.clone(), "{agauss(0, 1, 1)}".into()), new(z, r)]
            }
        };
        ps.splice(i..i, decl);
    }
    for (a, p) in names.iter().enumerate() {
        let i = declared(ps, p)?;
        let nom = ps[i].value.clone();
        let s = spread(vary[a], &nom);
        let sum: Vec<String> = (0..=a)
            .map(|b| format!("({})*{}__z", l[a][b], names[b]))
            .collect();
        ps[i].value = format!("{{({nom}) + ({s})*({})}}", sum.join(" + "));
    }
    Ok(names)
}

/// A correlation coefficient's number (an empty one is 0); `None` for an
/// expression.
fn number(s: &str) -> Option<f64> {
    let t = s.trim();
    let (sign, m) = t.strip_prefix('-').map_or((1.0, t), |m| (-1.0, m));
    match crate::unit::parse_spice_number(m) {
        _ if t.is_empty() => Some(0.0),
        Some((x, end, sc)) if end + sc.len() == m.len() => Some(sign * x),
        _ => None,
    }
}

/// The lower Cholesky factor of the `n`x`n` row-major `c`; `W0020` when `c`
/// is not positive definite.
fn cholesky(c: &[f64], n: usize) -> Result<Vec<Vec<f64>>, Code> {
    assert_eq!(c.len(), n * n, "a square matrix");
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let s: f64 = (0..j).map(|k| l[i][k] * l[j][k]).sum();
            if i == j {
                let d = c[i * n + i] - s;
                if d <= 0.0 {
                    return Err(Code::W0020);
                }
                l[i][j] = d.sqrt();
            } else {
                l[i][j] = (c[i * n + j] - s) / l[j][j];
            }
        }
    }
    Ok(l)
}

/// The blocks a SPICE deck has no place for a `statistics` block in: every
/// subcircuit and every branch, the ones inside them too.
fn placeless(deck: &Deck) -> Vec<&Block> {
    let tops = std::iter::once(&deck.top).chain(deck.sections.iter().map(|s| &s.body));
    let branches = tops.flat_map(|b| {
        b.conds
            .iter()
            .flat_map(|(_, c)| c.branches.iter().map(|(_, b)| b))
    });
    let mut work: Vec<&Block> = deck
        .defs
        .iter()
        .chain(deck.sections.iter().flat_map(|s| &s.defs))
        .chain(branches)
        .collect();
    let mut out = Vec::new();
    while let Some(b) = work.pop() {
        out.push(b);
        work.extend(
            b.conds
                .iter()
                .flat_map(|(_, c)| c.branches.iter().map(|(_, b)| b)),
        );
    }
    out
}

/// The identifier tokens naming `p` in the values of subcircuit `sc` and the
/// branches inside it: the references a mismatch draw renames.
fn references(rd: &Rd, sc: &Block, p: &str) -> Vec<u32> {
    let mut roots = Vec::new();
    let mut blocks = vec![sc];
    while let Some(b) = blocks.pop() {
        rd.value_roots(b, &mut roots);
        blocks.extend(
            b.conds
                .iter()
                .flat_map(|(_, c)| c.branches.iter().map(|(_, b)| b)),
        );
    }
    let mut out = Vec::new();
    for r in roots {
        // An element's `key=value`: the value only.
        let r = match rd.tag(r) {
            Tag::Assign => rd.ast.rhs(Node(r)),
            _ => r,
        };
        if r == NONE {
            continue;
        }
        let ident = |k: &u32| rd.ttag(*k) == T::Ident && rd.t.text(*k) == p;
        out.extend((rd.first(r)..=rd.last(r)).filter(ident));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cholesky_of_a_two_by_two_and_a_refusal() {
        let l = cholesky(&[1.0, 0.6, 0.6, 1.0], 2).unwrap();
        assert_eq!(l[0], [1.0, 0.0]);
        assert!((l[1][0] - 0.6).abs() < 1e-15);
        assert!((l[1][1] - 0.8).abs() < 1e-15);
        assert_eq!(cholesky(&[1.0, 2.0, 2.0, 1.0], 2), Err(Code::W0020));
    }

    #[test]
    fn draws_by_distribution() {
        let v = |dist, percent| Vary {
            param: "p".into(),
            dist,
            spread: "10".into(),
            percent,
        };
        assert_eq!(
            draw(&v("gauss", true), "1k").unwrap(),
            "agauss(1k, (1k)*(10)/100, 1)"
        );
        assert_eq!(draw(&v("unif", false), "3").unwrap(), "aunif(3, 10)");
        assert_eq!(
            draw(&v("lnorm", false), "2").unwrap(),
            "exp(agauss(log(2), 10, 1))"
        );
        assert_eq!(draw(&v("lnorm", true), "2"), Err(Code::W0018));
    }
}
