//! Cadence Spectre netlist generation (Spectre native language, `simulator lang=spectre`).
//!
//! # Provenance of every construct in this file
//!
//! Spectre is licence-gated and is **not** installed here, so nothing below was
//! ever executed. Every statement shape is taken from Cadence's own manuals;
//! the citation lives next to the code that emits it. The two primary sources:
//!
//! * `[REF]` — *Spectre Circuit Simulator Reference*, Product Version 19.1,
//!   January 2020. <https://ee.kpi.ua/~yv/edu/ok/book/spectre_refManual.pdf>
//!   (analysis statements, `options`, `save`, `ic`, `nodeset`, `include`,
//!   `subckt`, `parameters`, `global`).
//! * `[CMP]` — *Spectre Circuit Simulator Reference*, Product Version 5.0,
//!   September 2003. <http://eece.cu.edu.eg/~fhussien/Spectre_tutorial.pdf>
//!   (the component chapters: `resistor`, `capacitor`, `inductor`,
//!   `mutual_inductor`, `vsource`, `isource`, `vcvs`, `vccs`, `cccs`, `ccvs`,
//!   `tline`, `diode`, `relay`, `fourier`, `bsource`). 19.1 split these into a
//!   separate manual; the statement forms are unchanged.
//! * `[KUN]` — Kundert, *The Designer's Guide to SPICE and Spectre*,
//!   Appendix B "Spectre Netlist Language".
//!   <https://designers-guide.org/analysis/dg-spice/chB.pdf> (scale factors,
//!   case sensitivity, `simulator lang=` rules).
//!
//! # Deliberate refusals
//!
//! Where SPICE can express something Spectre has no documented spelling for,
//! this file returns `CodeGenError` instead of guessing. Currently: XSPICE
//! A-devices, raw SPICE lines, `bsource` expressions (SPICE `V(x)`/`I(Vx)`
//! syntax is not Spectre's lowercase `v(x)`/`i("x:0")` — see `[CMP]` bsource),
//! current-controlled switches, `AM` sources, and transient noise (Spectre
//! spells it `tran ... noisefmax=` and the IR carries no bandwidth).
//!
//! # Layout
//!
//! This file is the deck spine (`netlist`) and the deck-level statements
//! (models, subcircuits, options, measures); `values` spells numbers and
//! waveforms, `names` identifiers, `components` instance statements,
//! `analyses` analysis statements and sweeps. The citations above cover all
//! of them.
//!
//! Contents:
//! - `pub(super) fn netlist(ir: &CircuitIR) -> Result<String, CodeGenError>`
//! - re-exported for the xschem layout: `components::emit_component`, `names::Names`

mod analyses;
mod components;
mod names;
mod values;

use crate::ir::*;

use super::{CodeGenError, bare_node, param_defs};
use analyses::emit_analysis_block;
pub(crate) use components::emit_component;
pub(crate) use names::Names;
use names::{inst_ref, signal};
use values::{spectre_num, spectre_value, unsupported_component};

/// `[CMP]` p.628 `model resmod resistor rsh=150 l=2u w=2u etch=0.05u ...`,
/// p.645 `model lmodel tline f=10M z0=50 alphac=8501 fc=10M dcr=88`,
/// p.625 `model my_relay relay vt1=2.5 vt2=5 ropen=100M rclosed=0.1`,
/// `[REF]` p.498 `model nch bsim3v3 type=n mobmod=1 capmod=2 version=3.1`.
/// No Cadence example parenthesises model parameters; the previous version
/// of this file did.
/// `.model` type (and LEVEL) -> Spectre master and its `type=`. A model that
/// is not here is refused: written as its SPICE type it names no Spectre
/// master, and "the nearest model" would be a different set of equations.
/// `.model` types SPICE defines; a kind outside this list is a Spectre master.
const SPICE_MODEL_TYPES: &[&str] = &[
    "nmos", "pmos", "npn", "pnp", "d", "njf", "pjf", "nmf", "pmf", "r", "c", "l", "sw", "csw",
    "vdmos", "ltra", "txl", "cpl", "urc",
];

