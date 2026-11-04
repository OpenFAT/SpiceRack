//! Verilog: a `verilog(...)` call → a `veriloga::VerilogBlock`, and each
//! block → the cards that simulate it (a `d_cosim` XSPICE instance, or
//! synthesized standard cells). Cites ngspice's `d_cosim` and
//! `adc_bridge`/`dac_bridge` code models.
//!
//! Contents:
//! - `pub(super) fn verilog_block(py, source, mode, instance_name, connections, pdk, liberty, spice_models) -> PyResult<VerilogBlock>`
//! - `pub(super) fn lower_verilog(body: &mut Body) -> Result<(), String>`

use std::collections::HashMap;

use pyo3::prelude::*;

use super::model::{Body, Card, need};
use crate::veriloga::{VerilogBlock, VerilogConnection, VerilogMode};

/// The IR block for a `verilog(...)` call; connection values are a net name
/// or a list of net names (a bus).
#[allow(clippy::too_many_arguments)]
pub(super) fn verilog_block(
    py: pyo3::Python<'_>,
    source: &str,
    mode: &str,
    instance_name: &str,
    connections: &HashMap<String, pyo3::PyObject>,
    pdk: Option<&str>,
    liberty: Option<&str>,
    spice_models: Option<&str>,
) -> PyResult<VerilogBlock> {
    let mode = match mode {
        "simulate" => VerilogMode::Simulate,
        "synthesize" => VerilogMode::Synthesize,
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Invalid mode '{}'. Must be 'simulate' or 'synthesize'.",
                mode
            )));
        }
    };
    let mut conns = std::collections::BTreeMap::new();
    for (port, obj) in connections {
        let conn = if let Ok(s) = obj.extract::<String>(py) {
            VerilogConnection::Single(s)
        } else if let Ok(v) = obj.extract::<Vec<String>>(py) {
            VerilogConnection::Bus(v)
        } else {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "Connection value must be a string or list of strings",
            ));
        };
        conns.insert(port.clone(), conn);
    }
    Ok(VerilogBlock {
        source: source.to_string(),
        mode,
        instance_name: instance_name.to_string(),
        connections: conns,
        pdk: pdk.map(str::to_string),
        liberty: liberty.map(str::to_string),
        spice_models: spice_models.map(str::to_string),
    })
}

/// Expand each Verilog block of `body` into the cards that simulate it: a
/// `d_cosim` XSPICE instance (simulate) or the synthesized standard-cell
/// instances (synthesize).
pub(super) fn lower_verilog(body: &mut Body) -> Result<(), String> {
    for vb in std::mem::take(&mut body.verilog) {
        match crate::veriloga::lower(&vb)? {
            crate::veriloga::Lowered::Cosim { vvp, ports } => {
                let bridges = [
                    ("adc_buf", "adc_bridge", "in_low=0.5 in_high=0.5"),
                    ("dac_buf", "dac_bridge", "out_low=0.0 out_high=1.8"),
                ];
                for (name, kind, params) in bridges {
                    let kind_of = |m: &String| m.split_whitespace().nth(2) == Some(kind);
                    if !body.models.iter().any(kind_of) {
                        body.models.push(format!(".model {name} {kind} ({params})"));
                    }
                }
                let model = format!("cosim_{}", vb.instance_name);
                body.raw.push(format!(
                    ".model {model} d_cosim(delay=1n simulation_id=\"{vvp}\")"
                ));
                let text = format!("A{} {} {model}", vb.instance_name, ports.join(" "));
                let mut card = Card::element(&vb.instance_name, text);
                card.needs = need::XSPICE;
                body.elements.push(card);
            }
            crate::veriloga::Lowered::Gates {
                spice_models,
                cells,
            } => {
                body.includes.push(spice_models);
                for c in cells {
                    let text = format!("X{} {} {}", c.name, c.nodes.join(" "), c.cell);
                    body.instances.push(Card::element(&c.name, text));
                }
            }
        }
    }
    Ok(())
}
