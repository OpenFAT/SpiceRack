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
pub struct Results {
    pub raw: RawData,
    pub nodes: HashMap<String, usize>,
    pub branches: HashMap<String, usize>,
}


impl Results {
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

