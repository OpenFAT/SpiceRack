//! The Monte Carlo lift read back, a rule over the expression trees: in a Spectre deck, a call
//! `rack_f(args, v)` whose `v` is declared `parameters v=0` and varied with
//! exactly `dist=gauss std=1` (agauss, gauss) or `dist=unif N=1` (aunif,
//! unif, limit) is the call `f(args)`. The rule records fixups, never
//! touching the tree: each such `Call` node's replacement (its new name and
//! the token where its last argument starts), the variables it used (their
//! `parameters` and `vary` rows go), and the `rack_*` functions nothing
//! calls any more (they go). [`Rd::etext`] spells a value with them, and
//! with a writer's `Lift` fixups ([`Fix`]: a token respelled,
//! [`Rd::respell_token`]; a statistical call rewritten, [`Rd::lift_call`];
//! a `.func` call inlined, [`Rd::inline_call`]), one table keyed by token,
//! recorded before the writer writes.
//!
//! Cites: the `rack_*` functions of `render::named::lift`; the old writer's `statistics::unlift`, which did this over the IR's text.
//!
//! Contents:
//! - `pub(super) struct Unlift`, `pub(in crate::syntax::render) enum Fix`, `enum Lift`, `enum Step`
//! - `Rd::unlift(&mut self, deck: &mut Deck)`
//! - `Rd::etext(&self, n: u32) -> String`, `Rd::espan(&self, a: u32, b: u32) -> String`,
//!   `Rd::unlifted_var(&self, p: &str) -> bool`
//! - `Rd::respell_token(&mut self, k: u32, text: String)`, `Rd::lift_call(&mut self, name: u32, l: Lift)`,
//!   `Rd::inline_call(&mut self, name: u32, steps: Vec<(u32, Step)>)`,
//!   `Rd::unlifted_call(&self, name: u32) -> Option<(&'static str, u32)>`,
//!   `Rd::value_roots(&self, b: &Block, out: &mut Vec<u32>)`

use std::collections::{HashMap, HashSet};

use super::{Block, Comp, Deck, Func, Rd};
use crate::codegen::Dialect;
use crate::syntax::ast::{NONE, Node, Tag};

