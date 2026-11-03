//! The circuit's definitions: the top, every `.subckt` (ports, parameters,
//! locals, components, instances, models, raw lines) and library section,
//! model libraries, and the identifier walk a rename runs over them.
//!
//! Cites: none (SpiceRack's own IR; JSON in `schema/circuit-ir.schema.json`).
//!
//! Contents:
//! - `pub struct CircuitIR`, `pub struct LibSection`, `pub struct FuncDef`, `pub struct Subcircuit`
//! - `pub struct Conditional`, `pub struct Branch`, `Subcircuit::with_branches(&self) -> Vec<&Subcircuit>`
//! - `pub struct VerilogBlock`, `pub enum VerilogMode`, `pub enum VerilogConnection`
//! - `pub struct Port`, `pub enum PortDirection`, `pub struct ParamDef`
//! - `pub struct Instance`, `pub enum Element<'a>`, `pub struct ModelDef`, `pub struct ModelLibrary`
//! - `Subcircuit::elements(&self) -> Vec<Element<'_>>`
//! - `CircuitIR::subcircuits(&self) -> impl Iterator<Item = &Subcircuit>`
//! - `CircuitIR::names_mut(&mut self, f: impl FnMut(&mut String))`

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
    /// Library-section definitions, in file order: what a PDK model file
    /// holds per corner. A deck selects one by `(file, section)` in
    /// [`Subcircuit::libs`].
    #[serde(default)]
    pub sections: Vec<LibSection>,
}

/// One library section: SPICE `.lib <name>` … `.endl <name>`, Spectre
/// `section <name>` … `endsection` (bare or inside `library <lib>` …
/// `endlibrary`), VACASK `section <name>` … `endsection`. A section holds
/// definitions, so it is a [`Subcircuit`] body (`name` is the section's; no
/// ports) plus the `.subckt`s defined in it, each read as the same card
/// would be at the top of a deck. Sections do not nest in any dialect.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibSection {
    /// The Spectre `library` the section sits in; `None` for a bare one
    /// (SPICE's, VACASK's, or a Spectre section outside a library). A field
    /// per section, not a library table: SPICE has no library level and
    /// Spectre's `include "f" section=s` never names the library, so the
    /// name exists only to be written back. Consecutive sections of one
    /// library are written as one `library` block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<String>,
    /// `.param`s (`parameters`), models, `.include`s, `.lib` calls,
    /// elements and raw lines, as in a deck's top.
    pub body: Subcircuit,
    /// The `.subckt`s defined in the section; nested ones hoisted as at the
    /// top of a deck.
    #[serde(default)]
    pub subcircuit_defs: Vec<Subcircuit>,
    /// The section's functions (Spectre `real f(...)`, SPICE `.func`),
    /// visible to the files that include it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
    /// A definition's own `.param`s (SPICE `.param` inside a `.subckt`):
    /// computed in the body from `parameters`, never set by an instance. In
    /// card order, since a later one may read an earlier one; written before
    /// the models and elements that read them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locals: Vec<ParamDef>,
    /// Spectre `statistics` blocks, as the Spectre importer reads them; on a
    /// deck's top or a library section's body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statistics: Vec<Statistics>,
    /// Conditional blocks, each placed by its `at` among the components:
    /// each picks the cards of its first branch whose condition holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditionals: Vec<Conditional>,
}

/// A conditional block: ngspice and HSPICE `.if (c)` ... `.elseif (c)` ...
/// `.else` ... `.endif` (ngspice manual §2.15), Spectre's structural `if
/// (c) { ... } else if (c) { ... } else { ... }` (Spectre Reference 19.1
/// p.491), VACASK `@if c` ... `@elseif c` ... `@else` ... `@end`. The
/// conditions are evaluated when the hierarchy is flattened, over the
/// parameters in scope.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Conditional {
    pub branches: Vec<Branch>,
    /// Where the block sat among its body's components, as for
    /// [`Instance::at`]: card order is matrix order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u32>,
}

/// One branch of a [`Conditional`]: its condition (`None`: the `else`), an
/// expression as written, and the cards it selects (a body with no name or
/// ports; it may hold further conditionals).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    pub body: Subcircuit,
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
    Conditional(&'a Conditional),
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

impl Subcircuit {
    /// This body and every conditional branch's body in it, depth first.
    pub fn with_branches(&self) -> Vec<&Subcircuit> {
        let mut out = vec![self];
        for c in &self.conditionals {
            for b in &c.branches {
                out.extend(b.body.with_branches());
            }
        }
        out
    }

    /// Components and instances merged in card order (each instance before
    /// the component it preceded).
    pub fn elements(&self) -> Vec<Element<'_>> {
        let n = self.components.len();
        let at = |a: Option<u32>| a.map_or(n, |a| (a as usize).min(n));
        // Instances, then conditional blocks, at each place (stable sorts:
        // same-place ones keep their order).
        let mut placed: Vec<(usize, Element<'_>)> = self
            .instances
            .iter()
            .map(|i| (at(i.at), Element::Instance(i)))
            .collect();
        placed.extend(
            self.conditionals
                .iter()
                .map(|c| (at(c.at), Element::Conditional(c))),
        );
        placed.sort_by_key(|(k, e)| (*k, matches!(e, Element::Conditional(_))));
        let mut placed = placed.into_iter().peekable();
        let mut out = Vec::with_capacity(n + self.instances.len() + self.conditionals.len());
        for (k, c) in self.components.iter().enumerate() {
            while let Some((_, e)) = placed.next_if(|(p, _)| *p <= k) {
                out.push(e);
            }
            out.push(Element::Component(c));
        }
        out.extend(placed.map(|(_, e)| e));
        out
    }
}

impl CircuitIR {
    /// Every body the deck writes cards from: the top, its definitions,
    /// each section's body and definitions, and the conditional branches in
    /// each.
    pub fn subcircuits(&self) -> impl Iterator<Item = &Subcircuit> {
        let sections = self
            .sections
            .iter()
            .flat_map(|s| std::iter::once(&s.body).chain(&s.subcircuit_defs));
        std::iter::once(&self.top)
            .chain(&self.subcircuit_defs)
            .chain(sections)
            .flat_map(Subcircuit::with_branches)
    }

    /// Calls `f` on every identifier, for renaming: definition names (the
    /// top's is a title and a section's what other files select it by: both
    /// skipped), ports, model names, instance names, the subcircuits they
    /// call and their port mappings, every [`Component::names_mut`] in the
    /// bodies and the stimulus, and the globals. Expressions, saves and
    /// analyses are not visited.
    pub fn names_mut(&mut self, mut f: impl FnMut(&mut String)) {
        let section_defs = self
            .sections
            .iter_mut()
            .flat_map(|s| &mut s.subcircuit_defs);
        self.subcircuit_defs
            .iter_mut()
            .chain(section_defs)
            .for_each(|sc| f(&mut sc.name));
        let sections = self
            .sections
            .iter_mut()
            .flat_map(|s| std::iter::once(&mut s.body).chain(&mut s.subcircuit_defs));
        for sc in std::iter::once(&mut self.top)
            .chain(&mut self.subcircuit_defs)
            .chain(sections)
        {
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
