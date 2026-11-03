//! Analyses and `.step` sweeps -> Spectre analysis statements and the `sweep`
//! blocks around them. Sources as in `spectre/mod.rs`.
//!
//! Contents:
//! - `pub(super) fn emit_analysis_block(analyses: &[Analysis], step_params: &[StepParam], keep_case: bool) -> Result<Vec<String>, CodeGenError>`
//! - `pub(super) fn emit_analysis(analysis: &Analysis, keep_case: bool) -> Result<String, CodeGenError>`

use crate::ir::*;

use super::names::inst_ref;
use super::values::{spectre_num, unsupported_analysis};
use crate::codegen::{CodeGenError, bare_node};

/// Every analysis statement, nested inside one `sweep` block per step
/// parameter (the first parameter outermost), one entry per line.
pub(super) fn emit_analysis_block(
    analyses: &[Analysis],
    step_params: &[StepParam],
    keep_case: bool,
) -> Result<Vec<String>, CodeGenError> {
    let mut block = Vec::new();
    for analysis in analyses {
        block.push(emit_analysis(analysis, keep_case)?);
    }

    for (idx, sp) in step_params.iter().enumerate().rev() {
        let mut wrapped = Vec::new();
        wrapped.push(emit_sweep_header(idx, sp)?);
        for line in block {
            wrapped.push(format!("  {}", line.replace('\n', "\n  ")));
        }
        wrapped.push("}".into());
        block = wrapped;
    }

    Ok(block)
}

