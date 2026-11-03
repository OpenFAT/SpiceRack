//! Sources: a name, two nets and a value or waveform → a `V`/`I`
//! component; and the table of waveform-source methods Subcircuit and
//! Testbench both expose. Cites ngspice's SIN, PULSE, PWL, EXP, SFFM, AM,
//! TRNOISE and TRRANDOM argument orders, by meaning.
//!
//! Contents:
//! - `macro_rules! pymethods_with_waves` (scoped to `python` by `#[macro_use]`)
//! - `pub(super) fn source(current, name, np, nm, value, ac_magnitude, ac_phase, waveform) -> Component`
//! - `pub(super) fn wave_source(current, name, np, nm, wf, dc, ac, ac_phase) -> Component`
//! - `pub(super) fn either(a: Option<f64>, b: Option<f64>, an: &str, bn: &str) -> PyResult<Option<f64>>`

use pyo3::prelude::*;

use super::args::num;
use crate::ir::{Component, IrValue, IrWaveform};

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
            #[pyo3(signature = (name, positive, negative, *, offset=IrNum::Num(0.0), amplitude=IrNum::Num(1.0), frequency=IrNum::Num(1000.0), delay=IrNum::Num(0.0), damping_factor=IrNum::Num(0.0), phase=IrNum::Num(0.0), dc_offset=None, ac_magnitude=None, dc=None, ac=None, ac_phase=None))]
            fn $sin(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                offset: IrNum, amplitude: IrNum, frequency: IrNum, delay: IrNum, damping_factor: IrNum, phase: IrNum, dc_offset: Option<f64>, ac_magnitude: Option<f64>, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Sin { offset, amplitude, frequency, delay, damping: damping_factor, phase };
                Self::place_wave(slf, $current, name, positive, negative, wf, either(dc, dc_offset, "dc", "dc_offset")?, either(ac, ac_magnitude, "ac", "ac_magnitude")?, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, initial_value=IrNum::Num(0.0), pulsed_value=IrNum::Num(1.0), delay=IrNum::Num(0.0), pulse_width=IrNum::Num(50e-9), period=IrNum::Num(100e-9), rise_time=IrNum::Num(1e-9), fall_time=IrNum::Num(1e-9), delay_time=None, dc_offset=None, dc=None, ac=None, ac_phase=None))]
            fn $pulse(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                initial_value: IrNum, pulsed_value: IrNum, delay: IrNum, pulse_width: IrNum, period: IrNum, rise_time: IrNum, fall_time: IrNum, delay_time: Option<IrNum>, dc_offset: Option<f64>, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Pulse { initial: initial_value, pulsed: pulsed_value, delay: delay_time.unwrap_or(delay), rise_time, fall_time, pulse_width, period };
                Self::place_wave(slf, $current, name, positive, negative, wf, either(dc, dc_offset, "dc", "dc_offset")?, ac, ac_phase);
                Ok(())
            }

            // `repeat_time`/`delay_time`: PySpice's names for ngspice's `r=`/`td=`.
            #[pyo3(signature = (name, positive, negative, *, values, repeat_time=None, delay_time=None, dc=None, ac=None, ac_phase=None))]
            fn $pwl(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                values: Vec<(IrNum, IrNum)>, repeat_time: Option<IrNum>, delay_time: Option<IrNum>, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Pwl { values, repeat: repeat_time, delay: delay_time };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, initial_value, pulsed_value, rise_time_constant, fall_delay_time, fall_time_constant, rise_delay_time=IrNum::Num(0.0), dc=None, ac=None, ac_phase=None))]
            fn $exp(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                initial_value: IrNum, pulsed_value: IrNum, rise_time_constant: IrNum, fall_delay_time: IrNum, fall_time_constant: IrNum, rise_delay_time: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Exp { initial: initial_value, pulsed: pulsed_value, rise_delay: rise_delay_time, rise_tau: rise_time_constant, fall_delay: fall_delay_time, fall_tau: fall_time_constant };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, offset, amplitude, carrier_frequency, modulation_index, signal_frequency, dc=None, ac=None, ac_phase=None))]
            fn $sffm(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                offset: IrNum, amplitude: IrNum, carrier_frequency: IrNum, modulation_index: IrNum, signal_frequency: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Sffm { offset, amplitude, carrier_freq: carrier_frequency, modulation_index, signal_freq: signal_frequency };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            #[pyo3(signature = (name, positive, negative, *, offset, amplitude, modulating_frequency, carrier_frequency, signal_delay=IrNum::Num(0.0), dc=None, ac=None, ac_phase=None))]
            fn $am(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                offset: IrNum, amplitude: IrNum, modulating_frequency: IrNum, carrier_frequency: IrNum, signal_delay: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::Am { amplitude, offset, modulating_freq: modulating_frequency, carrier_freq: carrier_frequency, delay: signal_delay };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            // ngspice TRNOISE(NA NT NALPHA NAMP RTSAM RTSCAPT RTSEMT), by meaning.
            #[pyo3(signature = (name, positive, negative, *, noise_rms, time_step, flicker_exponent=IrNum::Num(0.0), flicker_rms=IrNum::Num(0.0), rts_amplitude=IrNum::Num(0.0), rts_capture_time=IrNum::Num(0.0), rts_emission_time=IrNum::Num(0.0), dc=None, ac=None, ac_phase=None))]
            fn $noise(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                noise_rms: IrNum, time_step: IrNum, flicker_exponent: IrNum, flicker_rms: IrNum, rts_amplitude: IrNum, rts_capture_time: IrNum, rts_emission_time: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let wf = IrWaveform::TrNoise { na: noise_rms, nt: time_step, nalpha: flicker_exponent, namp: flicker_rms, rtsam: rts_amplitude, rtscapt: rts_capture_time, rtsemt: rts_emission_time };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }

            // ngspice TRRANDOM(TYPE TS TD PARAM1 PARAM2), PySpice's names;
            // `random_type` is uniform, gaussian, exponential or poisson.
            #[pyo3(signature = (name, positive, negative, *, random_type, duration, time_delay=IrNum::Num(0.0), parameter1=IrNum::Num(1.0), parameter2=IrNum::Num(0.0), dc=None, ac=None, ac_phase=None))]
            fn $random(
                slf: PyRefMut<'_, Self>, name: &str, positive: &str, negative: &str,
                random_type: &str, duration: IrNum, time_delay: IrNum, parameter1: IrNum, parameter2: IrNum, dc: Option<f64>, ac: Option<f64>, ac_phase: Option<f64>,
            ) -> PyResult<()> {
                let kind = random_type.parse().map_err(PyValueError::new_err)?;
                let wf = IrWaveform::TrRandom { kind, ts: duration, td: time_delay, p1: parameter1, p2: parameter2 };
                Self::place_wave(slf, $current, name, positive, negative, wf, dc, ac, ac_phase);
                Ok(())
            }
            )*
        }
    };
}

/// A `V` (or, with `current`, an `I`) source.
pub(super) fn source(
    current: bool,
    name: &str,
    np: String,
    nm: String,
    value: IrValue,
    ac_magnitude: Option<f64>,
    ac_phase: Option<f64>,
    waveform: Option<IrWaveform>,
) -> Component {
    let name = name.to_string();
    if current {
        Component::CurrentSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform,
        }
    } else {
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
    }
}

/// A waveform source: its DC value is `dc`, else the waveform's t=0 value.
pub(super) fn wave_source(
    current: bool,
    name: &str,
    np: String,
    nm: String,
    wf: IrWaveform,
    dc: Option<f64>,
    ac: Option<f64>,
    ac_phase: Option<f64>,
) -> Component {
    let value = dc.map_or_else(|| wf.at_zero(), num);
    source(current, name, np, nm, value, ac, ac_phase, Some(wf))
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
