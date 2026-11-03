//! VACASK netlist generation.
//!
//! VACASK reads its own netlist language, not a SPICE dialect and not Spectre.
//! Everything below was probed against the binary
//! (`share/doc/vacask/{demo,test}/*.sim` are the authoritative examples); where
//! VACASK has no faithful spelling for something the IR can express, this file
//! returns a `CodeGenError` rather than emitting a deck that dies at parse time.
//!
//! Shape of a generated deck:
//!
//! ```text
//! <title>
//! ground 0
//! load "spice/resistor.osdi"  // one per device module actually used
//! model resistor sp_resistor  // VACASK has no anonymous instances
//! r1 (in out) resistor r=1e3
//! control
//!   options rawfile="binary"
//!   analysis op1 op
//! endc
//! ```
//!
//! Two properties make the mechanical translation trustworthy:
//!
//! 1. `spice/*.osdi` is VACASK's own build of the SPICE device set, under the
//!    same parameter names `.model` cards use, so model parameters pass through
//!    unchanged instead of through a whitelist that would go stale.
//! 2. VACASK hard-errors on an unknown parameter ("Parameter 'x' not found")
//!    rather than defaulting it, so a name that gets past this file and does not
//!    exist over there is a loud failure, never a silent substitution.
//!
//! Everything is lowercased: SPICE is case-insensitive, VACASK is not.
//!
//! This file is the deck spine (`netlist`), subcircuits and the options
//! line; `values` spells numbers, expressions and names, `models` the loads
//! and `model` lines, `components` instance lines, `analyses` the control
//! block's `analysis` and `sweep` statements, `sections` library sections.
//!
//! Contents:
//! - `pub(super) fn netlist(ir: &CircuitIR) -> Result<String, CodeGenError>`
//! - re-exported for Spectre's number respelling: `values::parse_spice_num`

mod analyses;
mod components;
mod models;
mod sections;
mod values;

use crate::ir::*;
use std::collections::BTreeSet;

use super::CodeGenError;
use analyses::{analysis_block, node_list, step_sweep};
use components::emit_component;
use models::{Need, emit_model, need};
use sections::emit_section;
pub(crate) use values::parse_spice_num;
use values::{expr, inst_params, num, q, respell, unsupported_analysis, unsupported_comp};

/// `.options` keys that only steer a simulator's own bookkeeping or printing.
/// Dropping one cannot change a number in the raw file, which is the only
/// reason a key is on this list.
const COSMETIC_OPTIONS: &[&str] = &[
    "noacct",
    "acct",
    "list",
    "node",
    "post",
    "trans",
    "nopage",
    "nomod",
    "lvlcod",
    "klu",
    "sparse",
    "savecurrents",
    "numdgt",
];

/// `.options KEY=VALUE` whose VACASK spelling is the same word and whose
/// meaning is the same quantity. Probed against the binary; `itl1` and `maxord`
/// are absent because VACASK rejects them.
const NUMERIC_OPTIONS: &[&str] = &[
    "reltol", "abstol", "vntol", "chgtol", "gmin", "temp", "tnom",
];

