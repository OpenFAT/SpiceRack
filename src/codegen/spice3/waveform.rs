//! A source's `IrWaveform` -> its SPICE3-family waveform spelling
//! (`SIN(...)`, `PULSE(...)`, ...), or the refusal of one a dialect lacks.
//!
//! Contents:
//! - `pub(super) fn refuse_wave(d: Dialect, letter: char, name: &str, wf: &IrWaveform) -> Result<(), CodeGenError>`
//! - `pub(super) fn waveform(out: &mut String, d: Dialect, x: bool, wf: &IrWaveform)`

use super::values::{dec, value};
use crate::codegen::{CodeGenError, Dialect};
use crate::ir::*;

/// Refuses a waveform `d` has no card for, before anything is written.
/// PWL `r=`/`td=`: ngspice reads them on a V source only (manual §4.1.4;
/// ngspice 45 answers an I source with "unknown parameter (r)"), ESPice on
/// both; no other dialect here has been shown to repeat the same way.
pub(super) fn refuse_wave(
    d: Dialect,
    letter: char,
    name: &str,
    wf: &IrWaveform,
) -> Result<(), CodeGenError> {
    let why = match wf {
        IrWaveform::Pwl { repeat, delay, .. } if repeat.is_some() || delay.is_some() => {
            match (d, letter) {
                (Dialect::Espice, _) | (Dialect::Ngspice, 'V') => return Ok(()),
                _ => "PWL r=/td= (repeat, delay)",
            }
        }
        // ESPice 0.3.0 refuses both cards ("the trnoise source waveform is
        // not supported"); HSPICE, LTspice and Xyce have no source form.
        IrWaveform::TrNoise { .. } if d != Dialect::Ngspice => "TRNOISE (ngspice only)",
        IrWaveform::TrRandom { .. } if d != Dialect::Ngspice => "TRRANDOM (ngspice only)",
        _ => return Ok(()),
    };
    Err(CodeGenError::UnsupportedComponent {
        backend: d.name().into(),
        component: format!("{letter}{name}: {why}"),
    })
}

/// `wf`'s spelling, written onto `out`; [`refuse_wave`] has already passed it.
pub(super) fn waveform(out: &mut String, d: Dialect, x: bool, wf: &IrWaveform) {
    let w = |n: &IrNum| match n {
        IrNum::Num(v) => dec(d, x, *v),
        n => value(d, x, &n.into()).into_owned(),
    };
    match wf {
        IrWaveform::Sin {
            offset,
            amplitude,
            frequency,
            delay,
            damping,
            phase,
        } => {
            w!(
                out,
                "SIN({} {} {} {} {} {})",
                w(offset),
                w(amplitude),
                w(frequency),
                w(delay),
                w(damping),
                w(phase)
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
            w!(
                out,
                "PULSE({} {} {} {} {} {} {})",
                w(initial),
                w(pulsed),
                w(delay),
                w(rise_time),
                w(fall_time),
                w(pulse_width),
                w(period)
            )
        }
        IrWaveform::Pwl {
            values,
            repeat,
            delay,
        } => {
            out.push_str("PWL(");
            for (i, (t, v)) in values.iter().enumerate() {
                w!(out, "{}{} {}", if i == 0 { "" } else { " " }, w(t), w(v));
            }
            out.push(')');
            if let Some(r) = repeat {
                w!(out, " r={}", w(r));
            }
            if let Some(td) = delay {
                w!(out, " td={}", w(td));
            }
        }
        IrWaveform::Exp {
            initial,
            pulsed,
            rise_delay,
            rise_tau,
            fall_delay,
            fall_tau,
        } => {
            w!(
                out,
                "EXP({} {} {} {} {} {})",
                w(initial),
                w(pulsed),
                w(rise_delay),
                w(rise_tau),
                w(fall_delay),
                w(fall_tau)
            )
        }
        IrWaveform::Sffm {
            offset,
            amplitude,
            carrier_freq,
            modulation_index,
            signal_freq,
        } => {
            w!(
                out,
                "SFFM({} {} {} {} {})",
                w(offset),
                w(amplitude),
                w(carrier_freq),
                w(modulation_index),
                w(signal_freq)
            )
        }
        IrWaveform::Am {
            amplitude,
            offset,
            modulating_freq,
            carrier_freq,
            delay,
        } => {
            w!(
                out,
                "AM({} {} {} {} {})",
                w(amplitude),
                w(offset),
                w(modulating_freq),
                w(carrier_freq),
                w(delay)
            )
        }
        // The RTS fields are left off when all zero, as the manual's
        // white and 1/f examples write the card.
        IrWaveform::TrNoise {
            na,
            nt,
            nalpha,
            namp,
            rtsam,
            rtscapt,
            rtsemt,
        } => {
            w!(out, "TRNOISE({} {} {} {}", w(na), w(nt), w(nalpha), w(namp));
            if !(*rtsam == 0.0 && *rtscapt == 0.0 && *rtsemt == 0.0) {
                w!(out, " {} {} {}", w(rtsam), w(rtscapt), w(rtsemt));
            }
            out.push(')');
        }
        IrWaveform::TrRandom {
            kind,
            ts,
            td,
            p1,
            p2,
        } => {
            w!(
                out,
                "TRRANDOM({} {} {} {} {})",
                *kind as u8,
                w(ts),
                w(td),
                w(p1),
                w(p2)
            )
        }
    }
}
