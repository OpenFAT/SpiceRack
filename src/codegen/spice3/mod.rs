//! SPICE3-family netlists: ngspice, LTspice, HSPICE, and ESPice (ngspice
//! text with Verilog-A loaded as `.hdl`), and the analysis cards of a Xyce
//! deck. Cites the ngspice manual, the Xyce Reference Guide (RG) and the
//! behaviour of the simulators named beside each card.
//!
//! This file is the deck spine (`netlist`) and the block-level cards (loads,
//! libraries, models, subcircuits, options); `values` spells numbers,
//! `waveform` source waveforms, `components` cards, `analyses` analysis and
//! `.step` cards, `control` the ngspice `.control` block, `sections` the
//! `.param` cards and library sections.
//!
//! Contents:
//! - `pub(crate) const NGBEHAVIOR: &str`
//! - `pub(super) fn netlist(d: Dialect, ir: &CircuitIR) -> Result<String, CodeGenError>`
//! - re-exported: `components::component`, `control::{is_four, is_meas, runs_in_control}`

mod analyses;
mod components;
mod control;
mod sections;
mod values;
mod waveform;

use crate::ir::*;

use super::{CodeGenError, Dialect, param_defs, params};
use analyses::{analysis, step_param};
pub(crate) use components::component;
use control::{control_run, raw_lines};
use sections::{param_cards, params_and_models, section};
// The ngspice backend reads its own deck back with these.
#[cfg(feature = "backend")]
pub(crate) use control::{is_four, is_meas, runs_in_control};
use values::{dec, key, ngspice, num, spelled};

/// The comment that carries `Testbench::ngbehavior` in an ngspice deck.
/// ngspice reads its compatibility mode only before parsing (no card sets
/// it), so the backend lifts this line onto its command line and the deck
/// text stays the whole input of a run.
pub(crate) const NGBEHAVIOR: &str = "*spicerack ngbehavior=";

/// The whole deck for `d`, ending in `.end` with no line end after it (an
/// HSPICE deck with no analysis, a library, ends without one).
pub(super) fn netlist(d: Dialect, ir: &CircuitIR) -> Result<String, CodeGenError> {
    check_raw_dialect(d, ir)?;
    // Numbers read from an ngspice deck carry ngspice's arithmetic: written
    // back for ngspice, each is spelled to land on the same double. Others
    // (Python's `3.3`) keep their decimal spelling.
    let x = ir.raw_dialect.map(Dialect::family) == Some(Dialect::Ngspice);
    let control = match &ir.testbench {
        Some(tb) if d == Dialect::Ngspice => control_run(x, ir, tb),
        _ => None,
    };
    let drop_four = control.is_some();
    // ponytail: ~64 bytes a card is a guess; the String grows past it.
    let cards = ir
        .subcircuits()
        .map(|s| s.components.len() + s.instances.len())
        .sum::<usize>();
    let mut out = String::with_capacity(64 * (cards + 16));
    // Title must be first line (SPICE treats line 1 as title/comment)
    w!(out, "* {}\n", ir.top.name);
    let mode = ir.testbench.as_ref().and_then(|tb| tb.ngbehavior.as_ref());
    if let (Dialect::Ngspice, Some(mode)) = (d, mode) {
        w!(out, "{NGBEHAVIOR}{mode}\n");
    }
    loads(&mut out, d, ir)?;

    if !ir.globals.is_empty() {
        w!(out, ".global {}\n", ir.globals.join(" "));
    }
    // HSPICE spells a function as a parameter: `.param f(x)='x*2'`.
    for f in &ir.functions {
        let args = f.args.join(", ");
        match d {
            Dialect::Hspice => w!(out, ".param {}({args})='{}'\n", f.name, f.body),
            _ => w!(out, ".func {}({args}) {{{}}}\n", f.name, f.body),
        }
    }
    file_cards(&mut out, d, &ir.top)?;
    let q = quote(d);

    for lib in &ir.model_libraries {
        for s in &lib.setup_includes {
            w!(out, ".include {q}{s}{q}\n");
        }
        let path = lib.backend_paths.get(key(d)).unwrap_or(&lib.path);
        match &lib.corner {
            Some(corner) => w!(out, ".lib {q}{path}{q} {corner}\n"),
            None => w!(out, ".include {q}{path}{q}\n"),
        }
    }

    params_and_models(&mut out, d, &ir.top);

    for sc in &ir.subcircuit_defs {
        out.push('\n');
        subcircuit(&mut out, d, x, sc, drop_four)?;
    }
    for s in &ir.sections {
        out.push('\n');
        section(&mut out, d, x, s, drop_four)?;
    }

    body(&mut out, d, x, &ir.top, drop_four)?;

    if let Some(ref tb) = ir.testbench {
        for comp in &tb.stimulus {
            component(&mut out, d, x, comp)?;
            out.push('\n');
        }

        if d == Dialect::Xyce {
            for l in xyce_options(tb)? {
                w!(out, "{l}\n");
            }
        } else {
            let opts = super::option_pairs(&tb.options, key(d), itl1);
            if !opts.is_empty() {
                w!(out, ".options {}\n", opts.join(" "));
            }
            if let Some(temp) = tb.temperature {
                w!(out, ".temp {}\n", dec(d, x, temp));
            }
            if let Some(tnom) = tb.nominal_temperature {
                w!(out, ".options tnom={}\n", dec(d, x, tnom));
            }
        }
        for (n, v) in &tb.initial_conditions {
            w!(out, ".ic V({})={}\n", n, dec(d, x, *v));
        }
        for (n, v) in &tb.node_sets {
            w!(out, ".nodeset V({})={}\n", n, dec(d, x, *v));
        }

        // A noise run produces `onoise_spectrum`/`inoise_spectrum`, not node
        // voltages, so a node-name save list can never name them. ngspice then
        // reports "no data saved for Noise analysis; analysis not run" and the
        // run fails outright. Saving everything is the only correct list here;
        // LTspice has no `.save all` and saves everything by default, so it
        // just gets no save list.
        // ngspice reads one `.save` card: `all` beside named outputs means
        // all of them plus those, and alone it is what no list means.
        let noise = tb
            .analyses
            .iter()
            .any(|a| matches!(a, Analysis::Noise { .. }));
        // Xyce's `.save` writes the operating point; its rawfile (-r) holds
        // every vector, so a save list has nothing to restrict there.
        if d == Dialect::Xyce {
        } else if d == Dialect::Hspice && !noise {
            // HSPICE's `.save` saves simulator state; `.probe` selects outputs.
            for s in &tb.saves {
                if s == "all" {
                    out.push_str(".option post\n")
                } else {
                    w!(out, ".probe {s}\n")
                }
            }
        } else if ngspice(d) && noise {
            out.push_str(".save all");
            for s in tb.saves.iter().filter(|s| *s != "all") {
                w!(out, " {s}");
            }
            out.push('\n');
        } else if !tb.saves.is_empty() && !noise {
            w!(out, ".save {}\n", tb.saves.join(" "));
        }
        // `.print` prints to stdout; LTspice has no such card, and a rawfile
        // run ignores it, so there it is omitted.
        if d != Dialect::Ltspice {
            for p in &tb.prints {
                let sep = if p.analysis.is_empty() { "" } else { " " };
                w!(out, ".print{sep}{} {}\n", p.analysis, p.outputs.join(" "));
            }
        }

        for meas in &tb.measures {
            // `.MEASURE TRAN ...` is already a card, in any case.
            let card = meas
                .get(..5)
                .is_some_and(|p| p.eq_ignore_ascii_case(".meas"));
            w!(out, "{}{meas}\n", if card { "" } else { ".meas " });
        }
        for sp in &tb.step_params {
            w!(out, "{}\n", step_param(d, sp)?);
        }
        raw_lines(&mut out, &tb.extra_lines, drop_four);
        match &control {
            Some(fourier) => {
                // `fourier` reads the current plot: it follows the first
                // transient (at the end of the block when there is none).
                let at = tb
                    .analyses
                    .iter()
                    .position(|a| matches!(a, Analysis::Transient { .. }));
                let four = |out: &mut String| fourier.iter().for_each(|f| w!(out, "{f}\n"));
                out.push_str(".control\n");
                for (i, a) in tb.analyses.iter().enumerate() {
                    // As a command, `.four` would run the transient again.
                    if !matches!(a, Analysis::Fourier { .. }) {
                        // `.tran 1u 1m` -> bare `tran 1u 1m` inside the block.
                        w!(out, "{}\n", analysis(d, x, a)?.trim_start_matches('.'));
                    }
                    if at == Some(i) {
                        four(&mut out);
                    }
                }
                if at.is_none() {
                    four(&mut out);
                }
                out.push_str(".endc\n");
            }
            None => {
                for a in &tb.analyses {
                    w!(out, "{}\n", analysis(d, x, a)?);
                }
            }
        }
    }

    // HSPICE's `.END` ends the run, even in a file read by `.include` or
    // `.lib` (hspice.peg L9; ESPice stops there): a deck with no analysis is
    // a library, and ends without one.
    let runs = ir
        .testbench
        .as_ref()
        .is_some_and(|tb| !tb.analyses.is_empty());
    if d == Dialect::Hspice && !runs {
        out.pop();
    } else {
        out.push_str(".end");
    }
    Ok(out)
}

