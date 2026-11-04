//! IR numbers, values and source waveforms -> their Spectre spelling, and the
//! two refusals every Spectre writer returns. Sources as in `spectre/mod.rs`.
//!
//! Contents:
//! - `pub(super) fn spectre_value(v: &str) -> Cow<'_, str>`
//! - `pub(super) fn spectre_num(v: f64) -> String`
//! - `pub(super) fn emit_value(v: &IrValue) -> Cow<'_, str>`
//! - `pub(super) fn emit_waveform_params(wf: &IrWaveform) -> Result<String, CodeGenError>`
//! - `pub(super) fn unsupported_component(what: &str) -> CodeGenError`
//! - `pub(super) fn unsupported_analysis(what: &str) -> CodeGenError`

use crate::ir::*;
use std::borrow::Cow;

use crate::codegen::{CodeGenError, format_spice_number};

/// A SPICE number in a parameter value, spelled without its scale factor:
/// Spectre reads `M` as mega where SPICE reads milli (`IKF=30M` is 30e-3),
/// so `30M` must reach Spectre as `0.03`. Anything else (an expression, a
/// name) passes without its SPICE delimiters (`'w*2'` is `w*2`).
pub(super) fn spectre_value(v: &str) -> Cow<'_, str> {
    let v = crate::codegen::undelimited(v);
    // The value, written in Spectre's own suffixes: VACASK's spelling of
    // mega is `meg`, which Spectre reads as milli with a unit tail.
    match crate::codegen::vacask::parse_spice_num(v) {
        Some((x, _)) => spectre_num(x).into(),
        None => v.into(),
    }
}

/// Format a number for the Spectre native language.
///
/// `format_spice_number` emits SPICE scale factors. Per `[KUN]` Table B.1 vs
/// B.2 the sets differ: Spectre uses SI factors (`T G M K k m u n p f a`) while
/// SPICE uses `t g meg k m u n p f`. `f p n u m k` mean the same thing in both,
/// so only the three large SPICE-only factors need rewriting. Cadence's own
/// examples use `=1k`, `=100k`, `=1.2K`, `=900M`, `=10M`, `=1G`, `=2.45G`
/// (`[REF]`/`[CMP]`), never `meg`/`g`/`t`.
pub(super) fn spectre_num(v: f64) -> String {
    let s = format_spice_number(v);
    if let Some(head) = s.strip_suffix("meg") {
        format!("{head}M")
    } else if let Some(head) = s.strip_suffix('g') {
        format!("{head}G")
    } else if let Some(head) = s.strip_suffix('t') {
        format!("{head}T")
    } else {
        s
    }
}

/// An IR value: a number in Spectre's scale factors, an expression or raw
/// text as written.
pub(super) fn emit_value(v: &IrValue) -> Cow<'_, str> {
    match v {
        IrValue::Numeric { value } => spectre_num(*value).into(),
        IrValue::Expression { expr } => expr.into(),
        IrValue::Raw { text } => text.into(),
    }
}