/// The whole Spectre deck, with no line end after its last line.
pub(super) fn netlist(ir: &CircuitIR) -> Result<String, CodeGenError> {
    super::refuse_raw_spice(ir, "spectre")?;
    let keep_case = ir.raw_dialect == Some(Dialect::Spectre);
    let n = Names {
        fold: !keep_case,
        escape: true,
    };
    let mut out = String::new();

    // `[KUN]` B.2: `//` is a Spectre comment; "all Spectre netlists must
    // begin with a lang=spectre statement".
    w!(out, "// {}\n\nsimulator lang=spectre\n\n", ir.top.name);
    // `real f(real x) { return x*2; }` (R5 pp.870-871). A body reading a
    // node or a current has no Spectre function form.
    for f in &ir.functions {
        let b = f.body.to_ascii_lowercase();
        if b.contains("v(") || b.contains("i(") {
            return Err(unsupported_component(&format!(
                ".func {}: a body over v()/i() has no Spectre function form",
                f.name
            )));
        }
        let args: Vec<String> = f.args.iter().map(|a| format!("real {a}")).collect();
        w!(
            out,
            "real {}({}) {{\n    return {};\n}}\n",
            f.name,
            args.join(", "),
            f.body
        );
    }
    // `global 0 a b`: Spectre takes the first global as ground (R5 p.871).
    if !ir.globals.is_empty() {
        out.push_str("global 0");
        for g in &ir.globals {
            w!(out, " {}", n.n(g));
        }
        out.push('\n');
    }

    // `[CMP]` p.888 / `[REF]` p.541: `ahdl_include "VerilogAfile.va"`.
    for path in &ir.top.osdi_loads {
        w!(out, "ahdl_include \"{}\"\n", path);
    }

    // `[REF]` p.494: `include "filename"`.
    for inc in &ir.top.includes {
        w!(out, "include \"{}\"\n", inc);
    }

    // `[REF]` p.493: `include "file" section=sectionName` — the documented
    // way to pick a PDK corner out of a `library`/`section` file (p.498).
    for (path, section) in &ir.top.libs {
        w!(out, "include \"{}\" section={}\n", path, section);
    }

    for lib in &ir.model_libraries {
        for setup in &lib.setup_includes {
            w!(out, "include \"{}\"\n", setup);
        }
        let path = lib.backend_paths.get("spectre").unwrap_or(&lib.path);
        if let Some(ref corner) = lib.corner {
            w!(out, "include \"{}\" section={}\n", path, corner);
        } else {
            w!(out, "include \"{}\"\n", path);
        }
    }

    // `[REF]` p.506: `parameters p1=1 p2=2`.
    if !ir.top.parameters.is_empty() {
        out.push_str("parameters");
        param_defs(&mut out, &ir.top.parameters, spectre_value);
        out.push('\n');
    }

    for m in &ir.top.models {
        emit_model(&mut out, m, n)?;
        out.push('\n');
    }

    for sc in &ir.subcircuit_defs {
        out.push('\n');
        emit_subcircuit(&mut out, sc, n)?;
        out.push('\n');
    }

    out.push('\n');

    for e in ir.top.elements() {
        match e {
            Element::Component(c) => emit_component(&mut out, c, n)?,
            Element::Instance(i) => emit_instance(&mut out, i, n),
        }
        out.push('\n');
    }
    for l in &ir.top.raw_spice {
        w!(out, "{l}\n");
    }

    if let Some(ref tb) = ir.testbench {
        for comp in &tb.stimulus {
            emit_component(&mut out, comp, n)?;
            out.push('\n');
        }

        let opts = emit_options(&tb.options)?;
        if !opts.is_empty() {
            w!(out, "{opts}\n");
        }

        // `[REF]` p.236 verbatim: `myopt options temp=27`; temp is options
        // parameter #6 and tnom #7 (p.187-188).
        if let Some(temp) = tb.temperature {
            w!(out, "mytemp options temp={}\n", spectre_num(temp));
        }
        if let Some(tnom) = tb.nominal_temperature {
            w!(out, "mytnom options tnom={}\n", spectre_num(tnom));
        }

        // `[REF]` p.490: `ic 7=0 out=1 OpAmp1.comp=5 L1:1=1.0u`.
        for (node, val) in &tb.initial_conditions {
            w!(
                out,
                "ic {}={}\n",
                bare_node(&signal(node, n)),
                spectre_num(*val)
            );
        }

        // `[REF]` p.502: `nodeset 7=0 out=1 OpAmp1.comp=5 L1:1=1.0u`.
        for (node, val) in &tb.node_sets {
            w!(
                out,
                "nodeset {}={}\n",
                bare_node(&signal(node, n)),
                spectre_num(*val)
            );
        }

        // `[REF]` p.517-518: `save 7 out OpAmp1.comp M1:currents ...` —
        // bare signal names, not `V(...)`.
        for save in &tb.saves {
            let save = signal(save, n);
            // `i(V1)` is the source's current: its terminal, under the
            // lowercased name the instance is written with.
            let t = save.trim();
            let current = t
                .strip_prefix("i(")
                .or_else(|| t.strip_prefix("I("))
                .and_then(|r| r.strip_suffix(')'));
            match current {
                Some(dev) => w!(out, "save {}:p\n", inst_ref(dev.trim(), keep_case)),
                None => w!(out, "save {}\n", bare_node(&save)),
            }
        }

        for meas in &tb.measures {
            w!(out, "{}\n", emit_measure(meas));
        }

        for line in &tb.extra_lines {
            w!(out, "{line}\n");
        }

        out.push('\n');

        for l in emit_analysis_block(&tb.analyses, &tb.step_params, keep_case)? {
            w!(out, "{l}\n");
        }
    }

    // Lines were each written with their end; the deck has none after the last.
    out.pop();
    Ok(out)
}

