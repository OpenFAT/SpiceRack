//! Sources: a name, two nets and a value or waveform → a `V`/`I` card;
//! and the table of waveform-source methods Subcircuit and Testbench both
//! expose. Cites ngspice manual §4.1 (SIN, PULSE, PWL with `r=`/`td=`
//! §4.1.4, EXP, SFFM, AM, TRNOISE §4.1.7, TRRANDOM §4.1.8), by meaning.
//!
//! Contents:
//! - `macro_rules! pymethods_with_waves` (scoped to `python` by `#[macro_use]`)
//! - `pub(super) struct Wave`, `Wave::new(form: &str, fields: &[&Num], zero: Value) -> Wave`,
//!   `Wave::pwl(values: &[(Num, Num)], repeat: Option<&Num>, delay: Option<&Num>) -> Wave`
//! - `pub(super) fn source(current, name, np, nm, value, ac, ac_phase, port, wave) -> Card`
//! - `pub(super) fn wave_source(current, name, np, nm, wave, dc, ac, ac_phase) -> Card`
//! - `pub(super) fn either(a: Option<f64>, b: Option<f64>, an: &str, bn: &str) -> PyResult<Option<f64>>`

use pyo3::prelude::*;

use super::args::Num;
use super::model::{Card, Value, dec};

/// A waveform's text (`SIN(0 1 1000 0 0 0)`) and the DC value SPICE takes
/// for a card that gives none: the value at t=0 (AM starts at 0), and 0 for
/// TRNOISE and TRRANDOM, which add to it.
pub(super) struct Wave {
    text: String,
    zero: Value,
}

