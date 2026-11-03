//! Cards: Python analysis arguments → one `Analysis` per card.
//! `CircuitSimulator.x()` runs it, `Testbench.add_x()` queues it; both
//! build it here. Cites the ngspice manual's analysis cards.
//!
//! Contents:
//! - `pub fn dc_sweeps(kwargs: Option<Bound<'_, PyDict>>) -> PyResult<Analysis>`
//! - `pub fn sweep(variation: &str, points: u32, start: f64, stop: f64) -> PyResult<FreqSweep>`
//! - `pub fn ac(variation, points, start, stop) -> PyResult<Analysis>`
//! - `pub fn transient(step, stop, start, max_step, uic) -> Analysis`
//! - `pub fn noise(output, reference, source, variation, points, start, stop, points_per_summary) -> PyResult<Analysis>`
//! - `pub fn tf(output: &str, source: &str) -> Analysis`
//! - `pub fn sensitivity(output: &str, ac: Option<FreqSweep>) -> Analysis`
//! - `pub fn polezero(nodes: [&str; 4], tf_type: &str, pz_type: &str) -> PyResult<Analysis>`
//! - `pub fn distortion(variation, points, start, stop, f2overf1) -> PyResult<Analysis>`
//! - `pub fn spectre_sweep(param, start, stop, step, inner, inner_type) -> PyResult<Analysis>`
//! - `pub fn spectre_monte_carlo(iterations, inner, inner_type, seed) -> PyResult<Analysis>`

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::ir::{self, Analysis, FreqSweep};
/// Extract DC sweep params: kwargs like Vinput=slice(-2, 5, 0.01)
pub fn dc_sweeps(kwargs: Option<Bound<'_, PyDict>>) -> PyResult<Analysis> {
    let mut sweeps = Vec::new();
    if let Some(dict) = kwargs {
        for (k, v) in dict.iter() {
            sweeps.push(ir::DcSweep {
                source: k.extract::<String>()?,
                start: v.getattr("start")?.extract::<f64>()?,
                stop: v.getattr("stop")?.extract::<f64>()?,
                step: v.getattr("step")?.extract::<f64>()?,
            });
        }
    }
    if sweeps.is_empty() {
        return Err(PyValueError::new_err(
            "dc() requires at least one sweep parameter",
        ));
    }
    Ok(Analysis::Dc { sweeps })
}

/// Where a Python sweep name becomes a [`crate::ir::Sweep`]: anything
/// but dec, oct, lin or log (any case) is a `ValueError` here.
pub fn sweep(variation: &str, points: u32, start: f64, stop: f64) -> PyResult<FreqSweep> {
    Ok(FreqSweep {
        variation: variation.parse().map_err(PyValueError::new_err)?,
        points,
        start,
        stop,
    })
}

/// `.ac`; `variation` per [`sweep`].
pub fn ac(variation: &str, points: u32, start: f64, stop: f64) -> PyResult<Analysis> {
    Ok(Analysis::Ac {
        sweep: sweep(variation, points, start, stop)?,
    })
}

/// `.tran step stop [start [max_step]] [uic]`.
pub fn transient(
    step: f64,
    stop: f64,
    start: Option<f64>,
    max_step: Option<f64>,
    uic: bool,
) -> Analysis {
    Analysis::Transient {
        step,
        stop,
        start,
        max_step,
        uic,
    }
}

/// `.noise`; `variation` per [`sweep`].
pub fn noise(
    output: &str,
    reference: &str,
    source: &str,
    variation: &str,
    points: u32,
    start: f64,
    stop: f64,
    points_per_summary: Option<u32>,
) -> PyResult<Analysis> {
    let sweep = sweep(variation, points, start, stop)?;
    Ok(Analysis::Noise {
        output: output.into(),
        reference: reference.into(),
        source: source.into(),
        sweep,
        points_per_summary,
    })
}

/// `.tf output source`.
pub fn tf(output: &str, source: &str) -> Analysis {
    Analysis::Tf {
        output: output.into(),
        source: source.into(),
    }
}

/// `.sens`: DC without `ac`, else over that AC sweep.
pub fn sensitivity(output: &str, ac: Option<FreqSweep>) -> Analysis {
    Analysis::Sensitivity {
        output: output.into(),
        ac,
    }
}

/// `tf_type` is vol or cur, `pz_type` pol, zer or pz (any case); else
/// a `ValueError`.
pub fn polezero(nodes: [&str; 4], tf_type: &str, pz_type: &str) -> PyResult<Analysis> {
    let [node1, node2, node3, node4] = nodes.map(String::from);
    let (tf_type, pz_type) = (
        tf_type.parse().map_err(PyValueError::new_err)?,
        pz_type.parse().map_err(PyValueError::new_err)?,
    );
    Ok(Analysis::PoleZero {
        node1,
        node2,
        node3,
        node4,
        tf_type,
        pz_type,
    })
}

/// `.disto`; `variation` per [`sweep`].
pub fn distortion(
    variation: &str,
    points: u32,
    start: f64,
    stop: f64,
    f2overf1: Option<f64>,
) -> PyResult<Analysis> {
    Ok(Analysis::Distortion {
        sweep: sweep(variation, points, start, stop)?,
        f2overf1,
    })
}

/// A Spectre `sweep` around `inner`; an `inner_type` that is not a Spectre
/// analysis kind is a `ValueError`.
pub fn spectre_sweep(
    param: &str,
    start: f64,
    stop: f64,
    step: f64,
    inner: &str,
    inner_type: &str,
) -> PyResult<Analysis> {
    let inner_type = inner_type.parse().map_err(PyValueError::new_err)?;
    Ok(Analysis::SpectreSweep {
        param: param.into(),
        start,
        stop,
        step,
        inner: inner.into(),
        inner_type,
    })
}

/// A Spectre `montecarlo` around `inner`; `inner_type` as in
/// [`spectre_sweep`].
pub fn spectre_monte_carlo(
    iterations: u32,
    inner: &str,
    inner_type: &str,
    seed: Option<u64>,
) -> PyResult<Analysis> {
    let inner_type = inner_type.parse().map_err(PyValueError::new_err)?;
    Ok(Analysis::SpectreMonteCarlo {
        iterations,
        inner: inner.into(),
        inner_type,
        seed,
        variations: None,
    })
}