// ---- private ----

fn emit_model(out: &mut String, m: &ModelDef, n: Names) -> Result<(), CodeGenError> {
    let (master, ty) = spectre_master(m, n)?;
    w!(out, "model {} {}", n.n(&m.name), master.unwrap_or(&m.kind));
    if let Some(t) = ty {
        w!(out, " type={t}");
    }
    // A mapped SPICE model: LEVEL chose the master, which takes no `level`,
    // and Spectre's names are lowercase. A Spectre master's card is kept.
    match master {
        Some(_) => {
            for (k, v) in m
                .parameters
                .iter()
                .filter(|(k, _)| !k.eq_ignore_ascii_case("level"))
            {
                let k = k.to_ascii_lowercase();
                if v.is_empty() {
                    w!(out, " {k}")
                } else {
                    w!(out, " {k}={}", spectre_value(v))
                }
            }
        }
        None => w!(out, "{}", super::params(&m.parameters, spectre_value)),
    }
    Ok(())
}

/// The Spectre master for `m` (`None`: write `m.kind` as it is) and its `type=`.
fn spectre_master(
    m: &ModelDef,
    n: Names,
) -> Result<(Option<&'static str>, Option<&'static str>), CodeGenError> {
    let level = m
        .parameters
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("level"))
        .and_then(|(_, v)| super::vacask::parse_spice_num(v.trim()))
        .map_or(1, |(l, _)| l as u32);
    let kind = m.kind.to_ascii_lowercase();
    let mos = |l: u32| match l {
        1 => Some("mos1"),
        2 => Some("mos2"),
        3 => Some("mos3"),
        8 | 49 => Some("bsim3v3"),
        14 | 54 => Some("bsim4"),
        _ => None,
    };
    let found = match kind.as_str() {
        "nmos" => mos(level).map(|m| (m, Some("n"))),
        "pmos" => mos(level).map(|m| (m, Some("p"))),
        "npn" if level == 1 => Some(("bjt", Some("npn"))),
        "pnp" if level == 1 => Some(("bjt", Some("pnp"))),
        "d" if level == 1 => Some(("diode", None)),
        // Not a SPICE type: already a Spectre master (`bsim3v3`), kept.
        k if !SPICE_MODEL_TYPES.contains(&k) => return Ok((None, None)),
        _ => None,
    };
    found.map(|(m, t)| (Some(m), t)).ok_or_else(|| {
        unsupported_component(&format!(
            "model {} ({} level {level}): no Spectre master mapped",
            n.n(&m.name),
            m.kind
        ))
    })
}

/// `[REF]` p.533-535: `[inline] subckt <Name> (<node1> ... <nodeN>)` ...
/// `ends <Name>`, verbatim example `subckt coax (i1 o1 i2 o2)` / `ends coax`.
/// Written with no line end after `ends`.
fn emit_subcircuit(out: &mut String, sc: &Subcircuit, n: Names) -> Result<(), CodeGenError> {
    w!(out, "subckt {} (", n.n(&sc.name));
    for (i, port) in sc.ports.iter().enumerate() {
        w!(out, "{}{}", if i == 0 { "" } else { " " }, n.n(&port.name));
    }
    out.push_str(")\n");
    let start = out.len();
    emit_subcircuit_body(out, sc, n)?;
    if out.len() == start {
        out.push('\n');
    }
    w!(out, "ends {}", n.n(&sc.name));
    Ok(())
}

