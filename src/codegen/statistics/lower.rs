//! Spectre `statistics` tables -> SPICE and VACASK statistical calls: a
//! `process` vary is the parameter drawn once per run, a
//! `mismatch` vary a draw per instance of every subcircuit that reads it,
//! correlated process parameters a Cholesky combination of normalized
//! draws. SPICE (ngspice, HSPICE, Xyce) draws a one-layer `.param` at every
//! reference, so each draw is a two-layer `.param` (`p__mc = agauss(..)`,
//! `p = p__mc`); a VACASK `parameters` call is already one value, one layer.
//!
//! Cites: ngspice `inpcom.c` `inp_fix_agauss_in_param` (a one-layer random
//! `.param` becomes a function, redrawn per reference; two layers draw once,
//! measured on ngspice-44.2); Spectre Reference 19.1 pp.175-179.
//!
//! Contents:
//! - `pub(crate) fn lower(ir: &CircuitIR, vacask: bool) -> Result<Option<CircuitIR>, CodeGenError>`

use super::{rename_ident, sub_exprs_mut};
use crate::codegen::CodeGenError;
use crate::ir::*;

/// `ir` with every `statistics` table lowered to calls; `None` when it has
/// none.
///
/// # Errors
/// What SPICE cannot hold: `correlate dev=`, a correlation of a
/// non-gauss or non-process parameter, an n>2 correlation with an
/// expression `cc` or a matrix that is not positive definite, `truncate`
/// below 4 sigma or not a number, `lnorm` with `percent`, a `vary` of an
/// undeclared parameter.
pub(crate) fn lower(ir: &CircuitIR, vacask: bool) -> Result<Option<CircuitIR>, CodeGenError> {
    let any = std::iter::once(&ir.top)
        .chain(ir.sections.iter().map(|s| &s.body))
        .any(|b| !b.statistics.is_empty());
    if !any {
        return Ok(None);
    }
    let mut ir = ir.clone();
    let mut mismatch: Vec<(String, String)> = Vec::new(); // (param, draw around it)
    for k in std::iter::once(None).chain((0..ir.sections.len()).map(Some)) {
        let body = match k {
            Some(k) => &mut ir.sections[k].body,
            None => &mut ir.top,
        };
        for st in std::mem::take(&mut body.statistics) {
            truncation(&st)?;
            let correlated = correlate(body, &st, vacask)?;
            for v in st.process.iter().filter(|v| !correlated.contains(&v.param)) {
                let i = declared(body, &v.param)?;
                let nom = body.parameters[i]
                    .default
                    .clone()
                    .unwrap_or_else(|| "0".into());
                let draw = draw(v, &nom)?;
                draws(body, i, &v.param, "mc", draw, vacask);
            }
            for v in &st.mismatch {
                mismatch.push((v.param.clone(), draw(v, &v.param)?));
            }
        }
    }
    for (p, draw) in &mismatch {
        let to = format!("{p}__mm");
        let subckts = ir
            .subcircuit_defs
            .iter_mut()
            .chain(ir.sections.iter_mut().flat_map(|s| &mut s.subcircuit_defs));
        for sc in subckts {
            let mut reads = false;
            sub_exprs_mut(sc, &mut |e| reads |= rename_ident(e, p, "\u{1}") != *e);
            if !reads {
                continue;
            }
            sub_exprs_mut(sc, &mut |e| *e = rename_ident(e, p, &to));
            let mut locals = if vacask {
                vec![ParamDef {
                    name: to.clone(),
                    default: Some(braced(draw)),
                }]
            } else {
                let r = format!("{to}_r");
                vec![
                    ParamDef {
                        name: r.clone(),
                        default: Some(braced(draw)),
                    },
                    ParamDef {
                        name: to.clone(),
                        default: Some(r),
                    },
                ]
            };
            locals.append(&mut sc.locals);
            sc.locals = locals;
        }
    }
    Ok(Some(ir))
}

// ---- private ----

/// SPICE does not truncate (Spectre does at 4 sigma by default, Spectre
/// Reference 19.1 p.179): 4 sigma or more costs under 0.06% of a
/// std and is dropped; less changes the distribution and is refused.
fn truncation(st: &Statistics) -> Result<(), CodeGenError> {
    for t in st.truncate.iter().flatten() {
        match crate::unit::parse_spice_number(t.trim()) {
            Some((x, end, s)) if end + s.len() == t.trim().len() && x >= 4.0 => {}
            _ => {
                return Err(CodeGenError::Other(format!(
                    "statistics: truncate tr={t} has no SPICE form (SPICE does not truncate; only tr>=4 is within 0.06% of the std)"
                )));
            }
        }
    }
    Ok(())
}

/// The index of `p`'s declaration in `body`.
fn declared(body: &Subcircuit, p: &str) -> Result<usize, CodeGenError> {
    body.parameters
        .iter()
        .position(|d| d.name == p)
        .ok_or_else(|| {
            CodeGenError::Other(format!(
                "statistics: vary {p}, which no `parameters` here declares"
            ))
        })
}

