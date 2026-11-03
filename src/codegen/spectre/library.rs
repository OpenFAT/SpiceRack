//! A PDK model file's structure -> Spectre: SPICE bins as one model group,
//! library sections in their `library`, `statistics` blocks.
//!
//! Cites: Spectre User Guide 5.1.41 p.108 (model groups); `[REF]` p.498
//! (`library`, `section`, `endsection`, `endlibrary`), pp.175-179
//! (`statistics`).
//!
//! Contents:
//! - `pub(super) fn emit_models(out: &mut String, models: &[ModelDef], n: Names)
//!   -> Result<(), CodeGenError>`
//! - `pub(super) fn emit_sections(out: &mut String, ir: &CircuitIR, n: Names)
//!   -> Result<(), CodeGenError>`
//! - `pub(super) fn emit_statistics(out: &mut String, blocks: &[Statistics])`

use super::{
    Names, emit_model, emit_model_params, emit_subcircuit, emit_subcircuit_body, spectre_master,
};
use crate::codegen::CodeGenError;
use crate::ir::*;

/// Models, one statement each, except that a run of SPICE bins (`nch.1`,
/// `nch.2`: one base, one master) is one model group, Spectre's binning:
/// `model nch bsim4 {` / `1: type=n lmin=...` / `}`.
pub(super) fn emit_models(
    out: &mut String,
    models: &[ModelDef],
    n: Names,
) -> Result<(), CodeGenError> {
    let mut i = 0;
    while i < models.len() {
        let first = &models[i];
        let master = spectre_master(first, n)?;
        let run = match bin(first) {
            Some((base, _)) => models[i..]
                .iter()
                .take_while(|m| {
                    bin(m).is_some_and(|(b, _)| b == base)
                        && spectre_master(m, n).ok() == Some(master)
                })
                .count(),
            None => 0,
        };
        let Some((base, _)) = bin(first).filter(|_| run > 0) else {
            emit_model(out, first, n)?;
            out.push('\n');
            i += 1;
            continue;
        };
        w!(
            out,
            "model {} {} {{\n",
            n.n(base),
            master.0.unwrap_or(&first.kind)
        );
        for m in &models[i..i + run] {
            out.push_str(bin(m).map_or("", |(_, k)| k));
            out.push(':');
            emit_model_params(out, m, master.0, master.1);
            out.push('\n');
        }
        out.push_str("}\n");
        i += run;
    }
    Ok(())
}

/// `library <lib>` / `section <name>` ... `endsection <name>` /
/// `endlibrary <lib>`, one `library` block per run of sections of one
/// library; a section of none (SPICE's, or a bare Spectre one) is written
/// bare. A section's body is written as a `subckt`'s is, after its
/// `include`s.
pub(super) fn emit_sections(
    out: &mut String,
    ir: &CircuitIR,
    n: Names,
) -> Result<(), CodeGenError> {
    for (i, s) in ir.sections.iter().enumerate() {
        let lib = s.library.as_ref();
        if let Some(name) = lib.filter(|_| i == 0 || ir.sections[i - 1].library.as_ref() != lib) {
            w!(out, "\nlibrary {}\n", n.n(name));
        }
        w!(out, "section {}\n", n.n(&s.body.name));
        super::emit_functions(out, &s.functions)?;
        for inc in &s.body.includes {
            w!(out, "include \"{inc}\"\n");
        }
        for (path, section) in &s.body.libs {
            w!(out, "include \"{path}\" section={section}\n");
        }
        emit_subcircuit_body(out, &s.body, n)?;
        for sc in &s.subcircuit_defs {
            emit_subcircuit(out, sc, n)?;
            out.push('\n');
        }
        w!(out, "endsection {}\n", n.n(&s.body.name));
        let last = ir
            .sections
            .get(i + 1)
            .is_none_or(|next| next.library.as_ref() != lib);
        if let Some(name) = lib.filter(|_| last) {
            w!(out, "endlibrary {}\n", n.n(name));
        }
    }
    Ok(())
}

/// `[REF]` pp.175-179: `statistics { process { vary p dist=gauss std=s }
/// mismatch { ... } correlate param=[a b] cc=c truncate tr=t }`, one block
/// per table, one statement a line; values in Spectre's spelling.
pub(super) fn emit_statistics(out: &mut String, blocks: &[Statistics]) {
    let v = |s: &str| super::spectre_value(s).into_owned();
    for st in blocks {
        out.push_str("statistics {\n");
        let truncate = |out: &mut String, t: &Option<String>, pad: &str| {
            if let Some(t) = t {
                w!(out, "{pad}truncate tr={}\n", v(t));
            }
        };
        for (name, varies, t) in [
            ("process", &st.process, &st.truncate[1]),
            ("mismatch", &st.mismatch, &st.truncate[2]),
        ] {
            if varies.is_empty() && t.is_none() {
                continue;
            }
            w!(out, "    {name} {{\n");
            for x in varies {
                let (dist, key) = match x.dist {
                    Dist::Gauss => ("gauss", "std"),
                    Dist::Lnorm => ("lnorm", "std"),
                    Dist::Unif => ("unif", "N"),
                };
                let pct = if x.percent { " percent=yes" } else { "" };
                w!(
                    out,
                    "        vary {} dist={dist} {key}={}{pct}\n",
                    x.param,
                    v(&x.spread)
                );
            }
            truncate(out, t, "        ");
            out.push_str("    }\n");
        }
        for c in &st.correlations {
            out.push_str("    correlate");
            if !c.devices.is_empty() {
                w!(out, " dev=[{}]", c.devices.join(" "));
            }
            if !c.params.is_empty() {
                w!(out, " param=[{}]", c.params.join(" "));
            }
            w!(out, " cc={}\n", v(&c.cc));
        }
        truncate(out, &st.truncate[0], "    ");
        out.push_str("}\n");
    }
}

// ---- private ----

/// A SPICE bin's base name and number: `nch.1` is `("nch", "1")`.
fn bin(m: &ModelDef) -> Option<(&str, &str)> {
    m.name
        .rsplit_once('.')
        .filter(|(b, k)| !b.is_empty() && !k.is_empty() && k.bytes().all(|c| c.is_ascii_digit()))
}