// ---- private ----

/// Raw lines imported from one dialect reach only that dialect (ngspice and
/// ESPice read the same text): ngspice's TRNOISE or `.probe` copied into an
/// HSPICE deck would be a silently different deck.
fn check_raw_dialect(d: Dialect, ir: &CircuitIR) -> Result<(), CodeGenError> {
    let Some(from) = ir.raw_dialect else {
        return Ok(());
    };
    let raw = ir.subcircuits().find_map(|s| {
        s.raw_spice.first().or_else(|| {
            s.components.iter().find_map(|c| match c {
                Component::RawSpice { line } => Some(line),
                _ => None,
            })
        })
    });
    match raw {
        Some(line) if from.family() != d.family() => Err(CodeGenError::UnsupportedComponent {
            backend: d.name().into(),
            component: format!(
                "{} line '{}' (kept verbatim on import; it is {} text)",
                from.name(),
                line.trim(),
                from.name()
            ),
        }),
        _ => Ok(()),
    }
}

/// Model loads that must precede any card instantiating them: ngspice's
/// `pre_osdi` control block, ESPice's and HSPICE's `.hdl` (they compile
/// Verilog-A themselves and load no OSDI binaries), nothing for LTspice.
fn loads(out: &mut String, d: Dialect, ir: &CircuitIR) -> Result<(), CodeGenError> {
    match d {
        Dialect::Ngspice if !ir.top.osdi_loads.is_empty() => {
            let loads =
                crate::veriloga::osdi_loads(&ir.top.osdi_loads).map_err(CodeGenError::Other)?;
            // pre_osdi is a control command, not a dot command
            out.push_str(".control\n");
            for p in &loads {
                w!(out, "pre_osdi {p}\n");
            }
            out.push_str(".endc\n");
        }
        Dialect::Espice => {
            let all = ir
                .top
                .osdi_loads
                .iter()
                .chain(ir.subcircuit_defs.iter().flat_map(|s| &s.osdi_loads));
            let mut seen: Vec<&str> = Vec::new();
            for l in all {
                if !crate::veriloga::is_source(l) {
                    return Err(CodeGenError::Other(format!(
                        "ESPice does not load OSDI binaries ({l}); load the Verilog-A source with veriloga(\"model.va\")"
                    )));
                }
                if !seen.contains(&l.as_str()) {
                    seen.push(l);
                    w!(out, ".hdl \"{l}\"\n");
                }
            }
        }
        Dialect::Xyce if !ir.top.osdi_loads.is_empty() => {
            return Err(CodeGenError::Other(
                "Xyce loads Verilog-A through -plugin, not a deck card".into(),
            ));
        }
        Dialect::Hspice => {
            for l in &ir.top.osdi_loads {
                if !crate::veriloga::is_source(l) {
                    return Err(CodeGenError::Other(format!(
                        "HSPICE loads Verilog-A source, not OSDI ({l}); use veriloga(\"model.va\")"
                    )));
                }
                w!(out, ".hdl \"{l}\"\n");
            }
        }
        _ => {}
    }
    Ok(())
}

/// A block's `.include` and `.lib` cards, in the block: SPICE scopes a
/// `.lib` inside a `.subckt` to that subcircuit (ngspice's
/// lib-processing/ex1b), so hoisting one changes which models it sees.
/// HSPICE quotes paths (`.lib 'file' section`).
/// A library with no section is LTspice's `.lib file` (the whole file);
/// ngspice and HSPICE would read that card as a section definition.
fn file_cards(out: &mut String, d: Dialect, sc: &Subcircuit) -> Result<(), CodeGenError> {
    let q = quote(d);
    for inc in &sc.includes {
        w!(out, ".include {q}{inc}{q}\n");
    }
    for (p, section) in &sc.libs {
        match (section.is_empty(), d) {
            (true, Dialect::Ltspice) => w!(out, ".lib {q}{p}{q}\n"),
            (true, _) => {
                return Err(CodeGenError::Other(format!(
                    ".lib {p} with no section is LTspice's whole-file form; {d:?} reads it as a section definition"
                )));
            }
            (false, _) => w!(out, ".lib {q}{p}{q} {section}\n"),
        }
    }
    Ok(())
}

/// The quote around a file path: HSPICE's `'file'`, nothing elsewhere.
fn quote(d: Dialect) -> &'static str {
    if d == Dialect::Hspice { "'" } else { "" }
}

/// Xyce rejects BSIM3 as level 8 and reads 49 (RG Table 2-36; 7.10 run).
/// The parameter list is separated from the type by a space: ngspice-45
/// silently drops the parameters of an OSDI (Verilog-A) model written
/// `vres(r=3k)`, while `vres (r=3k)` applies them; built-in types read both.
fn model(out: &mut String, d: Dialect, m: &ModelDef) {
    let mos = m.kind.eq_ignore_ascii_case("nmos") || m.kind.eq_ignore_ascii_case("pmos");
    w!(out, ".model {} {}", m.name, m.kind);
    for (i, (k, v)) in m.parameters.iter().enumerate() {
        let v = if d == Dialect::Xyce && mos && k.eq_ignore_ascii_case("level") && v.trim() == "8" {
            "49"
        } else {
            v
        };
        let open = if i == 0 { " (" } else { " " };
        if v.is_empty() {
            w!(out, "{open}{k}")
        } else {
            w!(out, "{open}{k}={}", spelled(d)(v))
        }
    }
    if !m.parameters.is_empty() {
        out.push(')');
    }
}