/// `#[pymethods] impl $ty { ... }` with the sixteen waveform sources (eight
/// waveforms, each as a V and an I) appended: one table for Subcircuit and
/// Testbench, which each add the built source with their own `place_wave`.
/// Every source takes `dc=` (else its waveform's t=0 value, as SPICE reads a
/// card with none), `ac=`, `ac_phase=`, and PySpice's spellings where they
/// exist. One block, not two: pyo3 takes one `#[pymethods]` per class
/// without its `multiple-pymethods` feature. The caller writes the
/// `#[pymethods]` so the names pyo3 generates for `**kwargs` share the
/// body's hygiene.
macro_rules! pymethods_with_waves {
    ($hash:tt $attr:tt impl $ty:ty { $($body:tt)* }) => {
        pymethods_with_waves!(@emit $hash $attr $ty { $($body)* }
            [SinusoidalVoltageSource PulseVoltageSource PieceWiseLinearVoltageSource ExponentialVoltageSource SingleFrequencyFMVoltageSource AmplitudeModulatedVoltageSource TransientNoiseVoltageSource RandomVoltageSource false]
            [SinusoidalCurrentSource PulseCurrentSource PieceWiseLinearCurrentSource ExponentialCurrentSource SingleFrequencyFMCurrentSource AmplitudeModulatedCurrentSource TransientNoiseCurrentSource RandomCurrentSource true]);
    };
    (@emit $hash:tt $attr:tt $ty:ty { $($body:tt)* } $([$sin:ident $pulse:ident $pwl:ident $exp:ident $sffm:ident $am:ident $noise:ident $random:ident $current:literal])*) => {
        $hash $attr
        impl $ty {
            $($body)*
            $(
            #[pyo3(signature = (name, positive, negative, *, offset=Num::of(0.0), amplitude=Num::of(1.0), frequency=Num::of(1000.0), delay=Num::of(0.0), damping_factor=Num::of(0.0), phase=Num::of(0.0), dc_offset=None, ac_magnitude=None, dc=None, ac=None, ac_phase=None))]
            fn $sin(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                offset: Num, amplitude: Num, frequency: Num, delay: Num, damping_factor: Num, phase: Num, dc_offset: Option<f64>, ac_magnitude: Option<f64>, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = Wave::new("SIN", &[&offset, &amplitude, &frequency, &delay, &damping_factor, &phase], offset.value());
                Self::place_wave(slf, $current, name, positive, negative, wf, either(dc, dc_offset, "dc", "dc_offset")?, either(ac, ac_magnitude, "ac", "ac_magnitude")?, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, initial_value=Num::of(0.0), pulsed_value=Num::of(1.0), delay=Num::of(0.0), pulse_width=Num::of(50e-9), period=Num::of(100e-9), rise_time=Num::of(1e-9), fall_time=Num::of(1e-9), delay_time=None, dc_offset=None, dc=None, ac=None, ac_phase=None))]
            fn $pulse(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                initial_value: Num, pulsed_value: Num, delay: Num, pulse_width: Num, period: Num, rise_time: Num, fall_time: Num, delay_time: Option<Num>, dc_offset: Option<f64>, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let delay = delay_time.unwrap_or(delay);
                let wf = Wave::new("PULSE", &[&initial_value, &pulsed_value, &delay, &rise_time, &fall_time, &pulse_width, &period], initial_value.value());
                Self::place_wave(slf, $current, name, positive, negative, wf, either(dc, dc_offset, "dc", "dc_offset")?, ac, ac_phase);
                Ok(())
            }

            // `repeat_time`/`delay_time`: PySpice's names for ngspice's `r=`/`td=`.
            #[pyo3(signature = (name, positive, negative, *, values, repeat_time=None, delay_time=None, dc=None, ac=None, ac_phase=None))]
            fn $pwl(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                values: Vec<(Num, Num)>, repeat_time: Option<Num>, delay_time: Option<Num>, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = Wave::pwl(&values, repeat_time.as_ref(), delay_time.as_ref());
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, initial_value, pulsed_value, rise_time_constant, fall_delay_time, fall_time_constant, rise_delay_time=Num::of(0.0), dc=None, ac=None, ac_phase=None))]
            fn $exp(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                initial_value: Num, pulsed_value: Num, rise_time_constant: Num, fall_delay_time: Num, fall_time_constant: Num, rise_delay_time: Num, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = Wave::new("EXP", &[&initial_value, &pulsed_value, &rise_delay_time, &rise_time_constant, &fall_delay_time, &fall_time_constant], initial_value.value());
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, offset, amplitude, carrier_frequency, modulation_index, signal_frequency, dc=None, ac=None, ac_phase=None))]
            fn $sffm(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                offset: Num, amplitude: Num, carrier_frequency: Num, modulation_index: Num, signal_frequency: Num, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = Wave::new("SFFM", &[&offset, &amplitude, &carrier_frequency, &modulation_index, &signal_frequency], offset.value());
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, offset, amplitude, modulating_frequency, carrier_frequency, signal_delay=Num::of(0.0), dc=None, ac=None, ac_phase=None))]
            fn $am(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                offset: Num, amplitude: Num, modulating_frequency: Num, carrier_frequency: Num, signal_delay: Num, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = Wave::new("AM", &[&amplitude, &offset, &modulating_frequency, &carrier_frequency, &signal_delay], Value::Num(0.0));
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            // ngspice TRNOISE(NA NT NALPHA NAMP RTSAM RTSCAPT RTSEMT), by meaning.
            #[pyo3(signature = (name, positive, negative, *, noise_rms, time_step, flicker_exponent=Num::of(0.0), flicker_rms=Num::of(0.0), rts_amplitude=Num::of(0.0), rts_capture_time=Num::of(0.0), rts_emission_time=Num::of(0.0), dc=None, ac=None, ac_phase=None))]
            fn $noise(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                noise_rms: Num, time_step: Num, flicker_exponent: Num, flicker_rms: Num, rts_amplitude: Num, rts_capture_time: Num, rts_emission_time: Num, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                // The RTS fields are left off when all zero, as the manual's white
                // and 1/f examples write the card.
                let rts = [&rts_amplitude, &rts_capture_time, &rts_emission_time];
                let n = if rts.iter().all(|f| f.zero()) { 4 } else { 7 };
                let wf = Wave::new("TRNOISE", &[&noise_rms, &time_step, &flicker_exponent, &flicker_rms, rts[0], rts[1], rts[2]][..n], Value::Num(0.0));
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            // ngspice TRRANDOM(TYPE TS TD PARAM1 PARAM2), PySpice's names;
            // `random_type` is uniform, gaussian, exponential or poisson.
            #[pyo3(signature = (name, positive, negative, *, random_type, duration, time_delay=Num::of(0.0), parameter1=Num::of(1.0), parameter2=Num::of(0.0), dc=None, ac=None, ac_phase=None))]
            fn $random(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                random_type: &str, duration: Num, time_delay: Num, parameter1: Num, parameter2: Num, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let kind: super::analysis::RandomKind = random_type.parse().map_err(PyValueError::new_err)?;
                let ty = Num::of((kind as u8 + 1).into());
                let wf = Wave::new("TRRANDOM", &[&ty, &duration, &time_delay, &parameter1, &parameter2], Value::Num(0.0));
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }
            )*
        }
    };
}

