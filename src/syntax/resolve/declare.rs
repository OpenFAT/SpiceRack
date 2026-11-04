//! One file's tree → its declarations, the files it names, its role and its
//! duplicate names: the pre-scan of resolve (Zig 0.17 AstGen's
//! `scanContainer`), which runs over every file
//! before any name is looked up, so a master may come before its
//! declaration or sit in another file.
//!
//! Cites: ngspice manual §2.4 (`.model` types), §2.5 (`.subckt name nodes
//! [params:] k=v`), §2.6 (`.include`), §2.7 (`.lib file section`, `.lib
//! name` … `.endl`), §2.8.2 (`.func`), §2.9 (`.param`); Spectre UG p.94
//! (`subckt`, `parameters`), p.98 (`include … section=`), p.108 (model
//! groups), p.118 (`ahdl_include`); VACASK `share/doc/vacask` (`load`,
//! `model NAME module type=±1`).
//!
//! Contents:
//! - `pub(super) struct Declared`, `pub(super) struct Ref`
//! - `pub(super) fn declare(file: FileId, t: &Tokens, ast: &Ast, fold: bool) -> Declared`
//! - `pub(super) fn word_text<'a>(ast: &'a Ast, n: u32) -> Option<&'a str>`

use std::collections::{HashMap, HashSet};

use super::{Def, DefKind, FileId, Polarity, Role, Scope, key, walk};
use crate::syntax::Dialect;
use crate::syntax::ast::{Ast, NONE, Name, Node, Tag};
use crate::syntax::device::{self, Primitive};
use crate::syntax::diag::{Code, Diag};
use crate::syntax::token::{Tag as T, Tokens};

/// What one file declares and names.
pub(super) struct Declared {
    pub(super) defs: Vec<Def>,
    pub(super) refs: Vec<Ref>,
    pub(super) role: Role,
    /// `R0002` rows.
    pub(super) diags: Vec<Diag>,
}

/// A file a statement names.
pub(super) struct Ref {
    pub(super) node: u32,
    pub(super) token: u32,
    /// The path as written, quotes stripped.
    pub(super) path: String,
    /// The section a `.lib file section` or `include … section=` selects.
    pub(super) section: Option<String>,
    pub(super) verilog_a: bool,
}

/// SPICE `.model` type words → the primitive and polarity (ngspice manual
/// §2.4's table; `res` is HSPICE's and Xyce's resistor model).
const SPICE_MODELS: &[(&str, Primitive, Option<Polarity>)] = &[
    ("r", Primitive::Resistor, None),
    ("res", Primitive::Resistor, None),
    ("c", Primitive::Capacitor, None),
    ("l", Primitive::Inductor, None),
    ("sw", Primitive::VoltageSwitch, None),
    ("csw", Primitive::CurrentSwitch, None),
    ("urc", Primitive::RcLine, None),
    ("ltra", Primitive::LossyLine, None),
    ("txl", Primitive::Txl, None),
    ("cpl", Primitive::CoupledLines, None),
    ("d", Primitive::Diode, None),
    ("npn", Primitive::Bjt, Some(Polarity::N)),
    ("pnp", Primitive::Bjt, Some(Polarity::P)),
    ("njf", Primitive::Jfet, Some(Polarity::N)),
    ("pjf", Primitive::Jfet, Some(Polarity::P)),
    ("nmos", Primitive::Mosfet, Some(Polarity::N)),
    ("pmos", Primitive::Mosfet, Some(Polarity::P)),
    ("nmf", Primitive::Mesfet, Some(Polarity::N)),
    ("pmf", Primitive::Mesfet, Some(Polarity::P)),
    ("vdmos", Primitive::Mosfet, None),
];

/// The namespaces a duplicate is counted in: one per declaration kind, and
/// instances (parameters may be redefined, later wins).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Space {
    Subckt,
    Model,
    Func,
    Section,
    Instance,
}

