//! Conditional blocks -> SPICE: `.if (c)` ... `.elseif (c)` ... `.else` ...
//! `.endif`, each branch's models then its elements, nesting.
//!
//! Cites: ngspice manual §2.15 (the form, and that `.subckt`, `.inc`,
//! `.lib` and `.param` may not sit in a branch); HSPICE reads the same form
//! (`.IF`/`.ELSEIF`/`.ELSE`/`.ENDIF`). Xyce and LTspice have none.
//!
//! Contents:
//! - `pub(super) fn conditionals(out: &mut String, d: Dialect, x: bool, blocks: &[Conditional],
//!   drop_four: bool) -> Result<(), CodeGenError>`

use super::{body, model};
use crate::codegen::{CodeGenError, Dialect};
use crate::ir::*;

/// Each block in `blocks`, written onto `out` one card a line.
///
/// # Errors
/// A dialect with no conditional (Xyce, LTspice), or a branch holding what
/// a SPICE branch may not (`.param`, a load).
pub(super) fn conditionals(
    out: &mut String,
    d: Dialect,
    x: bool,
    blocks: &[Conditional],
    drop_four: bool,
) -> Result<(), CodeGenError> {
    if blocks.is_empty() {
        return Ok(());
    }
    if matches!(d, Dialect::Xyce | Dialect::Ltspice) {
        return Err(CodeGenError::UnsupportedComponent {
            backend: d.name().into(),
            component: format!(
                "conditional block `if ({})`: {} has no .if",
                cond(&blocks[0].branches[0]),
                d.name()
            ),
        });
    }
    for c in blocks {
        for (i, b) in c.branches.iter().enumerate() {
            // ngspice §2.15 lists `.inc` and `.lib` as unsupported too, but
            // reads them (spice-bench examples/sp/filter.sp): written.
            let held = !b.body.parameters.is_empty()
                || !b.body.locals.is_empty()
                || !b.body.osdi_loads.is_empty();
            if held {
                return Err(CodeGenError::UnsupportedComponent {
                    backend: d.name().into(),
                    component: format!(
                        "`.param` or a load in the branch `if ({})`: not allowed in a SPICE .if (ngspice manual §2.15)",
                        cond(b)
                    ),
                });
            }
            match (&b.condition, i) {
                (Some(c), 0) => w!(out, ".if ({c})\n"),
                (Some(c), _) => w!(out, ".elseif ({c})\n"),
                (None, _) => out.push_str(".else\n"),
            }
            super::file_cards(out, d, &b.body)?;
            for m in &b.body.models {
                model(out, d, m);
                out.push('\n');
            }
            body(out, d, x, &b.body, drop_four)?;
        }
        out.push_str(".endif\n");
    }
    Ok(())
}

// ---- private ----

fn cond(b: &Branch) -> &str {
    b.condition.as_deref().unwrap_or("else")
}
