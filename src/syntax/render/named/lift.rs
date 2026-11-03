//! The Monte Carlo lift (`Lift(RackVars)`), a rule over the expression
//! trees: each SPICE statistical call `f(args)` becomes a deterministic
//! `rack_f(args, v)` of a normalized variable `v`, declared `parameters
//! v=0` and varied in a `statistics` block, `process` at a deck's or
//! section's top and `mismatch` inside a subcircuit (monte-carlo-spectre.md
//! §5.1-§5.2). The rule records fixups, never touching the tree: each
//! lifted `Call` node's replacement, by its name token, in the reader
//! (`Rd::lift_call`; its views spell values with them), and the variables
//! and `rack_*` functions the writer declares ([`Lifted`]).
//!
//! The walk is the old pass's (`codegen::statistics::exprs_mut`): every
//! expression the old IR held as text, block by block (the top, its
//! definitions, each section's body and definitions; in each its
//! parameters, models, instances, elements and branches), each call in
//! post-order (inner calls first), so variables are named and declared in
//! its order.
//!
//! Cites: monte-carlo-spectre.md §5, §10.2, §11; ngspice `xpressn.c:45-85`
//! (the guards); Spectre Reference 19.1 p.481 (user functions), p.474 (no
//! short-circuiting); XY-RG Table 2-33 (Xyce's `limit`, a clamp).
//!
//! Contents:
//! - `pub(super) struct Lifted`, `pub(super) struct Var`
//! - `pub(super) fn lift(rd: &mut Rd, deck: &Deck, refused: &mut Vec<Diag>) -> Lifted`
//! - `Lifted::vars_at(&self, at: Option<usize>) -> impl Iterator<Item = &Var>`,
//!   `Lifted::block_at(&self, at: Option<usize>) -> Option<Stats>`,
//!   `Lifted::rack(&self) -> impl Iterator<Item = (&str, [&str; 4], &str)>`

use std::collections::HashSet;

use crate::syntax::ast::{NONE, Name, Node, Tag};
use crate::syntax::diag::{Code, Diag};
use crate::syntax::render::read::{Block, Card, Comp, Deck, Func, Lift, Rd, Stats, Vary};

/// The lift's declarations: its variables in walk order and the `rack_*`
/// functions the deck then calls.
#[derive(Default)]
pub(super) struct Lifted {
    vars: Vec<Var>,
    /// Which of [`RACK`] the lifted deck calls.
    rack: [bool; 5],
}

/// One lifted variable: its name, its scope (`mismatch` inside a
/// subcircuit), its distribution (uniform on [-1, 1], else normal) and the
/// section it is declared in (`None`: the deck's top).
pub(super) struct Var {
    pub(super) name: String,
    mismatch: bool,
    unif: bool,
    section: Option<usize>,
}

/// The five statistical functions (ngspice §2.11.5).
const STATISTICAL: [&str; 5] = ["agauss", "gauss", "aunif", "unif", "limit"];

/// The `rack_*` functions (§5.2, §11): `nom` for a zero variable, the
/// guards of ngspice's `xpressn.c:48-49`, `:59-62`, no branch dividing by
/// zero; an empty name is no argument.
const RACK: [(&str, [&str; 4], &str); 5] = [
    (
        "rack_agauss",
        ["nom", "avar", "sigma", "z"],
        "nom + ((avar > 0 && sigma > 0) ? avar : 0) / ((sigma > 0) ? sigma : 1) * z",
    ),
    (
        "rack_gauss",
        ["nom", "rvar", "sigma", "z"],
        "nom + nom * ((rvar > 0 && sigma > 0) ? rvar : 0) / ((sigma > 0) ? sigma : 1) * z",
    ),
    ("rack_aunif", ["nom", "avar", "u", ""], "nom + avar * u"),
    (
        "rack_unif",
        ["nom", "rvar", "u", ""],
        "nom + nom * rvar * u",
    ),
    // `nom` at u = 0, so the nominal run is exactly `nom`; a draw is never
    // exactly 0, so ngspice's `limit` keeps its two values.
    (
        "rack_limit",
        ["nom", "avar", "u", ""],
        "(u > 0) ? nom + avar : ((u < 0) ? nom - avar : nom)",
    ),
];

/// One expression the old IR held as text: the owner and field its
/// variables are named after, and the value items its text is spelled
/// from.
struct Site {
    owner: String,
    field: String,
    roots: Vec<u32>,
}