/// One draw of `v` around the mean `nom`.
fn draw(v: &Vary, nom: &str) -> Result<String, CodeGenError> {
    let s = if v.percent {
        format!("({nom})*({})/100", v.spread)
    } else {
        v.spread.clone()
    };
    Ok(match v.dist {
        Dist::Gauss => format!("agauss({nom}, {s}, 1)"),
        Dist::Unif => format!("aunif({nom}, {s})"),
        Dist::Lnorm if v.percent => {
            return Err(CodeGenError::Other(format!(
                "statistics: vary {} dist=lnorm percent=yes is ambiguous (R19 recommends against it); refusing",
                v.param
            )));
        }
        Dist::Lnorm => format!("exp(agauss(log({nom}), {s}, 1))"),
    })
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

/// The parameter at `i` drawn once per run: SPICE `p__<tag> = draw` then
/// `p = p__<tag>`; VACASK `p = draw`.
fn draws(body: &mut Subcircuit, i: usize, p: &str, tag: &str, draw: String, vacask: bool) {
    if vacask {
        body.parameters[i].default = Some(braced(&draw));
    } else {
        let r = format!("{p}__{tag}");
        body.parameters[i].default = Some(r.clone());
        body.parameters.insert(
            i,
            ParamDef {
                name: r,
                default: Some(braced(&draw)),
            },
        );
    }
}

/// Each correlated set of gauss process parameters: normalized draws
/// `p__z`, then `p = v + s*sum(L*z)` with `L` the Cholesky factor of the
/// correlation matrix (unlisted pairs 0). Returns the parameters handled.
fn correlate(
    body: &mut Subcircuit,
    st: &Statistics,
    vacask: bool,
) -> Result<Vec<String>, CodeGenError> {
    let mut names: Vec<String> = Vec::new();
    for c in &st.correlations {
        if !c.devices.is_empty() {
            return Err(CodeGenError::Other(format!(
                "statistics: correlate dev=[{}] (matched instances) has no SPICE form",
                c.devices.join(" ")
            )));
        }
        for p in &c.params {
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
    for c in &st.correlations {
        for a in &c.params {
            for b in &c.params {
                let (i, j) = (
                    names.iter().position(|x| x == a).unwrap_or(0),
                    names.iter().position(|x| x == b).unwrap_or(0),
                );
                if i != j {
                    cc[i][j] = c.cc.clone();
                }
            }
        }
    }
    let vary: Vec<&Vary> = names
        .iter()
        .map(|p| {
            st.process.iter().find(|v| &v.param == p && v.dist == Dist::Gauss).ok_or_else(|| {
                CodeGenError::Other(format!("statistics: correlate of {p}, not a gauss process parameter, has no SPICE form"))
            })
        })
        .collect::<Result<_, _>>()?;
    let num = |s: &str| {
        let t = s.trim();
        let (sign, m) = t.strip_prefix('-').map_or((1.0, t), |m| (-1.0, m));
        match crate::unit::parse_spice_number(m) {
            _ if t.is_empty() => Some(0.0),
            Some((x, end, sc)) if end + sc.len() == m.len() => Some(sign * x),
            _ => None,
        }
    };
    let l: Vec<Vec<String>> = match cc
        .iter()
        .flatten()
        .map(|s| num(s))
        .collect::<Option<Vec<f64>>>()
    {
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
        None => {
            return Err(CodeGenError::Other(format!(
                "statistics: correlate over {n} parameters with an expression cc has no SPICE form"
            )));
        }
    };
    for p in &names {
        let i = declared(body, p)?;
        let z = format!("{p}__z");
        let mut decl = vec![ParamDef {
            name: z.clone(),
            default: Some("{agauss(0, 1, 1)}".into()),
        }];
        if !vacask {
            decl[0].default = Some(format!("{z}__r"));
            decl.insert(
                0,
                ParamDef {
                    name: format!("{z}__r"),
                    default: Some("{agauss(0, 1, 1)}".into()),
                },
            );
        }
        body.parameters.splice(i..i, decl);
    }
    for (a, p) in names.iter().enumerate() {
        let i = declared(body, p)?;
        let nom = body.parameters[i]
            .default
            .clone()
            .unwrap_or_else(|| "0".into());
        let s = if vary[a].percent {
            format!("({nom})*({})/100", vary[a].spread)
        } else {
            vary[a].spread.clone()
        };
        let sum: Vec<String> = (0..=a)
            .map(|b| format!("({})*{}__z", l[a][b], names[b]))
            .collect();
        body.parameters[i].default = Some(format!("{{({nom}) + ({s})*({})}}", sum.join(" + ")));
    }
    Ok(names)
}

/// The lower Cholesky factor of the `n`x`n` row-major `c`.
///
/// # Errors
/// `c` is not positive definite.
fn cholesky(c: &[f64], n: usize) -> Result<Vec<Vec<f64>>, CodeGenError> {
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let s: f64 = (0..j).map(|k| l[i][k] * l[j][k]).sum();
            if i == j {
                let d = c[i * n + i] - s;
                if d <= 0.0 {
                    return Err(CodeGenError::Other(
                        "statistics: the correlation matrix is not positive definite".into(),
                    ));
                }
                l[i][j] = d.sqrt();
            } else {
                l[i][j] = (c[i * n + j] - s) / l[j][j];
            }
        }
    }
    Ok(l)
}
