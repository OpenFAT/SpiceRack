//! SPICE statistical calls -> Spectre: each call a deterministic `rack_*`
//! function of a normalized variable declared `parameters v=0` and varied
//! in a `statistics` block, `process` at a deck's or section's top and
//! `mismatch` inside a subcircuit; and the inverse, which reads that pattern
//! back as the SPICE call.
//!
//! Cites: Spectre Reference 19.1 pp.175-179 (process sampled once per run,
//! mismatch per subcircuit instance); ngspice xpressn.c:45-85
//! (the guards); Spectre Reference 19.1 p.481 (user functions), p.474 (no
//! short-circuiting, so no branch divides by zero).
//!
//! Contents:
//! - `pub(crate) fn lift(ir: &CircuitIR) -> Result<Option<CircuitIR>, CodeGenError>`
//! - `pub(crate) fn unlift(ir: &mut CircuitIR)`

use std::collections::{BTreeMap, HashSet};

use super::{STATISTICAL, Site, exprs_mut, has_statistical, rename_idents, rewrite_calls};
use crate::codegen::CodeGenError;
use crate::ir::*;

/// One lifted variable: its name, its scope, its distribution (normal, or
/// uniform on [-1, 1]) and the section it is declared in.
struct Var {
    name: String,
    mismatch: bool,
    unif: bool,
    section: Option<usize>,
}

/// The `rack_*` functions: `nom` for a zero variable (so `rack_limit` gives
/// `nom` there, not `nom - avar`), the guards
/// of ngspice's `xpressn.c:48-49`, `:59-62`, no branch dividing by zero.
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
    // `nom` at u = 0, so the nominal run is exactly `nom`; a draw is
    // never exactly 0, so the two values of ngspice's `limit`.
    (
        "rack_limit",
        ["nom", "avar", "u", ""],
        "(u > 0) ? nom + avar : ((u < 0) ? nom - avar : nom)",
    ),
];

/// `ir` with every statistical call lifted, its variables declared and
/// varied in its scope (`process` at the top, `mismatch` in a subcircuit),
/// the `rack_*` functions it uses defined and
/// a `.func` that calls one inlined; `None` when `ir` has no call.
///
/// # Errors
/// An HSPICE four-argument (`multiplier`) call, or a call of the wrong
/// arity: no Spectre form (HSPICE UG B-2008.09 App. A, `multiplier`).
pub(crate) fn lift(ir: &CircuitIR) -> Result<Option<CircuitIR>, CodeGenError> {
    let mut any = ir.functions.iter().any(|f| has_statistical(&f.body));
    let mut probe = ir.clone();
    exprs_mut(&mut probe, &mut |_, e| any |= has_statistical(e));
    if !any {
        return Ok(None);
    }
    let mut ir = ir.clone();
    inline_functions(&mut ir);
    let mut used: HashSet<String> = HashSet::new();
    let mut vars: Vec<Var> = Vec::new();
    let mut err = None;
    exprs_mut(&mut ir, &mut |site, e| {
        let mut k = 0;
        let lifted = rewrite_calls(e, &mut |name, args| {
            let f = name.to_ascii_lowercase();
            if !STATISTICAL.contains(&f.as_str()) {
                return None;
            }
            // Xyce's three-argument `limit` is a clamp (XY-RG Table 2-33).
            if f == "limit" && args.len() == 3 {
                return Some(format!("min(max({}, {}), {})", args[0], args[1], args[2]));
            }
            let gauss = f.contains("gauss");
            let mut args = args.to_vec();
            match (gauss, args.len()) {
                // Xyce's two-argument agauss/gauss take sigma 1 (XY-RG).
                (true, 2) => args.push("1".into()),
                (true, 3) | (false, 2) => {}
                _ => {
                    err.get_or_insert_with(|| {
                        format!("{name}() with {} arguments in '{}' has no Spectre form (HSPICE's multiplier is not supported)", args.len(), e)
                    });
                    return None;
                }
            }
            k += 1;
            let name = var_name(site, k, &mut used);
            vars.push(Var {
                name: name.clone(),
                mismatch: site.subckt.is_some(),
                unif: !gauss,
                section: site.section,
            });
            Some(format!("rack_{f}({}, {name})", args.join(",")))
        });
        *e = lifted;
    });
    if let Some(e) = err {
        return Err(CodeGenError::Other(e));
    }
    declare(&mut ir, &vars);
    let mut calls: HashSet<String> = HashSet::new();
    exprs_mut(&mut ir, &mut |_, e| {
        rewrite_calls(e, &mut |name, _| {
            calls.insert(name.to_string());
            None
        });
    });
    for (name, args, body) in RACK {
        if calls.contains(name) {
            ir.functions.push(FuncDef {
                name: name.into(),
                args: args
                    .iter()
                    .filter(|a| !a.is_empty())
                    .map(|a| a.to_string())
                    .collect(),
                body: body.into(),
            });
        }
    }
    let mismatch = vars.iter().any(|v| v.mismatch);
    for a in ir.testbench.iter_mut().flat_map(|tb| &mut tb.analyses) {
        if let Analysis::SpectreMonteCarlo {
            variations: v @ None,
            ..
        } = a
            && mismatch
        {
            *v = Some("all".into());
        }
    }
    Ok(Some(ir))
}

