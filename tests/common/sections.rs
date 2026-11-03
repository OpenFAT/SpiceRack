//! A library section's meaning, for comparing sections across dialects
//! (`test_sections.rs`, `test_sections_pdk.rs`): values as numbers when they
//! are one, else without SPICE's delimiters; names lowercased; model
//! parameters sorted, without LEVEL and `type`; a definition's own `.param`s
//! after its parameters (Spectre and VACASK have no other kind).
//!
//! Contents: `pub struct Meaning`, `pub type Pairs`,
//! `pub fn meaning(ir: &CircuitIR) -> Vec<Meaning>`.

use spicerack::ir::*;

/// One section's content as the simulator reads it, the same in every
/// dialect's spelling: values as numbers when they are one, else without
/// SPICE's delimiters; names lowercased; model parameters sorted, without
/// LEVEL (the master Spectre picks by it) and `type`.
#[derive(Debug, PartialEq)]
pub struct Meaning {
    pub name: String,
    pub params: Pairs,
    pub includes: Vec<String>,
    pub libs: Pairs,
    /// Name, kind, parameters.
    pub models: Vec<(String, String, Pairs)>,
    /// Name, ports, parameters, element count.
    pub subckts: Vec<(String, Vec<String>, Pairs, usize)>,
}

pub type Pairs = Vec<(String, String)>;

pub fn meaning(ir: &CircuitIR) -> Vec<Meaning> {
    let value = |v: &str| {
        let t = v.trim().trim_matches(|c| c == '\'' || c == '{' || c == '}');
        let (sign, mag) = t.strip_prefix('-').map_or((1.0, t), |m| (-1.0, m));
        match spicerack::unit::parse_spice_number(mag) {
            Some((x, end, scale)) if end + scale.len() == mag.len() => format!("{:e}", sign * x),
            _ => t.to_ascii_lowercase(),
        }
    };
    let pairs = |ps: &[(String, String)]| -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = ps
            .iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("level") && !k.eq_ignore_ascii_case("type"))
            .map(|(k, x)| (k.to_ascii_lowercase(), value(x)))
            .collect();
        v.sort();
        v
    };
    let defs = |ps: &[ParamDef]| -> Vec<(String, String)> {
        ps.iter()
            .map(|p| {
                (
                    p.name.to_ascii_lowercase(),
                    p.default.as_deref().map_or(String::new(), value),
                )
            })
            .collect()
    };
    ir.sections
        .iter()
        .map(|s| Meaning {
            name: s.body.name.clone(),
            params: defs(&s.body.parameters),
            includes: s.body.includes.clone(),
            libs: s.body.libs.clone(),
            models: s
                .body
                .models
                .iter()
                .map(|m| {
                    (
                        m.name.to_ascii_lowercase(),
                        m.kind.to_ascii_lowercase(),
                        pairs(&m.parameters),
                    )
                })
                .collect(),
            subckts: s
                .subcircuit_defs
                .iter()
                .map(|sc| {
                    (
                        sc.name.clone(),
                        sc.ports.iter().map(|p| p.name.clone()).collect(),
                        defs(&[&sc.parameters[..], &sc.locals[..]].concat()),
                        sc.components.len() + sc.instances.len(),
                    )
                })
                .collect(),
        })
        .collect()
}