/// What the walk found in one block: where it sits, and its sites.
struct Scoped {
    subckt: Option<String>,
    section: Option<usize>,
    sites: Vec<Site>,
}

/// Lifts every statistical call of the deck `rd` read: records each
/// call's fixup in `rd` and returns the variables and functions to
/// declare; nothing when the deck calls none. A call with no Spectre form
/// is refused into `refused`.
pub(super) fn lift(rd: &mut Rd, deck: &Deck, refused: &mut Vec<Diag>) -> Lifted {
    let scoped = sites(rd, deck);
    let stat = |rd: &Rd, call: u32| {
        let (f, n) = rd.call_of(call);
        let f = f.to_ascii_lowercase();
        STATISTICAL.contains(&f.as_str()) && !(f == "limit" && n == 3)
    };
    let mut any = scoped
        .iter()
        .flat_map(|s| &s.sites)
        .flat_map(|s| &s.roots)
        .any(|&r| rd.calls_in(r).into_iter().any(|c| stat(rd, c)));
    // ponytail: a `.func` whose body calls one is inlined at each call site
    // by the old pass (§5.1); here it is refused, never written undrawn.
    for &f in &deck.funcs {
        let roots: Vec<u32> = match f {
            Func::Param(a) => vec![rd.ast.rhs(Node(a))],
            Func::Card(n) => rd.fields(n).to_vec(),
            Func::Real(n) => rd
                .body(n)
                .iter()
                .flat_map(|&s| rd.fields(s))
                .copied()
                .collect(),
        };
        let tok = match f {
            Func::Param(a) | Func::Card(a) | Func::Real(a) => rd.main(a),
        };
        if roots
            .iter()
            .any(|&r| rd.calls_in(r).into_iter().any(|c| stat(rd, c)))
        {
            any = true;
            refused.push(Diag {
                code: Code::W0107,
                token: tok,
            });
        }
    }
    if !any {
        return Lifted::default();
    }
    let mut out = Lifted::default();
    let mut used: HashSet<String> = HashSet::new();
    let mut fixes: Vec<(u32, Lift)> = Vec::new();
    for s in &scoped {
        for site in &s.sites {
            let mut k = 0;
            for &r in &site.roots {
                for call in rd.calls_in(r) {
                    let tok = rd.main(call);
                    let (name, args) = rd.call_of(call);
                    let f = name.to_ascii_lowercase();
                    let Some(f) = STATISTICAL.iter().copied().find(|s| *s == f) else {
                        if let Some(i) = RACK.iter().position(|r| r.0 == name) {
                            out.rack[i] = true;
                        }
                        continue;
                    };
                    if f == "limit" && args == 3 {
                        fixes.push((tok, Lift::Clamp));
                        continue;
                    }
                    let gauss = f.contains("gauss");
                    let sigma = match (gauss, args) {
                        // Xyce's two-argument agauss/gauss take sigma 1.
                        (true, 2) => true,
                        (true, 3) | (false, 2) => false,
                        // HSPICE's multiplier, or a wrong count (§5.3).
                        _ => {
                            refused.push(Diag {
                                code: Code::W0107,
                                token: tok,
                            });
                            continue;
                        }
                    };
                    k += 1;
                    let var = var_name(s.subckt.as_deref(), site, k, &mut used);
                    out.vars.push(Var {
                        name: var.clone(),
                        mismatch: s.subckt.is_some(),
                        unif: !gauss,
                        section: s.section,
                    });
                    let i = STATISTICAL
                        .iter()
                        .position(|s| *s == f)
                        .expect("a statistical call");
                    out.rack[i] = true;
                    fixes.push((tok, Lift::Call { f, sigma, var }));
                }
            }
        }
    }
    for (tok, l) in fixes {
        rd.lift_call(tok, l);
    }
    out
}

impl Lifted {
    /// The variables declared in section `at` (`None`: the deck's top).
    pub(super) fn vars_at(&self, at: Option<usize>) -> impl Iterator<Item = &Var> {
        self.vars.iter().filter(move |v| v.section == at)
    }

