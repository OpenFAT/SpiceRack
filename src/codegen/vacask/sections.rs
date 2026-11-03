//! A library section -> VACASK `section <name>` ... `endsection` (no name
//! after it): the section's `include`s, its body and its `subckt`s.
//!
//! Cites: VACASK lib/dfllexer.l L536-L557 (`section`, `endsection`).
//!
//! Contents:
//! - `pub(super) fn emit_section(out: &mut String, s: &LibSection, models: &[Vec<String>],
//!   low: bool) -> Result<(), CodeGenError>`

use super::{emit_body, emit_subcircuit, q};
use crate::codegen::CodeGenError;
use crate::ir::*;

/// `section <name>` ... `endsection`, with no line end after it; `models`
/// are the body's model lines, then each definition's.
pub(super) fn emit_section(
    out: &mut String,
    s: &LibSection,
    models: &[Vec<String>],
    low: bool,
) -> Result<(), CodeGenError> {
    w!(out, "section {}\n", q(&s.body.name));
    for inc in &s.body.includes {
        w!(out, "  include \"{inc}\"\n");
    }
    for (path, section) in &s.body.libs {
        w!(out, "  include \"{path}\" section={section}\n");
    }
    emit_body(out, &s.body, &models[0], low)?;
    for (sc, m) in s.subcircuit_defs.iter().zip(&models[1..]) {
        emit_subcircuit(out, sc, m, low)?;
        out.push('\n');
    }
    out.push_str("endsection");
    Ok(())
}