/// The body of a `subckt`, one line each.
fn emit_subcircuit_body(out: &mut String, sc: &Subcircuit, n: Names) -> Result<(), CodeGenError> {
    // `[REF]` p.506: `parameters <param=value> [param=value]...`, and p.535
    // shows it as the first line of a subckt body.
    if !sc.parameters.is_empty() {
        out.push_str("parameters");
        param_defs(out, &sc.parameters, spectre_value);
        out.push('\n');
    }

    for m in &sc.models {
        emit_model(out, m, n)?;
        out.push('\n');
    }

    for e in sc.elements() {
        match e {
            Element::Component(c) => emit_component(out, c, n)?,
            Element::Instance(i) => emit_instance(out, i, n),
        }
        out.push('\n');
    }
    // Spectre text kept verbatim on import (`refuse_raw_spice` has checked
    // that it is Spectre's).
    for l in &sc.raw_spice {
        w!(out, "{l}\n");
    }
    Ok(())
}

/// `[REF]` p.535: `Coax1 pin nin out gnd coax zin=75 zout=150 len=35m`
/// (instance name, node list, subcircuit master, parameters). Parentheses
/// around the node list are accepted and used throughout `[CMP]`.
fn emit_instance(out: &mut String, inst: &Instance, n: Names) {
    w!(out, "x{} (", n.d(&inst.name));
    for (i, port) in inst.port_mapping.iter().enumerate() {
        w!(out, "{}{}", if i == 0 { "" } else { " " }, n.n(port));
    }
    w!(
        out,
        ") {}{}",
        n.n(&inst.subcircuit),
        super::params(&inst.parameters, spectre_value)
    );
}

/// `[REF]` p.236 verbatim: `o1 options scale=1.2 subckt=chip1`,
/// `myopt options temp=27`.
fn emit_options(opts: &SimOptions) -> Result<String, CodeGenError> {
    // Spectre's `options` statement takes `name=value` only.
    if let Some((k, _)) = opts.portable.iter().find(|(_, v)| v.is_empty()) {
        return Err(CodeGenError::UnsupportedAnalysis {
            backend: "spectre".into(),
            analysis: format!("option flag '{k}' (Spectre options take name=value)"),
        });
    }
    // Portable options are SPICE's: lowercase Spectre names, and numbers
    // respelled (SPICE `VNTOL=1M` is milli; Spectre's M is mega).
    // Spectre-specific ones are Spectre text already.
    let portable = opts.portable.iter().map(|(k, v)| {
        format!(
            "{}={}",
            map_option_name(&k.to_ascii_lowercase()),
            spectre_value(v)
        )
    });
    let native = opts
        .backend_specific
        .get("spectre")
        .into_iter()
        .flatten()
        .map(|(k, v)| format!("{k}={v}"));
    let parts: Vec<String> = portable.chain(native).collect();
    Ok(if parts.is_empty() {
        String::new()
    } else {
        format!("myopts options {}", parts.join(" "))
    })
}

/// Map portable option names onto `options` statement parameters.
///
/// `[REF]` p.187-236 "Immediate Set Options (options)": `reltol` (#1),
/// `vabstol` (#3), `iabstol` (#4), `temp` (#6), `tnom` (#7), `gmin` (#128),
/// `dcmaxiters` (#56, "Maximum number of Newton iterations in DC
/// simulation"). The parameter index on p.237-240 has no plain `maxiters` —
/// that name belongs to the `dc` analysis statement (p.66 #29), not to
/// `options`, so the previous `max_iterations -> maxiters` mapping produced
/// an unknown option.
fn map_option_name(canonical: &str) -> &str {
    match canonical {
        "abstol" => "iabstol",
        "vntol" => "vabstol",
        "max_iterations" => "dcmaxiters",
        other => other,
    }
}

/// Spectre supports SPICE `.measure` directly — `[REF]` p.17: "In addition
/// to supporting standard SPICE measurement functions (.measure), it offers
/// a measurement description language (MDL)", and p.20 notes Spectre "saves
/// the .measure and .mt0 files in the .raw subdirectory".
///
/// `.measure` is SPICE syntax, so it has to be fenced by language switches
/// (`[REF]` p.493 include / `[KUN]` B.2 `simulator lang=`). The previous
/// implementation stripped the leading `.meas` and emitted the bare tail,
/// which Spectre would read as an instance statement named `tran`.
fn emit_measure(meas: &str) -> String {
    let body = meas.trim();
    let body = if body.starts_with('.') {
        body.to_string()
    } else {
        format!(".measure {body}")
    };
    format!("simulator lang=spice\n{body}\nsimulator lang=spectre")
}