/// The whole VACASK deck, ending in a newline.
pub(super) fn netlist(ir: &CircuitIR) -> Result<String, CodeGenError> {
    super::refuse_raw_spice(ir, "vacask")?;
    if let Some(f) = ir.functions.first() {
        return Err(unsupported_comp(format!(
            ".func {}: VACASK has no user functions",
            f.name
        )));
    }
    // A SPICE-origin IR's expressions are lowercased as they are written.
    let low = ir.raw_dialect != Some(Dialect::Vacask);
    let mut loads: BTreeSet<String> = BTreeSet::new();
    let mut needs: BTreeSet<Need> = BTreeSet::new();

    let subs: Vec<&Subcircuit> = ir.subcircuits().collect();
    for sub in &subs {
        for comp in &sub.components {
            if let Some(n) = need(comp) {
                needs.insert(n);
            }
        }
    }
    if let Some(tb) = &ir.testbench {
        for comp in &tb.stimulus {
            if let Some(n) = need(comp) {
                needs.insert(n);
            }
        }
    }

    // Models first: they contribute loads too. A subcircuit's models stay in
    // it, where VACASK scopes them (two subcircuits may both define `qmod`).
    let mut model_lines = Vec::new();
    for m in &ir.top.models {
        model_lines.push(emit_model(m, &mut loads, low)?);
    }
    let mut sub_models: Vec<Vec<String>> = Vec::new();
    for sc in &ir.subcircuit_defs {
        sub_models.push(
            sc.models
                .iter()
                .map(|m| emit_model(m, &mut loads, low))
                .collect::<Result<_, _>>()?,
        );
    }
    // A section's models stay in it: the body's, then each definition's.
    let mut section_models: Vec<Vec<Vec<String>>> = Vec::new();
    for s in &ir.sections {
        let blocks = std::iter::once(&s.body).chain(&s.subcircuit_defs);
        let models = blocks.map(|sc| {
            sc.models
                .iter()
                .map(|m| emit_model(m, &mut loads, low))
                .collect()
        });
        section_models.push(models.collect::<Result<_, _>>()?);
    }

    for n in &needs {
        if let Need::Sp(m) = n {
            loads.insert(format!("spice/{}.osdi", m));
        }
    }
    // A VACASK deck's own `load` lines are VACASK's (it compiles a `.va`
    // itself); a SPICE-origin Verilog-A load goes through OpenVAF first.
    let user_loads = if ir.raw_dialect == Some(Dialect::Vacask) {
        ir.top.osdi_loads.clone()
    } else {
        crate::veriloga::osdi_loads(&ir.top.osdi_loads).map_err(CodeGenError::Other)?
    };
    for path in user_loads {
        loads.insert(path);
    }

    let title = ir.top.name.replace(['\n', '\r'], " ");
    let title = if title.trim().is_empty() {
        "circuit"
    } else {
        &title
    };
    let mut out = String::new();
    w!(out, "{title}\n\nground 0\n\n");
    if !ir.globals.is_empty() {
        out.push_str("global");
        for g in &ir.globals {
            w!(out, " {}", q(g));
        }
        out.push('\n');
    }

    for path in &loads {
        w!(out, "load \"{}\"\n", path);
    }

    for inc in &ir.top.includes {
        w!(out, "include \"{}\"\n", inc);
    }
    for (path, section) in &ir.top.libs {
        w!(out, "include \"{}\" section={}\n", path, section);
    }
    for lib in &ir.model_libraries {
        for setup in &lib.setup_includes {
            w!(out, "include \"{}\"\n", setup);
        }
        let path = lib.backend_paths.get("vacask").unwrap_or(&lib.path);
        match &lib.corner {
            Some(corner) => w!(out, "include \"{}\" section={}\n", path, corner),
            None => w!(out, "include \"{}\"\n", path),
        }
    }
    out.push('\n');

    for n in &needs {
        // A user model wearing a synthesised name would be a duplicate
        // definition; VACASK takes the first and the deck then runs the
        // wrong device, so refuse instead.
        if subs.iter().any(|s| {
            s.models
                .iter()
                .any(|m| m.name.eq_ignore_ascii_case(n.auto_model()))
        }) {
            return Err(CodeGenError::Other(format!(
                "model '{}' collides with the model this backend synthesises for that device; rename it",
                n.auto_model()
            )));
        }
        match n {
            Need::Sp(m) => w!(out, "model {} sp_{}\n", n.auto_model(), m),
            Need::Builtin(m) => w!(out, "model {} {}\n", n.auto_model(), m),
        }
    }
    // VACASK scopes a model to the block it is written in. The IR keeps
    // models on their subcircuit; hoisting them all to the top means a
    // model shared between definitions still resolves.
    for l in &model_lines {
        w!(out, "{l}\n");
    }
    out.push('\n');

    // `.step param` names become control variables, so they must not also
    // be declared as netlist parameters.
    let stepped: BTreeSet<String> = ir
        .testbench
        .iter()
        .flat_map(|tb| tb.step_params.iter())
        .map(|sp| sp.param.to_ascii_lowercase())
        .collect();

    let top_params: Vec<&ParamDef> = ir
        .top
        .parameters
        .iter()
        .filter(|p| !stepped.contains(&p.name.to_ascii_lowercase()))
        .collect();
    if !top_params.is_empty() {
        out.push_str("parameters");
        param_list(&mut out, top_params.into_iter(), low);
        out.push_str("\n\n");
    }

    for (sc, models) in ir.subcircuit_defs.iter().zip(&sub_models) {
        emit_subcircuit(&mut out, sc, models, low)?;
        out.push_str("\n\n");
    }
    for (s, models) in ir.sections.iter().zip(&section_models) {
        emit_section(&mut out, s, models, low)?;
        out.push_str("\n\n");
    }

    for e in ir.top.elements() {
        match e {
            Element::Component(c) => emit_component(&mut out, c, low)?,
            Element::Instance(i) => emit_instance(&mut out, i, low),
        }
        out.push('\n');
    }
    // VACASK text kept verbatim on import (`refuse_raw_spice` has checked
    // that it is VACASK's).
    for l in &ir.top.raw_spice {
        w!(out, "{l}\n");
    }

    if let Some(tb) = &ir.testbench {
        for comp in &tb.stimulus {
            emit_component(&mut out, comp, low)?;
            out.push('\n');
        }
        out.push_str("\ncontrol\n");

        w!(
            out,
            "  options rawfile=\"binary\"{}",
            emit_options(&tb.options)?
        );
        if let Some(t) = tb.temperature {
            w!(out, " temp={}", num(t));
        }
        if let Some(t) = tb.nominal_temperature {
            w!(out, " tnom={}", num(t));
        }
        out.push('\n');

        if !tb.measures.is_empty() {
            // "Command not found" — VACASK has no measure statement at all.
            return Err(unsupported_analysis(
                ".measure (VACASK has no measurement statement; post-process the raw file)",
            ));
        }

        for sp in &tb.step_params {
            w!(
                out,
                "  var {}={}\n",
                sp.param.to_ascii_lowercase(),
                num(sp.start)
            );
        }

        let step_sweeps: Vec<String> = tb
            .step_params
            .iter()
            .map(step_sweep)
            .collect::<Result<_, _>>()?;

        // IC/nodeset are analysis settings in VACASK, not standalone cards.
        let mut ic = String::new();
        if !tb.initial_conditions.is_empty() {
            ic.push_str(&node_list("ic", &tb.initial_conditions));
        }
        if !tb.node_sets.is_empty() {
            ic.push_str(&node_list("nodeset", &tb.node_sets));
        }

        // ponytail: `tb.saves` is deliberately dropped. VACASK saves every
        // node voltage and every source branch flow unless `strictsave=1`
        // is set, so a `save` line restricts nothing and only risks a parse
        // error on a SPICE-shaped output expression. Emit `save <expr>`
        // here if internal device outputs (`p(r1,i)`) are ever needed.
        for line in &tb.extra_lines {
            w!(out, "  {line}\n");
        }

        for (idx, a) in tb.analyses.iter().enumerate() {
            // A `sweep` in VACASK binds to the next `analysis`, so the
            // stepped sweeps are repeated ahead of each one.
            for s in &step_sweeps {
                w!(out, "  {s}\n");
            }
            let block = match a {
                Analysis::SpectreMonteCarlo { .. } => monte_carlo(ir, tb, idx, a)?,
                a => analysis_block(idx, a, ir.raw_dialect == Some(Dialect::Vacask))?,
            };
            for l in block {
                let ic = if l.trim_start().starts_with("analysis") {
                    ic.as_str()
                } else {
                    ""
                };
                w!(out, "  {l}{ic}\n");
            }
        }
        out.push_str("endc\n");
    }

    Ok(out)
}