/// The fixups of the read-back rule.
#[derive(Default)]
pub(super) struct Unlift {
    /// A rewritten call, by its name token: the statistical function it
    /// calls, and the comma before its last argument.
    calls: HashMap<u32, (&'static str, u32)>,
    /// The variables the rewritten calls used.
    used: HashSet<String>,
}

/// A writer's fixup of one token (`Rd::fixes`).
#[derive(Clone, Debug)]
pub(in crate::syntax::render) enum Fix {
    /// The token is spelled as this text (a reference renamed with its
    /// declaration).
    Respell(String),
    /// The call whose name is the token is rewritten.
    Lift(Lift),
    /// The call whose name is the token is a function inlined: spelled by
    /// these steps, the first its own [`Step::Inline`].
    Inline(Vec<(u32, Step)>),
}

/// One step of spelling an inlined call, at a token: the steps of one
/// [`Fix::Inline`] are in the order its expansion's text meets their
/// tokens, a body's token met once per call and an argument's once per
/// use, so they are a script, not a table.
#[derive(Clone, Debug)]
pub(in crate::syntax::render) enum Step {
    /// The call whose name is the token is spelled `(body)`, the body the
    /// tokens `a..=b`.
    Inline(u32, u32),
    /// The token, a parameter of the inlined body, is spelled `(arg)`, the
    /// argument the tokens `a..=b`.
    Subst(u32, u32),
    /// The call whose name is the token is a statistical call: rewritten,
    /// or spelled as written (`None`).
    Call(Option<Lift>),
}

/// A statistical call a writer rewrites (Spectre's `rack_*` lift, VACASK's
/// spelled-out calls), by the `Call` node's name token.
#[derive(Clone, Debug)]
pub(in crate::syntax::render) enum Lift {
    /// `f(args)` → `rack_f(args, var)` (Spectre); `sigma` appends Xyce's
    /// default sigma (`,1`) to a two-argument `agauss`/`gauss`.
    Call {
        f: &'static str,
        sigma: bool,
        var: String,
    },
    /// `f(nom, var)` → `f(nom, var,1)`: Xyce's two-argument `agauss`/`gauss`
    /// with its sigma (VACASK).
    Sigma(&'static str),
    /// `limit(nom, avar)` → `((aunif(0, 1) > 0) ? (nom) + (avar) : (nom) -
    /// (avar))` (VACASK has no `limit`).
    Limit,
    /// Xyce's three-argument `limit(x, lo, hi)` → `min(max(x, lo), hi)`.
    Clamp,
}

/// The five statistical functions (ngspice §2.11.5).
const STATISTICAL: [&str; 5] = ["agauss", "gauss", "aunif", "unif", "limit"];

/// The `rack_*` functions SpiceRack's lift defines.
const RACK: [&str; 5] = [
    "rack_agauss",
    "rack_gauss",
    "rack_aunif",
    "rack_unif",
    "rack_limit",
];

/// The steps of one [`Fix::Inline`] and how many are spelled.
struct Cursor<'s> {
    steps: &'s [(u32, Step)],
    at: usize,
}

impl<'a> Rd<'a> {
    /// Records the read-back rule's fixups for a Spectre deck and drops
    /// what it frees from `deck`: the used variables' `parameters`, their
    /// `vary` rows (an emptied `statistics` block with them), and the
    /// `rack_*` functions no value calls.
    pub(super) fn unlift(&mut self, deck: &mut Deck) {
        if self.d != Dialect::Spectre {
            return;
        }
        let mut kinds: HashMap<String, bool> = HashMap::new(); // var -> unif
        let bodies = std::iter::once(&deck.top).chain(deck.sections.iter().map(|s| &s.body));
        for b in bodies {
            let zero = |p: &str| {
                b.params
                    .iter()
                    .any(|&a| self.param_view(a) == (p.to_string(), "0".to_string()))
            };
            for &st in &b.statistics {
                let s = self.statistics(st);
                for v in s.process.iter().chain(&s.mismatch) {
                    let shape = match v.dist {
                        "gauss" if v.spread == "1" && !v.percent => Some(false),
                        "unif" if v.spread == "1" && !v.percent => Some(true),
                        _ => None,
                    };
                    if let Some(unif) = shape
                        && zero(&v.param)
                    {
                        kinds.insert(v.param.clone(), unif);
                    }
                }
            }
        }
        if kinds.is_empty() {
            return;
        }
        let mut values = Vec::new();
        for b in all_blocks(deck) {
            self.value_roots(b, &mut values);
        }
        let mut left: HashSet<String> = HashSet::new();
        let mut work = values;
        while let Some(n) = work.pop() {
            if n == NONE {
                continue;
            }
            let node = Node(n);
            match self.tag(n) {
                Tag::Call => {
                    let args = self.ast.list(self.ast.rhs(node) + 1);
                    let name = self.ast.name(crate::syntax::ast::Name(self.ast.lhs(node)));
                    let hit = name.strip_prefix("rack_").and_then(|f| {
                        let f = STATISTICAL.iter().copied().find(|s| *s == f)?;
                        let unif = matches!(f, "aunif" | "unif" | "limit");
                        let (&last, _) = args.split_last()?;
                        let v = (self.tag(last) == Tag::Ident).then(|| self.text(last))?;
                        (kinds.get(&v) == Some(&unif)).then_some((f, v, last))
                    });
                    match hit {
                        Some((f, v, last)) => {
                            let cut = self.first(last) - 1;
                            self.unlifted.calls.insert(self.main(n), (f, cut));
                            self.unlifted.used.insert(v);
                            left.insert(f.to_string());
                        }
                        None => {
                            left.insert(name.to_string());
                        }
                    }
                    work.extend(args);
                }
                Tag::Unary | Tag::Paren | Tag::Delimited => work.push(self.ast.lhs(node)),
                Tag::Binary | Tag::Assign => work.extend([self.ast.lhs(node), self.ast.rhs(node)]),
                Tag::Ternary => {
                    let at = self.ast.rhs(node);
                    work.extend([
                        self.ast.lhs(node),
                        self.ast.extra(at),
                        self.ast.extra(at + 1),
                    ]);
                }
                Tag::Vector | Tag::List | Tag::Group => {
                    work.extend(self.ast.list(self.ast.rhs(node) + 1));
                }
                Tag::Seq => work.extend(self.ast.list(self.ast.lhs(node))),
                Tag::Index => work.extend([self.ast.lhs(node), self.ast.extra(self.ast.rhs(node))]),
                // else: leaves hold no call.
                _ => {}
            }
        }
        let used = &self.unlifted.used;
        let keep = |rd: &Rd, a: &u32| !used.contains(&rd.param_view(*a).0);
        let me: &Rd = self;
        deck.top.params.retain(|a| keep(me, a));
        for s in &mut deck.sections {
            s.body.params.retain(|a| keep(me, a));
        }
        let empty = |rd: &Rd, st: &u32| {
            let s = rd.statistics(*st);
            s.process.is_empty()
                && s.mismatch.is_empty()
                && s.correlations.is_empty()
                && s.truncate.iter().all(Option::is_none)
        };
        deck.top.statistics.retain(|st| !empty(me, st));
        for s in &mut deck.sections {
            s.body.statistics.retain(|st| !empty(me, st));
        }
        deck.funcs.retain(|&f| match f {
            Func::Real(n) => {
                let name = me.real_func(n).0;
                !RACK.contains(&name.as_str()) || left.contains(&name)
            }
            Func::Param(_) | Func::Card(_) => true,
        });
    }

