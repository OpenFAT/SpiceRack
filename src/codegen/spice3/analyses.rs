//! Analyses and `.step` sweeps -> SPICE3-family analysis cards.
//!
//! Contents:
//! - `pub(super) fn analysis(d: Dialect, x: bool, a: &Analysis) -> Result<String, CodeGenError>`
//! - `pub(super) fn step_param(d: Dialect, sp: &StepParam) -> Result<String, CodeGenError>`

use super::values::{key, ngspice, num};
use crate::codegen::{CodeGenError, Dialect};
use crate::ir::*;

/// `a`'s card for `d` (HSPICE's `.tran` with a step cap and `.noise` take
/// two lines), or the refusal of one `d` has no card for.
pub(super) fn analysis(d: Dialect, x: bool, a: &Analysis) -> Result<String, CodeGenError> {
    let (lt, ng, hs) = (d == Dialect::Ltspice, ngspice(d), d == Dialect::Hspice);
    Ok(match a {
        Analysis::Op => ".op".into(),
        Analysis::Dc { sweeps } => {
            let mut s = String::from(".dc");
            for sw in sweeps {
                w!(
                    s,
                    " {} {} {} {}",
                    sw.source,
                    num(d, x, sw.start),
                    num(d, x, sw.stop),
                    num(d, x, sw.step)
                );
            }
            s
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
            format!(
                ".ac {} {} {} {}",
                variation.name(),
                points,
                num(d, x, *start),
                num(d, x, *stop)
            )
        }
        // HSPICE reads extra positional fields as further step/stop
        // segments, so the start is `START=` and the step cap `delmax`.
        Analysis::Transient {
            step,
            stop,
            start,
            max_step,
            uic,
        } if hs => {
            let mut s = String::new();
            if let Some(ms) = max_step {
                w!(s, ".option delmax={}\n", num(d, x, *ms));
            }
            w!(s, ".tran {} {}", num(d, x, *step), num(d, x, *stop));
            if let Some(st) = start {
                w!(s, " START={}", num(d, x, *st));
            }
            if *uic {
                s.push_str(" UIC");
            }
            s
        }
        Analysis::Transient {
            step,
            stop,
            start,
            max_step,
            uic,
        } => {
            let mut s = format!(".tran {} {}", num(d, x, *step), num(d, x, *stop));
            if let Some(st) = start {
                w!(s, " {}", num(d, x, *st));
            }
            if let Some(ms) = max_step {
                if start.is_none() {
                    // Need a placeholder for start when max_step is given
                    s.push_str(" 0");
                }
                w!(s, " {}", num(d, x, *ms));
            }
            if *uic {
                s.push_str(" UIC");
            }
            s
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
            points_per_summary,
        } => {
            let out_spec = if reference.is_empty() || reference == "0" {
                format!("V({})", output)
            } else {
                format!("V({},{})", output, reference)
            };
            // HSPICE: `.noise ov src inter` takes its frequencies from `.ac`.
            if hs {
                return Ok(format!(
                    ".ac {} {} {} {}\n.noise {} {} {}",
                    variation.name(),
                    points,
                    num(d, x, *start),
                    num(d, x, *stop),
                    out_spec,
                    source,
                    points_per_summary.unwrap_or(0),
                ));
            }
            let mut s = format!(
                ".noise {} {} {} {} {} {}",
                out_spec,
                source,
                variation.name(),
                points,
                num(d, x, *start),
                num(d, x, *stop)
            );
            // `pts_per_summary` is an ngspice/SPICE3 extension; LTspice's
            // `.noise` card stops at the stop frequency.
            if let (Some(pps), true) = (points_per_summary, ng || d == Dialect::Xyce) {
                w!(s, " {}", pps);
            }
            s
        }
        // Xyce has no .tf (RG Table 6-1).
        Analysis::Tf { .. } if d == Dialect::Xyce => {
            return Err(CodeGenError::UnsupportedAnalysis {
                backend: "xyce".into(),
                analysis: "Tf".into(),
            });
        }
        Analysis::Tf { output, source } => format!(".tf {} {}", output, source),
        Analysis::Fourier {
            fundamental,
            outputs,
            num_harmonics,
        } => {
            // SPICE3/ngspice: `.four <freq> <ov1> [ov2 ...]` — there is no
            // harmonic-count field. Emitting one makes ngspice warn
            // "can't parse '10': ignored" and then fail the analysis with
            // "lengths don't match" because it treats the count as an output
            // vector. LTspice's card *does* take `[Nharmonics]`.
            let mut s = format!(".four {}", num(d, x, *fundamental));
            if let (Some(nh), true) = (num_harmonics, lt) {
                w!(s, " {}", nh);
            }
            for out in outputs {
                w!(s, " {}", out);
            }
            s
        }
        // LTspice has no `.sens`, `.pz`, `.disto`, `.pss` or `.sp` card;
        // HSPICE's `.sens` is DC only and it has none of the others.
        Analysis::Sensitivity { output, ac } if ng || (hs && ac.is_none()) => {
            let mut s = format!(".sens {}", output);
            if let Some(p) = ac {
                w!(
                    s,
                    " AC {} {} {} {}",
                    p.variation.name(),
                    p.points,
                    num(d, x, p.start),
                    num(d, x, p.stop)
                );
            }
            s
        }
        Analysis::PoleZero {
            node1,
            node2,
            node3,
            node4,
            tf_type,
            pz_type,
        } if ng => {
            format!(
                ".pz {} {} {} {} {} {}",
                node1,
                node2,
                node3,
                node4,
                tf_type.name(),
                pz_type.name()
            )
        }
        Analysis::Distortion {
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
            f2overf1,
        } if ng => {
            let mut s = format!(
                ".disto {} {} {} {}",
                variation.name(),
                points,
                num(d, x, *start),
                num(d, x, *stop)
            );
            if let Some(ratio) = f2overf1 {
                w!(s, " {}", ratio);
            }
            s
        }
        // ponytail: Rust `{}` numbers, byte-for-byte what the retired string
        // path sent ngspice for these two cards.
        Analysis::Pss {
            fundamental,
            stabilization,
            observe_node,
            points_per_period,
            harmonics,
            sc_iter,
            steady_coeff,
            uic,
        } if ng => {
            let mut s = format!(
                ".pss {} {} {} {} {}",
                fundamental, stabilization, observe_node, points_per_period, harmonics
            );
            // SCITER and STEADYCOEFF are positional (manual §11.3.12): one
            // given, or `uic`, writes both, the other at ngspice's default.
            if sc_iter.is_some() || steady_coeff.is_some() || *uic {
                w!(
                    s,
                    " {} {}",
                    sc_iter.unwrap_or(50),
                    steady_coeff.unwrap_or(1e-3)
                );
            }
            if *uic {
                s.push_str(" uic");
            }
            s
        }
        Analysis::SPar {
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
        } if ng => {
            format!(".sp {} {} {} {}", variation.name(), points, start, stop)
        }
        // Harmonic balance, loop-gain stability, transient noise and the
        // SpectreRF family have no SPICE3-family card.
        other => {
            let dbg = format!("{other:?}");
            return Err(CodeGenError::UnsupportedAnalysis {
                backend: key(d).into(),
                analysis: dbg.split([' ', '(']).next().unwrap_or_default().into(),
            });
        }
    })
}