fn subcircuit(
    out: &mut String,
    d: Dialect,
    x: bool,
    sc: &Subcircuit,
    drop_four: bool,
) -> Result<(), CodeGenError> {
    w!(out, ".subckt {}", sc.name);
    for port in &sc.ports {
        w!(out, " {}", port.name);
    }
    // HSPICE takes the defaults bare after the ports.
    if !sc.parameters.is_empty() && d != Dialect::Hspice {
        out.push_str(" PARAMS:");
    }
    param_defs(out, &sc.parameters, spelled(d));
    out.push('\n');
    file_cards(out, d, sc)?;
    param_cards(out, d, &sc.locals);
    for m in &sc.models {
        model(out, d, m);
        out.push('\n');
    }
    body(out, d, x, sc, drop_four)?;
    w!(out, ".ends {}\n", sc.name);
    Ok(())
}

/// Components, instances, raw text — the part of a block after its models,
/// one line each.
fn body(
    out: &mut String,
    d: Dialect,
    x: bool,
    sc: &Subcircuit,
    drop_four: bool,
) -> Result<(), CodeGenError> {
    for e in sc.elements() {
        match e {
            Element::Component(c) => component(out, d, x, c)?,
            Element::Instance(i) => instance(out, d, i),
        }
        out.push('\n');
    }
    raw_lines(out, &sc.raw_spice, drop_four);
    Ok(())
}

fn instance(out: &mut String, d: Dialect, inst: &Instance) {
    w!(out, "X{}", inst.name);
    for port in &inst.port_mapping {
        w!(out, " {port}");
    }
    w!(
        out,
        " {}{}",
        inst.subcircuit,
        params(&inst.parameters, spelled(d))
    );
}

