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
//! expression the old IR held as text (`sites`), each call in post-order
//! (inner calls first), so variables are named and declared in its order. A top-level `.func` whose body calls a statistical function
//! is inlined at each call, as the old pass's text was (`(body)`, each
//! parameter `(arg)`), each inlined call's draws its own; the call's fixup
//! is a script of steps (`Fix::Inline`), as its body's tokens are spelled
//! once per call.
//!
//! VACASK has `agauss`, `gauss`, `aunif` and `unif`, drawn once per call
//! site per scope as Spectre's process/mismatch split (VACASK
//! `docs/cmd-analysis-mc.md` lines 31-61), so its rule
//! (`Lift(VacaskCalls)`, [`vacask_calls`]) spells out only what it lacks.
//!
//! Cites: monte-carlo-spectre.md §5, §10.2, §11; ngspice `xpressn.c:45-85`
//! (the guards); Spectre Reference 19.1 p.481 (user functions), p.474 (no
//! short-circuiting); XY-RG Table 2-33 (Xyce's `limit`, a clamp).
//!
//! Contents:
//! - `pub(super) struct Lifted`, `pub(super) struct Var`
//! - `pub(super) fn lift(rd: &mut Rd, deck: &Deck, refused: &mut Vec<Diag>) -> Lifted`
//! - `pub(super) fn vacask_calls(rd: &mut Rd, deck: &Deck, refused: &mut Vec<Diag>)`
//! - `Lifted::writes(&self, f: Func) -> bool`,
//!   `Lifted::vars_at(&self, at: Option<usize>) -> impl Iterator<Item = &Var>`,
//!   `Lifted::block_at(&self, at: Option<usize>) -> Option<Stats>`,
//!   `Lifted::rack(&self) -> impl Iterator<Item = (&str, [&str; 4], &str)>`

use std::collections::HashSet;

use super::sites::{Site, sites};
use crate::syntax::ast::{NONE, Name, Node, Tag};
use crate::syntax::diag::{Code, Diag};
use crate::syntax::render::read::{Deck, Func, Lift, Rd, Stats, Step, Vary};

/// The lift's declarations: its variables in walk order and the `rack_*`
/// functions the deck then calls.
#[derive(Default)]
pub(super) struct Lifted {
    vars: Vec<Var>,
    /// Which of [`RACK`] the lifted deck calls.
    rack: [bool; 5],
    /// The functions inlined at their calls, by node: not written.
    inlined: HashSet<u32>,
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

/// A `.func` whose body calls a statistical function, inlined at each
/// call (§10.2 step 2): its name lowercased (the old pass matched a call
/// ignoring case), its parameters, the items its body's nodes are under,
/// and the tokens its body is spelled from.
struct Drawn {
    name: String,
    params: Vec<String>,
    roots: Vec<u32>,
    span: (u32, u32),
}

/// One item of [`Rd::expand`]'s walk: a node to enter (inside the inlined
/// call `frame`, its steps in script `script`), or a call to emit after
/// its arguments (`slot` its step).
enum Walk {
    Enter {
        n: u32,
        frame: Option<usize>,
        script: Option<usize>,
    },
    Exit {
        n: u32,
        slot: Option<(usize, usize)>,
    },
}

/// Lifts every statistical call of the deck `rd` read: records each
/// call's fixup in `rd` and returns the variables and functions to
/// declare; nothing when the deck calls none. A top-level `.func` whose
/// body calls one is inlined at each of its calls, each inlined call drawn
/// on its own (§10.2 step 2), and is not written. A call with no Spectre
/// form is refused into `refused`.
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
    let mut out = Lifted::default();
    let mut drawn = Vec::new();
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
        let n = match f {
            Func::Param(a) | Func::Card(a) | Func::Real(a) => a,
        };
        if !roots
            .iter()
            .any(|&r| rd.calls_in(r).into_iter().any(|c| stat(rd, c)))
        {
            continue;
        }
        any = true;
        out.inlined.insert(n);
        let Some(span) = rd.func_span(f) else {
            // A call found outside the body's tokens: nothing to inline.
            refused.push(Diag {
                code: Code::W0107,
                token: rd.main(n),
            });
            continue;
        };
        let (name, params, _) = rd.func_view(f);
        drawn.push(Drawn {
            name: name.to_ascii_lowercase(),
            params,
            roots,
            span,
        });
    }
    if !any {
        return Lifted::default();
    }
    let mut used: HashSet<String> = HashSet::new();
    let mut fixes: Vec<(u32, Lift)> = Vec::new();
    let mut scripts: Vec<(u32, Vec<(u32, Step)>)> = Vec::new();
    for s in &scoped {
        for site in &s.sites {
            let mut k = 0;
            for &r in &site.roots {
                for (call, slot) in rd.expand(r, &drawn, &mut scripts) {
                    let tok = rd.main(call);
                    let mut put = |l: Lift| match slot {
                        None => fixes.push((tok, l)),
                        Some((s, i)) => scripts[s].1[i].1 = Step::Call(Some(l)),
                    };
                    let (name, args) = rd.call_of(call);
                    let f = name.to_ascii_lowercase();
                    let Some(f) = STATISTICAL.iter().copied().find(|s| *s == f) else {
                        if let Some(i) = RACK.iter().position(|r| r.0 == name) {
                            out.rack[i] = true;
                        }
                        continue;
                    };
                    if f == "limit" && args == 3 {
                        put(Lift::Clamp);
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
                    put(Lift::Call { f, sigma, var });
                }
            }
        }
    }
    for (tok, l) in fixes {
        rd.lift_call(tok, l);
    }
    for (tok, steps) in scripts {
        rd.inline_call(tok, steps);
    }
    out
}