// ---- private ----

/// ` a=1 b=0`: parameter declarations, a missing default as 0.
fn param_list<'a>(out: &mut String, defs: impl Iterator<Item = &'a ParamDef>, low: bool) {
    for p in defs {
        match &p.default {
            Some(d) => w!(out, " {}={}", p.name.to_ascii_lowercase(), expr(d, low)),
            None => w!(out, " {}=0", p.name.to_ascii_lowercase()),
        }
    }
}

/// A Spectre `montecarlo` as VACASK's `mc` loop (`docs/cmd-analysis-mc.md`
/// lines 7-23) around the deck's analysis of the inner kind, `lh=0`
/// (Spectre's `sampling=standard`). VACASK has no `variations` switch, so a
/// `process` or `mismatch` that would drop the deck's variation is refused
/// (monte-carlo-spectre.md §7).
fn monte_carlo(
    ir: &CircuitIR,
    tb: &Testbench,
    idx: usize,
    a: &Analysis,
) -> Result<Vec<String>, CodeGenError> {
    let Analysis::SpectreMonteCarlo {
        iterations,
        inner_type,
        seed,
        variations,
        ..
    } = a
    else {
        unreachable!("monte_carlo takes a montecarlo");
    };
    if let Some(v) = variations.as_deref().filter(|v| *v != "all") {
        return Err(unsupported_analysis(format!(
            "montecarlo variations={v}: VACASK varies every scope at once"
        )));
    }
    let inner = tb
        .analyses
        .iter()
        .find(|x| {
            !matches!(x, Analysis::SpectreMonteCarlo { .. }) && x.kind_str() == inner_type.name()
        })
        .ok_or_else(|| {
            unsupported_analysis(format!(
                "montecarlo over {} with no {} analysis to run",
                inner_type.name(),
                inner_type.name()
            ))
        })?;
    let seed = seed.map(|s| format!(" seed={s}")).unwrap_or_default();
    let mut out = vec![format!("mc mc{} samples={iterations}{seed} lh=0", idx + 1)];
    out.extend(
        analysis_block(idx, inner, ir.raw_dialect == Some(Dialect::Vacask))?
            .into_iter()
            .map(|l| format!("  {l}")),
    );
    out.push("endmc".into());
    Ok(out)
}

