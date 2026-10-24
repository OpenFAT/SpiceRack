pub mod espice;
pub mod laplace;
pub mod spice3;
pub mod spectre;
pub mod vacask;

use crate::ir::*;

#[derive(Debug, thiserror::Error)]
pub enum CodeGenError {
    #[error("Unsupported component for backend {backend}: {component}")]
    UnsupportedComponent { backend: String, component: String },
    #[error("Unsupported analysis for backend {backend}: {analysis}")]
    UnsupportedAnalysis { backend: String, analysis: String },
    #[error("Code generation error: {0}")]
    Other(String),
}

pub trait CodeGen {
    fn backend_name(&self) -> &str;
    fn emit_netlist(&self, ir: &CircuitIR) -> Result<String, CodeGenError>;
    fn emit_subcircuit(&self, sc: &Subcircuit) -> Result<String, CodeGenError>;
    fn emit_component(&self, comp: &Component) -> Result<String, CodeGenError>;
    fn emit_analysis(&self, analysis: &Analysis) -> Result<String, CodeGenError>;
    fn emit_options(&self, opts: &SimOptions) -> Result<String, CodeGenError>;
}

/// `v` as a SPICE number with an SI suffix (`1000.0` → `1k`).
pub fn format_spice_number(v: f64) -> String {
    use crate::unit::SiPrefix;
    let prefix = SiPrefix::best_for(v);
    let mantissa = v / prefix.multiplier();
    let suffix = prefix.spice_suffix();
    // Round to avoid floating point artifacts (10e-6/1e-6 = 10.000000000000002)
    let rounded = mantissa.round();
    if (mantissa - rounded).abs() < 1e-9 && rounded.abs() < 1e15 {
        format!("{}{}", rounded as i64, suffix)
    } else {
        format!("{}{}", mantissa, suffix)
    }
}
