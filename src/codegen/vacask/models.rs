//! `.model` cards and devices -> the VACASK modules they need loaded and the
//! `model` lines naming them.

use std::collections::BTreeSet;

use super::values::{expr, q, unsupported_comp, value_of};
use crate::codegen::CodeGenError;
use crate::ir::*;

/// A device module the deck needs before it can be elaborated.
///
/// SPICE carries R/C/L values on the instance and never writes a `.model` for
/// them; VACASK has no anonymous instances, so every device needs a model. The
/// synthesised ones are named after the device, which is what upstream's own
/// decks do (`model resistor resistor` in `test/test_op.sim`); a user `.model`
/// wearing one of these names is a collision and is refused.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(super) enum Need {
    /// SPICE device set: `load "spice/<m>.osdi"` + `model <m> sp_<m>`.
    Sp(&'static str),
    /// Built into the simulator: no load, but still a `model` line.
    Builtin(&'static str),
}

impl Need {
    pub(super) fn auto_model(&self) -> &'static str {
        match self {
            Need::Sp(m) | Need::Builtin(m) => m,
        }
    }
}

/// The module a component needs loaded, if any.
pub(super) fn need(comp: &Component) -> Option<Need> {
    Some(match comp {
        Component::Resistor { .. } => Need::Sp("resistor"),
        Component::Capacitor { .. } => Need::Sp("capacitor"),
        Component::Inductor { .. } => Need::Sp("inductor"),
        Component::MutualInductor { .. } => Need::Builtin("mutual"),
        Component::VoltageSource { .. } => Need::Builtin("vsource"),
        Component::CurrentSource { .. } => Need::Builtin("isource"),
        Component::Vcvs { .. } => Need::Builtin("vcvs"),
        Component::Vccs { .. } => Need::Builtin("vccs"),
        Component::Cccs { .. } => Need::Builtin("cccs"),
        Component::Ccvs { .. } => Need::Builtin("ccvs"),
        _ => return None,
    })
}

pub(super) fn emit_model(
    m: &ModelDef,
    loads: &mut BTreeSet<String>,
    low: bool,
) -> Result<String, CodeGenError> {
    let kind = m.kind.to_ascii_lowercase();
    let name = q(&m.name);

    // LEVEL is the module SELECTOR, not a parameter, and defaults to 1.
    let mut level = 1u32;
    for (k, v) in &m.parameters {
        if k.eq_ignore_ascii_case("level") {
            level = value_of(v)
                .map(|f| f as u32)
                .ok_or_else(|| unsupported_comp(format!("model {}: LEVEL={}", name, v)))?;
        }
    }

    let (module, sign) = if kind == "nmos" || kind == "pmos" {
        let module = mos_module(level).ok_or_else(|| {
            unsupported_comp(format!(
                "model {}: {} LEVEL={} has no VACASK module",
                name, kind, level
            ))
        })?;
        (module, if kind.starts_with('n') { 1i8 } else { -1 })
    } else if let Some((module, sign)) = model_type(&kind) {
        // Every other type here is the LEVEL 1 device. `.model x NPN
        // (LEVEL=4)` is VBIC and `LEVEL=8` is HICUM: different equations
        // wearing the same keyword, and mapping on the keyword alone would
        // run them as Gummel-Poon.
        if level != 1 {
            return Err(unsupported_comp(format!(
                "model {}: {} LEVEL={} has no VACASK module",
                name, kind, level
            )));
        }
        (module, sign)
    } else if matches!(
        kind.as_str(),
        "r" | "c" | "l" | "res" | "cap" | "ind" | "sw" | "csw"
    ) {
        // See `model_type`: the passive modules rename their model-card
        // parameters, so a name-for-name transcription would silently set
        // the instance parameter instead. Switches have no VACASK module.
        return Err(unsupported_comp(format!(
            "model {}: '{}' model cards have no faithful VACASK spelling",
            name, kind
        )));
    } else {
        // A module the user loaded themselves (`osdi_loads`). Pass it
        // through: VACASK errors loudly with "Model not found" if it is not
        // there, which beats guessing at a substitute.
        return Ok(format!(
            "model {} {}{}",
            name,
            kind,
            model_params(&m.parameters, None, low)
        ));
    };

    loads.insert(format!("spice/{}.osdi", module));
    // `type=` is the polarity selector, not a `.model` parameter, and it
    // also keeps an otherwise-empty list non-empty: `model x sp_diode ( )`
    // is a VACASK syntax error.
    let lead = (sign != 0).then_some(sign);
    Ok(format!(
        "model {} sp_{}{}",
        name,
        module,
        model_params(&m.parameters, lead, low)
    ))
}

// ---- private ----

/// SPICE MOSFET LEVEL -> VACASK module. A level that is not here is refused:
/// substituting "the nearest model" compares two different sets of equations.
fn mos_module(level: u32) -> Option<&'static str> {
    Some(match level {
        1 => "mos1",
        2 => "mos2",
        3 => "mos3",
        6 => "mos6",
        9 => "mos9",
        49 | 8 => "bsim3v3",
        54 | 14 => "bsim4v8",
        _ => return None,
    })
}

/// `.model` TYPE -> (module, polarity), for the types whose parameter sets
/// transcribe name-for-name AT LEVEL 1.
///
/// `nmos`/`pmos` are absent on purpose: their LEVEL selects between seven
/// modules. R/C/L are absent too, and they are the one place the "same
/// parameter names" premise fails — `sp_resistor` renames the model card's
/// `r`/`tc1`/`tc2` to `model_r`/`model_tc1`/`model_tc2` to keep them apart from
/// the instance parameters of the same name, so a name-for-name transcription
/// would set the instance parameter and quietly change the device.
fn model_type(kind: &str) -> Option<(&'static str, i8)> {
    Some(match kind {
        "d" => ("diode", 0),
        "npn" => ("bjt", 1),
        "pnp" => ("bjt", -1),
        "njf" => ("jfet1", 1),
        "pjf" => ("jfet1", -1),
        "nmf" => ("mes1", 1),
        "pmf" => ("mes1", -1),
        "vdmos" => ("vdmos", 1),
        _ => return None,
    })
}

/// ` (type=1 k=v ...)`, or nothing for no `type=` and no parameters.
fn model_params(params: &[(String, String)], sign: Option<i8>, low: bool) -> String {
    let mut s = String::new();
    if let Some(t) = sign {
        w!(s, " type={t}");
    }
    for (k, v) in params
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("level"))
    {
        w!(s, " {}={}", k.to_ascii_lowercase(), expr(v, low));
    }
    if !s.is_empty() {
        s.replace_range(..1, " (");
        s.push(')');
    }
    s
}
