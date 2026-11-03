use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::*;

/// Backend-neutral intermediate representation for a complete simulation
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CircuitIR {
    pub top: Subcircuit,
    pub testbench: Option<Testbench>,
    pub subcircuit_defs: Vec<Subcircuit>,
    pub model_libraries: Vec<ModelLibrary>,
    /// The dialect the `raw_spice` lines are written in, when known (the
    /// importer sets it: "ngspice", "hspice"). Such lines reach only that
    /// dialect; `None` means generic SPICE, which every SPICE dialect takes.
    #[serde(default)]
    pub raw_dialect: Option<Dialect>,
    /// `.global` nodes: one net under the same name in every subcircuit.
    #[serde(default)]
    pub globals: Vec<String>,
    /// Top-level `.func` definitions: `name(args) = body`, the body an
    /// expression over the arguments and parameters.
    #[serde(default)]
    pub functions: Vec<FuncDef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuncDef {
    pub name: String,
    pub args: Vec<String>,
    pub body: String,
}

/// Named, parameterized, composable circuit fragment -- the unit of reuse
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Subcircuit {
    pub name: String,
    pub ports: Vec<Port>,
    pub parameters: Vec<ParamDef>,
    pub components: Vec<Component>,
    pub instances: Vec<Instance>,
    pub models: Vec<ModelDef>,
    pub raw_spice: Vec<String>,
    pub includes: Vec<String>,
    pub libs: Vec<(String, String)>,
    pub osdi_loads: Vec<String>,
    #[serde(default)]
    pub verilog_blocks: Vec<VerilogBlock>,
}

/// Verilog co-simulation or synthesis block stored in the IR
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerilogBlock {
    pub source: String,
    pub mode: VerilogMode,
    pub instance_name: String,
    pub connections: BTreeMap<String, VerilogConnection>,
    pub pdk: Option<String>,
    pub liberty: Option<String>,
    pub spice_models: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VerilogMode {
    Simulate,
    Synthesize,
}

/// A Verilog port connection: single net or a bus (vector of nets)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VerilogConnection {
    Single(String),
    Bus(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Port {
    pub name: String,
    pub direction: PortDirection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PortDirection {
    InOut,
    Input,
    Output,
}

/// Parameter definition with optional default
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamDef {
    pub name: String,
    pub default: Option<String>,
}

/// Reference to a Subcircuit definition + port mapping + parameter overrides
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub subcircuit: String,
    pub port_mapping: Vec<String>,
    pub parameters: Vec<(String, String)>,
    /// Where the instance sat among its subcircuit's components, as a reader
    /// saw it: before `components[at]`; `None` after them all. A simulator's
    /// matrix order follows card order, so a deck written back keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u32>,
}

/// One element of a subcircuit, in card order ([`Subcircuit::elements`]).
pub enum Element<'a> {
    Component(&'a Component),
    Instance(&'a Instance),
}

impl Subcircuit {
    /// Components and instances merged in card order (each instance before
    /// the component it preceded).
    pub fn elements(&self) -> Vec<Element<'_>> {
        let n = self.components.len();
        let pos = |i: &Instance| i.at.map_or(n, |a| (a as usize).min(n));
        let mut insts: Vec<&Instance> = self.instances.iter().collect();
        insts.sort_by_key(|i| pos(i)); // stable: same-place instances keep their order
        let mut insts = insts.into_iter().peekable();
        let mut out = Vec::with_capacity(n + self.instances.len());
        for (k, c) in self.components.iter().enumerate() {
            while let Some(i) = insts.next_if(|i| pos(i) <= k) {
                out.push(Element::Instance(i));
            }
            out.push(Element::Component(c));
        }
        out.extend(insts.map(Element::Instance));
        out
    }
}

/// Model definition
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelDef {
    pub name: String,
    pub kind: String,
    pub parameters: Vec<(String, String)>,
}

/// Opaque PDK/model library reference
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelLibrary {
    pub name: String,
    pub path: String,
    pub corner: Option<String>,
    pub backend_paths: BTreeMap<String, String>,
    #[serde(default)]
    pub setup_includes: Vec<String>,
}

impl CircuitIR {
    /// Calls `f` on every identifier, for renaming: definition names (the
    /// top's is a title and is skipped), ports, model names, instance names,
    /// the subcircuits they call and their port mappings, every
    /// [`Component::names_mut`] in the bodies and the stimulus, and the
    /// globals. Expressions, saves and analyses are not visited.
    pub fn names_mut(&mut self, mut f: impl FnMut(&mut String)) {
        self.subcircuit_defs
            .iter_mut()
            .for_each(|sc| f(&mut sc.name));
        for sc in std::iter::once(&mut self.top).chain(&mut self.subcircuit_defs) {
            sc.ports.iter_mut().for_each(|p| f(&mut p.name));
            sc.models.iter_mut().for_each(|m| f(&mut m.name));
            for i in &mut sc.instances {
                f(&mut i.name);
                f(&mut i.subcircuit);
                i.port_mapping.iter_mut().for_each(&mut f);
            }
            sc.components
                .iter_mut()
                .flat_map(Component::names_mut)
                .for_each(&mut f);
        }
        self.testbench
            .iter_mut()
            .flat_map(|tb| &mut tb.stimulus)
            .flat_map(Component::names_mut)
            .for_each(&mut f);
        self.globals.iter_mut().for_each(f);
    }
}