    /// The `statistics` block varying section `at`'s variables; `None`
    /// when it declares none.
    pub(super) fn block_at(&self, at: Option<usize>) -> Option<Stats> {
        let mut st = Stats::default();
        for v in self.vars_at(at) {
            let vary = Vary {
                param: v.name.clone(),
                dist: if v.unif { "unif" } else { "gauss" },
                spread: "1".into(),
                percent: false,
            };
            match v.mismatch {
                true => st.mismatch.push(vary),
                false => st.process.push(vary),
            }
        }
        (!st.process.is_empty() || !st.mismatch.is_empty()).then_some(st)
    }

    /// The `rack_*` functions the lifted deck calls, as (name, arguments,
    /// body).
    pub(super) fn rack(
        &self,
    ) -> impl Iterator<Item = (&'static str, [&'static str; 4], &'static str)> {
        RACK.into_iter()
            .zip(self.rack)
            .filter_map(|(r, used)| used.then_some(r))
    }
}

// ---- private ----

/// Every block's sites, in the old pass's order.
fn sites(rd: &Rd, deck: &Deck) -> Vec<Scoped> {
    let mut out = Vec::new();
    block(rd, &deck.top, None, None, &mut out);
    for sc in &deck.defs {
        block(rd, sc, Some(&sc.name), None, &mut out);
    }
    for (k, s) in deck.sections.iter().enumerate() {
        block(rd, &s.body, None, Some(k), &mut out);
        for sc in &s.defs {
            block(rd, sc, Some(&sc.name), Some(k), &mut out);
        }
    }
    out
}

/// Block `b`'s sites (inside subcircuit `subckt`, section `section`), then
/// its branches'.
fn block(rd: &Rd, b: &Block, subckt: Option<&str>, section: Option<usize>, out: &mut Vec<Scoped>) {
    let mut work = vec![b];
    while let Some(b) = work.pop() {
        let mut sites = Vec::new();
        let site = |owner: &str, field: &str, roots: Vec<u32>| Site {
            owner: owner.to_string(),
            field: field.to_string(),
            roots,
        };
        // Parameters: a subcircuit head's defaults, its `parameters`, its
        // `.param` locals.
        let head: Vec<u32> = match b.node {
            NONE => vec![],
            n => assigns(rd, rd.fields(n)),
        };
        for &a in head.iter().chain(&b.params).chain(&b.locals) {
            sites.push(site(&rd.key(a), "", vec![rd.ast.rhs(Node(a))]));
        }
        for &(m, bin) in &b.models {
            let name = rd.model_view(m, bin).name;
            let mut pairs = model_pairs(rd, m);
            if bin != NONE {
                pairs.extend(rd.pairs(bin));
            }
            for (k, v) in pairs {
                sites.push(site(&name, &k, vec![v]));
            }
        }
        for &(_, i) in &b.insts {
            let Card::Inst { name, .. } = rd.card_view(i) else {
                unreachable!("an instance's view");
            };
            for (k, v) in elem_pairs(rd, i) {
                sites.push(site(&format!("x{name}"), &k, vec![v]));
            }
        }
        for &c in &b.comps {
            if let Comp::Card(n) = c {
                card_sites(rd, n, &mut sites);
            }
        }
        out.push(Scoped {
            subckt: subckt.map(str::to_string),
            section,
            sites,
        });
        let branches = b.conds.iter().flat_map(|(_, c)| &c.branches);
        let mut inner: Vec<&Block> = branches.map(|(_, body)| body).collect();
        inner.reverse();
        work.extend(inner);
    }
}