#[cfg(test)]
mod tests {
    use super::analyses::emit_analysis;
    use super::*;
    use std::collections::BTreeMap;

    /// One component's statement, names as given.
    fn emit_component(c: &Component) -> Result<String, CodeGenError> {
        let mut s = String::new();
        super::emit_component(&mut s, c, Names::VERBATIM).map(|()| s)
    }

    /// One subcircuit block, names as given.
    fn emit_subcircuit(sc: &Subcircuit) -> Result<String, CodeGenError> {
        let mut s = String::new();
        super::emit_subcircuit(&mut s, sc, Names::VERBATIM).map(|()| s)
    }

    /// Every identifier field reaches the deck escaped (UG p.62): each `/`
    /// put into a name is written `\/`, wherever the name is written.
    #[test]
    fn every_identifier_field_is_escaped() {
        let s = |x: &str| x.to_string();
        let v = |x: f64| IrValue::Numeric { value: x };
        let comps = vec![
            Component::Resistor {
                name: s("1"),
                n1: s("a"),
                n2: s("b"),
                value: v(1e3),
                model: None,
                params: vec![],
            },
            Component::Capacitor {
                name: s("1"),
                n1: s("a"),
                n2: s("b"),
                value: v(1e-9),
                model: None,
                params: vec![],
            },
            Component::Inductor {
                name: s("1"),
                n1: s("a"),
                n2: s("b"),
                value: v(1e-6),
                model: None,
                params: vec![],
            },
            Component::Inductor {
                name: s("2"),
                n1: s("a"),
                n2: s("b"),
                value: v(1e-6),
                model: None,
                params: vec![],
            },
            Component::MutualInductor {
                name: s("1"),
                inductor1: s("1"),
                inductor2: s("2"),
                coupling: 0.9,
            },
            Component::VoltageSource {
                name: s("1"),
                np: s("a"),
                nm: s("0"),
                value: v(1.0),
                ac_magnitude: None,
                ac_phase: None,
                waveform: None,
                port: None,
            },
            Component::CurrentSource {
                name: s("1"),
                np: s("a"),
                nm: s("0"),
                value: v(1.0),
                ac_magnitude: None,
                ac_phase: None,
                waveform: None,
            },
            Component::Vcvs {
                name: s("1"),
                np: s("a"),
                nm: s("b"),
                ncp: s("c"),
                ncm: s("d"),
                gain: 2.0,
            },
            Component::Vccs {
                name: s("1"),
                np: s("a"),
                nm: s("b"),
                ncp: s("c"),
                ncm: s("d"),
                transconductance: 2.0,
            },
            Component::Cccs {
                name: s("1"),
                np: s("a"),
                nm: s("b"),
                vsense: s("V1"),
                gain: 2.0,
            },
            Component::Ccvs {
                name: s("1"),
                np: s("a"),
                nm: s("b"),
                vsense: s("V1"),
                transresistance: 2.0,
            },
            Component::Poly {
                name: s("2"),
                letter: 'E',
                np: s("a"),
                nm: s("b"),
                controls: vec![s("c"), s("d")],
                coeffs: vec![0.0, 1.0],
            },
            Component::Poly {
                name: s("2"),
                letter: 'F',
                np: s("a"),
                nm: s("b"),
                controls: vec![s("V1")],
                coeffs: vec![0.0, 1.0],
            },
            Component::Diode {
                name: s("1"),
                np: s("a"),
                nm: s("b"),
                model: s("dm"),
                params: vec![],
            },
            Component::Bjt {
                name: s("1"),
                nc: s("a"),
                nb: s("b"),
                ne: s("c"),
                extra_nodes: vec![s("d")],
                model: s("qm"),
                params: vec![],
            },
            Component::Mosfet {
                name: s("1"),
                nd: s("a"),
                ng: s("b"),
                ns: s("c"),
                nb: Some(s("d")),
                extra_nodes: vec![],
                model: s("mm"),
                params: vec![],
            },
            Component::Jfet {
                name: s("1"),
                nd: s("a"),
                ng: s("b"),
                ns: s("c"),
                model: s("jm"),
                params: vec![],
            },
            Component::Mesfet {
                name: s("1"),
                nd: s("a"),
                ng: s("b"),
                ns: s("c"),
                model: s("zm"),
                params: vec![],
            },
            Component::VSwitch {
                name: s("1"),
                np: s("a"),
                nm: s("b"),
                ncp: s("c"),
                ncm: s("d"),
                model: s("sm"),
            },
            Component::TLine {
                name: s("1"),
                inp: s("a"),
                inm: s("b"),
                outp: s("c"),
                outm: s("d"),
                z0: 50.0,
                td: 1e-9,
            },
        ];
        let dm = ModelDef {
            name: s("dm"),
            kind: s("d"),
            parameters: vec![],
        };
        let cell = Subcircuit {
            name: s("cell"),
            ports: vec![Port {
                name: s("p"),
                direction: PortDirection::InOut,
            }],
            components: comps[..1].to_vec(),
            models: vec![dm.clone()],
            ..Default::default()
        };
        let x1 = Instance {
            name: s("1"),
            subcircuit: s("cell"),
            port_mapping: vec![s("a")],
            ..Default::default()
        };
        let top = Subcircuit {
            name: s("t"),
            components: comps,
            instances: vec![x1],
            models: vec![dm],
            ..Default::default()
        };
        let tb = Testbench {
            analyses: vec![Analysis::Op],
            saves: vec![s("v(a/x)"), s("a/x")],
            initial_conditions: vec![(s("a/x"), 1.0)],
            ..Default::default()
        };
        let mut ir = CircuitIR {
            top,
            subcircuit_defs: vec![cell],
            testbench: Some(tb),
            globals: vec![s("g")],
            ..Default::default()
        };
        ir.names_mut(|n| {
            if n != "0" {
                n.push_str("/x")
            }
        });
        let deck = netlist(&ir).unwrap();
        for line in deck.lines().skip(1) {
            let b = line.as_bytes();
            assert!(
                (0..b.len()).all(|j| b[j] != b'/' || (j > 0 && b[j - 1] == b'\\')),
                "unescaped `/` in `{line}`"
            );
        }
        // A sampled field of each kind, folded and escaped.
        // A name starting with a digit escapes the digit too: `\1\/x`.
        for want in [
            "r\\1\\/x (a\\/x b\\/x)",
            "ind1=l\\1\\/x",
            "probe=v1\\/x",
            "probes=[v1\\/x]",
            "x\\1\\/x (a\\/x) cell\\/x",
            "subckt cell\\/x (p\\/x)",
            "ends cell\\/x",
            "model dm\\/x diode",
            "global 0 g\\/x",
            "ic a\\/x=1",
            "save a\\/x",
        ] {
            assert!(deck.contains(want), "no `{want}` in\n{deck}");
        }
    }

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
    fn test_spectre_resistor_divider() {
        let ir = sample_resistor_divider();
        let netlist = netlist(&ir).unwrap();
        assert!(
            netlist.contains("// Voltage Divider"),
            "missing title: {}",
            netlist
        );
        assert!(
            netlist.contains("simulator lang=spectre"),
            "missing lang: {}",
            netlist
        );
        assert!(
            netlist.contains("vin (input 0) vsource dc=10"),
            "missing vin: {}",
            netlist
        );
        assert!(
            netlist.contains("r1 (input output) resistor r=10k"),
            "missing r1: {}",
            netlist
        );
        assert!(
            netlist.contains("r2 (output 0) resistor r=10k"),
            "missing r2: {}",
            netlist
        );
        assert!(netlist.contains("op1 dc"), "missing op: {}", netlist);
        assert!(
            !netlist.contains(".end"),
            "should not have .end: {}",
            netlist
        );
    }

