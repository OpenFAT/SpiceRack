use std::collections::HashMap;
use num_complex::Complex64;

/// One simulator plot, stored by column: `real_data[v]` or `complex_data[v]` holds every
/// point of variable `v`. A real plot fills `real_data`; a complex plot fills
/// `complex_data` (a backend may also fill `real_data` for its real-valued vectors).
#[derive(Debug, Clone, Default)]
pub struct RawData {
    pub title: String,
    pub plot_name: String,
    pub variables: Vec<VarInfo>,
    pub real_data: Vec<Vec<f64>>,
    pub complex_data: Vec<Vec<Complex64>>,
    pub is_complex: bool,
    /// Captured stdout from simulator process (contains .meas results, etc.)
    pub stdout: String,
    /// Captured log file content (LTspice puts .meas results in .log)
    pub log_content: String,
    /// Parsed .meas results extracted from stdout/log
    pub measures: Vec<MeasureResult>,
    /// Backend that produced this data, for name normalization; empty means ngspice.
    pub backend_hint: String,
}

#[derive(Debug, Clone)]
pub struct VarInfo {
    /// Column in `real_data` / `complex_data`.
    pub index: usize,
    pub name: String,
    pub var_type: String, // "voltage", "current", "time", "frequency"
}

#[derive(Debug, Clone)]
pub struct MeasureResult {
    pub name: String,
    pub value: f64,
}

impl RawData {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Complex column `i`, or `None` when that variable is real-valued.
    pub fn complex(&self, i: usize) -> Option<&[Complex64]> {
        self.complex_data.get(i).filter(|c| !c.is_empty()).map(Vec::as_slice)
    }

    /// Column `i` as reals (the real part of a complex column); empty if absent.
    pub fn re(&self, i: usize) -> Vec<f64> {
        match self.complex(i) {
            Some(c) => c.iter().map(|c| c.re).collect(),
            None => self.real_data.get(i).cloned().unwrap_or_default(),
        }
    }
}

/// Any analysis result: the plot plus normalized-name -> column lookup.
/// Every analysis kind is this one type; the sweep axis (time, frequency, DC sweep)
/// is column 0.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub raw: RawData,
    pub nodes: HashMap<String, usize>,
    pub branches: HashMap<String, usize>,
}

pub type OperatingPoint = Analysis;
pub type DcAnalysis = Analysis;
pub type AcAnalysis = Analysis;
pub type TransientAnalysis = Analysis;
pub type NoiseAnalysis = Analysis;
pub type TransferFunctionAnalysis = Analysis;
pub type SensitivityAnalysis = Analysis;
pub type PoleZeroAnalysis = Analysis;
pub type DistortionAnalysis = Analysis;
pub type PssAnalysis = Analysis;
pub type SParamAnalysis = Analysis;
pub type HarmonicBalanceAnalysis = Analysis;
pub type StabilityAnalysis = Analysis;
pub type TransientNoiseAnalysis = Analysis;
pub type SamplingAnalysis = Analysis;

impl Analysis {
    pub fn from_raw(raw: RawData) -> Self {
        use crate::normalize::{is_current_name, normalize_var_name};
        let backend = if raw.backend_hint.is_empty() { "ngspice" } else { &raw.backend_hint };
        let mut nodes = HashMap::new();
        let mut branches = HashMap::new();
        for v in &raw.variables {
            let map = if v.var_type == "current" || is_current_name(&v.name, backend) {
                &mut branches
            } else {
                &mut nodes
            };
            map.insert(normalize_var_name(&v.name, backend), v.index);
        }
        Self { raw, nodes, branches }
    }

    /// Column of `name` (canonical form, any case), nodes before branches.
    pub fn column(&self, name: &str) -> Option<usize> {
        let lower = name.to_lowercase();
        self.nodes.get(&lower).or_else(|| self.branches.get(&lower)).copied()
    }

    /// Waveform of `name`; the real part for a complex plot.
    pub fn get(&self, name: &str) -> Option<Vec<f64>> {
        self.column(name).map(|i| self.raw.re(i))
    }

    pub fn complex(&self, name: &str) -> Option<&[Complex64]> {
        self.raw.complex(self.column(name)?)
    }

    /// The sweep axis (column 0).
    pub fn axis(&self) -> Vec<f64> {
        self.raw.re(0)
    }
}

/// Compute (ENOB, SFDR dB, SNR dB, THD dB) from FFT magnitude bins.
///
/// Bin 0 is DC, the largest other bin is the fundamental, and harmonics sit at
/// integer multiples of the fundamental's index.
pub fn compute_fft_metrics(magnitude: &[f64]) -> (f64, f64, f64, f64) {
    if magnitude.len() < 4 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let (fund, fund_mag) = magnitude.iter().copied().enumerate().skip(1)
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .expect("len >= 4");
    if fund_mag <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }

    let (mut harmonic, mut noise, mut spur) = (0.0, 0.0, 0.0_f64);
    for (i, &m) in magnitude.iter().enumerate().skip(1) {
        if i == fund {
            continue;
        }
        spur = spur.max(m);
        if i % fund == 0 { harmonic += m * m } else { noise += m * m }
    }

    let fund_power = fund_mag * fund_mag;
    let db = |ratio: f64| if ratio > 0.0 { 10.0 * ratio.log10() } else { f64::NEG_INFINITY };
    let thd_db = db(harmonic / fund_power);
    let snr_db = -db(noise / fund_power);
    let sfdr_db = -db(spur * spur / fund_power);
    let enob = (-db((noise + harmonic) / fund_power) - 1.76) / 6.02;
    (enob, sfdr_db, snr_db, thd_db)
}
