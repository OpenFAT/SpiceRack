//! `CircuitIR` -> `FeatureFlags` (what the deck needs from a simulator) and,
//! with `backend`, -> the `Issue`s one backend raises against it.
//!
//! Cites: none (the capability masks are `backend::Dialect::capabilities`).
//!
//! Contents:
//! - `pub mod feature`: `XSPICE`, `OSDI`, `MEASURES`, `STEP_PARAMS`, `CONTROL_BLOCKS`,
//!   `LAPLACE`, `VERILOG_COSIM`, `ALL` (`u8` bits)
//! - `pub struct FeatureFlags`, `FeatureFlags::has(&self, f: u8) -> bool`
//! - `pub struct Issue`, `pub enum IssueSeverity`
//! - `CircuitIR::compute_features(&self) -> FeatureFlags`
//! - `CircuitIR::check_backend(&self, backend: &str) -> Vec<Issue>` (feature `backend`)

use super::*;

/// What a deck needs from its simulator, one bit per feature
/// (`Dialect::capabilities`, in `backend`, is the same mask: what a simulator does).
pub mod feature {
    pub const XSPICE: u8 = 1 << 0;
    pub const OSDI: u8 = 1 << 1;
    pub const MEASURES: u8 = 1 << 2;
    pub const STEP_PARAMS: u8 = 1 << 3;
    pub const CONTROL_BLOCKS: u8 = 1 << 4;
    pub const LAPLACE: u8 = 1 << 5;
    pub const VERILOG_COSIM: u8 = 1 << 6;
    /// Every bit above.
    pub const ALL: u8 =
        XSPICE | OSDI | MEASURES | STEP_PARAMS | CONTROL_BLOCKS | LAPLACE | VERILOG_COSIM;
}

/// Feature flags computed from IR
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeatureFlags {
    /// `feature::*` bits.
    pub needs: u8,
    pub element_count: usize,
}

/// Issue found by check_backend
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: IssueSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IssueSeverity {
    Error,
    Warning,
}

// Seven features, one distinct bit each, inside the `u8`.
const _: () = assert!(feature::ALL.count_ones() == 7);

/// Each feature a backend may lack, how bad that is, and what to say
/// (`{}` is the backend).
#[cfg(feature = "backend")]
const MISSING: &[(u8, IssueSeverity, &str)] = &[
    (
        feature::XSPICE,
        IssueSeverity::Error,
        "Backend '{}' does not support XSPICE A-elements (only ngspice does)",
    ),
    (
        feature::OSDI,
        IssueSeverity::Error,
        "Backend '{}' does not support OSDI/Verilog-A model loading",
    ),
    (
        feature::MEASURES,
        IssueSeverity::Error,
        "Backend '{}' does not support .meas directives",
    ),
    (
        feature::STEP_PARAMS,
        IssueSeverity::Error,
        "Backend '{}' does not support .step param sweeps",
    ),
    (
        feature::CONTROL_BLOCKS,
        IssueSeverity::Error,
        "Backend '{}' does not support .control blocks (ngspice only)",
    ),
    (
        feature::LAPLACE,
        IssueSeverity::Warning,
        "Backend '{}' does not support Laplace-domain B-sources",
    ),
    (
        feature::VERILOG_COSIM,
        IssueSeverity::Error,
        "Backend '{}' does not support Verilog co-simulation",
    ),
];

// Exactly one `MISSING` row per feature bit, so a backend `select` refuses
// for a feature always has the issue `check_backend` names for it.
#[cfg(feature = "backend")]
const _: () = {
    let (mut seen, mut i) = (0u8, 0);
    while i < MISSING.len() {
        assert!(seen & MISSING[i].0 == 0, "a feature has two MISSING rows");
        assert!(
            MISSING[i].0.count_ones() == 1,
            "a MISSING row names one feature"
        );
        seen |= MISSING[i].0;
        i += 1;
    }
    assert!(seen == feature::ALL, "a feature has no MISSING row");
};

impl FeatureFlags {
    pub fn has(&self, f: u8) -> bool {
        self.needs & f != 0
    }
}

impl CircuitIR {
    /// Compute feature flags by walking the IR
    pub fn compute_features(&self) -> FeatureFlags {
        let mut flags = FeatureFlags::default();

        for sub in self.subcircuits() {
            self.scan_subcircuit(sub, &mut flags);
        }

        // Check testbench for measures, step params
        if let Some(ref tb) = self.testbench {
            if !tb.measures.is_empty() {
                flags.needs |= feature::MEASURES;
            }
            if !tb.step_params.is_empty() {
                flags.needs |= feature::STEP_PARAMS;
            }
        }

        flags
    }

    #[cfg(feature = "backend")]
    /// Check backend compatibility -- pure function, no I/O.
    ///
    /// Returns a list of issues (errors and warnings) for the given backend name.
    pub fn check_backend(&self, backend: &str) -> Vec<Issue> {
        let features = self.compute_features();
        let mut issues = Vec::new();

        let kind = match crate::backend::parse(backend) {
            Some(k) => k,
            None => {
                issues.push(Issue {
                    severity: IssueSeverity::Error,
                    message: format!("Unknown backend '{}'", backend),
                });
                return issues;
            }
        };
        let missing = features.needs & !kind.capabilities();
        for &(f, severity, message) in MISSING {
            if missing & f != 0 {
                issues.push(Issue {
                    severity,
                    message: message.replace("{}", backend),
                });
            }
        }

        if features.element_count > 10_000 && kind == Dialect::Ngspice {
            issues.push(Issue {
                severity: IssueSeverity::Warning,
                message: format!(
                    "Large circuit ({} elements) may be slow on '{}'",
                    features.element_count, backend
                ),
            });
        }

        issues
    }
}

// ---- private ----

impl CircuitIR {
    fn scan_subcircuit(&self, sub: &Subcircuit, flags: &mut FeatureFlags) {
        if !sub.osdi_loads.is_empty() {
            flags.needs |= feature::OSDI;
        }

        if sub
            .verilog_blocks
            .iter()
            .any(|vb| matches!(vb.mode, VerilogMode::Simulate))
        {
            flags.needs |= feature::VERILOG_COSIM;
        }

        for line in &sub.raw_spice {
            let trimmed = line.trim().to_lowercase();
            if trimmed == ".control" || trimmed.starts_with(".control ") {
                flags.needs |= feature::CONTROL_BLOCKS;
            }
        }

        for comp in &sub.components {
            flags.element_count += 1;
            match comp {
                Component::Xspice { .. } => {
                    flags.needs |= feature::XSPICE;
                }
                Component::BehavioralVoltage { expression, .. }
                | Component::BehavioralCurrent { expression, .. }
                    if crate::codegen::laplace::is_laplace(expression) =>
                {
                    flags.needs |= feature::LAPLACE;
                }
                _ => {}
            }
        }

        flags.element_count += sub.instances.len();
    }
}