    #[test]
    fn test_spectre_mosfet() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "CMOS Inverter".into(),
                components: vec![Component::Mosfet {
                    name: "1".into(),
                    nd: "out".into(),
                    ng: "in".into(),
                    ns: "vdd".into(),
                    nb: Some("vdd".into()),
                    extra_nodes: vec![],
                    model: "pmos_3p3".into(),
                    params: vec![("w".into(), "2u".into()), ("l".into(), "180n".into())],
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let netlist = netlist(&ir).unwrap();
        assert!(
            netlist.contains("m1 (out in vdd vdd) pmos_3p3 w=2u l=180n"),
            "missing m1: {}",
            netlist
        );
    }

    #[test]
    fn test_spectre_waveforms() {
        let sin_comp = Component::VoltageSource {
            name: "sin".into(),
            np: "out".into(),
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
        };

        let pulse_comp = Component::VoltageSource {
            name: "pulse".into(),
            np: "out".into(),
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
        };

        let sin_str = emit_component(&sin_comp).unwrap();
        assert!(sin_str.contains("type=sine"), "sin: {}", sin_str);
        assert!(sin_str.contains("ampl=1.65"), "sin ampl: {}", sin_str);
        assert!(sin_str.contains("freq=1M"), "sin freq: {}", sin_str);

        let pulse_str = emit_component(&pulse_comp).unwrap();
        assert!(pulse_str.contains("type=pulse"), "pulse: {}", pulse_str);
        assert!(pulse_str.contains("val0=0"), "pulse val0: {}", pulse_str);
        assert!(pulse_str.contains("val1=3.3"), "pulse val1: {}", pulse_str);
    }

    #[test]
    fn test_spectre_all_analyses() {
        assert_eq!(emit_analysis(&Analysis::Op, false).unwrap(), "op1 dc");

        let dc = Analysis::Dc {
            sweeps: vec![DcSweep {
                source: "Vsrc".into(),
                start: 0.0,
                stop: 5.0,
                step: 0.1,
            }],
        };
        let dc_str = emit_analysis(&dc, false).unwrap();
        assert!(dc_str.contains("dc1 dc"), "dc: {}", dc_str);
        assert!(
            dc_str.contains("dev=vsrc param=dc"),
            "dc selector: {}",
            dc_str
        );

        let ac = Analysis::Ac {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 100,
                start: 1.0,
                stop: 1e9,
            },
        };
        let ac_str = emit_analysis(&ac, false).unwrap();
        assert!(ac_str.contains("ac1 ac"), "ac: {}", ac_str);
        assert!(ac_str.contains("dec=100"), "ac dec: {}", ac_str);

        let tran = Analysis::Transient {
            step: 1e-9,
            stop: 1e-6,
            start: None,
            max_step: None,
            uic: false,
        };
        let tran_str = emit_analysis(&tran, false).unwrap();
        assert!(tran_str.contains("tran1 tran"), "tran: {}", tran_str);
        assert!(tran_str.contains("step=1n"), "tran step: {}", tran_str);

        let pss = Analysis::Pss {
            fundamental: 1e6,
            stabilization: 10e-6,
            observe_node: "out".into(),
            points_per_period: 128,
            harmonics: 10,
            sc_iter: None,
            steady_coeff: None,
            uic: false,
        };
        let pss_str = emit_analysis(&pss, false).unwrap();
        assert!(pss_str.contains("pss1 (out 0) pss"), "pss: {}", pss_str);
        assert!(pss_str.contains("fund=1M"), "pss fund: {}", pss_str);
        assert!(
            !pss_str.contains("ppv="),
            "pss must not invent ppv: {}",
            pss_str
        );
    }

