//! Verilog: a `verilog(...)` call → an `ir::VerilogBlock`, and each block
//! → what simulates it (a `d_cosim` XSPICE instance, or synthesized
//! standard cells). Cites ngspice's `d_cosim` and `adc_bridge`/`dac_bridge`
//! code models.
//!
//! Contents:
//! - `pub(super) fn verilog_block(py, source, mode, instance_name, connections, pdk, liberty, spice_models) -> PyResult<VerilogBlock>`
//! - `pub(super) fn lower_verilog(sub: &mut ir::Subcircuit) -> Result<(), String>`

use std::collections::HashMap;

use pyo3::prelude::*;

use crate::ir::{self, Component};

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
) -> PyResult<crate::ir::VerilogBlock> {
    use crate::ir::{VerilogConnection, VerilogMode};
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
    Ok(crate::ir::VerilogBlock {
        source: source.to_string(),
        mode,
        instance_name: instance_name.to_string(),
        connections: conns,
        pdk: pdk.map(str::to_string),
        liberty: liberty.map(str::to_string),
        spice_models: spice_models.map(str::to_string),
    })
}

/// Expand each Verilog block of `sub` into what simulates it: a `d_cosim`
/// XSPICE instance (simulate) or the synthesized standard-cell instances
/// (synthesize).
pub(super) fn lower_verilog(sub: &mut ir::Subcircuit) -> Result<(), String> {
    for vb in std::mem::take(&mut sub.verilog_blocks) {
        match crate::veriloga::lower(&vb)? {
            crate::veriloga::Lowered::Cosim { vvp, ports } => {
                let bridges = [
                    (
                        "adc_buf",
                        "adc_bridge",
                        [("in_low", "0.5"), ("in_high", "0.5")],
                    ),
                    (
                        "dac_buf",
                        "dac_bridge",
                        [("out_low", "0.0"), ("out_high", "1.8")],
                    ),
                ];
                for (name, kind, params) in bridges {
                    if !sub.models.iter().any(|m| m.kind == kind) {
                        sub.models.push(ir::ModelDef {
                            name: name.into(),
                            kind: kind.into(),
                            parameters: params
                                .iter()
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .collect(),
                        });
                    }
                }
                let model = format!("cosim_{}", vb.instance_name);
                sub.raw_spice.push(format!(
                    ".model {model} d_cosim(delay=1n simulation_id=\"{vvp}\")"
                ));
                sub.components.push(Component::Xspice {
                    name: vb.instance_name.clone(),
                    connections: ports,
                    model,
                });
            }
            crate::veriloga::Lowered::Gates {
                spice_models,
                cells,
            } => {
                sub.includes.push(spice_models);
                sub.instances
                    .extend(cells.into_iter().map(|c| ir::Instance {
                        name: c.name,
                        subcircuit: c.cell,
                        port_mapping: c.nodes,
                        parameters: vec![],
                        at: None,
                    }));
            }
        }
    }
    Ok(())
}