/// `[CMP]` p.683-685 (`vsource`) / p.380-382 (`isource`): the waveform is
/// selected with `type=`, whose documented values are exactly
/// `dc, pulse, pwl, sine, exp`.
pub(super) fn emit_waveform_params(wf: &IrWaveform) -> Result<String, CodeGenError> {
    let sn = |x: &IrNum| emit_value(&x.into()).into_owned();
    let s = match wf {
        // `[CMP]` p.684: sinedc, ampl, freq, sinephase, damp — and the
        // delay is the *general* waveform parameter `delay` (p.683 #4),
        // not a sine-specific one.
        IrWaveform::Sin {
            offset,
            amplitude,
            frequency,
            delay,
            damping,
            phase,
        } => {
            let mut s = format!(
                "type=sine sinedc={} ampl={} freq={}",
                sn(offset),
                sn(amplitude),
                sn(frequency),
            );
            if *delay != 0.0 {
                w!(s, " delay={}", sn(delay));
            }
            if *damping != 0.0 {
                w!(s, " damp={}", sn(damping));
            }
            if *phase != 0.0 {
                w!(s, " sinephase={}", sn(phase));
            }
            s
        }
        // `[CMP]` p.683: verbatim sample statement
        // `vpulse1 (1 0) vsource type=pulse val0=0 val1=5 period=100n
        //  rise=10n fall=10n width=40n`
        IrWaveform::Pulse {
            initial,
            pulsed,
            delay,
            rise_time,
            fall_time,
            pulse_width,
            period,
        } => {
            format!(
                "type=pulse val0={} val1={} delay={} rise={} fall={} width={} period={}",
                sn(initial),
                sn(pulsed),
                sn(delay),
                sn(rise_time),
                sn(fall_time),
                sn(pulse_width),
                sn(period),
            )
        }
        // `[CMP]` p.683: `vpwl1 (1 0) vsource type=pwl
        //  wave=[1n 0 1.1n 2 1.5n 0.5 2n 3 5n 5]`
        // `delay` (#4) is ngspice's `td=`. `pwlperiod` (#17) repeats from
        // time 0 with a `twidth` ramp (#18, p.686), which is ngspice's `r=0`
        // only where `spectre_pwl_period` says so; any other repeat is
        // refused rather than half-mapped.
        IrWaveform::Pwl {
            values,
            repeat,
            delay,
        } => {
            let period = match repeat {
                Some(r) if *r == 0.0 => Some(
                    IrWaveform::spectre_pwl_period(values)
                        .ok_or_else(|| unsupported_component("PWL r=0 (no exact pwlperiod)"))?,
                ),
                Some(_) => {
                    return Err(unsupported_component(
                        "PWL r= other than 0 (pwlperiod repeats from 0)",
                    ));
                }
                None => None,
            };
            let mut s = String::from("type=pwl wave=[");
            for (i, (t, v)) in values.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                w!(s, "{} {}", sn(t), sn(v));
            }
            s.push(']');
            if let Some(d) = delay {
                w!(s, " delay={}", sn(d));
            }
            if let Some(p) = period {
                w!(s, " pwlperiod={}", sn(&IrNum::Num(p)));
            }
            s
        }
        // `[CMP]` p.685 "Exponential waveform parameters": td1 tau1 td2 tau2,
        // with val0/val1 shared with the pulse waveform (p.683 #5/#6).
        IrWaveform::Exp {
            initial,
            pulsed,
            rise_delay,
            rise_tau,
            fall_delay,
            fall_tau,
        } => {
            format!(
                "type=exp val0={} val1={} td1={} tau1={} td2={} tau2={}",
                sn(initial),
                sn(pulsed),
                sn(rise_delay),
                sn(rise_tau),
                sn(fall_delay),
                sn(fall_tau),
            )
        }
        // SPICE SFFM is a sine carrier with sinusoidal frequency modulation.
        // `[CMP]` p.684-685 gives Spectre the identically-named knobs
        // fmmodindex ("FM index of modulation") and fmmodfreq ("FM
        // modulation frequency") on `type=sine`. The parameter names are
        // documented; the SPICE->Spectre mapping of the *values* is
        // inferred from the standard definition of an FM index and is not
        // stated in any Cadence document.
        IrWaveform::Sffm {
            offset,
            amplitude,
            carrier_freq,
            modulation_index,
            signal_freq,
        } => {
            format!(
                "type=sine sinedc={} ampl={} freq={} fmmodindex={} fmmodfreq={}",
                sn(offset),
                sn(amplitude),
                sn(carrier_freq),
                sn(modulation_index),
                sn(signal_freq),
            )
        }
        // Spectre has ammodindex/ammodfreq/ammodphase (`[CMP]` p.685) but
        // publishes no waveform equation, and SPICE's own AM() definition
        // varies between simulators. Refusing beats guessing the mapping.
        // Spectre's `noisefile`/`noisevec` are small-signal spot noise, not a
        // transient source (Spectre Reference 5.0 pp.685-686).
        IrWaveform::TrNoise { .. } => {
            return Err(unsupported_component("TRNOISE source (ngspice only)"));
        }
        IrWaveform::TrRandom { .. } => {
            return Err(unsupported_component("TRRANDOM source (ngspice only)"));
        }
        IrWaveform::Am { .. } => {
            return Err(unsupported_component(
                "AM source (Spectre documents ammodindex/ammodfreq but no waveform equation; \
                 the SPICE AM() -> Spectre mapping cannot be established)",
            ));
        }
    };
    Ok(s)
}

/// The refusal of a component (or model, source, line) Spectre cannot spell.
pub(super) fn unsupported_component(what: &str) -> CodeGenError {
    CodeGenError::UnsupportedComponent {
        backend: "spectre".into(),
        component: what.into(),
    }
}

/// The refusal of an analysis (or sweep) Spectre cannot spell.
pub(super) fn unsupported_analysis(what: &str) -> CodeGenError {
    CodeGenError::UnsupportedAnalysis {
        backend: "spectre".into(),
        analysis: what.into(),
    }
}
