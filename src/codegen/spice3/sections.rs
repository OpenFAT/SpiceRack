//! `.param` cards and library sections -> SPICE text: a block's parameters
//! and models as a deck's top writes them, and `.lib <name>` ... `.endl
//! <name>` around a section's body.
//!
//! Cites: ngspice manual §2.7 (`.lib`); HSPICE and Xyce read the same form
//! (Xyce RG `.LIB`); LTspice's `.lib <name>` is a file (its help, `.LIB`).
//!
//! Contents:
//! - `pub(super) fn params_and_models(out: &mut String, d: Dialect, sc: &Subcircuit)`
//! - `pub(super) fn param_cards(out: &mut String, d: Dialect, params: &[ParamDef])`
//! - `pub(super) fn section(out: &mut String, d: Dialect, x: bool, s: &LibSection,
//!   drop_four: bool) -> Result<(), CodeGenError>`

use super::values::spelled;
use super::{body, file_cards, model, subcircuit};
use crate::codegen::{CodeGenError, Dialect};
use crate::ir::*;

/// A block's `.param` cards and `.model` cards, as a deck's top and a
/// library section hold them.
pub(super) fn params_and_models(out: &mut String, d: Dialect, sc: &Subcircuit) {
    param_cards(out, d, &sc.parameters);
    for m in &sc.models {
        model(out, d, m);
        out.push('\n');
    }
}

/// `.param name=value`, one card each, in order (a parameter with no value
/// has none).
pub(super) fn param_cards(out: &mut String, d: Dialect, params: &[ParamDef]) {
    for p in params {
        if let Some(ref default) = p.default {
            w!(out, ".param {}={}\n", p.name, spelled(d)(default));
        }
    }
}

/// `.lib <name>` ... `.endl <name>`, the body written as a deck's top is.
/// LTspice reads `.lib <name>` as the file `<name>`, so it has no section
/// form. A Spectre library's name has no SPICE spelling and is not written.
pub(super) fn section(
    out: &mut String,
    d: Dialect,
    x: bool,
    s: &LibSection,
    drop_four: bool,
) -> Result<(), CodeGenError> {
    let name = &s.body.name;
    if d == Dialect::Ltspice {
        return Err(CodeGenError::UnsupportedComponent {
            backend: d.name().into(),
            component: format!(
                "library section '{name}': LTspice reads `.lib {name}` as a file, not a section"
            ),
        });
    }
    w!(out, ".lib {name}\n");
    file_cards(out, d, &s.body)?;
    params_and_models(out, d, &s.body);
    for sc in &s.subcircuit_defs {
        subcircuit(out, d, x, sc, drop_four)?;
    }
    body(out, d, x, &s.body, drop_four)?;
    w!(out, ".endl {name}\n");
    Ok(())
}
