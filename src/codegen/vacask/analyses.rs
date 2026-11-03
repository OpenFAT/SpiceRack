//! Analyses, `.step` sweeps and initial conditions -> VACASK `analysis` and
//! `sweep` statements.
//!
//! Contents:
//! - `pub(super) fn analysis_block(idx: usize, a: &Analysis, native: bool) -> Result<Vec<String>, CodeGenError>`
//! - `pub(super) fn step_sweep(sp: &StepParam) -> Result<String, CodeGenError>`
//! - `pub(super) fn node_list(key: &str, items: &[(String, f64)]) -> String`

use super::values::{num, sanitize, unsupported_analysis};
use crate::codegen::{CodeGenError, bare_node};
use crate::ir::*;

/// The `analysis` statement plus any `sweep` lines that must precede it.
/// `native`: the IR was read from a VACASK deck, whose `tran` without a
/// `maxstep` means VACASK's own default, not SPICE's TMAX.
pub(super) fn analysis_block(
    idx: usize,
    a: &Analysis,
    native: bool,
) -> Result<Vec<String>, CodeGenError> {
    let name = format!("{}{}", a.kind_str(), idx + 1);
    let mut out = Vec::new();
    match a {
        Analysis::Op => out.push(format!("analysis {} op", name)),

        // VACASK has no `.dc`: a DC sweep is a `sweep` block wrapped around
        // an operating point. SPICE's FIRST source is the fast one and
        // VACASK's LAST `sweep` is, so the blocks emit in the opposite
        // order.
        Analysis::Dc { sweeps } => {
            if sweeps.is_empty() {
                return Err(unsupported_analysis("dc with no swept source"));
            }
            for (i, sw) in sweeps.iter().enumerate().rev() {
                let scale = if i == 0 {
                    "vsweep".to_string()
                } else {
                    format!("sweep{}", i)
                };
                out.push(sweep_line(
                    &scale, "instance", &sw.source, "dc", sw.start, sw.stop, sw.step,
                )?);
            }
            out.push(format!("  analysis {} op", name));
        }

        Analysis::Ac {
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
        } => {
            out.push(format!(
                "analysis {} ac {}",
                name,
                freq_sweep(*variation, *points, *start, *stop)?
            ));
        }

        // `step` is only a STARTING step in VACASK; `maxstep` is the
        // separate bound. SPICE's TSTEP also caps the internal step because
        // TMAX defaults to min(TSTEP, (TSTOP-TSTART)/50). Emitting `step`
        // alone lets VACASK take strides SPICE never would.
        Analysis::Transient {
            step,
            stop,
            start,
            max_step,
            uic,
        } => {
            if start.unwrap_or(0.0) != 0.0 {
                return Err(unsupported_analysis(
                    "transient with a nonzero start time (VACASK always saves from t=0)",
                ));
            }
            let tmax = max_step.or((!native).then(|| step.min(stop / 50.0)));
            let mut s = format!(
                "analysis {} tran step={} stop={}",
                name,
                num(*step),
                num(*stop)
            );
            if let Some(t) = tmax {
                w!(s, " maxstep={}", num(t));
            }
            s.push_str(if *uic {
                " icmode=\"uic\""
            } else {
                " icmode=\"op\""
            });
            out.push(s);
        }

        Analysis::Noise {
            output,
            reference,
            source,
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
            ..
        } => {
            if !reference.is_empty() && reference != "0" {
                return Err(unsupported_analysis(
                    "noise with a non-ground reference node (VACASK's `out=` names one node)",
                ));
            }
            out.push(format!(
                "analysis {} noise out=\"{}\" in=\"{}\" {}",
                name,
                bare_node(output).to_ascii_lowercase(),
                source.to_ascii_lowercase(),
                freq_sweep(*variation, *points, *start, *stop)?,
            ));
        }

        // VACASK's `dcxf` reports the transfer function from EVERY source
        // to `out` in one plot, so the input source is implicit.
        Analysis::Tf { output, .. } => {
            out.push(format!(
                "analysis {} dcxf out=\"{}\"",
                name,
                bare_node(output).to_ascii_lowercase()
            ));
        }

        other => {
            return Err(unsupported_analysis(other.kind_str().to_string()));
        }
    }
    Ok(out)
}

