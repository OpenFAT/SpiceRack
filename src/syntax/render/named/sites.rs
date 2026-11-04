//! The Monte Carlo lift's sites (`lift`): every expression the old IR
//! held as text, block by block, in the old pass's order
//! (`codegen::statistics::exprs_mut`): the top, its definitions, each
//! section's body and definitions; in each its parameters, models,
//! instances, elements and branches. A site is the owner and field its
//! variables are named after, and the value items its text is spelled
//! from.
//!
//! Cites: monte-carlo-spectre.md §5.2, §10.2 step 1 (the walk).
//!
//! Contents:
//! - `pub(super) struct Site`, `pub(super) struct Scoped`
//! - `pub(super) fn sites(rd: &Rd, deck: &Deck) -> Vec<Scoped>`

use crate::syntax::ast::{NONE, Node, Tag};
use crate::syntax::render::read::{Block, Card, Comp, Deck, Rd};

/// One expression the old IR held as text: the owner and field its
/// variables are named after, and the value items its text is spelled
/// from.
pub(super) struct Site {
    pub(super) owner: String,
    pub(super) field: String,
    pub(super) roots: Vec<u32>,
}

/// What the walk found in one block: where it sits, and its sites.
pub(super) struct Scoped {
    pub(super) subckt: Option<String>,
    pub(super) section: Option<usize>,
    pub(super) sites: Vec<Site>,
}

/// Every block's sites, in the old pass's order.
pub(super) fn sites(rd: &Rd, deck: &Deck) -> Vec<Scoped> {
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

// ---- private ----

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
