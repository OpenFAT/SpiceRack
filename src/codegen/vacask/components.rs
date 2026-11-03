//! One `Component` -> its VACASK instance line, its source waveform included.
//!
//! Contents:
//! - `pub(super) fn emit_component(out: &mut String, comp: &Component, low: bool) -> Result<(), CodeGenError>`

use super::models::Need;
use super::values::{Q, ac_spec, inst_params, num, q, unsupported_comp, val, wave_num};
use crate::codegen::{CodeGenError, model_refuse};
use crate::ir::*;

/// `comp`'s instance line, written onto `out` with no line end. Nothing is
/// written when it errs.
pub(super) fn emit_component(
    out: &mut String,
    comp: &Component,
    low: bool,
) -> Result<(), CodeGenError> {
    let auto = |n: Need| n.auto_model();
    match comp {
        Component::Resistor {
            name,
            n1,
            n2,
            value,
            model,
            params,
        }
        | Component::Capacitor {
            name,
            n1,
            n2,
            value,
            model,
            params,
        }
        | Component::Inductor {
            name,
            n1,
            n2,
            value,
            model,
            params,
        } => {
            if model.is_some() {
                model_refuse("vacask", &Q(name, false).to_string(), model)?;
            }
            let (l, m) = match comp {
                Component::Resistor { .. } => ('r', "resistor"),
                Component::Capacitor { .. } => ('c', "capacitor"),
                _ => ('l', "inductor"),
            };
            w!(
                out,
                "{l}{} ({} {}) {} {l}={}",
                q(name),
                q(n1),
                q(n2),
                auto(Need::Sp(m)),
                val(value, low)
            );
            inst_params(out, params, low);
        }
        // VACASK's mutual inductance is a device with NO nodes that names
        // the two inductors it couples.
        Component::MutualInductor {
            name,
            inductor1,
            inductor2,
            coupling,
        } => w!(
            out,
            "k{} () {} k={} ind1=\"l{}\" ind2=\"l{}\"",
            q(name),
            auto(Need::Builtin("mutual")),
            num(*coupling),
            q(inductor1),
            q(inductor2),
        ),
        // VACASK's `acsp` ports are not ngspice's `portnum` (ir-gaps §2):
        // refused, not dropped.
        Component::VoltageSource {
            name,
            port: Some(_),
            ..
        } => return Err(unsupported_comp(format!("v{}: RF port", q(name)))),
        Component::VoltageSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform,
            port: None,
        }
        | Component::CurrentSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform,
        } => {
            let (l, m) = if matches!(comp, Component::VoltageSource { .. }) {
                ('v', "vsource")
            } else {
                ('i', "isource")
            };
            let wave = match waveform {
                Some(wf) => Some(wave(&format_args!("{l}{}", q(name)), wf, low)?),
                None => None,
            };
            w!(
                out,
                "{l}{} ({} {}) {} dc={}",
                q(name),
                q(np),
                q(nm),
                auto(Need::Builtin(m)),
                val(value, low)
            );
            ac_spec(out, ac_magnitude, ac_phase);
            if let Some(wave) = wave {
                w!(out, " {wave}");
            }
        }
        Component::Vcvs {
            name,
            np,
            nm,
            ncp,
            ncm,
            gain,
        } => w!(
            out,
            "e{} ({} {} {} {}) {} gain={}",
            q(name),
            q(np),
            q(nm),
            q(ncp),
            q(ncm),
            auto(Need::Builtin("vcvs")),
            num(*gain),
        ),
        Component::Vccs {
            name,
            np,
            nm,
            ncp,
            ncm,
            transconductance,
        } => w!(
            out,
            "g{} ({} {} {} {}) {} gain={}",
            q(name),
            q(np),
            q(nm),
            q(ncp),
            q(ncm),
            auto(Need::Builtin("vccs")),
            num(*transconductance),
        ),
        // The current-controlled pair names its sensing device with
        // `ctlinst` instead of taking it as a node.
        Component::Cccs {
            name,
            np,
            nm,
            vsense,
            gain,
        } => w!(
            out,
            "f{} ({} {}) {} ctlinst=\"{}\" gain={}",
            q(name),
            q(np),
            q(nm),
            auto(Need::Builtin("cccs")),
            q(vsense),
            num(*gain),
        ),
        Component::Ccvs {
            name,
            np,
            nm,
            vsense,
            transresistance,
        } => w!(
            out,
            "h{} ({} {}) {} ctlinst=\"{}\" gain={}",
            q(name),
            q(np),
            q(nm),
            auto(Need::Builtin("ccvs")),
            q(vsense),
            num(*transresistance),
        ),
        Component::Diode {
            name,
            np,
            nm,
            model,
            params,
        } => {
            w!(out, "d{} ({} {}) {}", q(name), q(np), q(nm), q(model));
            inst_params(out, params, low);
        }
        Component::Bjt {
            name,
            nc,
            nb,
            ne,
            extra_nodes,
            model,
            params,
        } => {
            w!(out, "q{} ({} {} {}", q(name), q(nc), q(nb), q(ne));
            for x in extra_nodes {
                w!(out, " {}", q(x));
            }
            w!(out, ") {}", q(model));
            inst_params(out, params, low);
        }
        Component::Mosfet { nb: None, .. } => {
            return Err(unsupported_comp(
                "MOSFET without a bulk node (VDMOS): terminal list unverified",
            ));
        }
        Component::Mosfet { extra_nodes, .. } if !extra_nodes.is_empty() => {
            return Err(unsupported_comp(
                "MOSFET with extra nodes (VDMOS thermal, SOI): terminal list unverified",
            ));
        }
        Component::Mosfet {
            name,
            nd,
            ng,
            ns,
            nb: Some(nb),
            model,
            params,
            ..
        } => {
            w!(
                out,
                "m{} ({} {} {} {}) {}",
                q(name),
                q(nd),
                q(ng),
                q(ns),
                q(nb),
                q(model)
            );
            inst_params(out, params, low);
        }
        Component::Jfet {
            name,
            nd,
            ng,
            ns,
            model,
            params,
        } => {
            w!(
                out,
                "j{} ({} {} {}) {}",
                q(name),
                q(nd),
                q(ng),
                q(ns),
                q(model)
            );
            inst_params(out, params, low);
        }
        Component::Mesfet {
            name,
            nd,
            ng,
            ns,
            model,
            params,
        } => {
            w!(
                out,
                "z{} ({} {} {}) {}",
                q(name),
                q(nd),
                q(ng),
                q(ns),
                q(model)
            );
            inst_params(out, params, low);
        }
        // SPICE text; native VACASK statements go in `Testbench::extra_lines`.
        Component::RawSpice { line } => {
            return Err(unsupported_comp(format!(
                "raw SPICE line '{}'",
                line.trim()
            )));
        }

        Component::Poly { name, .. } => {
            return Err(unsupported_comp(format!(
                "{}: POLY source (VACASK has no polynomial builtin)",
                Q(name, false)
            )));
        }
        Component::BehavioralVoltage { name, .. } | Component::BehavioralCurrent { name, .. } => {
            return Err(unsupported_comp(format!(
                "b{}: behavioural sources are an expression language VACASK does not share",
                q(name)
            )));
        }
        Component::VSwitch { name, .. } => {
            return Err(unsupported_comp(format!(
                "s{}: VACASK has no voltage-controlled switch",
                Q(name, false)
            )));
        }
        Component::ISwitch { name, .. } => {
            return Err(unsupported_comp(format!(
                "w{}: VACASK has no current-controlled switch",
                Q(name, false)
            )));
        }
        Component::TLine { name, .. } => {
            return Err(unsupported_comp(format!(
                "t{}: VACASK has no lossless transmission line",
                Q(name, false)
            )));
        }
        Component::Xspice { name, .. } => {
            return Err(unsupported_comp(format!(
                "a{}: XSPICE A-elements are ngspice-only",
                Q(name, false)
            )));
        }
    }
    Ok(())
}

