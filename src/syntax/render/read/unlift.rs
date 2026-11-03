//! The Monte Carlo lift read back (`docs/research/monte-carlo-spectre.md`
//! §6a), a rule over the expression trees: in a Spectre deck, a call
//! `rack_f(args, v)` whose `v` is declared `parameters v=0` and varied with
//! exactly `dist=gauss std=1` (agauss, gauss) or `dist=unif N=1` (aunif,
//! unif, limit) is the call `f(args)`. The rule records fixups, never
//! touching the tree: each such `Call` node's replacement (its new name and
//! the token where its last argument starts), the variables it used (their
//! `parameters` and `vary` rows go), and the `rack_*` functions nothing
//! calls any more (they go). [`Rd::etext`] spells a value with them, and
//! with a writer's `Lift` fixups ([`Fix`]: a token respelled,
//! [`Rd::respell_token`]; a statistical call rewritten, [`Rd::lift_call`]),
//! one table keyed by token, recorded before the writer writes.
//!
//! Cites: monte-carlo-spectre.md §5.2 (the `rack_*` functions), §6a; the
//! old writer's `statistics::unlift`, which did this over the IR's text.
//!
//! Contents:
//! - `pub(super) struct Unlift`, `pub(in crate::syntax::render) enum Fix`, `enum Lift`
//! - `Rd::unlift(&mut self, deck: &mut Deck)`
//! - `Rd::etext(&self, n: u32) -> String`, `Rd::espan(&self, a: u32, b: u32) -> String`,
//!   `Rd::unlifted_var(&self, p: &str) -> bool`
//! - `Rd::respell_token(&mut self, k: u32, text: String)`, `Rd::lift_call(&mut self, name: u32, l: Lift)`,
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
}

/// A statistical call a writer rewrites (monte-carlo-spectre.md §5.2,
/// §7), by the `Call` node's name token.
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

/// The `rack_*` functions SpiceRack's lift defines (§5.2).
const RACK: [&str; 5] = [
    "rack_agauss",
    "rack_gauss",
    "rack_aunif",
    "rack_unif",
    "rack_limit",
];

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
        let mut out = String::new();
        let mut k = a;
        while k <= b {
            if k > a {
                out.push_str(&self.gap(k - 1, k));
            }
            let unlifted = self.unlifted.calls.get(&k).copied();
            match (unlifted, self.fixes.get(&k)) {
                (_, Some(Fix::Lift(l))) => {
                    let close = self.call_close(k);
                    // The arguments' text, the read-back's variable left out.
                    let end = unlifted.map_or(close, |(_, cut)| cut);
                    out.push_str(&self.lifted(k, end, l));
                    k = close + 1;
                }
                (Some((f, cut)), _) => {
                    let close = self.call_close(k);
                    out.push_str(f);
                    out.push_str(&self.espan(k + 1, cut - 1));
                    out.push_str(&self.gap(cut - 1, cut));
                    out.push(')');
                    k = close + 1;
                }
                (None, Some(Fix::Respell(t))) => {
                    out.push_str(t);
                    k += 1;
                }
                (None, None) => {
                    out.push_str(self.t.text(k));
                    k += 1;
                }
            }
        }
        out
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
    /// The lifted spelling of the call named at token `name`, its
    /// arguments the tokens between `name + 1` (its `(`) and `end` (its
    /// `)`, or the read-back's comma), each as the old writer's
    /// `rewrite_calls` joined them: the text between the delimiters kept.
    fn lifted(&self, name: u32, end: u32, l: &Lift) -> String {
        let open = name + 1;
        let between = |a: u32, b: u32| match b - a {
            1 => self.gap(a, b).into_owned(),
            _ => {
                let inner = self.espan(a + 1, b - 1);
                format!("{}{inner}{}", self.gap(a, a + 1), self.gap(b - 1, b))
            }
        };
        // Each argument's text, the blanks around it kept.
        let args = || {
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
            cuts.windows(2)
                .map(|w| between(w[0], w[1]))
                .collect::<Vec<String>>()
        };
        match l {
            Lift::Call { f, sigma, var } => {
                let one = if *sigma { ",1" } else { "" };
                format!("rack_{f}({}{one}, {var})", between(open, end))
            }
            Lift::Sigma(f) => {
                let a = args();
                format!("{f}({},{},1)", a[0], a[1])
            }
            Lift::Limit => {
                let a = args();
                format!(
                    "((aunif(0, 1) > 0) ? ({}) + ({}) : ({}) - ({}))",
                    a[0], a[1], a[0], a[1]
                )
            }
            Lift::Clamp => {
                let a = args();
                format!("min(max({}, {}), {})", a[0], a[1], a[2])
            }
        }
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