/// A `subckt` ... `ends` block, with no line end after `ends`.
fn emit_subcircuit(
    out: &mut String,
    sc: &Subcircuit,
    models: &[String],
    low: bool,
) -> Result<(), CodeGenError> {
    w!(out, "subckt {} (", q(&sc.name));
    for (i, p) in sc.ports.iter().enumerate() {
        w!(out, "{}{}", if i == 0 { "" } else { " " }, q(&p.name));
    }
    out.push_str(")\n");
    emit_body(out, sc, models, low)?;
    out.push_str("ends");
    Ok(())
}

/// A block's models, `parameters`, elements and raw lines, one indented
/// line each: a `subckt`'s or a `section`'s body.
fn emit_body(
    out: &mut String,
    sc: &Subcircuit,
    models: &[String],
    low: bool,
) -> Result<(), CodeGenError> {
    for m in models {
        w!(out, "  {m}\n");
    }
    if !sc.parameters.is_empty() {
        out.push_str("parameters");
        param_list(out, sc.parameters.iter(), low);
        out.push('\n');
    }
    // A SPICE definition's own `.param`s, in order: VACASK's `parameters`
    // (one that reads another cannot be overridden, P848-855).
    if !sc.locals.is_empty() {
        out.push_str("parameters");
        param_list(out, sc.locals.iter(), low);
        out.push('\n');
    }
    for e in sc.elements() {
        out.push_str("  ");
        match e {
            Element::Component(c) => emit_component(out, c, low)?,
            Element::Instance(i) => emit_instance(out, i, low),
        }
        out.push('\n');
    }
    for l in &sc.raw_spice {
        w!(out, "  {l}\n");
    }
    Ok(())
}

fn emit_options(opts: &SimOptions) -> Result<String, CodeGenError> {
    let mut s = String::new();
    for (key, val) in &opts.portable {
        let k = key.to_ascii_lowercase();
        if NUMERIC_OPTIONS.contains(&k.as_str()) {
            w!(s, " {}={}", k, respell(val));
        } else if k == "method" {
            let m = match val.to_ascii_lowercase().as_str() {
                "trap" | "trapezoidal" => "trap",
                "gear" | "gear2" => "gear2",
                other => {
                    return Err(unsupported_analysis(format!("options method={}", other)));
                }
            };
            w!(s, " tran_method=\"{}\"", m);
        } else if COSMETIC_OPTIONS.contains(&k.as_str()) {
            continue;
        } else {
            return Err(unsupported_analysis(format!(
                "option '{}' has no known VACASK spelling (pass it through \
                 backend-specific options if you know VACASK's name for it)",
                key
            )));
        }
    }
    if let Some(specific) = opts.backend_specific.get("vacask") {
        for (key, val) in specific {
            w!(s, " {}={}", key, val);
        }
    }
    Ok(s)
}

