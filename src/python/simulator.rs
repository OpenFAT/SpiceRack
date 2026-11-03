//! CircuitSimulator: a `CircuitIR` plus a backend choice; each analysis
//! call runs a copy of the deck holding only that analysis and wraps the
//! `RawData` in its result class. Spectre-only analyses come from
//! `spectre.rs` through `pymethods_with_spectre!`.
//!
//! Contents:
//! - `pub(super) struct PySimulator` (Python `CircuitSimulator`)
//! - `PySimulator::tb(&mut self) -> &mut ir::Testbench`, `PySimulator::stim(&mut self, c: Component)`,
//!   `PySimulator::deck_as_queued(&self) -> CircuitIR`

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

// `PssBase`, `PyRawData`: names `pymethods_with_spectre!` resolves here.
use super::args::{kw_params, kw_volts};
use super::card::{self, dc_sweeps};
use super::deck::{issues, run_deck};
use super::results::*;
use crate::ir::{self, Analysis, CircuitIR, Component, PssBase};
use crate::result::{RawData, Results};

#[pyclass(name = "CircuitSimulator", subclass)]
pub(super) struct PySimulator {
    /// `testbench` holds the config; each analysis call runs a copy with only its analysis.
    pub(super) ir: CircuitIR,
    pub(super) backend: Option<String>,
    pub(super) save_currents: bool,
}

impl PySimulator {
    /// The deck's testbench, created empty on first use.
    pub(super) fn tb(&mut self) -> &mut ir::Testbench {
        self.ir.testbench.get_or_insert_with(Default::default)
    }

    /// Appends `c` to the testbench stimulus, outside the DUT.
    pub(super) fn stim(&mut self, c: Component) {
        self.tb().stimulus.push(c);
    }

    /// A copy of the deck with every queued analysis.
    pub(super) fn deck_as_queued(&self) -> CircuitIR {
        let mut ir = self.ir.clone();
        if self.save_currents {
            ir.testbench.get_or_insert_with(Default::default).saves =
                vec!["all".into(), "@all[i]".into()];
        }
        ir
    }
}

// ---- private ----