/// Reads SpiceRack's own lifted pattern back: `rack_f(args, v)` where
/// `v` is declared `parameters v=0` and varied with exactly `dist=gauss
/// std=1` (agauss, gauss) or `dist=unif N=1` (aunif, unif, limit) is the
/// call `f(args)`; `v`, its `vary` and the `rack_*` functions go. Anything
/// else (an edited variable) is left as written.
pub(crate) fn unlift(ir: &mut CircuitIR) {
    let mut kinds: BTreeMap<String, bool> = BTreeMap::new(); // var -> unif
    let bodies = std::iter::once(&ir.top).chain(ir.sections.iter().map(|s| &s.body));
    for b in bodies {
        let zero = |p: &str| {
            b.parameters
                .iter()
                .any(|d| d.name == p && d.default.as_deref() == Some("0"))
        };
        for st in &b.statistics {
            for v in st.process.iter().chain(&st.mismatch) {
                let shape = match v.dist {
                    Dist::Gauss if v.spread == "1" && !v.percent => Some(false),
                    Dist::Unif if v.spread == "1" && !v.percent => Some(true),
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
    let mut used: HashSet<String> = HashSet::new();
    exprs_mut(ir, &mut |_, e| {
        *e = rewrite_calls(e, &mut |name, args| {
            let f = name.strip_prefix("rack_")?;
            let unif = matches!(f, "aunif" | "unif" | "limit");
            let (last, rest) = args.split_last()?;
            let v = last.trim();
            (STATISTICAL.contains(&f) && kinds.get(v) == Some(&unif)).then(|| {
                used.insert(v.to_string());
                format!("{f}({})", rest.join(","))
            })
        });
    });
    let mut left: HashSet<String> = HashSet::new();
    exprs_mut(ir, &mut |_, e| {
        rewrite_calls(e, &mut |name, _| {
            left.insert(name.to_string());
            None
        });
    });
    ir.functions.retain(|f| {
        !(f.name.starts_with("rack_")
            && RACK.iter().any(|r| r.0 == f.name)
            && !left.contains(&f.name))
    });
    let bodies = std::iter::once(&mut ir.top).chain(ir.sections.iter_mut().map(|s| &mut s.body));
    for b in bodies {
        b.parameters.retain(|p| !used.contains(&p.name));
        for st in &mut b.statistics {
            st.process.retain(|v| !used.contains(&v.param));
            st.mismatch.retain(|v| !used.contains(&v.param));
        }
        b.statistics.retain(|st| *st != Statistics::default());
    }
}

// ---- private ----

/// `rack_<subckt>_<owner>_<field>_<k>`, identifier bytes only, a counter
/// suffix on a collision (the subcircuit names a mismatch
/// variable, since one local name recurs across a PDK's subcircuits).
fn var_name(site: &Site, k: usize, used: &mut HashSet<String>) -> String {
    let clean = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    let mut base = String::from("rack_");
    if let Some(s) = &site.subckt {
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

/// Each variable declared `=0` and varied in its deck's or section's top.
fn declare(ir: &mut CircuitIR, vars: &[Var]) {
    let mut at: BTreeMap<Option<usize>, Statistics> = BTreeMap::new();
    for v in vars {
        let body = match v.section {
            Some(k) => &mut ir.sections[k].body,
            None => &mut ir.top,
        };
        body.parameters.push(ParamDef {
            name: v.name.clone(),
            default: Some("0".into()),
        });
        let st = at.entry(v.section).or_default();
        let vary = Vary {
            param: v.name.clone(),
            dist: if v.unif { Dist::Unif } else { Dist::Gauss },
            spread: "1".into(),
            percent: false,
        };
        if v.mismatch {
            st.mismatch.push(vary)
        } else {
            st.process.push(vary)
        }
    }
    for (k, st) in at {
        match k {
            Some(k) => ir.sections[k].body.statistics.push(st),
            None => ir.top.statistics.push(st),
        }
    }
}

/// Every `.func` whose body calls a statistical function, inlined at its
/// call sites (drawn per call of the function) and dropped.
fn inline_functions(ir: &mut CircuitIR) {
    let stat: Vec<FuncDef> = ir
        .functions
        .iter()
        .filter(|f| has_statistical(&f.body))
        .cloned()
        .collect();
    if stat.is_empty() {
        return;
    }
    ir.functions.retain(|f| !has_statistical(&f.body));
    exprs_mut(ir, &mut |_, e| {
        *e = rewrite_calls(e, &mut |name, args| {
            let f = stat.iter().find(|f| f.name.eq_ignore_ascii_case(name))?;
            let map: Vec<(&str, String)> = f
                .args
                .iter()
                .map(String::as_str)
                .zip(args.iter().map(|v| format!("({})", v.trim())))
                .collect();
            Some(format!("({})", rename_idents(&f.body, &map)))
        });
    });
}