/// `.step` sweeps.
///
/// ngspice has no `.step` at all — it aborts the run with
/// "unimplemented dot command '.step'" (verified on 44.2). Emitting it
/// commented out is worse than failing: the sweep silently collapses to a
/// single run and the caller gets one curve where they asked for N. So this
/// is a hard error naming the two things that do work.
pub(super) fn step_param(d: Dialect, sp: &StepParam) -> Result<String, CodeGenError> {
    // HSPICE sweeps a parameter on the analysis card (`sweep`) or with
    // `.alter`; there is no `.step`.
    if d == Dialect::Hspice {
        return Err(CodeGenError::Other(format!(
            "HSPICE has no `.step` (stepping '{}'). Sweep it on the analysis card \
             or drive the sweep from Python by re-running the testbench per value.",
            sp.param,
        )));
    }
    // Xyce and LTspice step lin, dec or oct only; Spectre's `log` is not
    // theirs.
    if sp.sweep_type == Some(Sweep::Log) && matches!(d, Dialect::Xyce | Dialect::Ltspice) {
        return Err(CodeGenError::Other(format!(
            "{} `.step` has no log sweep (stepping '{}')",
            d.name(),
            sp.param
        )));
    }
    // Xyce: `.step [lin|dec|oct] <name> <start> <stop> <inc>`, no `param` (RG p.155).
    if d == Dialect::Xyce {
        let sweep = sp
            .sweep_type
            .map(|t| format!("{} ", t.name()))
            .unwrap_or_default();
        return Ok(format!(
            ".step {sweep}{} {} {} {}",
            sp.param, sp.start, sp.stop, sp.step
        ));
    }
    if d != Dialect::Ltspice {
        return Err(CodeGenError::Other(format!(
            "ngspice has no `.step` (stepping '{}' would silently run once). \
             Use `.dc {} {} {} {}` for a temperature or source sweep, \
             or drive the sweep from Python by re-running the testbench per value.",
            sp.param, sp.param, sp.start, sp.stop, sp.step,
        )));
    }
    // LTspice: `.step [lin|oct|dec] param <name> <start> <stop> <inc>`.
    // The `param` keyword is mandatory in both forms.
    Ok(match sp.sweep_type {
        Some(sweep) => format!(
            ".step {} param {} {} {} {}",
            sweep.name(),
            sp.param,
            sp.start,
            sp.stop,
            sp.step
        ),
        None => format!(
            ".step param {} {} {} {}",
            sp.param, sp.start, sp.stop, sp.step
        ),
    })
}