/// Spells out the statistical calls VACASK lacks (§7, §10.3), as fixups
/// in `rd`: `limit(nom, avar)` as an `aunif` ternary, Xyce's clamp
/// `limit(x, lo, hi)` as `min(max())`, a two-argument `agauss`/`gauss` with
/// its sigma 1; nothing when the deck calls none. HSPICE's four-argument
/// form is refused into `refused`.
pub(super) fn vacask_calls(rd: &mut Rd, deck: &Deck, refused: &mut Vec<Diag>) {
    let scoped = sites(rd, deck);
    let calls: Vec<u32> = scoped
        .iter()
        .flat_map(|s| &s.sites)
        .flat_map(|s| &s.roots)
        .flat_map(|&r| rd.calls_in(r))
        .collect();
    let stat = |rd: &Rd, call: u32| {
        let (f, n) = rd.call_of(call);
        let f = f.to_ascii_lowercase();
        STATISTICAL.contains(&f.as_str()) && !(f == "limit" && n == 3)
    };
    if !calls.iter().any(|&c| stat(rd, c)) {
        return;
    }
    let mut fixes = Vec::new();
    for call in calls {
        let (name, args) = rd.call_of(call);
        let f = name.to_ascii_lowercase();
        let Some(f) = STATISTICAL.iter().copied().find(|s| *s == f) else {
            continue;
        };
        let tok = rd.main(call);
        match (f, args) {
            ("limit", 2) => fixes.push((tok, Lift::Limit)),
            ("limit", 3) => fixes.push((tok, Lift::Clamp)),
            ("agauss" | "gauss", 2) => fixes.push((tok, Lift::Sigma(f))),
            ("agauss" | "gauss", 3) | ("aunif" | "unif", 2) => {}
            _ => refused.push(Diag {
                code: Code::W0107,
                token: tok,
            }),
        }
    }
    for (tok, l) in fixes {
        rd.lift_call(tok, l);
    }
}