/// Xyce's options: `.options <pkg> k=v` per package (RG pp.96-104). The
/// importer keeps Xyce's own as `pkg.key` under `backend_specific["xyce"]`;
/// portable ones map through the RG tables, and one with no Xyce spelling is
/// refused rather than guessed.
fn xyce_options(tb: &Testbench) -> Result<Vec<String>, CodeGenError> {
    // (package, `k=v` or bare `k`), in order; consecutive entries of one
    // package share a line, so a deck's own order reads back unchanged.
    let mut items: Vec<(String, String)> = Vec::new();
    let kv = |k: &str, v: &str| {
        if v.is_empty() {
            k.to_string()
        } else {
            format!("{k}={v}")
        }
    };
    for (k, v) in &tb.options.portable {
        let (pkg, key) = match k.to_ascii_lowercase().as_str() {
            "reltol" => ("timeint", "reltol"),
            "vntol" => ("nonlin", "abstol"),
            "gmin" => ("device", "gmin"),
            "tnom" => ("device", "tnom"),
            "temp" => ("device", "temp"),
            "itl1" | "max_iterations" => ("nonlin", "maxstep"),
            "itl4" => ("nonlin-tran", "maxstep"),
            other => {
                return Err(CodeGenError::UnsupportedAnalysis {
                    backend: "xyce".into(),
                    analysis: format!("option '{other}' (no Xyce package mapped)"),
                });
            }
        };
        items.push((pkg.into(), kv(key, v)));
    }
    if let Some(t) = tb.temperature {
        items.push(("device".into(), kv("temp", &num(Dialect::Xyce, false, t))));
    }
    if let Some(t) = tb.nominal_temperature {
        items.push(("device".into(), kv("tnom", &num(Dialect::Xyce, false, t))));
    }
    // `pkg.key`, a bare package `pkg.` (`.options post`), or a key with no
    // package (`.option scale=`, package ""), each kept as the deck wrote it.
    for (pk, v) in tb
        .options
        .backend_specific
        .get("xyce")
        .into_iter()
        .flatten()
    {
        match pk.split_once('.') {
            Some((pkg, "")) => items.push((pkg.into(), String::new())),
            Some((pkg, k)) => items.push((pkg.into(), kv(k, v))),
            None => items.push((String::new(), kv(pk, v))),
        }
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for (pkg, item) in items {
        match out.last_mut() {
            // A bare package (`.options linsol`) keeps its own line.
            Some((p, line)) if *p == pkg && !item.is_empty() && line != p => {
                line.push(' ');
                line.push_str(&item);
            }
            _ => out.push((
                pkg.clone(),
                if pkg.is_empty() {
                    item
                } else if item.is_empty() {
                    pkg
                } else {
                    format!("{pkg} {item}")
                },
            )),
        }
    }
    Ok(out
        .into_iter()
        .map(|(_, l)| format!(".options {l}"))
        .collect())
}

/// ngspice and LTspice share the SPICE3 option spelling; only the
/// iteration-limit alias needs mapping, and both call it `ITL1`.
fn itl1(k: &str) -> &str {
    if k == "max_iterations" { "ITL1" } else { k }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sample_resistor_divider() -> CircuitIR {
        CircuitIR {
            top: Subcircuit {
                name: "Voltage Divider".into(),
                components: vec![
                    Component::VoltageSource {
                        name: "in".into(),
                        np: "input".into(),
                        nm: "0".into(),
                        value: IrValue::Numeric { value: 10.0 },
                        ac_magnitude: None,
                        ac_phase: None,
                        waveform: None,
                        port: None,
                    },
                    Component::Resistor {
                        name: "1".into(),
                        n1: "input".into(),
                        n2: "output".into(),
                        value: IrValue::Numeric { value: 10000.0 },
                        model: None,
                        params: vec![],
                    },
                    Component::Resistor {
                        name: "2".into(),
                        n1: "output".into(),
                        n2: "0".into(),
                        value: IrValue::Numeric { value: 10000.0 },
                        model: None,
                        params: vec![],
                    },
                ],
                ..Default::default()
            },
            testbench: Some(Testbench {
                analyses: vec![Analysis::Op],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_ngspice_resistor_divider() {
        let ir = sample_resistor_divider();
        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(
            netlist.contains("* Voltage Divider"),
            "missing title: {}",
            netlist
        );
        assert!(
            netlist.contains("Vin input 0 10"),
            "missing Vin: {}",
            netlist
        );
        assert!(
            netlist.contains("R1 input output 10k"),
            "missing R1: {}",
            netlist
        );
        assert!(
            netlist.contains("R2 output 0 10k"),
            "missing R2: {}",
            netlist
        );
        assert!(netlist.contains(".op"), "missing .op: {}", netlist);
        assert!(netlist.contains(".end"), "missing .end: {}", netlist);
    }

    #[test]
    fn test_ltspice_resistor_divider() {
        let ir = sample_resistor_divider();
        let cg = Dialect::Ltspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(netlist.contains("* Voltage Divider"));
        assert!(netlist.contains("Vin input 0 10"));
        assert!(netlist.contains(".op"));
        assert!(netlist.contains(".end"));
        // LTspice has no OSDI loader
        assert!(!netlist.contains("pre_osdi"));
    }

    #[test]
    fn test_mosfet_circuit() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "CMOS Inverter".into(),
                components: vec![
                    Component::Mosfet {
                        name: "1".into(),
                        nd: "out".into(),
                        ng: "in".into(),
                        ns: "vdd".into(),
                        nb: Some("vdd".into()),
                        extra_nodes: vec![],
                        model: "pmos_3p3".into(),
                        params: vec![("W".into(), "2u".into()), ("L".into(), "180n".into())],
                    },
                    Component::Mosfet {
                        name: "2".into(),
                        nd: "out".into(),
                        ng: "in".into(),
                        ns: "0".into(),
                        nb: Some("0".into()),
                        extra_nodes: vec![],
                        model: "nmos_3p3".into(),
                        params: vec![("W".into(), "1u".into()), ("L".into(), "180n".into())],
                    },
                    Component::VoltageSource {
                        name: "dd".into(),
                        np: "vdd".into(),
                        nm: "0".into(),
                        value: IrValue::Numeric { value: 3.3 },
                        ac_magnitude: None,
                        ac_phase: None,
                        waveform: None,
                        port: None,
                    },
                ],
                models: vec![ModelDef {
                    name: "nmos_3p3".into(),
                    kind: "NMOS".into(),
                    parameters: vec![("VTO".into(), "0.7".into()), ("KP".into(), "110e-6".into())],
                }],
                ..Default::default()
            },
            testbench: Some(Testbench {
                analyses: vec![Analysis::Dc {
                    sweeps: vec![DcSweep {
                        source: "Vdd".into(),
                        start: 0.0,
                        stop: 3.3,
                        step: 0.01,
                    }],
                }],
                temperature: Some(27.0),
                ..Default::default()
            }),
            ..Default::default()
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(
            netlist.contains("M1 out in vdd vdd pmos_3p3 W=2u L=180n"),
            "missing M1: {}",
            netlist
        );
        assert!(
            netlist.contains("M2 out in 0 0 nmos_3p3 W=1u L=180n"),
            "missing M2: {}",
            netlist
        );
        assert!(
            netlist.contains(".model nmos_3p3 NMOS (VTO=0.7 KP=110e-6)"),
            "missing model: {}",
            netlist
        );
        assert!(netlist.contains(".temp 27"), "missing temp: {}", netlist);
        assert!(netlist.contains(".dc Vdd 0 3"), "missing dc: {}", netlist);
    }

    #[test]
    fn test_subcircuit_instances() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "Top".into(),
                components: vec![Component::VoltageSource {
                    name: "dd".into(),
                    np: "vdd".into(),
                    nm: "0".into(),
                    value: IrValue::Numeric { value: 1.8 },
                    ac_magnitude: None,
                    ac_phase: None,
                    waveform: None,
                    port: None,
                }],
                instances: vec![Instance {
                    name: "1".into(),
                    subcircuit: "inverter".into(),
                    port_mapping: vec!["in".into(), "out".into(), "vdd".into(), "0".into()],
                    parameters: vec![("wp".into(), "2u".into())],
                    ..Default::default()
                }],
                ..Default::default()
            },
            subcircuit_defs: vec![Subcircuit {
                name: "inverter".into(),
                ports: vec![
                    Port {
                        name: "in".into(),
                        direction: PortDirection::Input,
                    },
                    Port {
                        name: "out".into(),
                        direction: PortDirection::Output,
                    },
                    Port {
                        name: "vdd".into(),
                        direction: PortDirection::InOut,
                    },
                    Port {
                        name: "gnd".into(),
                        direction: PortDirection::InOut,
                    },
                ],
                parameters: vec![
                    ParamDef {
                        name: "wp".into(),
                        default: Some("1u".into()),
                    },
                    ParamDef {
                        name: "wn".into(),
                        default: Some("500n".into()),
                    },
                ],
                components: vec![
                    Component::Mosfet {
                        name: "p".into(),
                        nd: "out".into(),
                        ng: "in".into(),
                        ns: "vdd".into(),
                        nb: Some("vdd".into()),
                        extra_nodes: vec![],
                        model: "pmos".into(),
                        params: vec![("W".into(), "{wp}".into()), ("L".into(), "180n".into())],
                    },
                    Component::Mosfet {
                        name: "n".into(),
                        nd: "out".into(),
                        ng: "in".into(),
                        ns: "gnd".into(),
                        nb: Some("gnd".into()),
                        extra_nodes: vec![],
                        model: "nmos".into(),
                        params: vec![("W".into(), "{wn}".into()), ("L".into(), "180n".into())],
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(
            netlist.contains(".subckt inverter in out vdd gnd PARAMS: wp=1u wn=500n"),
            "missing subckt header: {}",
            netlist
        );
        assert!(
            netlist.contains("Mp out in vdd vdd pmos W={wp} L=180n"),
            "missing Mp: {}",
            netlist
        );
        assert!(
            netlist.contains(".ends inverter"),
            "missing .ends: {}",
            netlist
        );
        assert!(
            netlist.contains("X1 in out vdd 0 inverter wp=2u"),
            "missing instance: {}",
            netlist
        );
    }

    #[test]
    fn test_waveforms_sin_pulse_pwl() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "Waveforms".into(),
                components: vec![
                    Component::VoltageSource {
                        name: "sin".into(),
                        np: "sin_out".into(),
                        nm: "0".into(),
                        value: IrValue::Numeric { value: 0.0 },
                        ac_magnitude: None,
                        ac_phase: None,
                        waveform: Some(IrWaveform::Sin {
                            offset: 1.65.into(),
                            amplitude: 1.65.into(),
                            frequency: 1e6.into(),
                            delay: 0.0.into(),
                            damping: 0.0.into(),
                            phase: 0.0.into(),
                        }),
                        port: None,
                    },
                    Component::VoltageSource {
                        name: "pulse".into(),
                        np: "pulse_out".into(),
                        nm: "0".into(),
                        value: IrValue::Numeric { value: 0.0 },
                        ac_magnitude: None,
                        ac_phase: None,
                        waveform: Some(IrWaveform::Pulse {
                            initial: 0.0.into(),
                            pulsed: 3.3.into(),
                            delay: 0.0.into(),
                            rise_time: 1e-9.into(),
                            fall_time: 1e-9.into(),
                            pulse_width: 5e-7.into(),
                            period: 1e-6.into(),
                        }),
                        port: None,
                    },
                    Component::VoltageSource {
                        name: "pwl".into(),
                        np: "pwl_out".into(),
                        nm: "0".into(),
                        value: IrValue::Numeric { value: 0.0 },
                        ac_magnitude: None,
                        ac_phase: None,
                        waveform: Some(IrWaveform::Pwl {
                            values: vec![
                                ((0.0).into(), (0.0).into()),
                                ((1e-6).into(), (3.3).into()),
                                ((2e-6).into(), (0.0).into()),
                            ],
                            repeat: None,
                            delay: None,
                        }),
                        port: None,
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(
            netlist.contains("SIN(1.65 1.65 1000000"),
            "missing SIN: {}",
            netlist
        );
        assert!(
            netlist.contains("PULSE(0 3.3 0"),
            "missing PULSE: {}",
            netlist
        );
        assert!(netlist.contains("PWL(0 0"), "missing PWL: {}", netlist);
    }

    #[test]
    fn test_all_analysis_types() {
        let cg = Dialect::Ngspice;

        // Op
        assert_eq!(analysis(cg, false, &Analysis::Op).unwrap(), ".op");

        // DC
        let dc = Analysis::Dc {
            sweeps: vec![DcSweep {
                source: "V1".into(),
                start: 0.0,
                stop: 5.0,
                step: 0.1,
            }],
        };
        let dc_str = analysis(cg, false, &dc).unwrap();
        assert!(dc_str.starts_with(".dc V1"), "dc: {}", dc_str);

        // AC
        let ac = Analysis::Ac {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 100,
                start: 1.0,
                stop: 1e9,
            },
        };
        let ac_str = analysis(cg, false, &ac).unwrap();
        assert!(ac_str.starts_with(".ac dec 100"), "ac: {}", ac_str);

        // Transient
        let tran = Analysis::Transient {
            step: 1e-9,
            stop: 1e-6,
            start: None,
            max_step: None,
            uic: false,
        };
        let tran_str = analysis(cg, false, &tran).unwrap();
        assert!(tran_str.starts_with(".tran 1n"), "tran: {}", tran_str);

        // Transient with UIC
        let tran_uic = Analysis::Transient {
            step: 1e-9,
            stop: 1e-6,
            start: None,
            max_step: None,
            uic: true,
        };
        let tran_uic_str = analysis(cg, false, &tran_uic).unwrap();
        assert!(tran_uic_str.contains("UIC"), "tran_uic: {}", tran_uic_str);

        // Noise
        let noise = Analysis::Noise {
            output: "out".into(),
            reference: "0".into(),
            source: "V1".into(),
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1.0,
                stop: 1e6,
            },
            points_per_summary: None,
        };
        let noise_str = analysis(cg, false, &noise).unwrap();
        assert!(
            noise_str.starts_with(".noise V(out) V1 dec"),
            "noise: {}",
            noise_str
        );

        // TF
        let tf = Analysis::Tf {
            output: "V(out)".into(),
            source: "Vin".into(),
        };
        assert_eq!(analysis(cg, false, &tf).unwrap(), ".tf V(out) Vin");

        // Sensitivity
        let sens = Analysis::Sensitivity {
            output: "V(out)".into(),
            ac: None,
        };
        assert_eq!(analysis(cg, false, &sens).unwrap(), ".sens V(out)");

        // PoleZero
        let pz = Analysis::PoleZero {
            node1: "1".into(),
            node2: "0".into(),
            node3: "3".into(),
            node4: "0".into(),
            tf_type: PzTransfer::Vol,
            pz_type: PzKind::Pz,
        };
        assert_eq!(analysis(cg, false, &pz).unwrap(), ".pz 1 0 3 0 vol pz");

        // Distortion
        let disto = Analysis::Distortion {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1e3,
                stop: 1e6,
            },
            f2overf1: None,
        };
        let disto_str = analysis(cg, false, &disto).unwrap();
        assert!(
            disto_str.starts_with(".disto dec 10"),
            "disto: {}",
            disto_str
        );

        // Fourier
        let four = Analysis::Fourier {
            fundamental: 1e3,
            outputs: vec!["V(out)".into()],
            num_harmonics: Some(10),
        };
        // ngspice's `.four` card has no harmonic-count field: a count there is
        // read as an output vector and kills the analysis.
        let four_str = analysis(cg, false, &four).unwrap();
        assert_eq!(four_str, ".four 1k V(out)", "four: {}", four_str);

        // LTspice's card does take `[Nharmonics]`.
        let cg_lt = Dialect::Ltspice;
        assert_eq!(analysis(cg_lt, false, &four).unwrap(), ".four 1k 10 V(out)");
    }

    #[test]
    fn test_options_max_iterations_maps_to_itl1() {
        let opts = SimOptions {
            portable: vec![
                ("reltol".into(), "1e-3".into()),
                ("max_iterations".into(), "200".into()),
            ],
            backend_specific: BTreeMap::new(),
        };

        // Ngspice
        let cg_ng = Dialect::Ngspice;
        let ng_str = super::super::option_pairs(&opts, key(cg_ng), itl1).join(" ");
        assert!(ng_str.contains("reltol=1e-3"), "ng reltol: {}", ng_str);
        assert!(ng_str.contains("ITL1=200"), "ng ITL1: {}", ng_str);

        // LTspice uses the same SPICE3 spellings
        let cg_lt = Dialect::Ltspice;
        let lt_str = super::super::option_pairs(&opts, key(cg_lt), itl1).join(" ");
        assert!(lt_str.contains("reltol=1e-3"), "lt reltol: {}", lt_str);
        assert!(lt_str.contains("ITL1=200"), "lt ITL1: {}", lt_str);
    }

    #[test]
    fn test_options_backend_specific() {
        let mut backend_specific = BTreeMap::new();
        backend_specific.insert("ngspice".into(), vec![("SEED".into(), "42".into())]);

        let opts = SimOptions {
            portable: vec![("reltol".into(), "1e-4".into())],
            backend_specific,
        };

        let cg = Dialect::Ngspice;
        let s = super::super::option_pairs(&opts, key(cg), itl1).join(" ");
        assert!(s.contains("reltol=1e-4"), "opts: {}", s);
        assert!(s.contains("SEED=42"), "opts: {}", s);
    }

    #[test]
    fn test_osdi_ngspice_only() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "OSDI Test".into(),
                osdi_loads: vec!["/path/to/model.osdi".into()],
                ..Default::default()
            },
            ..Default::default()
        };

        // ngspice wraps OSDI loads in .control block
        let cg_ng = Dialect::Ngspice;
        let ng_str = netlist(cg_ng, &ir).unwrap();
        assert!(
            ng_str.contains(".control\npre_osdi /path/to/model.osdi\n.endc"),
            "ng osdi control block: {}",
            ng_str
        );
        assert!(
            !ng_str.contains(".pre_osdi"),
            "no dot-command form: {}",
            ng_str
        );

        // LTspice does not
        let cg_lt = Dialect::Ltspice;
        let lt_str = netlist(cg_lt, &ir).unwrap();
        assert!(
            !lt_str.contains("pre_osdi"),
            "lt should not have osdi: {}",
            lt_str
        );
    }

    #[test]
    fn test_xspice_ngspice_vs_others() {
        let comp = Component::Xspice {
            name: "1".into(),
            connections: vec!["[in1 in2]".into(), "out".into()],
            model: "d_and".into(),
        };

        let cg_ng = Dialect::Ngspice;
        let ng_str = crate::codegen::card(&comp, cg_ng).unwrap();
        assert!(ng_str.starts_with("A1 "), "ng xspice: {}", ng_str);
        assert!(ng_str.contains("d_and"), "ng xspice model: {}", ng_str);

        let cg_lt = Dialect::Ltspice;
        let lt_str = crate::codegen::card(&comp, cg_lt).unwrap();
        assert!(
            lt_str.starts_with("* XSPICE"),
            "lt xspice should be comment: {}",
            lt_str
        );
    }

    #[test]
    fn test_step_params_ltspice_ok_ngspice_errors() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "Step Test".into(),
                components: vec![Component::Resistor {
                    name: "1".into(),
                    n1: "a".into(),
                    n2: "0".into(),
                    value: IrValue::Expression {
                        expr: "rval".into(),
                    },
                    model: None,
                    params: vec![],
                }],
                ..Default::default()
            },
            testbench: Some(Testbench {
                analyses: vec![Analysis::Op],
                step_params: vec![StepParam {
                    param: "rval".into(),
                    start: 1000.0,
                    stop: 10000.0,
                    step: 1000.0,
                    sweep_type: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        // LTspice emits a native .step
        let cg = Dialect::Ltspice;
        let lt = netlist(cg, &ir).unwrap();
        assert!(
            lt.contains(".step param rval 1000 10000 1000"),
            "step: {}",
            lt
        );

        // ngspice has no .step — it must fail loudly rather than run once.
        let ng = Dialect::Ngspice;
        let err = netlist(ng, &ir).unwrap_err();
        assert!(err.to_string().contains("no `.step`"), "err: {}", err);
    }

    #[test]
    fn test_step_params_ltspice_sweep_type_keeps_param_keyword() {
        // LTspice: `.step dec param x 1 10 10` — the `param` keyword is required
        // in the lin/oct/dec form too.
        let cg = Dialect::Ltspice;
        let sp = StepParam {
            param: "rval".into(),
            start: 1000.0,
            stop: 10000.0,
            step: 10.0,
            sweep_type: Some(Sweep::Dec),
        };
        assert_eq!(
            step_param(cg, &sp).unwrap(),
            ".step dec param rval 1000 10000 10"
        );
    }

    #[test]
    fn test_spice3_testbench_measure_and_nominal_temperature() {
        let mut ir = sample_resistor_divider();
        let tb = ir.testbench.as_mut().unwrap();
        tb.nominal_temperature = Some(27.0);
        tb.measures.push("tran vmax MAX V(output)".into());

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(netlist.contains(".options tnom=27"), "tnom: {}", netlist);
        assert!(
            netlist.contains(".meas tran vmax MAX V(output)"),
            "measure: {}",
            netlist
        );
    }

    // ── `.measure` / `.four` run from a `.control` block on ngspice ──

    /// The divider with `analyses`, `measures` and raw testbench lines.
    fn divider(analyses: Vec<Analysis>, measures: &[&str], extra: &[&str]) -> CircuitIR {
        let mut ir = sample_resistor_divider();
        let tb = ir.testbench.as_mut().unwrap();
        tb.analyses = analyses;
        tb.measures = measures.iter().map(|m| m.to_string()).collect();
        tb.extra_lines = extra.iter().map(|m| m.to_string()).collect();
        ir
    }

    fn ac() -> Analysis {
        Analysis::Ac {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1.0,
                stop: 1e3,
            },
        }
    }

    fn tran() -> Analysis {
        Analysis::Transient {
            step: 1e-6,
            stop: 1e-2,
            start: None,
            max_step: None,
            uic: false,
        }
    }

    fn four(outputs: &[&str]) -> Analysis {
        Analysis::Fourier {
            fundamental: 1e3,
            outputs: outputs.iter().map(|o| o.to_string()).collect(),
            num_harmonics: None,
        }
    }

    #[test]
    fn measures_move_every_analysis_into_a_control_block() {
        let ir = divider(
            vec![Analysis::Op, ac(), tran()],
            &["ac g find vdb(output) at=100"],
            &[],
        );
        let deck = netlist(Dialect::Ngspice, &ir).unwrap();
        assert!(deck.ends_with(".meas ac g find vdb(output) at=100\n.control\nop\nac dec 10 1 1k\ntran 1u 10m\n.endc\n.end"), "{deck}");
        // ESPice and LTspice read `.measure` under their rawfile option.
        for d in [Dialect::Espice, Dialect::Ltspice] {
            assert!(!netlist(d, &ir).unwrap().contains(".control"), "{d:?}");
        }
    }

    #[test]
    fn a_raw_meas_line_counts_as_a_measure() {
        let deck = netlist(
            Dialect::Ngspice,
            &divider(
                vec![tran()],
                &[],
                &[".MEASURE tran v1 find v(output)", "+ at=1m"],
            ),
        )
        .unwrap();
        // The card and its continuation stay where they were written.
        assert!(
            deck.ends_with(
                ".MEASURE tran v1 find v(output)\n+ at=1m\n.control\ntran 1u 10m\n.endc\n.end"
            ),
            "{deck}"
        );
        let mut ir = divider(vec![tran()], &[], &[]);
        ir.top.raw_spice = vec![".meas tran v2 find v(output) at=1m".into()];
        assert!(
            netlist(Dialect::Ngspice, &ir)
                .unwrap()
                .contains(".control\ntran 1u 10m\n.endc")
        );
    }

    #[test]
    fn no_block_without_a_measure_or_fourier() {
        let deck = netlist(Dialect::Ngspice, &divider(vec![ac()], &[], &[])).unwrap();
        assert!(
            deck.ends_with(".ac dec 10 1 1k\n.end") && !deck.contains(".control"),
            "{deck}"
        );
    }

    #[test]
    fn noise_never_moves_into_a_block() {
        // `write` would keep only "Integrated Noise" and drop the spectra.
        let noise = Analysis::Noise {
            output: "output".into(),
            reference: "0".into(),
            source: "Vin".into(),
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1.0,
                stop: 1e3,
            },
            points_per_summary: None,
        };
        let deck = netlist(
            Dialect::Ngspice,
            &divider(vec![noise], &["tran x find v(output) at=1"], &[]),
        )
        .unwrap();
        assert!(!deck.contains(".control"), "{deck}");
    }

    #[test]
    fn defers_to_a_raw_control_block_that_runs() {
        let deck = netlist(
            Dialect::Ngspice,
            &divider(
                vec![ac()],
                &["ac g find vdb(output) at=100"],
                &[".control", "run", ".endc"],
            ),
        )
        .unwrap();
        assert!(
            deck.contains(".control\nrun\n.endc\n.ac dec 10 1 1k"),
            "{deck}"
        );
        assert_eq!(deck.matches(".control").count(), 1, "{deck}");
    }

    #[test]
    fn four_becomes_the_fourier_command_after_the_transient() {
        let deck = netlist(
            Dialect::Ngspice,
            &divider(
                vec![tran(), four(&["v(output)"]), four(&["v(input)"])],
                &[],
                &[],
            ),
        )
        .unwrap();
        // Each `.four` gets its command; the old backend rewrite kept only the first.
        assert!(
            deck.ends_with(
                ".control\ntran 1u 10m\nfourier 1k v(output)\nfourier 1k v(input)\n.endc\n.end"
            ),
            "{deck}"
        );
    }

    #[test]
    fn four_without_a_transient_stays_a_card() {
        // No analysis to run from a block: the backend refuses this deck.
        let alone = netlist(
            Dialect::Ngspice,
            &divider(vec![four(&["v(output)"])], &[], &[]),
        )
        .unwrap();
        assert!(
            alone.ends_with(".four 1k v(output)\n.end") && !alone.contains(".control"),
            "{alone}"
        );
    }

    #[test]
    fn raw_four_cards_with_continuations_become_fourier_commands() {
        // Spec (ngspice manual, `.four`): `.four freq ov1 <ov2 ...>`; a `+`
        // line continues the card. Raw cards in the body and the testbench
        // lines, and the testbench's own `.four`, each get a command.
        let mut ir = divider(
            vec![tran(), four(&["v(output)"])],
            &[],
            &[".FOUR 2k v(input)", "+ v(output)", ".print tran v(output)"],
        );
        ir.top.raw_spice = vec![".four 3k".into(), "  + v(input)".into()];
        let deck = netlist(Dialect::Ngspice, &ir).unwrap();
        assert!(deck.ends_with(".print tran v(output)\n.control\ntran 1u 10m\nfourier 3k v(input)\nfourier 2k v(input) v(output)\nfourier 1k v(output)\n.endc\n.end"), "{deck}");
        assert!(
            !deck.to_ascii_lowercase().contains(".four") && !deck.contains("+ v("),
            "{deck}"
        );
        // Other dialects keep the cards as written.
        assert!(
            netlist(Dialect::Ltspice, &ir)
                .unwrap()
                .contains(".FOUR 2k v(input)\n+ v(output)")
        );
    }

    #[test]
    fn a_raw_four_line_alone_moves_the_transient() {
        let deck = netlist(
            Dialect::Ngspice,
            &divider(vec![tran()], &[], &[".four 1k v(output)"]),
        )
        .unwrap();
        assert!(
            deck.ends_with(".control\ntran 1u 10m\nfourier 1k v(output)\n.endc\n.end"),
            "{deck}"
        );
    }

    // ── Issue 03: ModelLibrary per-backend path + corner ──

    #[test]
    fn test_model_library_backend_path_ngspice() {
        let mut backend_paths = BTreeMap::new();
        backend_paths.insert("ngspice".into(), "/pdk/ngspice/sky130.lib".into());
        backend_paths.insert("spectre".into(), "/pdk/spectre/sky130.scs".into());

        let ir = CircuitIR {
            top: Subcircuit {
                name: "pdk_test".into(),
                ..Default::default()
            },
            testbench: Some(Testbench {
                analyses: vec![Analysis::Op],
                ..Default::default()
            }),
            model_libraries: vec![ModelLibrary {
                name: "sky130".into(),
                path: "/pdk/default/sky130.lib".into(),
                corner: Some("tt".into()),
                backend_paths,
                setup_includes: vec![],
            }],
            ..Default::default()
        };

        // Ngspice selects its path
        let ng = Dialect::Ngspice;
        let ng_netlist = netlist(ng, &ir).unwrap();
        assert!(
            ng_netlist.contains(".lib /pdk/ngspice/sky130.lib tt"),
            "ngspice path: {}",
            ng_netlist
        );
        assert!(
            !ng_netlist.contains("spectre"),
            "no spectre path in ngspice: {}",
            ng_netlist
        );

        // LTspice falls back to the default path (no ltspice key)
        let lt = Dialect::Ltspice;
        let lt_netlist = netlist(lt, &ir).unwrap();
        assert!(
            lt_netlist.contains(".lib /pdk/default/sky130.lib tt"),
            "ltspice fallback: {}",
            lt_netlist
        );
    }

    #[test]
    fn test_model_library_corner_switch() {
        let ir_tt = CircuitIR {
            top: Subcircuit {
                name: "corner".into(),
                ..Default::default()
            },
            model_libraries: vec![ModelLibrary {
                name: "sky130".into(),
                path: "/pdk/sky130.lib".into(),
                corner: Some("tt".into()),
                backend_paths: BTreeMap::new(),
                setup_includes: vec![],
            }],
            ..Default::default()
        };

        let mut ir_ss = ir_tt.clone();
        ir_ss.model_libraries[0].corner = Some("ss".into());

        let cg = Dialect::Ngspice;

        let tt_netlist = netlist(cg, &ir_tt).unwrap();
        let ss_netlist = netlist(cg, &ir_ss).unwrap();

        assert!(
            tt_netlist.contains(".lib /pdk/sky130.lib tt"),
            "tt: {}",
            tt_netlist
        );
        assert!(
            ss_netlist.contains(".lib /pdk/sky130.lib ss"),
            "ss: {}",
            ss_netlist
        );
        assert!(!tt_netlist.contains("ss"), "tt has no ss");
    }

    #[test]
    fn test_model_library_no_corner_uses_include() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "nocorner".into(),
                ..Default::default()
            },
            model_libraries: vec![ModelLibrary {
                name: "custom".into(),
                path: "/models/custom.lib".into(),
                corner: None,
                backend_paths: BTreeMap::new(),
                setup_includes: vec![],
            }],
            ..Default::default()
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(
            netlist.contains(".include /models/custom.lib"),
            "include: {}",
            netlist
        );
        assert!(
            !netlist.contains(".lib /models"),
            "no .lib without corner: {}",
            netlist
        );
    }

    // ── Issue 05: Dialect correctness ──

    #[test]
    fn test_ngspice_uses_save() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "ng_save".into(),
                ..Default::default()
            },
            testbench: Some(Testbench {
                analyses: vec![Analysis::Op],
                saves: vec!["V(a)".into()],
                ..Default::default()
            }),
            ..Default::default()
        };

        let ng = Dialect::Ngspice;
        let netlist = netlist(ng, &ir).unwrap();
        assert!(
            netlist.contains(".save V(a)"),
            "ngspice uses .save: {}",
            netlist
        );
    }

    #[test]
    fn test_pz_disto_sens_error_on_ltspice() {
        let cg_xy = Dialect::Ltspice;
        let pz = Analysis::PoleZero {
            node1: "1".into(),
            node2: "0".into(),
            node3: "3".into(),
            node4: "0".into(),
            tf_type: PzTransfer::Vol,
            pz_type: PzKind::Pz,
        };
        assert!(
            analysis(cg_xy, false, &pz).is_err(),
            "pz should error on ltspice"
        );

        let disto = Analysis::Distortion {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1e3,
                stop: 1e6,
            },
            f2overf1: None,
        };
        assert!(
            analysis(cg_xy, false, &disto).is_err(),
            "disto should error on ltspice"
        );

        let sens = Analysis::Sensitivity {
            output: "V(out)".into(),
            ac: None,
        };
        assert!(
            analysis(cg_xy, false, &sens).is_err(),
            "sens should error on ltspice"
        );
    }

    #[test]
    fn test_pz_disto_ok_on_ngspice() {
        let cg_ng = Dialect::Ngspice;
        let pz = Analysis::PoleZero {
            node1: "1".into(),
            node2: "0".into(),
            node3: "3".into(),
            node4: "0".into(),
            tf_type: PzTransfer::Vol,
            pz_type: PzKind::Pz,
        };
        assert!(
            analysis(cg_ng, false, &pz).is_ok(),
            "pz should work on ngspice"
        );
    }

    #[test]
    fn test_voltage_source_ac_magnitude_and_phase() {
        let cg = Dialect::Ngspice;

        // AC magnitude only
        let v_ac = Component::VoltageSource {
            name: "in".into(),
            np: "inp".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0),
            ac_phase: None,
            waveform: None,
            port: None,
        };
        let s = crate::codegen::card(&v_ac, cg).unwrap();
        assert_eq!(s, "Vin inp 0 0 AC 1", "ac mag only: {}", s);

        // AC magnitude + phase
        let v_ac_phase = Component::VoltageSource {
            name: "in".into(),
            np: "inp".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0),
            ac_phase: Some(45.0),
            waveform: None,
            port: None,
        };
        let s = crate::codegen::card(&v_ac_phase, cg).unwrap();
        assert_eq!(s, "Vin inp 0 0 AC 1 45", "ac mag+phase: {}", s);

        // No AC — regression check
        let v_dc = Component::VoltageSource {
            name: "dd".into(),
            np: "vdd".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 3.3 },
            ac_magnitude: None,
            ac_phase: None,
            waveform: None,
            port: None,
        };
        let s = crate::codegen::card(&v_dc, cg).unwrap();
        assert_eq!(s, "Vdd vdd 0 3.3", "dc only: {}", s);
        assert!(!s.contains("AC"), "no AC in dc-only source");
    }

    #[test]
    fn test_current_source_ac_magnitude() {
        let cg = Dialect::Ngspice;
        let i_ac = Component::CurrentSource {
            name: "ac".into(),
            np: "a".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1e-3),
            ac_phase: Some(90.0),
            waveform: None,
        };
        let s = crate::codegen::card(&i_ac, cg).unwrap();
        assert!(s.contains("AC 0.001 90"), "current AC: {}", s);
    }

    #[test]
    fn test_pdk_netlist_lib_before_subckt_and_components() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "cs_amp_tb".into(),
                instances: vec![Instance {
                    name: "dut".into(),
                    subcircuit: "cs_amp".into(),
                    port_mapping: vec!["vdd".into(), "vin".into(), "vout".into()],
                    ..Default::default()
                }],
                ..Default::default()
            },
            testbench: Some(Testbench {
                stimulus: vec![Component::VoltageSource {
                    name: "dd".into(),
                    np: "vdd".into(),
                    nm: "0".into(),
                    value: IrValue::Numeric { value: 1.8 },
                    ac_magnitude: None,
                    ac_phase: None,
                    waveform: None,
                    port: None,
                }],
                analyses: vec![Analysis::Op],
                ..Default::default()
            }),
            subcircuit_defs: vec![Subcircuit {
                name: "cs_amp".into(),
                ports: vec![
                    Port {
                        name: "vdd".into(),
                        direction: PortDirection::InOut,
                    },
                    Port {
                        name: "vin".into(),
                        direction: PortDirection::Input,
                    },
                    Port {
                        name: "vout".into(),
                        direction: PortDirection::Output,
                    },
                ],
                components: vec![
                    Component::Resistor {
                        name: "drain".into(),
                        n1: "vdd".into(),
                        n2: "vout".into(),
                        value: IrValue::Numeric { value: 5e3 },
                        model: None,
                        params: vec![],
                    },
                    Component::Mosfet {
                        name: "n1".into(),
                        nd: "vout".into(),
                        ng: "vin".into(),
                        ns: "0".into(),
                        nb: Some("0".into()),
                        extra_nodes: vec![],
                        model: "sky130_fd_pr__nfet_01v8".into(),
                        params: vec![],
                    },
                ],
                ..Default::default()
            }],
            model_libraries: vec![ModelLibrary {
                name: "sky130".into(),
                path: "/pdk/sky130.lib.spice".into(),
                corner: Some("tt".into()),
                backend_paths: BTreeMap::new(),
                setup_includes: vec![],
            }],
            ..Default::default()
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();

        // Title must be line 1
        let lines: Vec<&str> = netlist.lines().collect();
        assert!(
            lines[0].starts_with("*"),
            "first line is title: {}",
            lines[0]
        );

        // .lib must come before .subckt
        let lib_pos = netlist.find(".lib").expect("must have .lib");
        let subckt_pos = netlist.find(".subckt").expect("must have .subckt");
        assert!(
            lib_pos < subckt_pos,
            ".lib ({}) must come before .subckt ({}):\n{}",
            lib_pos,
            subckt_pos,
            netlist
        );

        // .lib must come before components
        let mosfet_pos = netlist.find("Mn1").expect("must have Mn1");
        assert!(
            lib_pos < mosfet_pos,
            ".lib must come before MOSFET:\n{}",
            netlist
        );

        // Verify the .lib line itself
        assert!(
            netlist.contains(".lib /pdk/sky130.lib.spice tt"),
            "lib line: {}",
            netlist
        );
    }

    #[test]
    fn test_setup_includes_emitted_before_lib() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "gf180_test".into(),
                ..Default::default()
            },
            testbench: Some(Testbench {
                analyses: vec![Analysis::Op],
                ..Default::default()
            }),
            model_libraries: vec![ModelLibrary {
                name: "gf180".into(),
                path: "/pdk/gf180/sm141064.ngspice".into(),
                corner: Some("typical".into()),
                backend_paths: BTreeMap::new(),
                setup_includes: vec!["/pdk/gf180/design.ngspice".into()],
            }],
            ..Default::default()
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();

        // setup_includes emitted as .include
        assert!(
            netlist.contains(".include /pdk/gf180/design.ngspice"),
            "setup include present: {}",
            netlist
        );

        // setup_includes comes BEFORE the .lib
        let setup_pos = netlist.find(".include /pdk/gf180/design.ngspice").unwrap();
        let lib_pos = netlist
            .find(".lib /pdk/gf180/sm141064.ngspice typical")
            .unwrap();
        assert!(
            setup_pos < lib_pos,
            "setup ({}) before lib ({}): {}",
            setup_pos,
            lib_pos,
            netlist
        );
    }

    fn loads_ir(loads: Vec<String>) -> CircuitIR {
        let mut ir = sample_resistor_divider();
        ir.testbench = None;
        ir.top.osdi_loads = loads;
        ir
    }

    #[test]
    fn espice_veriloga_becomes_hdl_without_pre_osdi() {
        let n = netlist(Dialect::Espice, &loads_ir(vec!["/m/vres.va".into()])).unwrap();
        assert!(
            n.starts_with("* Voltage Divider\n.hdl \"/m/vres.va\"\n"),
            "{n}"
        );
        assert!(!n.contains("pre_osdi") && !n.contains(".control"), "{n}");
    }

    #[test]
    fn espice_refuses_osdi_binaries() {
        let e = netlist(Dialect::Espice, &loads_ir(vec!["/m/vres.osdi".into()])).unwrap_err();
        assert!(e.to_string().contains("does not load OSDI"), "{e}");
    }

    /// The cards the retired string path sent ngspice, byte for byte.
    #[test]
    fn pss_and_sp_are_ngspice_only() {
        let pss = Analysis::Pss {
            fundamental: 1e9,
            stabilization: 1e-7,
            observe_node: "v(out)".into(),
            points_per_period: 128,
            harmonics: 10,
            sc_iter: None,
            steady_coeff: None,
            uic: false,
        };
        let sp = Analysis::SPar {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1e6,
                stop: 1e10,
            },
        };
        assert_eq!(
            analysis(Dialect::Ngspice, false, &pss).unwrap(),
            ".pss 1000000000 0.0000001 v(out) 128 10"
        );
        assert_eq!(
            analysis(Dialect::Ngspice, false, &sp).unwrap(),
            ".sp dec 10 1000000 10000000000"
        );
        for a in [&pss, &sp] {
            assert!(analysis(Dialect::Ltspice, false, a).is_err(), "{a:?}");
        }
        for a in [
            Analysis::HarmonicBalance {
                frequencies: vec![1e6],
                harmonics: vec![7],
            },
            Analysis::TransientNoise {
                step: 1e-9,
                stop: 1e-6,
            },
        ] {
            assert!(matches!(
                analysis(Dialect::Ngspice, false, &a),
                Err(CodeGenError::UnsupportedAnalysis { .. })
            ));
        }
    }
}