/// Pre-scans `ast` (file `file`, tokens `t`; names compared case-blind
/// where `fold`).
pub(super) fn declare(file: FileId, t: &Tokens, ast: &Ast, fold: bool) -> Declared {
    let mut out = Declared {
        defs: Vec::new(),
        refs: Vec::new(),
        role: Role::Library,
        diags: Vec::new(),
    };
    let mut names: HashSet<(u32, Space, String)> = HashSet::new();
    // A Spectre subckt's def, by node: its `parameters` are its params.
    let mut subckt_def: HashMap<u32, usize> = HashMap::new();
    walk(ast, |n, scope, chain| {
        let node = Node(n);
        let within = chain.last().copied().unwrap_or(NONE);
        let lex = t.lexicon_at(ast.main_token(node));
        // A `Verbatim` or `Line` keeps a token in `lhs`, not fields.
        let fields = match ast.tag(node) {
            Tag::Verbatim | Tag::Line => &[][..],
            _ => ast.fields(node),
        };
        let def = |name: &str, kind: DefKind, out: &mut Declared| {
            out.defs.push(Def {
                name: name.to_string(),
                kind,
                file,
                node,
                scope,
                within,
            });
        };
        let space = match ast.tag(node) {
            Tag::Subckt => {
                let Some(name) = fields.first().and_then(|&w| word_text(ast, w)) else {
                    return;
                };
                let (ports, params) = subckt_head(ast, &fields[1..], lex.named);
                subckt_def.insert(n, out.defs.len());
                def(name, DefKind::Subckt { ports, params }, &mut out);
                Some((Space::Subckt, name))
            }
            Tag::Model => {
                let (Some(name), Some(word)) = (
                    fields.first().and_then(|&w| word_text(ast, w)),
                    fields.get(1).and_then(|&w| word_text(ast, w)),
                ) else {
                    return;
                };
                let kind = model(t, ast, node, word, lex.named);
                def(name, kind, &mut out);
                Some((Space::Model, name))
            }
            Tag::Func => {
                let Some(name) = fields.first().and_then(|&f| func_name(ast, f)) else {
                    return;
                };
                def(name, DefKind::Func, &mut out);
                Some((Space::Func, name))
            }
            Tag::Section => {
                let Some(name) = fields.first().and_then(|&w| word_text(ast, w)) else {
                    return;
                };
                def(name, DefKind::Section, &mut out);
                Some((Space::Section, name))
            }
            Tag::Param => {
                for (k, _) in assigns(ast, fields) {
                    if let Some(p) = word_text(ast, k) {
                        def(p, DefKind::Param, &mut out);
                        if let (true, Scope::Subckt(s)) = (lex.named, scope)
                            && let Some(&d) = subckt_def.get(&s.0)
                            && let DefKind::Subckt { params, .. } = &mut out.defs[d].kind
                        {
                            params.push(p.to_string());
                        }
                    }
                }
                None
            }
            Tag::Include | Tag::LibCall => {
                if let Some(r) = reference(t, ast, node) {
                    out.refs.push(r);
                }
                None
            }
            Tag::Card => {
                let head = t.tags()[ast.main_token(node) as usize];
                if matches!(head, T::KwAhdlInclude | T::KwLoad)
                    && let Some(path) = fields.first().and_then(|&p| path_text(t, ast, p))
                    && crate::veriloga::is_source(&path)
                {
                    out.refs.push(Ref {
                        node: n,
                        token: ast.main_token(node),
                        path,
                        section: None,
                        verilog_a: true,
                    });
                }
                None
            }
            Tag::Analysis | Tag::Control => {
                if within == NONE {
                    out.role = Role::Deck;
                }
                None
            }
            Tag::Instance => {
                let h = ast.header(node);
                let source = matches!(
                    Primitive::from_u32(h.code),
                    Some(Primitive::VoltageSource | Primitive::CurrentSource)
                );
                if source && within == NONE {
                    out.role = Role::Deck;
                }
                word_text(ast, h.name).map(|name| (Space::Instance, name))
            }
            // else: no other statement declares or names a file.
            _ => None,
        };
        if let Some((space, name)) = space {
            let container = match scope {
                Scope::Top => 0,
                Scope::Subckt(s) | Scope::Section(s) | Scope::Branch(s) => s.0,
            };
            if !names.insert((container, space, key(name, fold))) {
                out.diags.push(Diag {
                    code: Code::R0002,
                    token: ast.main_token(node),
                });
            }
        }
    });
    out
}

/// A `Word` item's text as written; `None` for any other item.
pub(super) fn word_text<'a>(ast: &'a Ast, n: u32) -> Option<&'a str> {
    (n != NONE && ast.tag(Node(n)) == Tag::Word).then(|| ast.name(Name(ast.rhs(Node(n)))))
}

// ---- private ----

/// A name item's text: a `Word`, or an expression that is one name (a
/// Spectre value `type=n`, `section=tt`).
fn name_text<'a>(ast: &'a Ast, n: u32) -> Option<&'a str> {
    match ast.tag(Node(n)) {
        Tag::Ident => Some(ast.name(Name(ast.lhs(Node(n))))),
        // else: a word, or no name.
        _ => word_text(ast, n),
    }
}

/// A subckt head's items after its name → its ports and parameter names:
/// SPICE's ports are the words before `params:` or the first `k=v`, its
/// parameters the `k=v`s; a named dialect's items are all ports (its
/// parameters are body `parameters`, added by the caller).
fn subckt_head(ast: &Ast, items: &[u32], named: bool) -> (Vec<String>, Vec<String>) {
    let mut ports = Vec::new();
    let mut params = Vec::new();
    for &k in items {
        match (ast.tag(Node(k)), word_text(ast, k)) {
            (Tag::Word, Some(w)) if !named && w.eq_ignore_ascii_case("params:") => {}
            (Tag::Word, Some(w)) if params.is_empty() => ports.push(w.to_string()),
            (Tag::Assign, _) => params.extend(word_text(ast, ast.lhs(Node(k))).map(String::from)),
            // else: a word after the parameters, or an item no head holds.
            _ => {}
        }
    }
    (ports, params)
}