    #[test]
    fn test_spectre_options() {
        let opts = SimOptions {
            portable: vec![
                ("reltol".into(), "1e-3".into()),
                ("max_iterations".into(), "200".into()),
            ],
            backend_specific: BTreeMap::new(),
        };
        let s = emit_options(&opts).unwrap();
        assert!(s.contains("myopts options"), "opts header: {}", s);
        assert!(s.contains("reltol=1m"), "reltol: {}", s);
        assert!(s.contains("dcmaxiters=200"), "dcmaxiters: {}", s);
    }

    #[test]
    fn test_spectre_testbench_measures_temperature_and_steps() {
        let mut ir = sample_resistor_divider();
        let tb = ir.testbench.as_mut().unwrap();
        tb.temperature = Some(85.0);
        tb.nominal_temperature = Some(27.0);
        tb.measures.push(".meas tran vmax max V(output)".into());
        tb.step_params.push(StepParam {
            param: "rload".into(),
            start: 1e3,
            stop: 10e3,
            step: 1e3,
            sweep_type: None,
        });
        tb.analyses = vec![
            Analysis::Transient {
                step: 1e-9,
                stop: 1e-6,
                start: None,
                max_step: None,
                uic: false,
            },
            Analysis::Op,
        ];
        let netlist = netlist(&ir).unwrap();
        assert!(
            netlist.contains("mytemp options temp=85"),
            "temp: {}",
            netlist
        );
        assert!(
            netlist.contains("mytnom options tnom=27"),
            "tnom: {}",
            netlist
        );
        assert!(
            netlist.contains(
                "simulator lang=spice\n.meas tran vmax max V(output)\nsimulator lang=spectre"
            ),
            "measure must be fenced by language switches: {}",
            netlist,
        );
        assert!(
            netlist.contains("sweep1_rload sweep param=rload start=1k stop=10k step=1k {"),
            "sweep: {}",
            netlist
        );
        assert!(
            netlist.contains("  tran1 tran"),
            "wrapped tran: {}",
            netlist
        );
        assert!(netlist.contains("  op1 dc"), "wrapped op: {}", netlist);
    }