/// Element `n`'s sites, as the old pass named them: an R, C or L value
/// first (its field the letter), a charge (`q`), then its parameters; a
/// source's and a semiconductor's every expression under `e`.
fn card_sites(rd: &Rd, n: u32, sites: &mut Vec<Site>) {
    let site = |owner: &str, field: &str, roots: Vec<u32>| Site {
        owner: owner.to_string(),
        field: field.to_string(),
        roots,
    };
    let pairs = elem_pairs(rd, n);
    match rd.card_view(n) {
        Card::Rcl {
            letter,
            name,
            value,
            ..
        } => {
            let l = letter.to_ascii_lowercase();
            let owner = format!("{l}{name}");
            let key = l.to_string();
            // The value: the item after the nodes, else the first `l=` pair
            // (all of them, read named).
            let positional = match rd.named() {
                true => None,
                false => rd
                    .items(n)
                    .get(2)
                    .copied()
                    .filter(|&v| rd.tag(v) != Tag::Assign),
            };
            let from_pair = positional.is_none();
            if let crate::syntax::render::read::Val::Expr(_) = value {
                let v = positional.or_else(|| {
                    pairs
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                        .map(|p| p.1)
                });
                if let Some(v) = v {
                    sites.push(site(&owner, &key, vec![v]));
                }
            }
            let mut skipped = false;
            for (k, v) in pairs {
                let is_key = k.eq_ignore_ascii_case(&key);
                if from_pair && is_key && (rd.named() || !skipped) {
                    skipped = true;
                    continue;
                }
                sites.push(site(&owner, &k, vec![v]));
            }
        }
        Card::Charge { name, .. } => {
            let owner = format!("c{name}");
            let q = pairs.iter().position(|(k, _)| k.eq_ignore_ascii_case("q"));
            if let Some(i) = q {
                sites.push(site(&owner, "q", vec![pairs[i].1]));
            }
            for (i, (k, v)) in pairs.into_iter().enumerate() {
                if Some(i) != q {
                    sites.push(site(&owner, &k, vec![v]));
                }
            }
        }
        // ponytail: a source's expressions in item order (the old pass:
        // the waveform's, then the value); no corpus source calls one.
        Card::Source { letter, name, .. } => {
            let owner = format!("{}{name}", letter.to_ascii_lowercase());
            let items: Vec<u32> = match rd.named() {
                true => pairs.into_iter().map(|p| p.1).collect(),
                false => rd.items(n).into_iter().skip(2).collect(),
            };
            for v in items {
                sites.push(site(&owner, "e", vec![v]));
            }
        }
        Card::Semi { letter, name, .. } => {
            let l = match letter {
                'D' | 'Q' | 'M' => letter.to_ascii_lowercase(),
                _ => 'e',
            };
            for (_, v) in pairs {
                sites.push(site(&format!("{l}{name}"), "e", vec![v]));
            }
        }
        // A behavioural source has no Spectre form (refused); the rest
        // carry no expression.
        Card::Behavioural { .. }
        | Card::Coupling { .. }
        | Card::Linear { .. }
        | Card::Poly { .. }
        | Card::Switch { .. }
        | Card::Line { .. }
        | Card::Inst { .. } => {}
    }
}

/// The `Assign` items among `items`.
fn assigns(rd: &Rd, items: &[u32]) -> Vec<u32> {
    items
        .iter()
        .copied()
        .filter(|&k| rd.tag(k) == Tag::Assign)
        .collect()
}

/// A model's `k=v` pairs (key, value item), in card order.
fn model_pairs(rd: &Rd, m: u32) -> Vec<(String, u32)> {
    match rd.named() {
        true => rd.pairs(m),
        false => pairs_of(rd, &assigns(rd, &rd.fields(m)[2..])),
    }
}

/// An element's `k=v` pairs (key, value item), in card order.
fn elem_pairs(rd: &Rd, n: u32) -> Vec<(String, u32)> {
    match rd.named() {
        true => rd.pairs(n),
        false => pairs_of(rd, &assigns(rd, &rd.items(n))),
    }
}

fn pairs_of(rd: &Rd, assigns: &[u32]) -> Vec<(String, u32)> {
    assigns
        .iter()
        .map(|&a| (rd.key(a), rd.ast.rhs(Node(a))))
        .collect()
}

/// `rack_<subckt>_<owner>_<field>_<k>`, identifier bytes only, a counter
/// suffix on a collision (§5.2, §11: the subcircuit names a mismatch
/// variable, as one local name recurs across a PDK's subcircuits).
fn var_name(subckt: Option<&str>, site: &Site, k: usize, used: &mut HashSet<String>) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| match c.is_ascii_alphanumeric() || c == '_' {
                true => c,
                false => '_',
            })
            .collect()
    };
    let mut base = String::from("rack_");
    if let Some(s) = subckt {
        base.push_str(&clean(s));
        base.push('_');
    }
    base.push_str(&clean(&site.owner));
    if !site.field.is_empty() {
        base.push('_');
        base.push_str(&clean(&site.field));
    }
    let base = format!("{base}_{k}");
    let mut name = base.clone();
    let mut n = 1;
    while !used.insert(name.clone()) {
        n += 1;
        name = format!("{base}_{n}");
    }
    name
}