impl Wave {
    /// `FORM(f0 f1 ...)`.
    pub(super) fn new(form: &str, fields: &[&Num], zero: Value) -> Wave {
        let f: Vec<String> = fields.iter().map(|n| n.text()).collect();
        Wave {
            text: format!("{form}({})", f.join(" ")),
            zero,
        }
    }

    /// `PWL(t0 v0 t1 v1 ...) [r=…] [td=…]`.
    pub(super) fn pwl(values: &[(Num, Num)], repeat: Option<&Num>, delay: Option<&Num>) -> Wave {
        let f: Vec<String> = values
            .iter()
            .map(|(t, v)| format!("{} {}", t.text(), v.text()))
            .collect();
        let mut text = format!("PWL({})", f.join(" "));
        if let Some(r) = repeat {
            text.push_str(&format!(" r={}", r.text()));
        }
        if let Some(td) = delay {
            text.push_str(&format!(" td={}", td.text()));
        }
        let zero = values.first().map_or(Value::Num(0.0), |p| p.1.value());
        Wave { text, zero }
    }
}

/// A `V` (or, with `current`, an `I`) card: `V<name> np nm value [AC mag
/// [phase]] [portnum n z0 z] [wave]`.
pub(super) fn source(
    current: bool,
    name: &str,
    np: String,
    nm: String,
    value: Value,
    ac: Option<f64>,
    ac_phase: Option<f64>,
    port: Option<(u32, f64)>,
    wave: Option<&Wave>,
) -> Card {
    let letter = if current { 'I' } else { 'V' };
    let mut tail = String::new();
    if let Some(mag) = ac {
        tail.push_str(&format!(" AC {}", dec(mag)));
        if let Some(phase) = ac_phase {
            tail.push_str(&format!(" {}", dec(phase)));
        }
    }
    if let Some((n, z0)) = port {
        tail.push_str(&format!(" portnum {n} z0 {}", dec(z0)));
    }
    if let Some(w) = wave {
        tail.push(' ');
        tail.push_str(&w.text);
    }
    Card::source(name, format!("{letter}{name} {np} {nm}"), &value, tail)
}

/// A waveform source: its DC value is `dc`, else the waveform's t=0 value.
pub(super) fn wave_source(
    current: bool,
    name: &str,
    np: String,
    nm: String,
    wave: Wave,
    dc: Option<f64>,
    ac: Option<f64>,
    ac_phase: Option<f64>,
) -> Card {
    let value = dc.map_or_else(|| wave.zero.clone(), Value::Num);
    source(
        current,
        name,
        np,
        nm,
        value,
        ac,
        ac_phase,
        None,
        Some(&wave),
    )
}

/// One of two spellings of the same argument (`dc`/`dc_offset`), never both.
pub(super) fn either(a: Option<f64>, b: Option<f64>, an: &str, bn: &str) -> PyResult<Option<f64>> {
    match (a, b) {
        (Some(_), Some(_)) => Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "pass {an} or {bn}, not both"
        ))),
        _ => Ok(a.or(b)),
    }
}