/// `.step param` -> a control variable plus a sweep over it. A `var`
/// declared in the control block IS visible to the netlist body, verified
/// against the binary, so `r1 (a b) resistor r=rval` tracks the sweep.
pub(super) fn step_sweep(sp: &StepParam) -> Result<String, CodeGenError> {
    let name = sp.param.to_ascii_lowercase();
    let scale = format!("step_{}", sanitize(&name));
    match sp.sweep_type {
        Some(mode @ (Sweep::Dec | Sweep::Oct)) => Ok(format!(
            "sweep {} variable=\"{}\" from={} to={} mode=\"{}\" points={}",
            scale,
            name,
            num(sp.start),
            num(sp.stop),
            mode.name(),
            sp.step as u64,
        )),
        // Spectre's total-count log sweep; it used to fall through to a
        // linear step here.
        Some(Sweep::Log) => Err(unsupported_analysis(format!(
            "step '{name}': log sweep (VACASK sweeps lin, dec or oct)"
        ))),
        Some(Sweep::Lin) | None => Ok(format!(
            "sweep {} variable=\"{}\" from={} to={} step={}",
            scale,
            name,
            num(sp.start),
            num(sp.stop),
            num(sp.step),
        )),
    }
}

/// `ic=[...]` / `nodeset=[...]`: a flat list, semicolon-separated, node
/// names quoted and values bare.
pub(super) fn node_list(key: &str, items: &[(String, f64)]) -> String {
    let body: Vec<String> = items
        .iter()
        .map(|(n, v)| format!("\"{}\"; {}", n.to_ascii_lowercase(), num(*v)))
        .collect();
    format!(" {}=[{}]", key, body.join("; "))
}

// ---- private ----

/// One `sweep` line. `points` counts INTERVALS, not points, and the
/// endpoint is RECOMPUTED rather than copied: SPICE walks from START by
/// STEP and stops before passing STOP, while VACASK divides `from`..`to`
/// into equal intervals and always lands ON `to`. Copying STOP across puts
/// every sample on a different grid.
fn sweep_line(
    scale: &str,
    target: &str,
    name: &str,
    param: &str,
    start: f64,
    stop: f64,
    step: f64,
) -> Result<String, CodeGenError> {
    if step == 0.0 {
        return Err(unsupported_analysis("sweep with a zero step"));
    }
    let span = ((stop - start) / step).abs();
    let n = (span + 1e-9 * span.max(1.0)).floor();
    if n < 1.0 || n.is_nan() || n > 1e7 {
        return Err(unsupported_analysis(format!(
            "sweep of '{}' covers {} intervals",
            name, n
        )));
    }
    let last = start + (stop - start).signum() * n * step.abs();
    // Within rounding of the stop, the stop is the endpoint (a VACASK-origin
    // sweep's step is (to - from) / points, which multiplies back an ulp off).
    let last = if (last - stop).abs() <= 1e-9 * step.abs() {
        stop
    } else {
        last
    };
    Ok(format!(
        "sweep {} {}=\"{}\" parameter=\"{}\" from={} to={} mode=\"lin\" points={}",
        scale,
        target,
        name.to_ascii_lowercase(),
        param,
        num(start),
        num(last),
        n as u64,
    ))
}

/// Frequency-swept analyses. For `dec`/`oct` SPICE's N is points per
/// decade/octave and so is VACASK's; for `lin` SPICE's N is the TOTAL point
/// count while VACASK counts intervals, hence the `- 1`.
fn freq_sweep(
    variation: Sweep,
    points: u32,
    start: f64,
    stop: f64,
) -> Result<String, CodeGenError> {
    let mode = variation.name();
    let n = match variation {
        Sweep::Dec | Sweep::Oct => points,
        Sweep::Lin => points.saturating_sub(1),
        Sweep::Log => return Err(unsupported_analysis("frequency sweep type 'log'")),
    };
    if n == 0 {
        return Err(unsupported_analysis(format!(
            "frequency sweep with {} points",
            points
        )));
    }
    Ok(format!(
        "from={} to={} mode=\"{}\" points={}",
        num(start),
        num(stop),
        mode,
        n
    ))
}