    /// Item `n`'s text, each call the read-back rule rewrote spelled as
    /// the statistical call (`rack_agauss(1k, 100, 3, v)` is
    /// `agauss(1k, 100, 3)`).
    pub(in crate::syntax::render) fn etext(&self, n: u32) -> String {
        if self.unlifted.calls.is_empty() && self.fixes.is_empty() {
            return self.text(n);
        }
        self.espan(self.first(n), self.last(n))
    }

    /// Whether the read-back rule dropped `vary` rows of variable `p`.
    pub(super) fn unlifted_var(&self, p: &str) -> bool {
        self.unlifted.used.contains(p)
    }

    /// Token `k` of a value is spelled `text` wherever [`Rd::etext`] spells
    /// it: a writer's `Lift` fixup, recorded before it writes.
    ///
    /// # Panics
    /// When `k` is no identifier token.
    pub(in crate::syntax::render) fn respell_token(&mut self, k: u32, text: String) {
        assert_eq!(
            self.ttag(k),
            crate::syntax::token::Tag::Ident,
            "an identifier"
        );
        assert!(!text.is_empty(), "a token spelled as something");
        self.fixes.insert(k, Fix::Respell(text));
    }

    /// The call whose name is token `name` is spelled as `l` wherever
    /// [`Rd::etext`] spells it: a writer's `Lift` fixup, recorded before it
    /// writes.
    pub(in crate::syntax::render) fn lift_call(&mut self, name: u32, l: Lift) {
        self.fixes.insert(name, Fix::Lift(l));
    }

    /// The call whose name is token `name` is spelled by `steps` wherever
    /// [`Rd::etext`] spells it: a writer's `Lift` fixup, recorded before it
    /// writes.
    ///
    /// # Panics
    /// When the first step is not `name`'s [`Step::Inline`].
    pub(in crate::syntax::render) fn inline_call(&mut self, name: u32, steps: Vec<(u32, Step)>) {
        assert!(
            matches!(steps.first(), Some(&(k, Step::Inline(..))) if k == name),
            "a script opening with its own call"
        );
        self.fixes.insert(name, Fix::Inline(steps));
    }