    #[test]
    fn test_spectre_subcircuit() {
        let sc = Subcircuit {
            name: "mybuf".into(),
            ports: vec![
                Port {
                    name: "in".into(),
                    direction: PortDirection::Input,
                },
                Port {
                    name: "out".into(),
                    direction: PortDirection::Output,
                },
            ],
            parameters: vec![ParamDef {
                name: "wp".into(),
                default: Some("1u".into()),
            }],
            components: vec![Component::Mosfet {
                name: "p".into(),
                nd: "out".into(),
                ng: "in".into(),
                ns: "vdd".into(),
                nb: Some("vdd".into()),
                extra_nodes: vec![],
                model: "pmos".into(),
                params: vec![("w".into(), "wp".into())],
            }],
            ..Default::default()
        };
        let s = emit_subcircuit(&sc).unwrap();
        assert!(s.contains("subckt mybuf (in out)"), "subckt header: {}", s);
        assert!(s.contains("parameters wp=1u"), "params: {}", s);
        assert!(s.contains("mp (out in vdd vdd) pmos w=wp"), "mosfet: {}", s);
        assert!(s.contains("ends mybuf"), "ends: {}", s);
    }

    #[test]
    fn test_spectre_osdi() {
        let ir = CircuitIR {
            top: Subcircuit {
                name: "VA Test".into(),
                osdi_loads: vec!["/path/to/model.va".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let netlist = netlist(&ir).unwrap();
        assert!(
            netlist.contains("ahdl_include \"/path/to/model.va\""),
            "osdi: {}",
            netlist
        );
    }

    #[test]
    fn test_spectre_xspice_rejected() {
        let comp = Component::Xspice {
            name: "1".into(),
            connections: vec!["in".into(), "out".into()],
            model: "d_and".into(),
        };
        assert!(matches!(
            emit_component(&comp),
            Err(CodeGenError::UnsupportedComponent { .. })
        ));
    }

    #[test]
    fn test_spectre_model_library_uses_spectre_path() {
        let mut backend_paths = BTreeMap::new();
        backend_paths.insert("ngspice".into(), "/pdk/ngspice/sky130.lib".into());
        backend_paths.insert("spectre".into(), "/pdk/spectre/sky130.scs".into());

        let ir = CircuitIR {
            top: Subcircuit {
                name: "pdk_spectre".into(),
                ..Default::default()
            },
            model_libraries: vec![ModelLibrary {
                name: "sky130".into(),
                path: "/pdk/default/sky130.lib".into(),
                corner: Some("tt".into()),
                backend_paths,
                setup_includes: vec![],
            }],
            ..Default::default()
        };
        let netlist = netlist(&ir).unwrap();
        assert!(
            netlist.contains("include \"/pdk/spectre/sky130.scs\" section=tt"),
            "spectre path: {}",
            netlist
        );
        assert!(!netlist.contains("ngspice"), "no ngspice path: {}", netlist);
    }

    #[test]
    fn test_spectre_voltage_source_ac() {
        let v = Component::VoltageSource {
            name: "in".into(),
            np: "inp".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0),
            ac_phase: Some(45.0),
            waveform: None,
            port: None,
        };
        let s = emit_component(&v).unwrap();
        assert!(s.contains("mag=1"), "spectre ac mag: {}", s);
        assert!(s.contains("phase=45"), "spectre ac phase: {}", s);

        let v_no_ac = Component::VoltageSource {
            name: "dd".into(),
            np: "vdd".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 3.3 },
            ac_magnitude: None,
            ac_phase: None,
            waveform: None,
            port: None,
        };
        let s = emit_component(&v_no_ac).unwrap();
        assert!(!s.contains("mag="), "no mag without ac: {}", s);
    }

    #[test]
    fn test_spectre_scale_factors_are_si() {
        // `[KUN]` Table B.1 vs B.2: SPICE's meg/g/t are not Spectre factors.
        assert_eq!(spectre_num(1e6), "1M");
        assert_eq!(spectre_num(2.4e9), "2.4G");
        assert_eq!(spectre_num(1e12), "1T");
        assert_eq!(spectre_num(1e3), "1k");
        assert_eq!(spectre_num(1e-9), "1n");
        assert_eq!(spectre_num(1e-15), "1f");
    }
}
