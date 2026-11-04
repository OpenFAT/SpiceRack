//! SPICE statistical calls -> VACASK: its `agauss`, `gauss`, `aunif` and
//! `unif` take ngspice's arguments and draw once per call site per scope,
//! Spectre's process/mismatch split (VACASK `docs/cmd-analysis-mc.md`
//! lines 31-61), so they are written as they are; only what VACASK lacks is
//! spelled out.
//!
//! Cites: VACASK `include/rpnbuiltin.h:590-594` (the four, no `limit`).
//!
//! Contents:
//! - `pub(crate) fn vacask_calls(ir: &CircuitIR) -> Result<Option<CircuitIR>, CodeGenError>`

use super::{STATISTICAL, exprs_mut, has_statistical, rewrite_calls};
use crate::codegen::CodeGenError;
use crate::ir::*;

/// `ir` with `limit(nom, avar)` as `((aunif(0, 1) > 0) ? (nom) + (avar) :
/// (nom) - (avar))` (VACASK has no `limit`), Xyce's clamp `limit(x, y, z)`
/// as `min(max(x, y), z)` and a two-argument `agauss`/`gauss` given sigma 1
/// (XY-RG); `None` when nothing changes.
///
/// # Errors
/// An HSPICE four-argument (`multiplier`) call: no VACASK form.
pub(crate) fn vacask_calls(ir: &CircuitIR) -> Result<Option<CircuitIR>, CodeGenError> {
    let mut any = false;
    let mut probe = ir.clone();
    exprs_mut(&mut probe, &mut |_, e| any |= has_statistical(e));
    if !any {
        return Ok(None);
    }
    let mut ir = ir.clone();
    let mut err = None;
    exprs_mut(&mut ir, &mut |_, e| {
        *e = rewrite_calls(e, &mut |name, args| {
            let f = name.to_ascii_lowercase();
            if !STATISTICAL.contains(&f.as_str()) {
                return None;
            }
            match (f.as_str(), args.len()) {
                ("limit", 2) => Some(format!(
                    "((aunif(0, 1) > 0) ? ({}) + ({}) : ({}) - ({}))",
                    args[0], args[1], args[0], args[1]
                )),
                ("limit", 3) => Some(format!("min(max({}, {}), {})", args[0], args[1], args[2])),
                ("agauss" | "gauss", 2) => Some(format!("{f}({},{},1)", args[0], args[1])),
                ("agauss" | "gauss", 3) | ("aunif" | "unif", 2) => None,
                _ => {
                    err.get_or_insert_with(|| {
                        format!("{name}() with {} arguments has no VACASK form", args.len())
                    });
                    None
                }
            }
        });
    });
    match err {
        Some(e) => Err(CodeGenError::Other(e)),
        None => Ok(Some(ir)),
    }
}