/// A model statement's kind, level and polarity, read with its own lexicon
/// (`named`: Spectre or VACASK).
fn model(t: &Tokens, ast: &Ast, node: Node, word: &str, named: bool) -> DefKind {
    let lex = t.lexicon_at(ast.main_token(node));
    let mut pairs = assigns(ast, ast.fields(node));
    // A model group's polarity is its first bin's.
    if let Some(&bin) = ast.body(node).first() {
        pairs.extend(assigns(ast, ast.fields(Node(bin))));
    }
    let get = |k: &str| {
        pairs
            .iter()
            .find(|(key, _)| word_text(ast, *key).is_some_and(|w| w.eq_ignore_ascii_case(k)))
            .map(|&(_, v)| v)
    };
    let level = get("level")
        .and_then(|v| number(t, ast, v))
        .filter(|l| *l >= 0.0 && l.fract() == 0.0 && *l <= f64::from(u32::MAX))
        .map(|l| l as u32);
    let (kind, polarity) = if named {
        let ty = get("type");
        let polarity = match ty.and_then(|v| number(t, ast, v)) {
            Some(x) if x < 0.0 => Some(Polarity::P),
            Some(_) => Some(Polarity::N),
            None => match ty
                .and_then(|v| name_text(ast, v))
                .map(str::to_ascii_lowercase)
            {
                Some(w) if w == "p" || w == "pnp" => Some(Polarity::P),
                Some(w) if w == "n" || w == "npn" => Some(Polarity::N),
                _ => None,
            },
        };
        // VACASK's instances name no primitive, so its lexicon has no device
        // table; its `model` statements name `device::VACASK`'s modules.
        let row = match lex.dialect {
            Dialect::Vacask => device::find(&[device::VACASK], word, lex.fold_case),
            _ => lex.device(word),
        };
        (row.map(|d| d.primitive), polarity)
    } else {
        match SPICE_MODELS
            .iter()
            .find(|(w, _, _)| w.eq_ignore_ascii_case(word))
        {
            Some(&(_, p, pol)) => (Some(p), pol),
            None => (None, None),
        }
    };
    DefKind::Model {
        word: word.to_string(),
        kind,
        level,
        polarity,
    }
}

/// The `key=value` items of `fields`: `(key, value)` node pairs.
fn assigns(ast: &Ast, fields: &[u32]) -> Vec<(u32, u32)> {
    fields
        .iter()
        .filter(|&&k| ast.tag(Node(k)) == Tag::Assign)
        .map(|&k| (ast.lhs(Node(k)), ast.rhs(Node(k))))
        .collect()
}

/// A value item's number: a one-token `Word` or `Num` that is a number, or
/// `-` one.
fn number(t: &Tokens, ast: &Ast, v: u32) -> Option<f64> {
    if v == NONE {
        return None;
    }
    let n = Node(v);
    let k = ast.main_token(n);
    let one = match ast.tag(n) {
        Tag::Word => ast.lhs(n) == k,
        Tag::Num => true,
        Tag::Unary if t.tags()[k as usize] == T::Minus => {
            return number(t, ast, ast.lhs(n)).map(|x| -x);
        }
        // else: an expression or a group is no number here.
        _ => false,
    };
    (one && t.tags()[k as usize] == T::Number)
        .then(|| t.value(k))
        .filter(|x| !x.is_nan())
}

/// A function's name: `f` of `f(args)`, `f(args) = body` or a bare `f`.
fn func_name<'a>(ast: &'a Ast, f: u32) -> Option<&'a str> {
    match ast.tag(Node(f)) {
        Tag::Word => word_text(ast, f),
        Tag::Group => word_text(ast, ast.lhs(Node(f))),
        Tag::Assign => func_name(ast, ast.lhs(Node(f))),
        // else: no other item names a function.
        _ => None,
    }
}

/// The file an `Include` or `LibCall` names, and the section it selects.
fn reference(t: &Tokens, ast: &Ast, node: Node) -> Option<Ref> {
    let fields = ast.fields(node);
    let path = path_text(t, ast, *fields.first()?)?;
    let section = match ast.tag(node) {
        Tag::LibCall => fields
            .get(1)
            .and_then(|&s| word_text(ast, s))
            .map(String::from),
        // Spectre `include "f" section=s`.
        _ => assigns(ast, &fields[1..])
            .into_iter()
            .find(|&(k, _)| word_text(ast, k) == Some("section"))
            .and_then(|(_, v)| name_text(ast, v))
            .map(String::from),
    };
    Some(Ref {
        node: node.0,
        token: ast.main_token(node),
        path,
        section,
        verilog_a: false,
    })
}

/// A path item's text without its quotes: a `Word` (a SPICE chunk) or a
/// `Str`.
fn path_text(t: &Tokens, ast: &Ast, p: u32) -> Option<String> {
    let text = match ast.tag(Node(p)) {
        Tag::Word => word_text(ast, p)?,
        Tag::Str => t.text(ast.main_token(Node(p))),
        // else: an expression is no path.
        _ => return None,
    };
    let text = text.trim();
    let bare = match text.as_bytes() {
        [b'"', .., b'"'] | [b'\'', .., b'\''] if text.len() >= 2 => &text[1..text.len() - 1],
        _ => text,
    };
    Some(bare.to_string())
}