impl Lifted {
    /// Whether function `f` is written: not when it is inlined at its
    /// calls.
    pub(super) fn writes(&self, f: Func) -> bool {
        let (Func::Param(n) | Func::Card(n) | Func::Real(n)) = f;
        !self.inlined.contains(&n)
    }

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
        let calls = self.expand(root, &[], &mut Vec::new());
        calls.into_iter().map(|(c, _)| c).collect()
    }

    /// The `Call` nodes of expression `root` as [`Rd::calls_in`] orders
    /// them, each call of a `drawn` function inlined as the old pass's text
    /// was: its body's calls in its place, an argument's at each use of its
    /// parameter. Each inlined call outside another opens a script in
    /// `scripts` (its name token, its steps in the order its text meets
    /// them); a statistical call inside one comes with its step's slot.
    fn expand(
        &self,
        root: u32,
        drawn: &[Drawn],
        scripts: &mut Vec<(u32, Vec<(u32, Step)>)>,
    ) -> Vec<(u32, Option<(usize, usize)>)> {
        let a = self.ast;
        let mut out = Vec::new();
        // The inlined calls: the function and its argument nodes.
        let mut frames: Vec<(usize, Vec<u32>)> = Vec::new();
        let mut work = vec![Walk::Enter {
            n: root,
            frame: None,
            script: None,
        }];
        while let Some(w) = work.pop() {
            let (n, frame, script) = match w {
                Walk::Exit { n, slot } => {
                    out.push((n, slot));
                    continue;
                }
                Walk::Enter { n, frame, script } => (n, frame, script),
            };
            if n == NONE {
                continue;
            }
            let node = Node(n);
            let at = a.rhs(node);
            let tok = a.main_token(node);
            // Inside an inlined body, only its tokens are spelled.
            let spelled = frame.is_none_or(|f| {
                let (x, y) = drawn[frames[f].0].span;
                (x..=y).contains(&tok)
            });
            let enter = |k: u32| Walk::Enter {
                n: k,
                frame,
                script,
            };
            // Children pushed last first, so the first is walked first.
            match a.tag(node) {
                Tag::Ident => {
                    let Some(f) = frame.filter(|_| spelled) else {
                        continue;
                    };
                    let (d, args) = (&drawn[frames[f].0], &frames[f].1);
                    // quirk: the old pass's `rename_idents`: a parameter
                    // matched as written, an argument it has none for kept.
                    let text = self.text(n);
                    let arg = d.params.iter().position(|p| *p == text);
                    if let Some(&arg) = arg.and_then(|j| args.get(j)) {
                        let s = script.expect("a body is inlined in a script");
                        let step = Step::Subst(self.first(arg), self.last(arg));
                        scripts[s].1.push((tok, step));
                        work.push(Walk::Enter {
                            n: arg,
                            frame: None,
                            script,
                        });
                    }
                }
                Tag::Call => {
                    let unlifted = self.unlifted_call(tok).is_some();
                    let mut args = a.list(at + 1);
                    if unlifted {
                        args = &args[..args.len() - 1];
                    }
                    let name = self.call_of(n).0.to_ascii_lowercase();
                    // quirk: the old pass inlined once, so a call inside an
                    // inlined body stays a call.
                    let d = drawn.iter().position(|d| d.name == name);
                    if let Some(d) = d.filter(|_| frame.is_none() && !unlifted) {
                        let step = (tok, Step::Inline(drawn[d].span.0, drawn[d].span.1));
                        let s = match script {
                            Some(s) => {
                                scripts[s].1.push(step);
                                s
                            }
                            None => {
                                scripts.push((tok, vec![step]));
                                scripts.len() - 1
                            }
                        };
                        frames.push((d, args.to_vec()));
                        let f = frames.len() - 1;
                        work.extend(drawn[d].roots.iter().rev().map(|&r| Walk::Enter {
                            n: r,
                            frame: Some(f),
                            script: Some(s),
                        }));
                        continue;
                    }
                    if spelled {
                        let slot = match script {
                            Some(s) if STATISTICAL.contains(&name.as_str()) => {
                                scripts[s].1.push((tok, Step::Call(None)));
                                Some((s, scripts[s].1.len() - 1))
                            }
                            _ => None,
                        };
                        work.push(Walk::Exit { n, slot });
                    }
                    work.extend(args.iter().rev().map(|&k| enter(k)));
                }
                Tag::Unary | Tag::Paren | Tag::Delimited => work.push(enter(a.lhs(node))),
                Tag::Binary | Tag::Assign => work.extend([enter(at), enter(a.lhs(node))]),
                Tag::Ternary => work.extend([
                    enter(a.extra(at + 1)),
                    enter(a.extra(at)),
                    enter(a.lhs(node)),
                ]),
                Tag::Vector | Tag::List | Tag::Group => {
                    work.extend(a.list(at + 1).iter().rev().map(|&k| enter(k)));
                }
                Tag::Seq => work.extend(a.list(a.lhs(node)).iter().rev().map(|&k| enter(k))),
                Tag::Index => work.extend([enter(a.extra(at)), enter(a.lhs(node))]),
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
        let out = render_text(deck, Dialect::Ngspice, "t", Dialect::Spectre)
            .map(|w| w.text)
            .unwrap_or_else(|_| panic!("written"));
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
        let out = render_text(deck, Dialect::Xyce, "t", Dialect::Spectre)
            .map(|w| w.text)
            .unwrap_or_else(|_| panic!("written"));
        let want = "r=rack_agauss(rack_agauss(0,1,1, rack_r1_r_1),1,1, rack_r1_r_2) \
                    + min(max(x, 1), 2) + rack_agauss(1,2,1, rack_r1_r_3)";
        assert!(out.contains(want), "no `{want}` in\n{out}");
    }

    #[test]
    fn the_decks_spectre_ran_are_written_byte_for_byte() {
        // The oracle is `tests/spectre/`: the T-S1 to T-S3 decks real
        // Spectre 21.1 ran (monte-carlo-spectre.md §11), written from these
        // SPICE sources. A `montecarlo` statement has no SPICE source: the
        // committed deck is the written one and its `mc1` statement.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/spectre");
        let divider =
            |r: &str| format!("t\nv1 in 0 1\nr1 in out r='{r}'\nr2 out 0 1k\n.op\n.end\n");
        let pair = "t\n.param g_r='agauss(0,100,1)'\n.param g=g_r\n.subckt s a b\n.param rl='agauss(1k,100,1)'\nr1 a b {rl+g}\nr2 b 0 1k\n.ends s\nv1 in 0 1\nx1 in o1 s\nx2 in o2 s\n.op\n.end\n";
        let nested = "t\n.subckt leaf a b\n.param rl='agauss(1k,100,1)'\nr1 a b {rl}\nr2 b 0 1k\n.ends leaf\n.subckt two a b c\nxa a b leaf\nxb a c leaf\n.ends two\nv1 in 0 1\nx1 in o1 o2 two\nx2 in o3 o4 two\n.op\n.end\n";
        let decks = [
            (
                "t_s1_nominal.scs",
                divider("agauss(1k,100,3)+aunif(0,100)+limit(0,50)"),
                None,
            ),
            ("t_s2_agauss.scs", divider("agauss(1k,100,3)"), Some("all")),
            ("t_s2_aunif.scs", divider("aunif(1k,100)"), Some("all")),
            ("t_s2_limit.scs", divider("limit(1k,50)"), Some("all")),
            ("t_s3_mismatch.scs", pair.to_string(), Some("mismatch")),
            ("t_s3_nested.scs", nested.to_string(), Some("mismatch")),
        ];
        for (file, deck, variations) in decks {
            let mut text = render_text(&deck, Dialect::Ngspice, file, Dialect::Spectre)
                .map(|w| w.text)
                .unwrap_or_else(|_| panic!("{file}"));
            text.push('\n');
            if let Some(v) = variations {
                text.push_str(&format!(
                    "mc1 montecarlo numruns=2000 seed=1 variations={v} {{\n  dcmc dc\n}}\n"
                ));
            }
            let want = std::fs::read_to_string(dir.join(file)).expect(file);
            assert_eq!(text, want, "{file}");
        }
    }

    #[test]
    fn vacask_gets_its_four_calls_as_written_and_limit_spelled_out() {
        // The oracle is monte-carlo-spectre.md §7: VACASK has agauss,
        // gauss, aunif and unif with ngspice's arguments, no `limit` (an
        // `aunif` ternary), and Xyce's sigma-less `agauss` takes sigma 1.
        let deck = "t\nR1 a 0 {agauss(1k,100,3) + limit(0, 50)}\nR2 a 0 {agauss(1, 2)}\n.end\n";
        let out = render_text(deck, Dialect::Xyce, "t", Dialect::Vacask)
            .map(|w| w.text)
            .unwrap_or_else(|_| panic!("written"));
        for want in [
            "r1 (a 0) resistor r=agauss(1k,100,3) + ((aunif(0, 1) > 0) ? (0) + ( 50) : (0) - ( 50))",
            "r2 (a 0) resistor r=agauss(1, 2,1)",
        ] {
            assert!(out.contains(want), "no `{want}` in\n{out}");
        }
    }
}