pymethods_with_spectre! {
#[pymethods]
impl PySimulator {
    // ── Config ──

    #[pyo3(signature = (**kwargs))]
    fn options(&mut self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.tb().options.portable.extend(kw_params(kwargs)?);
        Ok(())
    }

    #[pyo3(signature = (**kwargs))]
    fn initial_condition(&mut self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.tb().initial_conditions.extend(kw_volts(kwargs)?);
        Ok(())
    }

    #[pyo3(signature = (**kwargs))]
    fn node_set(&mut self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        self.tb().node_sets.extend(kw_volts(kwargs)?);
        Ok(())
    }

    #[pyo3(signature = (*args))]
    fn save(&mut self, args: Vec<String>) {
        self.tb().saves.extend(args);
    }

    #[pyo3(signature = (*args))]
    fn measure(&mut self, args: Vec<String>) {
        self.tb().measures.push(args.join(" "));
    }

    #[setter]
    fn set_save_currents(&mut self, v: bool) {
        self.save_currents = v;
    }

    #[getter]
    fn get_save_currents(&self) -> bool {
        self.save_currents
    }

    #[setter]
    fn set_temperature(&mut self, temp: f64) {
        self.tb().temperature = Some(temp);
    }

    #[setter]
    fn set_nominal_temperature(&mut self, temp: f64) {
        self.tb().nominal_temperature = Some(temp);
    }

    /// ngspice's compatibility mode (`"hsa"` for sky130), set before the deck
    /// is read; other simulators ignore it. Lowercase letters only.
    #[setter]
    fn set_ngbehavior(&mut self, mode: Option<String>) -> PyResult<()> {
        if let Some(m) = &mode
            && (m.is_empty() || !m.bytes().all(|b| b.is_ascii_lowercase()))
        {
            return Err(PyValueError::new_err(format!(
                "ngbehavior must be lowercase letters (\"hsa\", \"ps\", \"lt\", ...), got {m:?}"
            )));
        }
        self.tb().ngbehavior = mode;
        Ok(())
    }

    #[getter]
    fn get_ngbehavior(&self) -> Option<String> {
        self.ir
            .testbench
            .as_ref()
            .and_then(|tb| tb.ngbehavior.clone())
    }

    #[pyo3(signature = (param, start, stop, step))]
    fn step(&mut self, param: &str, start: f64, stop: f64, step: f64) {
        self.tb().step_params.push(ir::StepParam {
            param: param.to_string(),
            start,
            stop,
            step,
            sweep_type: None,
        });
    }

    #[pyo3(signature = (param, start, stop, step, sweep_type))]
    fn step_sweep(
        &mut self,
        param: &str,
        start: f64,
        stop: f64,
        step: f64,
        sweep_type: &str,
    ) -> PyResult<()> {
        let sweep_type = Some(sweep_type.parse().map_err(PyValueError::new_err)?);
        self.tb().step_params.push(ir::StepParam {
            param: param.to_string(),
            start,
            stop,
            step,
            sweep_type,
        });
        Ok(())
    }

    // ── Analyses ──

    fn operating_point(&self) -> PyResult<PyOperatingPoint> {
        Ok(PyOperatingPoint {
            inner: Results::from_raw(self.run(Analysis::Op)?),
        })
    }

    #[pyo3(signature = (**kwargs))]
    fn dc(&self, kwargs: Option<Bound<'_, PyDict>>) -> PyResult<PyDcAnalysis> {
        Ok(PyDcAnalysis {
            inner: Results::from_raw(self.run(dc_sweeps(kwargs)?)?),
        })
    }

    #[pyo3(signature = (variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e9))]
    fn ac(
        &self,
        variation: &str,
        number_of_points: u32,
        start_frequency: f64,
        stop_frequency: f64,
    ) -> PyResult<PyAcAnalysis> {
        let a = card::ac(variation, number_of_points, start_frequency, stop_frequency)?;
        Ok(PyAcAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    #[pyo3(signature = (step_time, end_time, start_time=None, max_time=None, use_initial_condition=false))]
    fn transient(
        &self,
        step_time: f64,
        end_time: f64,
        start_time: Option<f64>,
        max_time: Option<f64>,
        use_initial_condition: bool,
    ) -> PyResult<PyTransientAnalysis> {
        let a = card::transient(
            step_time,
            end_time,
            start_time,
            max_time,
            use_initial_condition,
        );
        Ok(PyTransientAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    #[pyo3(signature = (output_node, ref_node, src, variation="dec", points=10, start_frequency=1e3, stop_frequency=1e8, points_per_summary=None))]
    fn noise(
        &self,
        output_node: &str,
        ref_node: &str,
        src: &str,
        variation: &str,
        points: u32,
        start_frequency: f64,
        stop_frequency: f64,
        points_per_summary: Option<u32>,
    ) -> PyResult<PyNoiseAnalysis> {
        let a = card::noise(
            output_node,
            ref_node,
            src,
            variation,
            points,
            start_frequency,
            stop_frequency,
            points_per_summary,
        )?;
        Ok(PyNoiseAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    #[pyo3(signature = (outvar, insrc))]
    fn transfer_function(&self, outvar: &str, insrc: &str) -> PyResult<PyTransferFunctionAnalysis> {
        Ok(PyTransferFunctionAnalysis {
            inner: Results::from_raw(self.run(card::tf(outvar, insrc))?),
        })
    }

    #[pyo3(signature = (outvar, insrc))]
    fn tf(&self, outvar: &str, insrc: &str) -> PyResult<PyTransferFunctionAnalysis> {
        self.transfer_function(outvar, insrc)
    }

    #[pyo3(signature = (output_variable))]
    fn dc_sensitivity(&self, output_variable: &str) -> PyResult<PySensitivityAnalysis> {
        let a = card::sensitivity(output_variable, None);
        Ok(PySensitivityAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    #[pyo3(signature = (output_variable, variation="dec", number_of_points=10, start_frequency=100.0, stop_frequency=1e5))]
    fn ac_sensitivity(
        &self,
        output_variable: &str,
        variation: &str,
        number_of_points: u32,
        start_frequency: f64,
        stop_frequency: f64,
    ) -> PyResult<PySensitivityAnalysis> {
        let a = card::sensitivity(
            output_variable,
            Some(card::sweep(
                variation,
                number_of_points,
                start_frequency,
                stop_frequency,
            )?),
        );
        Ok(PySensitivityAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    #[pyo3(signature = (node1, node2, node3, node4, tf_type, pz_type))]
    fn polezero(
        &self,
        node1: &str,
        node2: &str,
        node3: &str,
        node4: &str,
        tf_type: &str,
        pz_type: &str,
    ) -> PyResult<PyPoleZeroAnalysis> {
        let a = card::polezero([node1, node2, node3, node4], tf_type, pz_type)?;
        Ok(PyPoleZeroAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    #[pyo3(signature = (variation="dec", points=10, start_frequency=100.0, stop_frequency=1e8, f2overf1=None))]
    fn distortion(
        &self,
        variation: &str,
        points: u32,
        start_frequency: f64,
        stop_frequency: f64,
        f2overf1: Option<f64>,
    ) -> PyResult<PyDistortionAnalysis> {
        let a = card::distortion(variation, points, start_frequency, stop_frequency, f2overf1)?;
        Ok(PyDistortionAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    /// `sc_iter`, `steady_coeff`, `uic`: ngspice's shooting controls
    /// (`sciter`, `steadycoeff`, `uic`; defaults 50 and 1e-3 when unset).
    #[pyo3(signature = (fundamental_frequency, stabilization_time, observe_node, points_per_period=128, harmonics=10, sc_iter=None, steady_coeff=None, uic=false))]
    fn pss(
        &self,
        fundamental_frequency: f64,
        stabilization_time: f64,
        observe_node: &str,
        points_per_period: u32,
        harmonics: u32,
        sc_iter: Option<u32>,
        steady_coeff: Option<f64>,
        uic: bool,
    ) -> PyResult<PyPssAnalysis> {
        let a = Analysis::Pss {
            fundamental: fundamental_frequency,
            stabilization: stabilization_time,
            observe_node: observe_node.into(),
            points_per_period,
            harmonics,
            sc_iter,
            steady_coeff,
            uic,
        };
        Ok(PyPssAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    #[pyo3(signature = (variation="dec", number_of_points=10, start_frequency=1e6, stop_frequency=1e10))]
    fn s_param(
        &self,
        variation: &str,
        number_of_points: u32,
        start_frequency: f64,
        stop_frequency: f64,
    ) -> PyResult<PySParamAnalysis> {
        let a = Analysis::SPar {
            sweep: card::sweep(variation, number_of_points, start_frequency, stop_frequency)?,
        };
        Ok(PySParamAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    /// LTspice `.net` network-parameter analysis over an AC sweep.
    #[pyo3(signature = (output_current, input_source, z_in=50.0, z_out=50.0, variation="dec", points=100, start_freq=1e3, stop_freq=1e9))]
    fn network_params(
        &self,
        output_current: &str,
        input_source: &str,
        z_in: f64,
        z_out: f64,
        variation: &str,
        points: u32,
        start_freq: f64,
        stop_freq: f64,
    ) -> PyResult<PySParamAnalysis> {
        let mut ir = self.deck(card::ac(variation, points, start_freq, stop_freq)?);
        if let Some(tb) = &mut ir.testbench {
            tb.extra_lines.push(format!(
                ".net {output_current} {input_source} Rout={z_out} Rin={z_in}"
            ));
        }
        Ok(PySParamAnalysis {
            inner: Results::from_raw(run_deck(ir, self.backend.as_deref())?),
        })
    }

    #[pyo3(signature = (fundamental_frequencies, num_harmonics=None))]
    fn harmonic_balance(
        &self,
        fundamental_frequencies: Vec<f64>,
        num_harmonics: Option<Vec<u32>>,
    ) -> PyResult<PyHarmonicBalanceAnalysis> {
        let harmonics = num_harmonics.unwrap_or_else(|| vec![7; fundamental_frequencies.len()]);
        let a = Analysis::HarmonicBalance {
            frequencies: fundamental_frequencies,
            harmonics,
        };
        Ok(PyHarmonicBalanceAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    #[pyo3(signature = (probe, variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e10))]
    fn stability(
        &self,
        probe: &str,
        variation: &str,
        number_of_points: u32,
        start_frequency: f64,
        stop_frequency: f64,
    ) -> PyResult<PyStabilityAnalysis> {
        let a = Analysis::Stability {
            probe: probe.into(),
            sweep: card::sweep(variation, number_of_points, start_frequency, stop_frequency)?,
        };
        Ok(PyStabilityAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }

    #[pyo3(signature = (step_time, end_time))]
    fn transient_noise(&self, step_time: f64, end_time: f64) -> PyResult<PyTransientNoiseAnalysis> {
        let a = Analysis::TransientNoise {
            step: step_time,
            stop: end_time,
        };
        Ok(PyTransientNoiseAnalysis {
            inner: Results::from_raw(self.run(a)?),
        })
    }


    /// List all available simulator backends on this system
    #[staticmethod]
    fn available_backends() -> Vec<String> {
        crate::backend::available()
            .iter()
            .map(|b| b.name().to_string())
            .collect()
    }

    /// Check backend compatibility for this deck.
    fn check_backend(&self, name: &str) -> Vec<String> {
        issues(&self.ir, name)
    }

    fn __repr__(&self) -> &str {
        "CircuitSimulator"
    }
}
}

impl PySimulator {
    /// A copy of the deck that runs `analysis` alone.
    fn deck(&self, analysis: Analysis) -> CircuitIR {
        let mut ir = self.deck_as_queued();
        ir.testbench.get_or_insert_with(Default::default).analyses = vec![analysis];
        ir
    }

    fn run(&self, analysis: Analysis) -> PyResult<RawData> {
        run_deck(self.deck(analysis), self.backend.as_deref())
    }

    /// Spectre-only analyses always run on Spectre.
    fn run_spectre(&self, analysis: Analysis) -> PyResult<RawData> {
        run_deck(self.deck(analysis), Some("spectre"))
    }
}