// ---- private ----

/// A source's waveform parameters; `name` (the instance) only names it in an error.
fn wave(name: &dyn std::fmt::Display, wf: &IrWaveform, low: bool) -> Result<String, CodeGenError> {
    let n = |x: &IrNum| wave_num(x, low);
    Ok(match wf {
        IrWaveform::Sin {
            offset,
            amplitude,
            frequency,
            delay,
            damping,
            phase,
        } => {
            // VACASK accepts `sinephase` and does not apply it: the waveform
            // is byte-identical for 0, 45 and 90 degrees. `delay` and
            // `theta` beside it ARE honoured, which is what makes dropping
            // this one plausible and wrong — a 90-degree SIN would run and
            // sit 1.414 V away from what the deck asked for.
            if *phase != 0.0 || phase.num().is_none() {
                return Err(unsupported_comp(format!(
                    "{}: SIN phase ({} deg) is accepted but ignored by VACASK",
                    name,
                    n(phase)
                )));
            }
            format!(
                "type=\"sine\" sinedc={} ampl={} freq={} delay={} theta={}",
                n(offset),
                n(amplitude),
                n(frequency),
                n(delay),
                n(damping),
            )
        }
        IrWaveform::Pulse {
            initial,
            pulsed,
            delay,
            rise_time,
            fall_time,
            pulse_width,
            period,
        } => {
            // VACASK rejects a period that is not strictly greater than
            // rise+fall+width, while SPICE's own PER=PW default violates
            // exactly that. A zero period means "single pulse" to VACASK,
            // which is what SPICE means by PER >= TSTOP anyway.
            let period = match (
                period.num(),
                rise_time.num(),
                fall_time.num(),
                pulse_width.num(),
            ) {
                (Some(p), Some(r), Some(f), Some(w)) if p > 0.0 && p <= r + f + w => {
                    IrNum::Num(0.0)
                }
                // ponytail: an expression period passes as written; VACASK
                // itself rejects one that turns out too short.
                _ => period.clone(),
            };
            format!(
                "type=\"pulse\" val0={} val1={} delay={} rise={} fall={} width={} period={}",
                n(initial),
                n(pulsed),
                n(delay),
                n(rise_time),
                n(fall_time),
                n(pulse_width),
                n(&period),
            )
        }
        IrWaveform::Exp {
            initial,
            pulsed,
            rise_delay,
            rise_tau,
            fall_delay,
            fall_tau,
        } => {
            // SPICE's TD2 is an ABSOLUTE time ("start falling at t = TD2");
            // VACASK measures its `td2` from `delay`. Passed straight
            // through the fall starts TD1 late, which reads as an
            // integration disagreement rather than a different waveform.
            let (Some(fd), Some(rd)) = (fall_delay.num(), rise_delay.num()) else {
                return Err(unsupported_comp(format!(
                    "{}: EXP delays must be numbers (VACASK counts td2 from delay)",
                    name
                )));
            };
            if fd < rd {
                return Err(unsupported_comp(format!(
                    "{}: EXP fall delay ({}) precedes rise delay ({})",
                    name, fd, rd
                )));
            }
            format!(
                "type=\"exp\" val0={} val1={} delay={} tau1={} td2={} tau2={}",
                n(initial),
                n(pulsed),
                n(rise_delay),
                n(rise_tau),
                num(fd - rd),
                n(fall_tau),
            )
        }
        IrWaveform::Sffm {
            offset,
            amplitude,
            carrier_freq,
            modulation_index,
            signal_freq,
        } => {
            // The FM pedestal is `sinedc`, same as the sine's. `offset` is
            // also a real vsource parameter and is silently ignored on an
            // FM source, which is what makes the wrong spelling dangerous.
            format!(
                "type=\"fm\" sinedc={} ampl={} freq={} modindex={} modfreq={}",
                n(offset),
                n(amplitude),
                n(carrier_freq),
                n(modulation_index),
                n(signal_freq),
            )
        }
        IrWaveform::Am {
            amplitude,
            offset,
            modulating_freq,
            carrier_freq,
            delay,
        } => {
            format!(
                "type=\"am\" sinedc={} ampl={} freq={} modfreq={} delay={}",
                n(offset),
                n(amplitude),
                n(carrier_freq),
                n(modulating_freq),
                n(delay),
            )
        }
        // `type="pwl" wave=[t0, v0, ...]` is the documented spelling and
        // VACASK does parse the list (`print instance` reads it back
        // correctly), but the transient it then runs is identically zero
        // with uninitialised memory at t=0 — measured 6.9e-310 V on a deck
        // whose source should have ramped to 1 V. A named refusal beats a
        // plausible wrong answer.
        IrWaveform::Pwl { .. } => {
            return Err(unsupported_comp(format!(
                "{}: PWL sources are broken in VACASK (the transient comes back identically zero)",
                name
            )));
        }
        // VACASK's trannoise injects device noise; it has no noise or
        // random source (dev/research/ir-gaps.md §2).
        IrWaveform::TrNoise { .. } => {
            return Err(unsupported_comp(format!(
                "{name}: TRNOISE source (ngspice only)"
            )));
        }
        IrWaveform::TrRandom { .. } => {
            return Err(unsupported_comp(format!(
                "{name}: TRRANDOM source (ngspice only)"
            )));
        }
    })
}