/// `analysis`'s statement; several lines (`\n`-joined) where Spectre needs a
/// companion analysis first (`sens`, the SpectreRF family) or one statement
/// per output (`fourier`).
pub(super) fn emit_analysis(analysis: &Analysis, keep_case: bool) -> Result<String, CodeGenError> {
    let s = match analysis {
        // `[REF]` p.20143 verbatim: `dc1 dc`. A `dc` with no sweep
        // parameter is the operating point.
        Analysis::Op => "op1 dc".into(),

        // `[REF]` p.64: "sweep the circuit temperature by giving the
        // parameter name as param=temp without a dev, mod or sub
        // parameter", "sweep a top-level netlist parameter by giving the
        // parameter name without a dev, mod or sub parameter", and #12
        // `dev` = "Device instance whose parameter value is to be swept".
        // SPICE `.dc Vin 0 5 0.1` sweeps the *dc value of a source*, which
        // in Spectre is `dev=vin param=dc` — not `param=Vin`.
        // A second sweep needs a `sweep` block around the first; written as
        // one `dc` it would silently drop the outer sweep.
        Analysis::Dc { sweeps } if sweeps.len() > 1 => {
            return Err(unsupported_analysis(
                "a .dc with nested sweeps (only one is written as a Spectre dc)",
            ));
        }
        Analysis::Dc { sweeps } => {
            if let Some(sw) = sweeps.first() {
                let selector = dc_sweep_selector(&sw.source);
                format!(
                    "dc1 dc {} start={} stop={} step={}",
                    selector,
                    spectre_num(sw.start),
                    spectre_num(sw.stop),
                    spectre_num(sw.step),
                )
            } else {
                "dc1 dc".into()
            }
        }

        // `[REF]` p.43 `Name ac parameter=value ...` with start/stop/dec.
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
                "ac1 ac start={} stop={} {}",
                spectre_num(*start),
                spectre_num(*stop),
                sweep_points(*variation, *points)?,
            )
        }

        // `[REF]` p.416-417 `Name tran parameter=value ...`: `stop` (#1),
        // `start` (#3), `maxstep` (#8), `step` (#9, "Minimum time step used
        // by the simulator solely to maintain the aesthetics of the
        // computed waveforms") — the closest analogue of SPICE's tstep.
        // Verbatim example `[REF]` p.433: `tran1 tran stop=0.5u ...`.
        Analysis::Transient {
            step,
            stop,
            start,
            max_step,
            uic,
        } => {
            let mut s = format!(
                "tran1 tran step={} stop={}",
                spectre_num(*step),
                spectre_num(*stop),
            );
            if let Some(st) = start {
                w!(s, " start={}", spectre_num(*st));
            }
            if let Some(ms) = max_step {
                w!(s, " maxstep={}", spectre_num(*ms));
            }
            // SPICE `uic`: start from the initial conditions, no DC solve.
            if *uic {
                s.push_str(" skipdc=yes");
            }
            s
        }

        // `[REF]` p.181-182: `Name [p] [n] noise parameter=value ...`,
        // "The optional terminals (p and n) specify the output of the
        // circuit". #16 `oprobe` and #17 `iprobe` name *components*, not
        // nodes — so the output goes in the terminal list and only the
        // input source becomes `iprobe`.
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
            let neg = if reference.is_empty() {
                "0"
            } else {
                bare_node(reference)
            };
            format!(
                "noise1 ({} {}) noise start={} stop={} {} iprobe={}",
                bare_node(output),
                neg,
                spectre_num(*start),
                spectre_num(*stop),
                sweep_points(*variation, *points)?,
                inst_ref(source, keep_case),
            )
        }

        // `[REF]` p.438-439: `Name [p] [n] xf parameter=value ...`,
        // "you can simply specify a voltage to be the output by giving a
        // pair of nodes on the xf analysis statement". xf computes the
        // transfer function from *every* independent source to that output,
        // so there is no `source=` parameter (the previous version emitted
        // one). `freq` is parameter #15.
        //
        // INFERRED: SPICE `.tf` is a DC transfer function and Spectre's xf
        // is a small-signal/AC one; `freq=0` is this file's choice for the
        // DC case and is not stated in any Cadence document.
        Analysis::Tf { output, source: _ } => {
            format!("xf1 ({} 0) xf freq=0", bare_node(output))
        }

        // `[REF]` p.524: `sens (output_variables_list) to
        // (design_parameters_list) for (analyses_list)`, verbatim example
        // `sens (1 n2 7) for (analAC)`. `sens` is a control statement that
        // refers to a named analysis, so the analysis is emitted too.
        Analysis::Sensitivity { output, ac } => match ac {
            Some(p) => format!(
                "sensac1 ac start={} stop={} {}\nsens ({}) for (sensac1)",
                spectre_num(p.start),
                spectre_num(p.stop),
                sweep_points(p.variation, p.points)?,
                bare_node(output),
            ),
            None => format!("sensdc1 dc\nsens ({}) for (sensdc1)", bare_node(output)),
        },

        // `[REF]` p.272-277: `Name [p] [n] pss parameter=value ...` with
        // `fund` (#2), `harms` (#4), `tstab` (#6). There is no `ppv` and no
        // `probe` parameter (the previous version emitted both); for an
        // autonomous circuit the observed node goes in the terminal list.
        // `points_per_period` has no documented pss parameter and is
        // dropped. ngspice's shooting controls (`sciter`, `steadycoeff`,
        // `uic`) have no mapped Spectre parameter and are refused.
        Analysis::Pss {
            sc_iter,
            steady_coeff,
            uic,
            ..
        } if sc_iter.is_some() || steady_coeff.is_some() || *uic => {
            return Err(unsupported_analysis(
                "pss sciter/steadycoeff/uic (ngspice shooting controls)",
            ));
        }
        Analysis::Pss {
            fundamental,
            stabilization,
            observe_node,
            harmonics,
            ..
        } => {
            let terms = if observe_node.is_empty() {
                String::new()
            } else {
                format!("({} 0) ", bare_node(observe_node))
            };
            format!(
                "pss1 {}pss fund={} tstab={} harms={}",
                terms,
                spectre_num(*fundamental),
                spectre_num(*stabilization),
                harmonics,
            )
        }

        // `[REF]` p.90-91: `Name [p] [n] hb parameter=value ...` with
        // `fundfreqs=[...]` (#2, "Array of fundamental frequencies") and
        // `maxharms=[...]` (#3). There are no `toneN`/`nharmN` parameters.
        Analysis::HarmonicBalance {
            frequencies,
            harmonics,
        } => {
            let freqs: Vec<String> = frequencies.iter().map(|f| spectre_num(*f)).collect();
            let harms: Vec<String> = harmonics.iter().map(|h| h.to_string()).collect();
            format!(
                "hb1 hb fundfreqs=[{}] maxharms=[{}]",
                freqs.join(" "),
                harms.join(" ")
            )
        }

        // `[REF]` p.387-389: `Name sp parameter=value ...`. NOTE the deck
        // must also contain `port` instances — "There must be at least one
        // active port statement in the circuit" — which the IR cannot
        // express, so this statement alone is necessary but not sufficient.
        Analysis::SPar {
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
        } => {
            format!(
                "sp1 sp start={} stop={} {}",
                spectre_num(*start),
                spectre_num(*stop),
                sweep_points(*variation, *points)?,
            )
        }

        // `[REF]` p.393-394: `Name stb parameter=value ...`, `probe` =
        // "Probe instance around which the loop gain is calculated".
        Analysis::Stability {
            probe,
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
        } => {
            format!(
                "stb1 stb start={} stop={} {} probe={}",
                spectre_num(*start),
                spectre_num(*stop),
                sweep_points(*variation, *points)?,
                inst_ref(probe, keep_case),
            )
        }

        // Spectre has no `trnoise` analysis. Transient noise is a `tran`
        // with `noisefmax`/`noiseseed` (`[REF]` p.433 verbatim:
        // `tran1 tran stop=0.5u noisefmax=10G noiseseed=1`) and the IR
        // carries no noise bandwidth, so there is nothing to emit.
        Analysis::TransientNoise { .. } => {
            return Err(unsupported_analysis(
                "transient noise (Spectre spells it `tran ... noisefmax=<Hz>`; \
                 the IR carries no noise bandwidth)",
            ));
        }

        // `[CMP]` p.323-325: `fourier` is a *component*, not an analysis —
        // `Name [p] [n] [pr] [nr] fourier parameter=value ...`, verbatim
        // `four1 (1 0) fourmod harms=50` with `fund` as instance parameter
        // #1. It is active during transient analysis.
        Analysis::Fourier {
            fundamental,
            outputs,
            num_harmonics,
        } => {
            let harms = num_harmonics
                .map(|h| format!(" harms={h}"))
                .unwrap_or_default();
            outputs
                .iter()
                .enumerate()
                .map(|(i, out)| {
                    format!(
                        "four{} ({} 0) fourier fund={}{}",
                        i + 1,
                        bare_node(out),
                        spectre_num(*fundamental),
                        harms,
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        // `[REF]` p.406-410 `sweep`. `inner` is the child analysis name and
        // `inner_type` its type, which together form the child statement.
        Analysis::SpectreSweep {
            param,
            start,
            stop,
            step,
            inner,
            inner_type,
        } => {
            format!(
                "sweep1 sweep param={} start={} stop={} step={} {{\n  {} {}\n}}",
                param,
                spectre_num(*start),
                spectre_num(*stop),
                spectre_num(*step),
                inner,
                inner_type.name(),
            )
        }

        // `[REF]` p.174 verbatim:
        // `mc1 montecarlo variations=process seed=1234 numruns=200 { ... }`
        // — every parameter precedes the brace. The previous version put
        // `seed=` *after* the closing brace.
        //
        // NOTE: montecarlo only varies parameters declared in a
        // `statistics` block (`[REF]` p.174-180), which the IR cannot
        // express; without one this runs `numruns` identical simulations.
        Analysis::SpectreMonteCarlo {
            iterations,
            inner,
            inner_type,
            seed,
        } => {
            let seed_str = seed.map(|s| format!(" seed={s}")).unwrap_or_default();
            format!(
                "mc1 montecarlo numruns={}{} {{\n  {} {}\n}}",
                iterations,
                seed_str,
                inner,
                inner_type.name(),
            )
        }

        // `[REF]` p.245-246 `pac`: sweep interval params plus `sweeptype`
        // (#11, values absolute/relative/unspecified). PSS is its
        // prerequisite (`[REF]` p.272).
        Analysis::SpectrePac {
            pss:
                PssBase {
                    pss_fundamental,
                    pss_stabilization,
                    pss_harmonics,
                },
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
            sweep_type,
        } => {
            format!(
                "pss1 pss fund={} tstab={} harms={}\npac1 pac start={} stop={} {} sweeptype={}",
                spectre_num(*pss_fundamental),
                spectre_num(*pss_stabilization),
                pss_harmonics,
                spectre_num(*start),
                spectre_num(*stop),
                sweep_points(*variation, *points)?,
                sweep_type,
            )
        }

        // `[REF]` p.253-256 `pnoise`: `Name [p] [n] ... pnoise ...`, probe
        // parameters are `oprobe` (#13) and `iprobe` (#14) — both component
        // names. There is no `refprobe` (the previous version emitted one);
        // the reference node belongs in the terminal list.
        Analysis::SpectrePnoise {
            pss:
                PssBase {
                    pss_fundamental,
                    pss_stabilization,
                    pss_harmonics,
                },
            output,
            reference,
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
        } => {
            let neg = if reference.is_empty() {
                "0"
            } else {
                bare_node(reference)
            };
            format!(
                "pss1 pss fund={} tstab={} harms={}\npnoise1 ({} {}) pnoise start={} stop={} {}",
                spectre_num(*pss_fundamental),
                spectre_num(*pss_stabilization),
                pss_harmonics,
                bare_node(output),
                neg,
                spectre_num(*start),
                spectre_num(*stop),
                sweep_points(*variation, *points)?,
            )
        }

        // `[REF]` p.303-304 `pxf`: `Name [p] [n] ... pxf ...`, the only
        // probe parameter is `probe` (#13, the *output*). Like `xf` it
        // computes transfer functions from every source, so there is no
        // input-source parameter (the previous version emitted `isrc=`).
        Analysis::SpectrePxf {
            pss:
                PssBase {
                    pss_fundamental,
                    pss_stabilization,
                    pss_harmonics,
                },
            output,
            source: _,
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
        } => {
            format!(
                "pss1 pss fund={} tstab={} harms={}\npxf1 ({} 0) pxf start={} stop={} {}",
                spectre_num(*pss_fundamental),
                spectre_num(*pss_stabilization),
                pss_harmonics,
                bare_node(output),
                spectre_num(*start),
                spectre_num(*stop),
                sweep_points(*variation, *points)?,
            )
        }

        // `[REF]` p.298-299 `pstb`: `probe` (#11) = "Probe instance around
        // which the loop gain is calculated".
        Analysis::SpectrePstb {
            pss:
                PssBase {
                    pss_fundamental,
                    pss_stabilization,
                    pss_harmonics,
                },
            probe,
            sweep:
                FreqSweep {
                    variation,
                    points,
                    start,
                    stop,
                },
        } => {
            format!(
                "pss1 pss fund={} tstab={} harms={}\npstb1 pstb start={} stop={} {} probe={}",
                spectre_num(*pss_fundamental),
                spectre_num(*pss_stabilization),
                pss_harmonics,
                spectre_num(*start),
                spectre_num(*stop),
                sweep_points(*variation, *points)?,
                inst_ref(probe, keep_case),
            )
        }

        // Spectre does have `pz` (`[REF]` p.312) but its parameter set does
        // not match SPICE's `.pz` node quadruple, and everything else left
        // here is another simulator's dialect.
        other => {
            return Err(unsupported_analysis(other.kind_str()));
        }
    };
    Ok(s)
}

// ---- private ----

/// `[REF]` p.406-410 `sweep`, verbatim example:
/// ```text
/// swp sweep param=temp values=[-50 0 50 100 125] {
///                oppoint dc oppoint=logfile
/// }
/// ```
fn emit_sweep_header(idx: usize, sp: &StepParam) -> Result<String, CodeGenError> {
    let suffix: String = sp
        .param
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let sweep = match sp.sweep_type {
        Some(Sweep::Dec) => format!("dec={}", spectre_num(sp.step)),
        Some(Sweep::Lin) | None => format!("step={}", spectre_num(sp.step)),
        Some(Sweep::Log) => format!("log={}", spectre_num(sp.step)),
        Some(Sweep::Oct) => {
            return Err(unsupported_analysis(
                "parameter sweep type 'oct' (Spectre sweep accepts step/lin/dec/log)",
            ));
        }
    };
    Ok(format!(
        "sweep{}_{} sweep param={} start={} stop={} {} {{",
        idx + 1,
        suffix,
        sp.param,
        spectre_num(sp.start),
        spectre_num(sp.stop),
        sweep,
    ))
}

/// Frequency-sweep point spec shared by `ac`, `noise`, `xf`, `sp`, `stb`,
/// `pac`, `pnoise`, `pxf`, `pstb`.
///
/// `[REF]` p.43/44 (ac), p.182 (noise), p.387 (sp), p.393 (stb), p.245
/// (pac), p.255 (pnoise), p.299 (pstb), p.303 (pxf), p.439 (xf) all list the
/// identical "Sweep interval parameters" block:
/// `start stop center span step lin dec log values valuesfile`.
/// There is no `oct` — SPICE's octave sweep has no Spectre spelling.
fn sweep_points(variation: Sweep, points: u32) -> Result<String, CodeGenError> {
    match variation {
        Sweep::Dec | Sweep::Lin | Sweep::Log => Ok(format!("{}={points}", variation.name())),
        Sweep::Oct => Err(unsupported_analysis(
            "frequency sweep type 'oct' (Spectre accepts only dec/lin/log, see spectre -h ac)",
        )),
    }
}

/// Pick the Spectre sweep-variable spelling for a SPICE `.dc` first argument.
///
/// `[REF]` p.64 lists the three forms: `param=temp` for temperature, a bare
/// `param=<name>` for a top-level netlist parameter, and `dev=<instance>`
/// combined with `param=<instance parameter>` for a device. SPICE sweeps the
/// `dc` value of an independent source, whose Spectre instance parameter is
/// `dc` (`[CMP]` p.683 vsource #1, p.380 isource #1).
fn dc_sweep_selector(source: &str) -> String {
    let lower = source.to_lowercase();
    if lower == "temp" {
        return "param=temp".into();
    }
    // SPICE requires the swept element of a `.dc` card to be an independent
    // source; V/I prefixes are the only legal spellings for one.
    match lower.as_bytes().first() {
        Some(b'v') | Some(b'i') => format!("dev={lower} param=dc"),
        _ => format!("param={source}"),
    }
}