fn emit_instance(out: &mut String, inst: &Instance, low: bool) {
    w!(out, "x{} (", q(&inst.name));
    for (i, n) in inst.port_mapping.iter().enumerate() {
        w!(out, "{}{}", if i == 0 { "" } else { " " }, q(n));
    }
    w!(out, ") {}", q(&inst.subcircuit));
    inst_params(out, &inst.parameters, low);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every identifier field reaches the deck quoted and lowercased
    /// (docs/input-identifiers.md): each `/` put into a name sits inside
    /// `'...'`, wherever the name is written; expressions are lowercased,
    /// numbers kept.
    #[test]
    fn every_identifier_field_is_quoted() {
        let s = |x: &str| x.to_string();
        let v = |x: f64| IrValue::Numeric { value: x };
        let comps = vec![
            Component::Resistor {
                name: s("1"),
                n1: s("A"),
                n2: s("b"),
                value: IrValue::Expression { expr: s("Rload") },
                model: None,
                params: vec![(s("tc1"), s("1M"))],
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
        for line in deck.lines().filter(|l| !l.starts_with("load ")) {
            let mut quoted = false;
            for c in line.chars() {
                quoted ^= c == '\'';
                assert!(c != '/' || quoted, "unquoted `/` in `{line}`");
            }
        }
        for want in [
            "r'1/x' ('a/x' 'b/x') resistor r=rload tc1=1m",
            "ind1=\"l'1/x'\"",
            "ctlinst=\"'v1/x'\"",
            "x'1/x' ('a/x') 'cell/x'",
            "subckt 'cell/x' ('p/x')",
            "model 'dm/x' sp_diode",
            "global 'g/x'",
        ] {
            assert!(deck.contains(want), "no `{want}` in\n{deck}");
        }
    }

    fn emit_analysis(a: &Analysis) -> Result<String, CodeGenError> {
        Ok(analysis_block(0, a, false)?.join("\n"))
    }
    use std::collections::BTreeMap;

    fn sub(name: &str) -> Subcircuit {
        Subcircuit {
            name: name.into(),
            ..Default::default()
        }
    }

    fn tb(analyses: Vec<Analysis>) -> Testbench {
        Testbench {
            analyses,
            ..Default::default()
        }
    }

    #[test]
    fn spice_suffixes_are_respelled_for_vacask() {
        // SPICE `M` is milli, VACASK `M` is MEGA: a factor of 1e9 with nothing
        // to notice it. `1G` folded to lowercase is a VACASK parse error.
        assert_eq!(respell("1M"), "1m");
        assert_eq!(respell("1meg"), "1meg");
        assert_eq!(respell("1G"), "1G");
        assert_eq!(respell("10U"), "10u");
        assert_eq!(respell("{1k}"), "1k");
        assert_eq!(respell("myparam"), "myparam");
    }

    #[test]
    fn tran_always_carries_maxstep() {
        // `step` is only a starting step in VACASK; without `maxstep` it takes
        // strides SPICE never would.
        let a = Analysis::Transient {
            step: 1e-6,
            stop: 1e-3,
            start: None,
            max_step: None,
            uic: false,
        };
        let s = emit_analysis(&a).unwrap();
        assert!(s.contains("maxstep="), "{}", s);
        // min(TSTEP, TSTOP/50) = min(1e-6, 2e-5) = 1e-6
        assert!(s.contains("maxstep=1e-6"), "{}", s);
    }

    #[test]
    fn dc_sweep_counts_intervals_and_recomputes_the_endpoint() {
        // 0..10 by 0.5 is 21 SPICE samples => 20 intervals.
        let a = Analysis::Dc {
            sweeps: vec![DcSweep {
                source: "V1".into(),
                start: 0.0,
                stop: 10.0,
                step: 0.5,
            }],
        };
        let s = emit_analysis(&a).unwrap();
        assert!(s.contains("points=20"), "{}", s);
        assert!(s.contains("instance=\"v1\" parameter=\"dc\""), "{}", s);

        // 0..1.1 by 0.2: SPICE stops at 1.0 without ever reaching 1.1.
        let a = Analysis::Dc {
            sweeps: vec![DcSweep {
                source: "Vgs".into(),
                start: 0.0,
                stop: 1.1,
                step: 0.2,
            }],
        };
        let s = emit_analysis(&a).unwrap();
        assert!(s.contains("points=5"), "{}", s);
        assert!(s.contains("to=1e0"), "{}", s);
    }

    #[test]
    fn two_source_dc_emits_blocks_in_reverse() {
        let a = Analysis::Dc {
            sweeps: vec![
                DcSweep {
                    source: "Vds".into(),
                    start: 0.0,
                    stop: 1.0,
                    step: 0.1,
                },
                DcSweep {
                    source: "Vgs".into(),
                    start: 0.0,
                    stop: 1.0,
                    step: 0.5,
                },
            ],
        };
        let s = emit_analysis(&a).unwrap();
        let gs = s.find("\"vgs\"").unwrap();
        let ds = s.find("\"vds\"").unwrap();
        // SPICE's first source is the fast one; VACASK's last sweep is.
        assert!(gs < ds, "outer sweep must come first:\n{}", s);
    }

    #[test]
    fn ac_lin_counts_intervals_but_dec_does_not() {
        let lin = emit_analysis(&Analysis::Ac {
            sweep: FreqSweep {
                variation: Sweep::Lin,
                points: 101,
                start: 1.0,
                stop: 1e3,
            },
        })
        .unwrap();
        assert!(lin.contains("points=100"), "{}", lin);
        let dec = emit_analysis(&Analysis::Ac {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1.0,
                stop: 1e3,
            },
        })
        .unwrap();
        assert!(dec.contains("points=10"), "{}", dec);
    }

    #[test]
    fn pwl_is_refused_rather_than_emitted() {
        let c = Component::VoltageSource {
            name: "1".into(),
            np: "a".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: None,
            ac_phase: None,
            waveform: Some(IrWaveform::Pwl {
                values: vec![((0.0).into(), (0.0).into()), ((1e-3).into(), (1.0).into())],
                repeat: None,
                delay: None,
            }),
            port: None,
        };
        let err = emit_component(&mut String::new(), &c, true).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("pwl"), "{}", err);
    }

    #[test]
    fn measures_are_refused() {
        let mut t = tb(vec![Analysis::Op]);
        t.measures = vec!["tran vmax MAX V(out)".into()];
        let ir = CircuitIR {
            top: sub("m"),
            testbench: Some(t),
            ..Default::default()
        };
        assert!(netlist(&ir).is_err());
    }

    #[test]
    fn step_params_become_a_control_variable_and_a_sweep() {
        let mut t = tb(vec![Analysis::Op]);
        t.step_params = vec![StepParam {
            param: "rval".into(),
            start: 1e3,
            stop: 3e3,
            step: 1e3,
            sweep_type: None,
        }];
        let mut top = sub("stepped");
        top.parameters = vec![ParamDef {
            name: "rval".into(),
            default: Some("1k".into()),
        }];
        let ir = CircuitIR {
            top,
            testbench: Some(t),
            ..Default::default()
        };
        let n = netlist(&ir).unwrap();
        assert!(n.contains("var rval=1e3"), "{}", n);
        assert!(n.contains("sweep step_rval variable=\"rval\""), "{}", n);
        // A stepped name must not also be a netlist parameter.
        assert!(!n.contains("parameters rval"), "{}", n);
        // VACASK sweeps lin, dec or oct: a log step is refused, not run linear.
        let mut log = ir.clone();
        log.testbench.as_mut().unwrap().step_params[0].sweep_type = Some(Sweep::Log);
        assert!(netlist(&log).unwrap_err().to_string().contains("log sweep"));
        log.testbench.as_mut().unwrap().step_params[0].sweep_type = Some(Sweep::Oct);
        assert!(netlist(&log).unwrap().contains("mode=\"oct\""));
    }

    #[test]
    fn model_library_uses_the_vacask_path() {
        let mut backend_paths = BTreeMap::new();
        backend_paths.insert("ngspice".into(), "/pdk/ngspice/sky130.lib.spice".into());
        backend_paths.insert("vacask".into(), "/pdk/vacask/sky130.sim".into());
        let ir = CircuitIR {
            top: sub("pdk_test"),
            model_libraries: vec![ModelLibrary {
                name: "sky130".into(),
                path: "/pdk/default.lib".into(),
                corner: Some("tt".into()),
                backend_paths,
                setup_includes: vec![],
            }],
            ..Default::default()
        };
        let n = netlist(&ir).unwrap();
        assert!(
            n.contains("include \"/pdk/vacask/sky130.sim\" section=tt"),
            "{}",
            n
        );
        assert!(!n.contains("ngspice"), "{}", n);
    }

    #[test]
    fn model_library_falls_back_to_the_default_path() {
        let ir = CircuitIR {
            top: sub("fallback"),
            model_libraries: vec![ModelLibrary {
                name: "custom".into(),
                path: "/models/custom.sim".into(),
                corner: None,
                backend_paths: BTreeMap::new(),
                setup_includes: vec![],
            }],
            ..Default::default()
        };
        let n = netlist(&ir).unwrap();
        assert!(n.contains("include \"/models/custom.sim\""), "{}", n);
    }

    #[test]
    fn bjt_model_carries_its_polarity_and_loads_its_module() {
        let mut top = sub("bjt");
        top.models = vec![ModelDef {
            name: "QN".into(),
            kind: "NPN".into(),
            parameters: vec![("IS".into(), "1e-15".into()), ("BF".into(), "150".into())],
        }];
        let ir = CircuitIR {
            top,
            ..Default::default()
        };
        let n = netlist(&ir).unwrap();
        assert!(n.contains("load \"spice/bjt.osdi\""), "{}", n);
        assert!(
            n.contains("model qn sp_bjt (type=1 is=1e-15 bf=150)"),
            "{}",
            n
        );
    }

    #[test]
    fn a_mosfet_level_without_a_module_is_refused() {
        let mut top = sub("m");
        top.models = vec![ModelDef {
            name: "nm".into(),
            kind: "NMOS".into(),
            parameters: vec![("LEVEL".into(), "77".into())],
        }];
        let ir = CircuitIR {
            top,
            ..Default::default()
        };
        assert!(netlist(&ir).is_err());
    }

    #[test]
    fn passive_model_cards_are_refused() {
        // sp_resistor renames the model card's parameters; transcribing them
        // name-for-name would set the instance parameters instead.
        let mut top = sub("r");
        top.models = vec![ModelDef {
            name: "rm".into(),
            kind: "R".into(),
            parameters: vec![("tc1".into(), "1e-3".into())],
        }];
        let ir = CircuitIR {
            top,
            ..Default::default()
        };
        assert!(netlist(&ir).is_err());
    }

    #[test]
    fn unknown_options_refuse_instead_of_being_dropped() {
        let mut t = tb(vec![Analysis::Op]);
        t.options.portable = vec![("itl1".into(), "100".into())];
        let ir = CircuitIR {
            top: sub("o"),
            testbench: Some(t),
            ..Default::default()
        };
        assert!(netlist(&ir).is_err());

        let mut t = tb(vec![Analysis::Op]);
        t.options.portable = vec![
            ("reltol".into(), "1e-4".into()),
            ("noacct".into(), "".into()),
        ];
        let ir = CircuitIR {
            top: sub("o"),
            testbench: Some(t),
            ..Default::default()
        };
        let n = netlist(&ir).unwrap();
        assert!(n.contains("reltol=1e-4"), "{}", n);
        assert!(!n.contains("noacct"), "{}", n);
    }
}

#[cfg(test)]
mod number_tests {
    /// An expression that starts like a number is not one.
    #[test]
    fn expressions_are_not_truncated_to_their_leading_number() {
        assert_eq!(super::parse_spice_num("1k*(1+x)"), None);
        assert_eq!(super::parse_spice_num("108f").map(|x| x.0), Some(1.08e-13));
        assert_eq!(super::parse_spice_num("5V").map(|x| x.0), Some(5.0));
    }
}
