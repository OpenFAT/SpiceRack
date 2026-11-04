//! Simulator output as tables: one [`RawData`] per plot (column-major, as
//! `rawfile` reads it) and [`Results`], the same plot with its vectors looked
//! up by canonical name (`normalize::canonical`).
//!
//! Cites: none (the raw-file layout is `rawfile`'s).
//!
//! Contents:
//! - `pub struct RawData`, `RawData::complex(&self, i: usize) -> Option<&[Complex64]>`,
//!   `RawData::re(&self, i: usize) -> Cow<'_, [f64]>`
//! - `pub struct VarInfo`
//! - `pub enum VarType`, `VarType::parse(word: &str) -> VarType`
//! - `pub struct MeasureResult`
//! - `pub struct Results`, `Results::from_raw(raw: RawData) -> Results`,
//!   `Results::column(&self, name: &str) -> Option<usize>`,
//!   `Results::get(&self, name: &str) -> Option<Cow<'_, [f64]>>`,
//!   `Results::complex(&self, name: &str) -> Option<&[Complex64]>`,
//!   `Results::axis(&self) -> Cow<'_, [f64]>`

use num_complex::Complex64;
use std::borrow::Cow;
use std::collections::HashMap;

use crate::syntax::Dialect;

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
    /// The simulator that wrote the plot, whose naming rules [`Results`]
    /// reads its vectors by; `None` (a raw file read on its own) reads them
    /// as ngspice names them.
    pub backend_hint: Option<Dialect>,
}

#[derive(Debug, Clone)]
pub struct VarInfo {
    /// Column in `real_data` / `complex_data`.
    pub index: usize,
    pub name: String,
    pub var_type: VarType,
}

/// A raw-file variable's type word (the `Variables:` section's third field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarType {
    Voltage,
    Current,
    Time,
    Frequency,
    /// Any other word (`notype`, LTspice's `device_current`, ...).
    Other,
}

#[derive(Debug, Clone)]
pub struct MeasureResult {
    pub name: String,
    pub value: f64,
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

impl VarType {
    /// The type a raw file's type word names, spelled exactly as ngspice
    /// writes it.
    pub fn parse(word: &str) -> VarType {
        match word {
            "voltage" => VarType::Voltage,
            "current" => VarType::Current,
            "time" => VarType::Time,
            "frequency" => VarType::Frequency,
            _ => VarType::Other,
        }
    }
}

impl RawData {
    /// Complex column `i`, or `None` when that variable is real-valued.
    pub fn complex(&self, i: usize) -> Option<&[Complex64]> {
        self.complex_data
            .get(i)
            .filter(|c| !c.is_empty())
            .map(Vec::as_slice)
    }

    /// Column `i` as reals: a real column borrowed, the real part of a
    /// complex one collected into a new `Vec`; empty if absent.
    pub fn re(&self, i: usize) -> Cow<'_, [f64]> {
        match self.complex(i) {
            Some(c) => Cow::Owned(c.iter().map(|c| c.re).collect()),
            None => Cow::Borrowed(self.real_data.get(i).map_or(&[][..], Vec::as_slice)),
        }
    }
}

impl Results {
    pub fn from_raw(raw: RawData) -> Self {
        let mut nodes = HashMap::new();
        let mut branches = HashMap::new();
        for v in &raw.variables {
            let (name, current) = crate::normalize::canonical(&v.name, raw.backend_hint);
            let map = if v.var_type == VarType::Current || current {
                &mut branches
            } else {
                &mut nodes
            };
            map.insert(name, v.index);
        }
        Self {
            raw,
            nodes,
            branches,
        }
    }

    /// Column of `name` (canonical form, any case), nodes before branches.
    /// Allocates only when `name` is not already lowercase ASCII.
    pub fn column(&self, name: &str) -> Option<usize> {
        let lower: Cow<str> = if name
            .bytes()
            .all(|b| b.is_ascii() && !b.is_ascii_uppercase())
        {
            Cow::Borrowed(name)
        } else {
            Cow::Owned(name.to_lowercase())
        };
        self.nodes
            .get(lower.as_ref())
            .or_else(|| self.branches.get(lower.as_ref()))
            .copied()
    }

    /// Waveform of `name`; the real part for a complex plot (computed, the
    /// one case that allocates).
    pub fn get(&self, name: &str) -> Option<Cow<'_, [f64]>> {
        self.column(name).map(|i| self.raw.re(i))
    }

    pub fn complex(&self, name: &str) -> Option<&[Complex64]> {
        self.raw.complex(self.column(name)?)
    }

    /// The sweep axis (column 0).
    pub fn axis(&self) -> Cow<'_, [f64]> {
        self.raw.re(0)
    }
}