    /// The read-back of the call whose name is token `name`: the
    /// statistical function it reads as, and the comma before the
    /// variable it drops.
    pub(in crate::syntax::render) fn unlifted_call(
        &self,
        name: u32,
    ) -> Option<(&'static str, u32)> {
        self.unlifted.calls.get(&name).copied()
    }

    /// Tokens `a..=b` as [`Rd::span`] joins them, the read-back's calls
    /// spelled as their statistical call and a writer's fixups applied.
    pub(in crate::syntax::render) fn espan(&self, a: u32, b: u32) -> String {
        self.spell(a, b, None)
    }

    /// The value nodes of block `b` the old IR held as expressions:
    /// parameter values, model and instance parameters, element fields.
    pub(in crate::syntax::render) fn value_roots(&self, b: &Block, out: &mut Vec<u32>) {
        out.extend(
            b.params
                .iter()
                .chain(&b.locals)
                .map(|&a| self.ast.rhs(Node(a))),
        );
        for &(m, bin) in &b.models {
            out.extend(self.pairs(m).into_iter().map(|(_, v)| v));
            if bin != NONE {
                out.extend(self.pairs(bin).into_iter().map(|(_, v)| v));
            }
        }
        let elems = b.comps.iter().filter_map(|c| match c {
            Comp::Card(n) => Some(*n),
            Comp::Raw(_) => None,
        });
        for n in elems.chain(b.insts.iter().map(|&(_, n)| n)) {
            out.extend(self.fields(n));
        }
    }
}

// ---- private ----

impl<'a> Rd<'a> {
    /// The `)` closing the call whose name is token `name`.
    fn call_close(&self, name: u32) -> u32 {
        let mut depth = 0i32;
        let mut k = name + 1;
        loop {
            match self.ttag(k) {
                crate::syntax::token::Tag::LParen => depth += 1,
                crate::syntax::token::Tag::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return k;
                    }
                }
                _ => {}
            }
            k += 1;
        }
    }
    /// Tokens `a..=b` as [`Rd::espan`] spells them; inside an inlined call,
    /// `cur` the script its tokens are spelled by.
    fn spell(&self, a: u32, b: u32, mut cur: Option<&mut Cursor>) -> String {
        let mut out = String::new();
        let mut k = a;
        while k <= b {
            if k > a {
                out.push_str(&self.gap(k - 1, k));
            }
            let step = cur.as_deref_mut().and_then(|c| c.take(k));
            let unlifted = self.unlifted.calls.get(&k).copied();
            let fix = match step {
                Some(_) => None,
                None => self.fixes.get(&k),
            };
            let lift = match (step, fix) {
                (Some(Step::Call(l)), _) => l.as_ref(),
                (None, Some(Fix::Lift(l))) => Some(l),
                _ => None,
            };
            match (step, fix) {
                (Some(&Step::Inline(x, y)), _) => {
                    out.push('(');
                    out.push_str(&self.spell(x, y, cur.as_deref_mut()));
                    out.push(')');
                    k = self.call_close(k) + 1;
                    continue;
                }
                (Some(&Step::Subst(x, y)), _) => {
                    out.push('(');
                    out.push_str(&self.spell(x, y, cur.as_deref_mut()));
                    out.push(')');
                    k += 1;
                    continue;
                }
                (None, Some(Fix::Inline(steps))) => {
                    assert!(cur.is_none(), "an inlined call is one script");
                    let close = self.call_close(k);
                    let mut c = Cursor { steps, at: 0 };
                    out.push_str(&self.spell(k, close, Some(&mut c)));
                    assert_eq!(c.at, steps.len(), "every step spelled");
                    k = close + 1;
                    continue;
                }
                _ => {}
            }
            match (unlifted, lift, fix) {
                (_, Some(l), _) => {
                    let close = self.call_close(k);
                    // The arguments' text, the read-back's variable left out.
                    let end = unlifted.map_or(close, |(_, cut)| cut);
                    out.push_str(&self.lifted(k, end, l, cur.as_deref_mut()));
                    k = close + 1;
                }
                (Some((f, cut)), None, _) => {
                    let close = self.call_close(k);
                    out.push_str(f);
                    out.push_str(&self.spell(k + 1, cut - 1, cur.as_deref_mut()));
                    out.push_str(&self.gap(cut - 1, cut));
                    out.push(')');
                    k = close + 1;
                }
                (None, None, Some(Fix::Respell(t))) => {
                    out.push_str(t);
                    k += 1;
                }
                (None, None, _) => {
                    out.push_str(self.t.text(k));
                    k += 1;
                }
            }
        }
        out
    }

    /// The lifted spelling of the call named at token `name`, its
    /// arguments the tokens between `name + 1` (its `(`) and `end` (its
    /// `)`, or the read-back's comma), each as the old writer's
    /// `rewrite_calls` joined them: the text between the delimiters kept.
    fn lifted(&self, name: u32, end: u32, l: &Lift, mut cur: Option<&mut Cursor>) -> String {
        let open = name + 1;
        let mut between = |a: u32, b: u32| match b - a {
            1 => self.gap(a, b).into_owned(),
            _ => {
                let inner = self.spell(a + 1, b - 1, cur.as_deref_mut());
                format!("{}{inner}{}", self.gap(a, a + 1), self.gap(b - 1, b))
            }
        };
        if let Lift::Call { f, sigma, var } = l {
            let one = if *sigma { ",1" } else { "" };
            return format!("rack_{f}({}{one}, {var})", between(open, end));
        }
        // Each argument's text, the blanks around it kept.
        let mut depth = 0i32;
        let mut cuts = vec![open];
        for k in open..end {
            match self.ttag(k) {
                crate::syntax::token::Tag::LParen => depth += 1,
                crate::syntax::token::Tag::RParen => depth -= 1,
                crate::syntax::token::Tag::Comma if depth == 1 => cuts.push(k),
                _ => {}
            }
        }
        cuts.push(end);
        let a: Vec<String> = cuts.windows(2).map(|w| between(w[0], w[1])).collect();
        match l {
            Lift::Call { .. } => unreachable!("written above"),
            Lift::Sigma(f) => format!("{f}({},{},1)", a[0], a[1]),
            Lift::Limit => format!(
                "((aunif(0, 1) > 0) ? ({}) + ({}) : ({}) - ({}))",
                a[0], a[1], a[0], a[1]
            ),
            Lift::Clamp => format!("min(max({}, {}), {})", a[0], a[1], a[2]),
        }
    }
}

impl<'s> Cursor<'s> {
    /// The next step, when it is token `k`'s.
    fn take(&mut self, k: u32) -> Option<&'s Step> {
        let steps = self.steps;
        let (t, step) = steps.get(self.at)?;
        (*t == k).then(|| {
            self.at += 1;
            step
        })
    }
}

/// Every block of `deck`, branches included.
fn all_blocks(deck: &Deck) -> Vec<&Block> {
    let sections = deck
        .sections
        .iter()
        .flat_map(|s| std::iter::once(&s.body).chain(&s.defs));
    let mut work: Vec<&Block> = std::iter::once(&deck.top)
        .chain(&deck.defs)
        .chain(sections)
        .collect();
    let mut out = Vec::new();
    while let Some(b) = work.pop() {
        out.push(b);
        work.extend(
            b.conds
                .iter()
                .flat_map(|(_, c)| c.branches.iter().map(|(_, body)| body)),
        );
    }
    out
}
