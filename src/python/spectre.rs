//! Spectre-only analyses: sweep, Monte Carlo and the SpectreRF PSS family
//! (PAC, PNoise, PXF, PSTB) → a deck that always runs on Spectre. Cites
//! the Spectre reference manual's `sweep`, `montecarlo`, `pac`, `pnoise`,
//! `pxf` and `pstb` statements.
//!
//! Contents:
//! - `macro_rules! pymethods_with_spectre` (scoped to `python` by `#[macro_use]`)

/// `#[pymethods] impl PySimulator { ... }` with the Spectre-only analyses
/// appended: pyo3 takes one `#[pymethods]` per class without its
/// `multiple-pymethods` feature, so this file adds its methods to the one
/// block in `simulator.rs`, where every name in them resolves.
macro_rules! pymethods_with_spectre {
    ($hash:tt $attr:tt impl $ty:ty { $($body:tt)* }) => {
        $hash $attr
        impl $ty {
            $($body)*
            /// Spectre parametric sweep wrapping an inner analysis (Spectre-native text).
            #[pyo3(signature = (param, start, stop, step, inner_analysis, inner_type="ac"))]
            fn spectre_sweep(
                &self,
                param: &str,
                start: f64,
                stop: f64,
                step: f64,
                inner_analysis: &str,
                inner_type: &str,
            ) -> PyResult<PyRawData> {
                Ok(PyRawData {
                    inner: self.run_spectre(card::spectre_sweep(
                        param,
                        start,
                        stop,
                        step,
                        inner_analysis,
                        inner_type,
                    )?)?,
                })
            }

            /// Spectre Monte Carlo wrapping an inner analysis.
            #[pyo3(signature = (num_iterations, inner_analysis, inner_type="ac", seed=None))]
            fn spectre_monte_carlo(
                &self,
                num_iterations: u32,
                inner_analysis: &str,
                inner_type: &str,
                seed: Option<u64>,
            ) -> PyResult<PyRawData> {
                Ok(PyRawData {
                    inner: self.run_spectre(card::spectre_monte_carlo(
                        num_iterations,
                        inner_analysis,
                        inner_type,
                        seed,
                    )?)?,
                })
            }

            /// Deprecated: [`Self::spectre_monte_carlo`], the spelling
            /// `Testbench.add_spectre_monte_carlo` uses. Warns `DeprecationWarning`.
            #[pyo3(signature = (num_iterations, inner_analysis, inner_type="ac", seed=None))]
            fn spectre_montecarlo(
                &self,
                py: Python<'_>,
                num_iterations: u32,
                inner_analysis: &str,
                inner_type: &str,
                seed: Option<u64>,
            ) -> PyResult<PyRawData> {
                let category = py.get_type::<pyo3::exceptions::PyDeprecationWarning>();
                PyErr::warn(
                    py,
                    &category,
                    c"spectre_montecarlo is deprecated; use spectre_monte_carlo",
                    1,
                )?;
                self.spectre_monte_carlo(num_iterations, inner_analysis, inner_type, seed)
            }

            /// SpectreRF Periodic AC (PAC) analysis with automatic PSS prerequisite.
            #[pyo3(signature = (pss_fundamental, pss_stabilization, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9, sweep_type="relative"))]
            fn spectre_pac(
                &self,
                pss_fundamental: f64,
                pss_stabilization: f64,
                pss_harmonics: u32,
                variation: &str,
                points: u32,
                start_freq: f64,
                stop_freq: f64,
                sweep_type: &str,
            ) -> PyResult<PyAcAnalysis> {
                let a = Analysis::SpectrePac {
                    pss: PssBase {
                        fundamental: pss_fundamental,
                        stabilization: pss_stabilization,
                        harmonics: pss_harmonics,
                    },
                    sweep: card::sweep(variation, points, start_freq, stop_freq)?,
                    sweep_type: sweep_type.into(),
                };
                Ok(PyAcAnalysis {
                    inner: Results::from_raw(self.run_spectre(a)?),
                })
            }

            /// SpectreRF Periodic Noise (PNoise) analysis with automatic PSS prerequisite.
            #[pyo3(signature = (pss_fundamental, pss_stabilization, output_node, ref_node, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9))]
            fn spectre_pnoise(
                &self,
                pss_fundamental: f64,
                pss_stabilization: f64,
                output_node: &str,
                ref_node: &str,
                pss_harmonics: u32,
                variation: &str,
                points: u32,
                start_freq: f64,
                stop_freq: f64,
            ) -> PyResult<PyNoiseAnalysis> {
                let a = Analysis::SpectrePnoise {
                    pss: PssBase {
                        fundamental: pss_fundamental,
                        stabilization: pss_stabilization,
                        harmonics: pss_harmonics,
                    },
                    output: output_node.into(),
                    reference: ref_node.into(),
                    sweep: card::sweep(variation, points, start_freq, stop_freq)?,
                };
                Ok(PyNoiseAnalysis {
                    inner: Results::from_raw(self.run_spectre(a)?),
                })
            }

            /// SpectreRF Periodic Transfer Function (PXF) analysis with automatic PSS prerequisite.
            #[pyo3(signature = (pss_fundamental, pss_stabilization, output_node, source, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9))]
            fn spectre_pxf(
                &self,
                pss_fundamental: f64,
                pss_stabilization: f64,
                output_node: &str,
                source: &str,
                pss_harmonics: u32,
                variation: &str,
                points: u32,
                start_freq: f64,
                stop_freq: f64,
            ) -> PyResult<PyAcAnalysis> {
                // `[REF]` p.303-304: like `xf`, `pxf` takes every source's
                // transfer function, so it names no input source.
                let _ = source;
                let a = Analysis::SpectrePxf {
                    pss: PssBase {
                        fundamental: pss_fundamental,
                        stabilization: pss_stabilization,
                        harmonics: pss_harmonics,
                    },
                    output: output_node.into(),
                    sweep: card::sweep(variation, points, start_freq, stop_freq)?,
                };
                Ok(PyAcAnalysis {
                    inner: Results::from_raw(self.run_spectre(a)?),
                })
            }

            /// SpectreRF Periodic Stability (PSTB) analysis with automatic PSS prerequisite.
            #[pyo3(signature = (pss_fundamental, pss_stabilization, probe, pss_harmonics=10, variation="dec", points=100, start_freq=1.0, stop_freq=1e9))]
            fn spectre_pstb(
                &self,
                pss_fundamental: f64,
                pss_stabilization: f64,
                probe: &str,
                pss_harmonics: u32,
                variation: &str,
                points: u32,
                start_freq: f64,
                stop_freq: f64,
            ) -> PyResult<PyStabilityAnalysis> {
                let a = Analysis::SpectrePstb {
                    pss: PssBase {
                        fundamental: pss_fundamental,
                        stabilization: pss_stabilization,
                        harmonics: pss_harmonics,
                    },
                    probe: probe.into(),
                    sweep: card::sweep(variation, points, start_freq, stop_freq)?,
                };
                Ok(PyStabilityAnalysis {
                    inner: Results::from_raw(self.run_spectre(a)?),
                })
            }
        }
    };
}