impl Rd<'_> {
    /// The `Call` nodes of expression `root`, inner calls first, left to
    /// right (the order the old pass's scanner rewrote them in).
    fn calls_in(&self, root: u32) -> Vec<u32> {
        let a = self.ast;
        let mut out = Vec::new();
        let mut work = vec![(root, false)];
        while let Some((n, done)) = work.pop() {
            if n == NONE {
                continue;
            }
            if done {
                out.push(n);
                continue;
            }
            let node = Node(n);
            let at = a.rhs(node);
            // Children pushed last first, so the first is walked first.
            match a.tag(node) {
                Tag::Call => {
                    work.push((n, true));
                    let mut args = a.list(at + 1);
                    if self.unlifted_call(a.main_token(node)).is_some() {
                        args = &args[..args.len() - 1];
                    }
                    work.extend(args.iter().rev().map(|&k| (k, false)));
                }
                Tag::Unary | Tag::Paren | Tag::Delimited => work.push((a.lhs(node), false)),
                Tag::Binary | Tag::Assign => work.extend([(at, false), (a.lhs(node), false)]),
                Tag::Ternary => work.extend([
                    (a.extra(at + 1), false),
                    (a.extra(at), false),
                    (a.lhs(node), false),
                ]),
                Tag::Vector | Tag::List | Tag::Group => {
                    work.extend(a.list(at + 1).iter().rev().map(|&k| (k, false)));
                }
                Tag::Seq => work.extend(a.list(a.lhs(node)).iter().rev().map(|&k| (k, false))),
                Tag::Index => work.extend([(a.extra(at), false), (a.lhs(node), false)]),
                // else: leaves hold no call.
                _ => {}
            }
        }
        out
    }

    /// Call `n`'s name as it is written (a read-back call's statistical
    /// function) and its argument count (the read-back's variable left
    /// out).
    fn call_of(&self, n: u32) -> (&str, usize) {
        let node = Node(n);
        let args = self.ast.list(self.ast.rhs(node) + 1).len();
        match self.unlifted_call(self.ast.main_token(node)) {
            Some((f, _)) => (f, args - 1),
            None => (self.ast.name(Name(self.ast.lhs(node))), args),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::codegen::Dialect;
    use crate::syntax::render::render_text;

    #[test]
    fn a_call_becomes_a_varied_variable_in_its_scope() {
        // The oracle is monte-carlo-spectre.md §10.6's hand-worked text
        // (T-G1, T-G2, T-G4): a top call varies in `process`, a
        // subcircuit's in `mismatch`, each named after its owner and field,
        // its arguments kept as written.
        let deck = "t\n\
                    R1 a 0 {agauss(1k, 100, 3)}\n\
                    .subckt s p\n\
                    .param w={aunif(1, 0.1)}\n\
                    R2 p 0 {w}\n\
                    .ends\n\
                    .end\n";
        let out = render_text(deck, Dialect::Ngspice, "t", Dialect::Spectre).expect("written");
        for want in [
            "r1 (a 0) resistor r=rack_agauss(1k, 100, 3, rack_r1_r_1)",
            "parameters w=rack_aunif(1, 0.1, rack_s_w_1)",
            "parameters rack_r1_r_1=0 rack_s_w_1=0",
            "    process {\n        vary rack_r1_r_1 dist=gauss std=1\n    }",
            "    mismatch {\n        vary rack_s_w_1 dist=unif N=1\n    }",
            "real rack_agauss(real nom, real avar, real sigma, real z) {",
            "real rack_aunif(real nom, real avar, real u) {",
        ] {
            assert!(out.contains(want), "no `{want}` in\n{out}");
        }
    }

    #[test]
    fn a_nested_call_gets_its_own_variable_and_a_clamp_gets_none() {
        // T-G3 (two variables, inner first) and T-G9 (Xyce's three-argument
        // `limit` is a clamp, its two-argument `agauss` sigma 1).
        let deck = "t\n\
                    R1 a 0 {agauss(agauss(0,1,1),1,1) + limit(x,1,2) + agauss(1,2)}\n\
                    .end\n";
        let out = render_text(deck, Dialect::Xyce, "t", Dialect::Spectre).expect("written");
        let want = "r=rack_agauss(rack_agauss(0,1,1, rack_r1_r_1),1,1, rack_r1_r_2) \
                    + min(max(x, 1), 2) + rack_agauss(1,2,1, rack_r1_r_3)";
        assert!(out.contains(want), "no `{want}` in\n{out}");
    }
}
