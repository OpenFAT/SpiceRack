//! Golden-string tests for the Spectre codegen.
//!
//! Spectre is licence-gated and not installed, so nothing here can be executed
//! against the real simulator. The strongest evidence available is that the
//! text we emit matches the statement forms Cadence documents. Every assertion
//! below carries the citation it is checking.
//!
//! Sources:
//! * `[REF]` *Spectre Circuit Simulator Reference*, Product Version 19.1,
//!   January 2020 <https://ee.kpi.ua/~yv/edu/ok/book/spectre_refManual.pdf>
//! * `[CMP]` *Spectre Circuit Simulator Reference*, Product Version 5.0,
//!   September 2003 <http://eece.cu.edu.eg/~fhussien/Spectre_tutorial.pdf>
//!   (component chapters)
//! * `[KUN]` Kundert, *The Designer's Guide to SPICE and Spectre*, Appendix B
//!   <https://designers-guide.org/analysis/dg-spice/chB.pdf>
//!
//! These tests prove only "we emit what the manual describes". They do NOT
//! prove any simulation result.
//!
//! Contents: none (tests only).

use spicerack::codegen::{CodeGenError, Dialect, netlist};
use spicerack::ir::*;

mod analyses;
mod components;
mod deck;

// ---- private ----

fn spectre(ir: &CircuitIR) -> Result<String, CodeGenError> {
    netlist(ir, Dialect::Spectre)
}

/// The deck's last section: the analyses, or the components when there is no
/// testbench. Neither contains a blank line.
fn last_block(ir: &CircuitIR) -> Result<String, CodeGenError> {
    Ok(spectre(ir)?
        .rsplit("\n\n")
        .next()
        .unwrap_or_default()
        .to_string())
}

fn try_analysis(a: &Analysis) -> Result<String, CodeGenError> {
    let mut tb = empty_tb();
    tb.analyses.push(a.clone());
    last_block(&ir_with(empty_top("t"), Some(tb)))
}

fn try_component(c: &Component) -> Result<String, CodeGenError> {
    let mut top = empty_top("t");
    top.components.push(c.clone());
    last_block(&ir_with(top, None))
}

fn empty_top(name: &str) -> Subcircuit {
    Subcircuit {
        name: name.into(),
        ..Default::default()
    }
}

fn empty_tb() -> Testbench {
    Testbench {
        ..Default::default()
    }
}

fn ir_with(top: Subcircuit, tb: Option<Testbench>) -> CircuitIR {
    CircuitIR {
        top,
        testbench: tb,
        ..Default::default()
    }
}

fn analysis(a: Analysis) -> String {
    try_analysis(&a).unwrap()
}

fn component(c: Component) -> String {
    try_component(&c).unwrap()
}
