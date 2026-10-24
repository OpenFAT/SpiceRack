//! SPICE3-family netlists: ngspice, LTspice, HSPICE, and ESPice (ngspice
//! text with Verilog-A loaded as `.hdl`).

use super::{format_spice_number as num, laplace, param_defs, params, CodeGenError, Dialect};
use crate::ir::*;

/// ESPice reads ngspice decks, so everywhere but the model loads it is ngspice
/// — including the `backend_paths`/`backend_specific` key it looks up.
fn key(d: Dialect) -> &'static str {
    match d {
        Dialect::Ltspice => "ltspice",
        Dialect::Hspice => "hspice",
        Dialect::Xyce => "xyce",
        _ => "ngspice",
    }
}

/// ngspice proper, or ESPice reading an ngspice deck.
fn ngspice(d: Dialect) -> bool {
    matches!(d, Dialect::Ngspice | Dialect::Espice)
}

/// ngspice and LTspice share the SPICE3 option spelling; only the
/// iteration-limit alias needs mapping, and both call it `ITL1`.
fn itl1(k: &str) -> &str {
    if k == "max_iterations" { "ITL1" } else { k }
}

fn value(d: Dialect, v: &IrValue) -> String {
    match v {
        IrValue::Numeric { value } => num(*value),
        // HSPICE reads braces as name bytes; only quotes make an expression.
        IrValue::Expression { expr } if d == Dialect::Hspice => format!("'{}'", expr),
        IrValue::Expression { expr } => format!("{{{}}}", expr),
        IrValue::Raw { text } => text.clone(),
    }
}

/// R/C/L value field, with an optional `.model` reference after it:
/// ` [value] model`. The import side leaves `value` an empty `Raw` when the
/// bare word was a model name, so an empty text is skipped rather than
/// written as a blank field.
fn rclvalue(d: Dialect, v: &IrValue, model: &Option<String>) -> String {
    let val = match v {
        IrValue::Raw { text } if text.is_empty() => String::new(),
        v => format!(" {}", value(d, v)),
    };
    match model {
        Some(m) => format!("{val} {m}"),
        None => val,
    }
}

fn waveform(d: Dialect, wf: &IrWaveform) -> String {
    let w = |x: &IrNum| match x {
        IrNum::Num(v) => format!("{v}"),
        x => value(d, &x.into()),
    };
    match wf {
        IrWaveform::Sin { offset, amplitude, frequency, delay, damping, phase } => {
            format!("SIN({} {} {} {} {} {})", w(offset), w(amplitude), w(frequency), w(delay), w(damping), w(phase))
        }
        IrWaveform::Pulse { initial, pulsed, delay, rise_time, fall_time, pulse_width, period } => {
            format!("PULSE({} {} {} {} {} {} {})", w(initial), w(pulsed), w(delay), w(rise_time), w(fall_time), w(pulse_width), w(period))
        }
        IrWaveform::Pwl { values } => {
            let pts: Vec<String> = values.iter().map(|(t, v)| format!("{} {}", w(t), w(v))).collect();
            format!("PWL({})", pts.join(" "))
        }
        IrWaveform::Exp { initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau } => {
            format!("EXP({} {} {} {} {} {})", w(initial), w(pulsed), w(rise_delay), w(rise_tau), w(fall_delay), w(fall_tau))
        }
        IrWaveform::Sffm { offset, amplitude, carrier_freq, modulation_index, signal_freq } => {
            format!("SFFM({} {} {} {} {})", w(offset), w(amplitude), w(carrier_freq), w(modulation_index), w(signal_freq))
        }
        IrWaveform::Am { amplitude, offset, modulating_freq, carrier_freq, delay } => {
            format!("AM({} {} {} {} {})", w(amplitude), w(offset), w(modulating_freq), w(carrier_freq), w(delay))
        }
    }
}

/// A `Laplace(...)` behavioural source, rewritten into something the
/// dialect actually parses.
///
/// The verbatim `B1 out 0 V=Laplace(v(in), 1/(1+s*1.59e-4))` this replaces
/// is not syntax any supported simulator has: ngspice 44.2 answers it with
/// `Undefined parameter [s]` and `exit(1)`. See `codegen::laplace` for the
/// evidence behind both replacements.
///
/// * ngspice: the XSPICE `s_xfer` code model, two lines (instance +
///   `.model`). Verified numerically.
/// * LTspice: `Laplace=` on an E/G source. **Unverified** — LTspice cannot
///   be run here. The expression is still parsed first, so what goes out is
///   a transfer function we understand rather than an opaque string.
fn laplace_source(
    d: Dialect,
    name: &str,
    np: &str,
    nm: &str,
    expression: &str,
    current_output: bool,
) -> Result<String, CodeGenError> {
    let parsed = laplace::parse(expression).map_err(|why| CodeGenError::Other(format!("B{name}: {why}")))?;
    let (ip, im) = parsed.input().clone();
    let letter = if current_output { 'G' } else { 'E' };

    // A constant H(s) is a plain controlled source in every dialect. It is
    // also the only safe option on ngspice: `den_coeff=[1]` segfaults
    // ngspice 44.2 outright.
    if let laplace::Laplace::Gain { gain, .. } = parsed {
        return Ok(format!("{letter}B{name} {np} {nm} {ip} {im} {}", num(gain)));
    }
    let laplace::Laplace::Xfer { num, den, .. } = parsed else { unreachable!() };

    // Xyce has no LAPLACE E/G form (RG §6.1.5).
    if d == Dialect::Xyce {
        return Err(CodeGenError::UnsupportedComponent { backend: "xyce".into(), component: format!("B{name}: Laplace source") });
    }
    if d == Dialect::Ltspice {
        return Ok(format!(
            "{letter}B{name} {np} {nm} {ip} {im} Laplace={}/{}",
            laplace::horner(&num),
            laplace::horner(&den),
        ));
    }
    // `E1 b 0 LAPLACE a 0 k0 k1 ... / d0 d1 ...`, ascending powers.
    if d == Dialect::Hspice {
        let asc = |c: &[f64]| laplace::coeff_array(&c.iter().rev().copied().collect::<Vec<_>>());
        return Ok(format!("{letter}B{name} {np} {nm} LAPLACE {ip} {im} {} / {}", asc(&num), asc(&den)));
    }
    let port = if current_output { "id" } else { "vd" };
    // One integrator stage per denominator order; `int_ic=[]` is
    // rejected with "Array parameter must have at least one value".
    let ic = vec!["0"; den.len() - 1].join(" ");
    Ok(format!(
        "AB{name} %vd({ip} {im}) %{port}({np} {nm}) B{name}_xfer\n\
         .model B{name}_xfer s_xfer(num_coeff=[{}] den_coeff=[{}] int_ic=[{ic}])",
        laplace::coeff_array(&num),
        laplace::coeff_array(&den),
    ))
}

/// Xyce rejects BSIM3 as level 8 and reads 49 (RG Table 2-36; 7.10 run).
fn model(d: Dialect, m: &ModelDef) -> String {
    let mos = matches!(m.kind.to_ascii_lowercase().as_str(), "nmos" | "pmos");
    let level8 = |(k, v): &(String, String)| k.eq_ignore_ascii_case("level") && v.trim() == "8";
    let m = &ModelDef {
        parameters: m.parameters.iter().map(|p| if d == Dialect::Xyce && mos && level8(p) { (p.0.clone(), "49".into()) } else { p.clone() }).collect(),
        ..m.clone()
    };
    let mut s = format!(".model {} {}", m.name, m.kind);
    if !m.parameters.is_empty() {
        let ps: Vec<String> = m.parameters.iter().map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{k}={v}") }).collect();
        s.push_str(&format!("({})", ps.join(" ")));
    }
    s
}

fn instance(inst: &Instance) -> String {
    let mut s = format!("X{}", inst.name);
    for port in &inst.port_mapping {
        s.push_str(&format!(" {}", port));
    }
    format!("{s} {}{}", inst.subcircuit, params(&inst.parameters))
}

/// Components, instances, raw text — the part of a block after its models.
fn body(d: Dialect, sc: &Subcircuit, lines: &mut Vec<String>) -> Result<(), CodeGenError> {
    for comp in &sc.components {
        lines.push(component(d, comp)?);
    }
    lines.extend(sc.instances.iter().map(instance));
    lines.extend(sc.raw_spice.iter().cloned());
    Ok(())
}

/// A block's `.include` and `.lib` cards, in the block: SPICE scopes a
/// `.lib` inside a `.subckt` to that subcircuit (ngspice's
/// lib-processing/ex1b), so hoisting one changes which models it sees.
/// HSPICE quotes paths (`.lib 'file' section`).
/// A library with no section is LTspice's `.lib file` (the whole file);
/// ngspice and HSPICE would read that card as a section definition.
fn file_cards(d: Dialect, sc: &Subcircuit, lines: &mut Vec<String>) -> Result<(), CodeGenError> {
    let path = |p: &str| if d == Dialect::Hspice { format!("'{p}'") } else { p.to_string() };
    lines.extend(sc.includes.iter().map(|inc| format!(".include {}", path(inc))));
    for (p, section) in &sc.libs {
        match (section.is_empty(), d) {
            (true, Dialect::Ltspice) => lines.push(format!(".lib {}", path(p))),
            (true, _) => return Err(CodeGenError::Other(format!(".lib {p} with no section is LTspice's whole-file form; {d:?} reads it as a section definition"))),
            (false, _) => lines.push(format!(".lib {} {}", path(p), section)),
        }
    }
    Ok(())
}

fn subcircuit(d: Dialect, sc: &Subcircuit) -> Result<String, CodeGenError> {
    let mut header = format!(".subckt {}", sc.name);
    for port in &sc.ports {
        header.push_str(&format!(" {}", port.name));
    }
    // HSPICE takes the defaults bare after the ports.
    if !sc.parameters.is_empty() && d != Dialect::Hspice {
        header.push_str(" PARAMS:");
    }
    header.push_str(&param_defs(&sc.parameters));
    let mut lines = vec![header];
    file_cards(d, sc, &mut lines)?;
    lines.extend(sc.models.iter().map(|m| model(d, m)));
    body(d, sc, &mut lines)?;
    lines.push(format!(".ends {}", sc.name));
    Ok(lines.join("\n"))
}

/// `.step` sweeps.
///
/// ngspice has no `.step` at all — it aborts the run with
/// "unimplemented dot command '.step'" (verified on 44.2). Emitting it
/// commented out is worse than failing: the sweep silently collapses to a
/// single run and the caller gets one curve where they asked for N. So this
/// is a hard error naming the two things that do work.
fn step_param(d: Dialect, sp: &StepParam) -> Result<String, CodeGenError> {
    // HSPICE sweeps a parameter on the analysis card (`sweep`) or with
    // `.alter`; there is no `.step`.
    if d == Dialect::Hspice {
        return Err(CodeGenError::Other(format!(
            "HSPICE has no `.step` (stepping '{}'). Sweep it on the analysis card \
             or drive the sweep from Python by re-running the testbench per value.",
            sp.param,
        )));
    }
    // Xyce: `.step [lin|dec|oct] <name> <start> <stop> <inc>`, no `param` (RG p.155).
    if d == Dialect::Xyce {
        let sweep = sp.sweep_type.as_ref().map(|t| format!("{t} ")).unwrap_or_default();
        return Ok(format!(".step {sweep}{} {} {} {}", sp.param, sp.start, sp.stop, sp.step));
    }
    if d != Dialect::Ltspice {
        return Err(CodeGenError::Other(format!(
            "ngspice has no `.step` (stepping '{}' would silently run once). \
             Use `.dc {} {} {} {}` for a temperature or source sweep, \
             or drive the sweep from Python by re-running the testbench per value.",
            sp.param, sp.param, sp.start, sp.stop, sp.step,
        )));
    }
    // LTspice: `.step [lin|oct|dec] param <name> <start> <stop> <inc>`.
    // The `param` keyword is mandatory in both forms.
    Ok(match sp.sweep_type {
        Some(ref sweep) => format!(".step {} param {} {} {} {}", sweep, sp.param, sp.start, sp.stop, sp.step),
        None => format!(".step param {} {} {} {}", sp.param, sp.start, sp.stop, sp.step),
    })
}

/// Model loads that must precede any card instantiating them: ngspice's
/// `pre_osdi` control block, ESPice's and HSPICE's `.hdl` (they compile
/// Verilog-A themselves and load no OSDI binaries), nothing for LTspice.
fn loads(d: Dialect, ir: &CircuitIR, lines: &mut Vec<String>) -> Result<(), CodeGenError> {
    match d {
        Dialect::Ngspice if !ir.top.osdi_loads.is_empty() => {
            let loads = crate::veriloga::osdi_loads(&ir.top.osdi_loads).map_err(CodeGenError::Other)?;
            // pre_osdi is a control command, not a dot command
            lines.push(".control".into());
            lines.extend(loads.iter().map(|p| format!("pre_osdi {}", p)));
            lines.push(".endc".into());
        }
        Dialect::Espice => {
            let all = ir.top.osdi_loads.iter().chain(ir.subcircuit_defs.iter().flat_map(|s| &s.osdi_loads));
            for l in all {
                if !crate::veriloga::is_source(l) {
                    return Err(CodeGenError::Other(format!(
                        "ESPice does not load OSDI binaries ({l}); load the Verilog-A source with veriloga(\"model.va\")"
                    )));
                }
                let hdl = format!(".hdl \"{}\"", l);
                if !lines.contains(&hdl) {
                    lines.push(hdl);
                }
            }
        }
        Dialect::Xyce if !ir.top.osdi_loads.is_empty() => {
            return Err(CodeGenError::Other("Xyce loads Verilog-A through -plugin, not a deck card".into()));
        }
        Dialect::Hspice => {
            for l in &ir.top.osdi_loads {
                if !crate::veriloga::is_source(l) {
                    return Err(CodeGenError::Other(format!(
                        "HSPICE loads Verilog-A source, not OSDI ({l}); use veriloga(\"model.va\")"
                    )));
                }
                lines.push(format!(".hdl \"{}\"", l));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Xyce's options: `.options <pkg> k=v` per package (RG pp.96-104). The
/// importer keeps Xyce's own as `pkg.key` under `backend_specific["xyce"]`;
/// portable ones map through the RG tables, and one with no Xyce spelling is
/// refused rather than guessed.
fn xyce_options(tb: &Testbench) -> Result<Vec<String>, CodeGenError> {
    // (package, `k=v` or bare `k`), in order; consecutive entries of one
    // package share a line, so a deck's own order reads back unchanged.
    let mut items: Vec<(String, String)> = Vec::new();
    let kv = |k: &str, v: &str| if v.is_empty() { k.to_string() } else { format!("{k}={v}") };
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
                return Err(CodeGenError::UnsupportedAnalysis { backend: "xyce".into(), analysis: format!("option '{other}' (no Xyce package mapped)") });
            }
        };
        items.push((pkg.into(), kv(key, v)));
    }
    if let Some(t) = tb.temperature {
        items.push(("device".into(), kv("temp", &num(t))));
    }
    if let Some(t) = tb.nominal_temperature {
        items.push(("device".into(), kv("tnom", &num(t))));
    }
    // `pkg.key`, a bare package `pkg.` (`.options post`), or a key with no
    // package (`.option scale=`, package ""), each kept as the deck wrote it.
    for (pk, v) in tb.options.backend_specific.get("xyce").into_iter().flatten() {
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
            _ => out.push((pkg.clone(), if pkg.is_empty() { item } else if item.is_empty() { pkg } else { format!("{pkg} {item}") })),
        }
    }
    Ok(out.into_iter().map(|(_, l)| format!(".options {l}")).collect())
}

/// Raw lines imported from one dialect reach only that dialect (ngspice and
/// ESPice read the same text): ngspice's TRNOISE or `.probe` copied into an
/// HSPICE deck would be a silently different deck.
fn check_raw_dialect(d: Dialect, ir: &CircuitIR) -> Result<(), CodeGenError> {
    let Some(from) = ir.raw_dialect.as_deref() else { return Ok(()) };
    let family = |name: &str| if name == "espice" { "ngspice".to_string() } else { name.to_string() };
    let to = format!("{d:?}").to_ascii_lowercase();
    let raw = std::iter::once(&ir.top).chain(&ir.subcircuit_defs).find_map(|s| s.raw_spice.first());
    match raw {
        Some(line) if family(from) != family(&to) => Err(CodeGenError::UnsupportedComponent {
            backend: to,
            component: format!("{from} line '{}' (kept verbatim on import; it is {from} text)", line.trim()),
        }),
        _ => Ok(()),
    }
}

pub(super) fn netlist(d: Dialect, ir: &CircuitIR) -> Result<String, CodeGenError> {
    check_raw_dialect(d, ir)?;
    // Title must be first line (SPICE treats line 1 as title/comment)
    let mut lines = vec![format!("* {}", ir.top.name)];
    loads(d, ir, &mut lines)?;

    if !ir.globals.is_empty() {
        lines.push(format!(".global {}", ir.globals.join(" ")));
    }
    // HSPICE spells a function as a parameter: `.param f(x)='x*2'`.
    for f in &ir.functions {
        let args = f.args.join(", ");
        lines.push(match d {
            Dialect::Hspice => format!(".param {}({args})='{}'", f.name, f.body),
            _ => format!(".func {}({args}) {{{}}}", f.name, f.body),
        });
    }
    file_cards(d, &ir.top, &mut lines)?;
    let path = |p: &str| if d == Dialect::Hspice { format!("'{p}'") } else { p.to_string() };

    for lib in &ir.model_libraries {
        lines.extend(lib.setup_includes.iter().map(|s| format!(".include {}", path(s))));
        let path = path(lib.backend_paths.get(key(d)).unwrap_or(&lib.path));
        lines.push(match &lib.corner {
            Some(corner) => format!(".lib {} {}", path, corner),
            None => format!(".include {}", path),
        });
    }

    for p in &ir.top.parameters {
        if let Some(ref default) = p.default {
            lines.push(format!(".param {}={}", p.name, default));
        }
    }
    lines.extend(ir.top.models.iter().map(|m| model(d, m)));

    for sc in &ir.subcircuit_defs {
        lines.push(String::new());
        lines.push(subcircuit(d, sc)?);
    }

    body(d, &ir.top, &mut lines)?;

    if let Some(ref tb) = ir.testbench {
        for comp in &tb.stimulus {
            lines.push(component(d, comp)?);
        }

        if d == Dialect::Xyce {
            lines.extend(xyce_options(tb)?);
        } else {
            let opts = super::option_pairs(&tb.options, key(d), itl1);
            if !opts.is_empty() {
                lines.push(format!(".options {}", opts.join(" ")));
            }
            if let Some(temp) = tb.temperature {
                lines.push(format!(".temp {}", temp));
            }
            if let Some(tnom) = tb.nominal_temperature {
                lines.push(format!(".options tnom={}", tnom));
            }
        }
        lines.extend(tb.initial_conditions.iter().map(|(n, v)| format!(".ic V({})={}", n, v)));
        lines.extend(tb.node_sets.iter().map(|(n, v)| format!(".nodeset V({})={}", n, v)));

        // A noise run produces `onoise_spectrum`/`inoise_spectrum`, not node
        // voltages, so a node-name save list can never name them. ngspice then
        // reports "no data saved for Noise analysis; analysis not run" and the
        // run fails outright. Saving everything is the only correct list here;
        // LTspice has no `.save all` and saves everything by default, so it
        // just gets no save list.
        // ngspice reads one `.save` card: `all` beside named outputs means
        // all of them plus those, and alone it is what no list means.
        let noise = tb.analyses.iter().any(|a| matches!(a, Analysis::Noise { .. }));
        // Xyce's `.save` writes the operating point; its rawfile (-r) holds
        // every vector, so a save list has nothing to restrict there.
        if d == Dialect::Xyce {
        } else if d == Dialect::Hspice && !noise {
            // HSPICE's `.save` saves simulator state; `.probe` selects outputs.
            lines.extend(tb.saves.iter().map(|s| if s == "all" { ".option post".into() } else { format!(".probe {s}") }));
        } else if ngspice(d) && noise {
            let named = tb.saves.iter().filter(|s| *s != "all").map(|s| format!(" {s}")).collect::<String>();
            lines.push(format!(".save all{named}"));
        } else if !tb.saves.is_empty() && !noise {
            lines.push(format!(".save {}", tb.saves.join(" ")));
        }
        // `.print` prints to stdout; LTspice has no such card, and a rawfile
        // run ignores it, so there it is omitted.
        if d != Dialect::Ltspice {
            for p in &tb.prints {
                let analysis = if p.analysis.is_empty() { String::new() } else { format!(" {}", p.analysis) };
                lines.push(format!(".print{analysis} {}", p.outputs.join(" ")));
            }
        }

        for meas in &tb.measures {
            // `.MEASURE TRAN ...` is already a card, in any case.
            lines.push(if meas.to_ascii_lowercase().starts_with(".meas") { meas.clone() } else { format!(".meas {}", meas) });
        }
        for sp in &tb.step_params {
            lines.push(step_param(d, sp)?);
        }
        lines.extend(tb.extra_lines.iter().cloned());
        for a in &tb.analyses {
            lines.push(analysis(d, a)?);
        }
    }

    lines.push(".end".into());
    Ok(lines.join("\n"))
}

pub(crate) fn component(d: Dialect, comp: &Component) -> Result<String, CodeGenError> {
    Ok(match comp {
        Component::Resistor { name, n1, n2, value: v, model, params: p } => {
            if model.is_some() && d == Dialect::Ltspice {
                return Err(CodeGenError::UnsupportedComponent { backend: "ltspice".into(), component: format!("R{name}: model reference") });
            }
            format!("R{} {} {}{}{}", name, n1, n2, rclvalue(d, v, model), params(p))
        }
        Component::Capacitor { name, n1, n2, value: v, model, params: p } => {
            if model.is_some() && d == Dialect::Ltspice {
                return Err(CodeGenError::UnsupportedComponent { backend: "ltspice".into(), component: format!("C{name}: model reference") });
            }
            format!("C{} {} {}{}{}", name, n1, n2, rclvalue(d, v, model), params(p))
        }
        Component::Inductor { name, n1, n2, value: v, model, params: p } => {
            if model.is_some() && d == Dialect::Ltspice {
                return Err(CodeGenError::UnsupportedComponent { backend: "ltspice".into(), component: format!("L{name}: model reference") });
            }
            format!("L{} {} {}{}{}", name, n1, n2, rclvalue(d, v, model), params(p))
        }
        Component::MutualInductor { name, inductor1, inductor2, coupling } => {
            format!("K{} L{} L{} {}", name, inductor1, inductor2, coupling)
        }
        Component::VoltageSource { name, np, nm, value: v, ac_magnitude, ac_phase, waveform: wf }
        | Component::CurrentSource { name, np, nm, value: v, ac_magnitude, ac_phase, waveform: wf } => {
            let letter = if matches!(comp, Component::VoltageSource { .. }) { 'V' } else { 'I' };
            let mut s = format!("{}{} {} {} {}", letter, name, np, nm, value(d, v));
            if let Some(mag) = ac_magnitude {
                s.push_str(&format!(" AC {}", mag));
                if let Some(phase) = ac_phase {
                    s.push_str(&format!(" {}", phase));
                }
            }
            if let Some(wf) = wf {
                s.push_str(&format!(" {}", waveform(d, wf)));
            }
            s
        }
        Component::BehavioralVoltage { name, np, nm, expression } => {
            if laplace::is_laplace(expression) {
                return laplace_source(d, name, np, nm, expression, false);
            }
            if d == Dialect::Hspice {
                return Ok(format!("E{} {} {} vol='{}'", name, np, nm, expression));
            }
            // Xyce takes the B expression braced (RG p.238).
            if d == Dialect::Xyce {
                return Ok(format!("B{name} {np} {nm} V={{{expression}}}"));
            }
            format!("B{} {} {} V={}", name, np, nm, expression)
        }
        Component::BehavioralCurrent { name, np, nm, expression } => {
            if laplace::is_laplace(expression) {
                return laplace_source(d, name, np, nm, expression, true);
            }
            if d == Dialect::Hspice {
                return Ok(format!("G{} {} {} cur='{}'", name, np, nm, expression));
            }
            if d == Dialect::Xyce {
                return Ok(format!("B{name} {np} {nm} I={{{expression}}}"));
            }
            format!("B{} {} {} I={}", name, np, nm, expression)
        }
        Component::Vcvs { name, np, nm, ncp, ncm, gain } => format!("E{} {} {} {} {} {}", name, np, nm, ncp, ncm, gain),
        Component::Vccs { name, np, nm, ncp, ncm, transconductance } => {
            format!("G{} {} {} {} {} {}", name, np, nm, ncp, ncm, transconductance)
        }
        Component::Cccs { name, np, nm, vsense, gain } => format!("F{} {} {} {} {}", name, np, nm, vsense, gain),
        Component::Poly { name, letter, np, nm, controls, coeffs } => {
            let dims = if matches!(letter, 'E' | 'G') { controls.len() / 2 } else { controls.len() };
            let k: Vec<String> = coeffs.iter().map(|c| super::format_spice_number(*c)).collect();
            format!("{letter}{name} {np} {nm} POLY({dims}) {} {}", controls.join(" "), k.join(" "))
        }
        Component::Ccvs { name, np, nm, vsense, transresistance } => {
            format!("H{} {} {} {} {}", name, np, nm, vsense, transresistance)
        }
        Component::Diode { name, np, nm, model, params: p } => format!("D{} {} {} {}{}", name, np, nm, model, params(p)),
        Component::Bjt { name, nc, nb, ne, extra_nodes, model, params: p } => {
            format!("Q{} {} {} {}{} {}{}", name, nc, nb, ne, extra_nodes.iter().map(|n| format!(" {n}")).collect::<String>(), model, params(p))
        }
        Component::Mosfet { name, nd, ng, ns, nb, extra_nodes, model, params: p } => {
            let rest: String = nb.iter().chain(extra_nodes).map(|n| format!(" {n}")).collect();
            format!("M{} {} {} {}{rest} {}{}", name, nd, ng, ns, model, params(p))
        }
        Component::Jfet { name, nd, ng, ns, model, params: p } => format!("J{} {} {} {} {}{}", name, nd, ng, ns, model, params(p)),
        Component::Mesfet { name, nd, ng, ns, model, params: p } => format!("Z{} {} {} {} {}{}", name, nd, ng, ns, model, params(p)),
        Component::VSwitch { name, np, nm, ncp, ncm, model } => format!("S{} {} {} {} {} {}", name, np, nm, ncp, ncm, model),
        Component::ISwitch { name, np, nm, vcontrol, model } => format!("W{} {} {} {} {}", name, np, nm, vcontrol, model),
        Component::TLine { name, inp, inm, outp, outm, z0, td } => {
            format!("T{} {} {} {} {} Z0={} TD={}", name, inp, inm, outp, outm, z0, td)
        }
        Component::Xspice { name, .. } if matches!(d, Dialect::Hspice | Dialect::Xyce) => {
            return Err(CodeGenError::UnsupportedComponent {
                backend: key(d).into(),
                component: format!("XSPICE A{}", name),
            });
        }
        Component::Xspice { name, connections, model } => {
            // LTspice has no XSPICE A-elements -- emit as comment
            let lead = if d == Dialect::Ltspice { "* XSPICE (unsupported): " } else { "" };
            let mut s = format!("{lead}A{}", name);
            for conn in connections {
                s.push_str(&format!(" {}", conn));
            }
            s.push_str(&format!(" {}", model));
            s
        }
        Component::RawSpice { line } => line.clone(),
    })
}

fn analysis(d: Dialect, a: &Analysis) -> Result<String, CodeGenError> {
    let (lt, ng, hs) = (d == Dialect::Ltspice, ngspice(d), d == Dialect::Hspice);
    Ok(match a {
        Analysis::Op => ".op".into(),
        Analysis::Dc { sweeps } => {
            let mut s = String::from(".dc");
            for sw in sweeps {
                s.push_str(&format!(" {} {} {} {}", sw.source, num(sw.start), num(sw.stop), num(sw.step)));
            }
            s
        }
        Analysis::Ac { variation, points, start, stop } => {
            format!(".ac {} {} {} {}", variation, points, num(*start), num(*stop))
        }
        // HSPICE reads extra positional fields as further step/stop
        // segments, so the start is `START=` and the step cap `delmax`.
        Analysis::Transient { step, stop, start, max_step, uic } if hs => {
            let mut s = String::new();
            if let Some(ms) = max_step {
                s.push_str(&format!(".option delmax={}\n", num(*ms)));
            }
            s.push_str(&format!(".tran {} {}", num(*step), num(*stop)));
            if let Some(st) = start {
                s.push_str(&format!(" START={}", num(*st)));
            }
            if *uic {
                s.push_str(" UIC");
            }
            s
        }
        Analysis::Transient { step, stop, start, max_step, uic } => {
            let mut s = format!(".tran {} {}", num(*step), num(*stop));
            if let Some(st) = start {
                s.push_str(&format!(" {}", num(*st)));
            }
            if let Some(ms) = max_step {
                if start.is_none() {
                    // Need a placeholder for start when max_step is given
                    s.push_str(" 0");
                }
                s.push_str(&format!(" {}", num(*ms)));
            }
            if *uic {
                s.push_str(" UIC");
            }
            s
        }
        Analysis::Noise { output, reference, source, variation, points, start, stop, points_per_summary } => {
            let out_spec = if reference.is_empty() || reference == "0" {
                format!("V({})", output)
            } else {
                format!("V({},{})", output, reference)
            };
            // HSPICE: `.noise ov src inter` takes its frequencies from `.ac`.
            if hs {
                return Ok(format!(
                    ".ac {} {} {} {}\n.noise {} {} {}",
                    variation, points, num(*start), num(*stop), out_spec, source, points_per_summary.unwrap_or(0),
                ));
            }
            let mut s = format!(".noise {} {} {} {} {} {}", out_spec, source, variation, points, num(*start), num(*stop));
            // `pts_per_summary` is an ngspice/SPICE3 extension; LTspice's
            // `.noise` card stops at the stop frequency.
            if let (Some(pps), true) = (points_per_summary, ng || d == Dialect::Xyce) {
                s.push_str(&format!(" {}", pps));
            }
            s
        }
        // Xyce has no .tf (RG Table 6-1).
        Analysis::Tf { .. } if d == Dialect::Xyce => {
            return Err(CodeGenError::UnsupportedAnalysis { backend: "xyce".into(), analysis: "Tf".into() });
        }
        Analysis::Tf { output, source } => format!(".tf {} {}", output, source),
        Analysis::Fourier { fundamental, outputs, num_harmonics } => {
            // SPICE3/ngspice: `.four <freq> <ov1> [ov2 ...]` — there is no
            // harmonic-count field. Emitting one makes ngspice warn
            // "can't parse '10': ignored" and then fail the analysis with
            // "lengths don't match" because it treats the count as an output
            // vector. LTspice's card *does* take `[Nharmonics]`.
            let mut s = format!(".four {}", num(*fundamental));
            if let (Some(nh), true) = (num_harmonics, lt) {
                s.push_str(&format!(" {}", nh));
            }
            for out in outputs {
                s.push_str(&format!(" {}", out));
            }
            s
        }
        // LTspice has no `.sens`, `.pz`, `.disto`, `.pss` or `.sp` card;
        // HSPICE's `.sens` is DC only and it has none of the others.
        Analysis::Sensitivity { output, ac } if ng || (hs && ac.is_none()) => {
            let mut s = format!(".sens {}", output);
            if let Some(p) = ac {
                s.push_str(&format!(" AC {} {} {} {}", p.variation, p.points, num(p.start), num(p.stop)));
            }
            s
        }
        Analysis::PoleZero { node1, node2, node3, node4, tf_type, pz_type } if ng => {
            format!(".pz {} {} {} {} {} {}", node1, node2, node3, node4, tf_type, pz_type)
        }
        Analysis::Distortion { variation, points, start, stop, f2overf1 } if ng => {
            let mut s = format!(".disto {} {} {} {}", variation, points, num(*start), num(*stop));
            if let Some(ratio) = f2overf1 {
                s.push_str(&format!(" {}", ratio));
            }
            s
        }
        // ponytail: Rust `{}` numbers, byte-for-byte what the retired string
        // path sent ngspice for these two cards.
        Analysis::Pss { fundamental, stabilization, observe_node, points_per_period, harmonics } if ng => {
            format!(".pss {} {} {} {} {}", fundamental, stabilization, observe_node, points_per_period, harmonics)
        }
        Analysis::SPar { variation, points, start, stop } if ng => {
            format!(".sp {} {} {} {}", variation, points, start, stop)
        }
        // Harmonic balance, loop-gain stability, transient noise and the
        // SpectreRF family have no SPICE3-family card.
        other => {
            let dbg = format!("{other:?}");
            return Err(CodeGenError::UnsupportedAnalysis {
                backend: key(d).into(),
                analysis: dbg.split([' ', '(']).next().unwrap_or_default().into(),
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sample_resistor_divider() -> CircuitIR {
        CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "Voltage Divider".into(),
                ports: vec![],
                parameters: vec![],
                components: vec![
                    Component::VoltageSource {
                        name: "in".into(),
                        np: "input".into(),
                        nm: "0".into(),
                        value: IrValue::Numeric { value: 10.0 },
                        ac_magnitude: None,
                        ac_phase: None,
                        waveform: None,
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
                instances: vec![],
                models: vec![],
                raw_spice: vec![],
                includes: vec![],
                libs: vec![],
                osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: Some(Testbench { prints: vec![],
                dut: "Voltage Divider".into(),
                stimulus: vec![],
                analyses: vec![Analysis::Op],
                options: SimOptions::default(),
                saves: vec![],
                measures: vec![],
                temperature: None,
                nominal_temperature: None,
                initial_conditions: vec![],
                node_sets: vec![],
                step_params: vec![],
                extra_lines: vec![],
            }),
            subcircuit_defs: vec![],
            model_libraries: vec![],
        }
    }

    #[test]
    fn test_ngspice_resistor_divider() {
        let ir = sample_resistor_divider();
        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(netlist.contains("* Voltage Divider"), "missing title: {}", netlist);
        assert!(netlist.contains("Vin input 0 10"), "missing Vin: {}", netlist);
        assert!(netlist.contains("R1 input output 10k"), "missing R1: {}", netlist);
        assert!(netlist.contains("R2 output 0 10k"), "missing R2: {}", netlist);
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
        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "CMOS Inverter".into(),
                ports: vec![],
                parameters: vec![],
                components: vec![
                    Component::Mosfet {
                        name: "1".into(),
                        nd: "out".into(),
                        ng: "in".into(),
                        ns: "vdd".into(),
                        nb: Some("vdd".into()), extra_nodes: vec![],
                        model: "pmos_3p3".into(),
                        params: vec![("W".into(), "2u".into()), ("L".into(), "180n".into())],
                    },
                    Component::Mosfet {
                        name: "2".into(),
                        nd: "out".into(),
                        ng: "in".into(),
                        ns: "0".into(),
                        nb: Some("0".into()), extra_nodes: vec![],
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
                    },
                ],
                instances: vec![],
                models: vec![
                    ModelDef {
                        name: "nmos_3p3".into(),
                        kind: "NMOS".into(),
                        parameters: vec![("VTO".into(), "0.7".into()), ("KP".into(), "110e-6".into())],
                    },
                ],
                raw_spice: vec![],
                includes: vec![],
                libs: vec![],
                osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: Some(Testbench { prints: vec![],
                dut: "CMOS Inverter".into(),
                stimulus: vec![],
                analyses: vec![
                    Analysis::Dc { sweeps: vec![DcSweep { source: "Vdd".into(), start: 0.0, stop: 3.3, step: 0.01 }] },
                ],
                options: SimOptions::default(),
                saves: vec![],
                measures: vec![],
                temperature: Some(27.0),
                nominal_temperature: None,
                initial_conditions: vec![],
                node_sets: vec![],
                step_params: vec![],
                extra_lines: vec![],
            }),
            subcircuit_defs: vec![],
            model_libraries: vec![],
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(netlist.contains("M1 out in vdd vdd pmos_3p3 W=2u L=180n"), "missing M1: {}", netlist);
        assert!(netlist.contains("M2 out in 0 0 nmos_3p3 W=1u L=180n"), "missing M2: {}", netlist);
        assert!(netlist.contains(".model nmos_3p3 NMOS(VTO=0.7 KP=110e-6)"), "missing model: {}", netlist);
        assert!(netlist.contains(".temp 27"), "missing temp: {}", netlist);
        assert!(netlist.contains(".dc Vdd 0 3"), "missing dc: {}", netlist);
    }

    #[test]
    fn test_subcircuit_instances() {
        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "Top".into(),
                ports: vec![],
                parameters: vec![],
                components: vec![
                    Component::VoltageSource {
                        name: "dd".into(),
                        np: "vdd".into(),
                        nm: "0".into(),
                        value: IrValue::Numeric { value: 1.8 },
                        ac_magnitude: None,
                        ac_phase: None,
                        waveform: None,
                    },
                ],
                instances: vec![
                    Instance {
                        name: "1".into(),
                        subcircuit: "inverter".into(),
                        port_mapping: vec!["in".into(), "out".into(), "vdd".into(), "0".into()],
                        parameters: vec![("wp".into(), "2u".into())],
                    },
                ],
                models: vec![],
                raw_spice: vec![],
                includes: vec![],
                libs: vec![],
                osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: None,
            subcircuit_defs: vec![
                Subcircuit {
                    name: "inverter".into(),
                    ports: vec![
                        Port { name: "in".into(), direction: PortDirection::Input },
                        Port { name: "out".into(), direction: PortDirection::Output },
                        Port { name: "vdd".into(), direction: PortDirection::InOut },
                        Port { name: "gnd".into(), direction: PortDirection::InOut },
                    ],
                    parameters: vec![
                        ParamDef { name: "wp".into(), default: Some("1u".into()) },
                        ParamDef { name: "wn".into(), default: Some("500n".into()) },
                    ],
                    components: vec![
                        Component::Mosfet {
                            name: "p".into(),
                            nd: "out".into(),
                            ng: "in".into(),
                            ns: "vdd".into(),
                            nb: Some("vdd".into()), extra_nodes: vec![],
                            model: "pmos".into(),
                            params: vec![("W".into(), "{wp}".into()), ("L".into(), "180n".into())],
                        },
                        Component::Mosfet {
                            name: "n".into(),
                            nd: "out".into(),
                            ng: "in".into(),
                            ns: "gnd".into(),
                            nb: Some("gnd".into()), extra_nodes: vec![],
                            model: "nmos".into(),
                            params: vec![("W".into(), "{wn}".into()), ("L".into(), "180n".into())],
                        },
                    ],
                    instances: vec![],
                    models: vec![],
                    raw_spice: vec![],
                    includes: vec![],
                    libs: vec![],
                    osdi_loads: vec![],
                    verilog_blocks: vec![],
                },
            ],
            model_libraries: vec![],
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(netlist.contains(".subckt inverter in out vdd gnd PARAMS: wp=1u wn=500n"), "missing subckt header: {}", netlist);
        assert!(netlist.contains("Mp out in vdd vdd pmos W={wp} L=180n"), "missing Mp: {}", netlist);
        assert!(netlist.contains(".ends inverter"), "missing .ends: {}", netlist);
        assert!(netlist.contains("X1 in out vdd 0 inverter wp=2u"), "missing instance: {}", netlist);
    }

    #[test]
    fn test_waveforms_sin_pulse_pwl() {
        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "Waveforms".into(),
                ports: vec![],
                parameters: vec![],
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
                    },
                    Component::VoltageSource {
                        name: "pwl".into(),
                        np: "pwl_out".into(),
                        nm: "0".into(),
                        value: IrValue::Numeric { value: 0.0 },
                        ac_magnitude: None,
                        ac_phase: None,
                        waveform: Some(IrWaveform::Pwl {
                            values: vec![((0.0).into(), (0.0).into()), ((1e-6).into(), (3.3).into()), ((2e-6).into(), (0.0).into())],
                        }),
                    },
                ],
                instances: vec![],
                models: vec![],
                raw_spice: vec![],
                includes: vec![],
                libs: vec![],
                osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: None,
            subcircuit_defs: vec![],
            model_libraries: vec![],
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(netlist.contains("SIN(1.65 1.65 1000000"), "missing SIN: {}", netlist);
        assert!(netlist.contains("PULSE(0 3.3 0"), "missing PULSE: {}", netlist);
        assert!(netlist.contains("PWL(0 0"), "missing PWL: {}", netlist);
    }

    #[test]
    fn test_all_analysis_types() {
        let cg = Dialect::Ngspice;

        // Op
        assert_eq!(analysis(cg, &Analysis::Op).unwrap(), ".op");

        // DC
        let dc = Analysis::Dc {
            sweeps: vec![DcSweep { source: "V1".into(), start: 0.0, stop: 5.0, step: 0.1 }],
        };
        let dc_str = analysis(cg, &dc).unwrap();
        assert!(dc_str.starts_with(".dc V1"), "dc: {}", dc_str);

        // AC
        let ac = Analysis::Ac { variation: "dec".into(), points: 100, start: 1.0, stop: 1e9 };
        let ac_str = analysis(cg, &ac).unwrap();
        assert!(ac_str.starts_with(".ac dec 100"), "ac: {}", ac_str);

        // Transient
        let tran = Analysis::Transient { step: 1e-9, stop: 1e-6, start: None, max_step: None, uic: false };
        let tran_str = analysis(cg, &tran).unwrap();
        assert!(tran_str.starts_with(".tran 1n"), "tran: {}", tran_str);

        // Transient with UIC
        let tran_uic = Analysis::Transient { step: 1e-9, stop: 1e-6, start: None, max_step: None, uic: true };
        let tran_uic_str = analysis(cg, &tran_uic).unwrap();
        assert!(tran_uic_str.contains("UIC"), "tran_uic: {}", tran_uic_str);

        // Noise
        let noise = Analysis::Noise {
            output: "out".into(),
            reference: "0".into(),
            source: "V1".into(),
            variation: "dec".into(),
            points: 10,
            start: 1.0,
            stop: 1e6,
            points_per_summary: None,
        };
        let noise_str = analysis(cg, &noise).unwrap();
        assert!(noise_str.starts_with(".noise V(out) V1 dec"), "noise: {}", noise_str);

        // TF
        let tf = Analysis::Tf { output: "V(out)".into(), source: "Vin".into() };
        assert_eq!(analysis(cg, &tf).unwrap(), ".tf V(out) Vin");

        // Sensitivity
        let sens = Analysis::Sensitivity { output: "V(out)".into(), ac: None };
        assert_eq!(analysis(cg, &sens).unwrap(), ".sens V(out)");

        // PoleZero
        let pz = Analysis::PoleZero {
            node1: "1".into(), node2: "0".into(), node3: "3".into(), node4: "0".into(),
            tf_type: "vol".into(), pz_type: "pz".into(),
        };
        assert_eq!(analysis(cg, &pz).unwrap(), ".pz 1 0 3 0 vol pz");

        // Distortion
        let disto = Analysis::Distortion {
            variation: "dec".into(), points: 10, start: 1e3, stop: 1e6, f2overf1: None,
        };
        let disto_str = analysis(cg, &disto).unwrap();
        assert!(disto_str.starts_with(".disto dec 10"), "disto: {}", disto_str);

        // Fourier
        let four = Analysis::Fourier {
            fundamental: 1e3,
            outputs: vec!["V(out)".into()],
            num_harmonics: Some(10),
        };
        // ngspice's `.four` card has no harmonic-count field: a count there is
        // read as an output vector and kills the analysis.
        let four_str = analysis(cg, &four).unwrap();
        assert_eq!(four_str, ".four 1k V(out)", "four: {}", four_str);

        // LTspice's card does take `[Nharmonics]`.
        let cg_lt = Dialect::Ltspice;
        assert_eq!(analysis(cg_lt, &four).unwrap(), ".four 1k 10 V(out)");
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
        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "OSDI Test".into(),
                ports: vec![],
                parameters: vec![],
                components: vec![],
                instances: vec![],
                models: vec![],
                raw_spice: vec![],
                includes: vec![],
                libs: vec![],
                osdi_loads: vec!["/path/to/model.osdi".into()],
                verilog_blocks: vec![],
            },
            testbench: None,
            subcircuit_defs: vec![],
            model_libraries: vec![],
        };

        // ngspice wraps OSDI loads in .control block
        let cg_ng = Dialect::Ngspice;
        let ng_str = netlist(cg_ng, &ir).unwrap();
        assert!(ng_str.contains(".control\npre_osdi /path/to/model.osdi\n.endc"),
            "ng osdi control block: {}", ng_str);
        assert!(!ng_str.contains(".pre_osdi"), "no dot-command form: {}", ng_str);

        // LTspice does not
        let cg_lt = Dialect::Ltspice;
        let lt_str = netlist(cg_lt, &ir).unwrap();
        assert!(!lt_str.contains("pre_osdi"), "lt should not have osdi: {}", lt_str);
    }

    #[test]
    fn test_xspice_ngspice_vs_others() {
        let comp = Component::Xspice {
            name: "1".into(),
            connections: vec!["[in1 in2]".into(), "out".into()],
            model: "d_and".into(),
        };

        let cg_ng = Dialect::Ngspice;
        let ng_str = component(cg_ng, &comp).unwrap();
        assert!(ng_str.starts_with("A1 "), "ng xspice: {}", ng_str);
        assert!(ng_str.contains("d_and"), "ng xspice model: {}", ng_str);

        let cg_lt = Dialect::Ltspice;
        let lt_str = component(cg_lt, &comp).unwrap();
        assert!(lt_str.starts_with("* XSPICE"), "lt xspice should be comment: {}", lt_str);
    }

    #[test]
    fn test_step_params_ltspice_ok_ngspice_errors() {
        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "Step Test".into(),
                ports: vec![],
                parameters: vec![],
                components: vec![
                    Component::Resistor {
                        name: "1".into(),
                        n1: "a".into(),
                        n2: "0".into(),
                        value: IrValue::Expression { expr: "rval".into() },
                        model: None,
                        params: vec![],
                    },
                ],
                instances: vec![],
                models: vec![],
                raw_spice: vec![],
                includes: vec![],
                libs: vec![],
                osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: Some(Testbench { prints: vec![],
                dut: "Step Test".into(),
                stimulus: vec![],
                analyses: vec![Analysis::Op],
                options: SimOptions::default(),
                saves: vec![],
                measures: vec![],
                temperature: None,
                nominal_temperature: None,
                initial_conditions: vec![],
                node_sets: vec![],
                step_params: vec![StepParam {
                    param: "rval".into(),
                    start: 1000.0,
                    stop: 10000.0,
                    step: 1000.0,
                    sweep_type: None,
                }],
                extra_lines: vec![],
            }),
            subcircuit_defs: vec![],
            model_libraries: vec![],
        };

        // LTspice emits a native .step
        let cg = Dialect::Ltspice;
        let lt = netlist(cg, &ir).unwrap();
        assert!(lt.contains(".step param rval 1000 10000 1000"), "step: {}", lt);

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
            sweep_type: Some("dec".into()),
        };
        assert_eq!(step_param(cg, &sp).unwrap(), ".step dec param rval 1000 10000 10");
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
        assert!(netlist.contains(".meas tran vmax MAX V(output)"), "measure: {}", netlist);
    }


    // ── Issue 03: ModelLibrary per-backend path + corner ──

    #[test]
    fn test_model_library_backend_path_ngspice() {
        let mut backend_paths = BTreeMap::new();
        backend_paths.insert("ngspice".into(), "/pdk/ngspice/sky130.lib".into());
        backend_paths.insert("spectre".into(), "/pdk/spectre/sky130.scs".into());

        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "pdk_test".into(),
                ports: vec![], parameters: vec![], components: vec![],
                instances: vec![], models: vec![], raw_spice: vec![],
                includes: vec![], libs: vec![], osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: Some(Testbench { prints: vec![],
                dut: "pdk_test".into(), stimulus: vec![],
                analyses: vec![Analysis::Op],
                options: SimOptions::default(), saves: vec![],
                measures: vec![], temperature: None, nominal_temperature: None,
                initial_conditions: vec![], node_sets: vec![],
                step_params: vec![], extra_lines: vec![],
            }),
            subcircuit_defs: vec![],
            model_libraries: vec![ModelLibrary {
                name: "sky130".into(),
                path: "/pdk/default/sky130.lib".into(),
                corner: Some("tt".into()),
                backend_paths,
                setup_includes: vec![],
            }],
        };

        // Ngspice selects its path
        let ng = Dialect::Ngspice;
        let ng_netlist = netlist(ng, &ir).unwrap();
        assert!(ng_netlist.contains(".lib /pdk/ngspice/sky130.lib tt"),
            "ngspice path: {}", ng_netlist);
        assert!(!ng_netlist.contains("spectre"), "no spectre path in ngspice: {}", ng_netlist);

        // LTspice falls back to the default path (no ltspice key)
        let lt = Dialect::Ltspice;
        let lt_netlist = netlist(lt, &ir).unwrap();
        assert!(lt_netlist.contains(".lib /pdk/default/sky130.lib tt"),
            "ltspice fallback: {}", lt_netlist);
    }

    #[test]
    fn test_model_library_corner_switch() {
        let ir_tt = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "corner".into(),
                ports: vec![], parameters: vec![], components: vec![],
                instances: vec![], models: vec![], raw_spice: vec![],
                includes: vec![], libs: vec![], osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: None, subcircuit_defs: vec![],
            model_libraries: vec![ModelLibrary {
                name: "sky130".into(),
                path: "/pdk/sky130.lib".into(),
                corner: Some("tt".into()),
                backend_paths: BTreeMap::new(),
                setup_includes: vec![],
            }],
        };

        let mut ir_ss = ir_tt.clone();
        ir_ss.model_libraries[0].corner = Some("ss".into());

        let cg = Dialect::Ngspice;

        let tt_netlist = netlist(cg, &ir_tt).unwrap();
        let ss_netlist = netlist(cg, &ir_ss).unwrap();

        assert!(tt_netlist.contains(".lib /pdk/sky130.lib tt"), "tt: {}", tt_netlist);
        assert!(ss_netlist.contains(".lib /pdk/sky130.lib ss"), "ss: {}", ss_netlist);
        assert!(!tt_netlist.contains("ss"), "tt has no ss");
    }

    #[test]
    fn test_model_library_no_corner_uses_include() {
        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "nocorner".into(),
                ports: vec![], parameters: vec![], components: vec![],
                instances: vec![], models: vec![], raw_spice: vec![],
                includes: vec![], libs: vec![], osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: None, subcircuit_defs: vec![],
            model_libraries: vec![ModelLibrary {
                name: "custom".into(),
                path: "/models/custom.lib".into(),
                corner: None,
                backend_paths: BTreeMap::new(),
                setup_includes: vec![],
            }],
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();
        assert!(netlist.contains(".include /models/custom.lib"), "include: {}", netlist);
        assert!(!netlist.contains(".lib /models"), "no .lib without corner: {}", netlist);
    }

    // ── Issue 05: Dialect correctness ──

    #[test]
    fn test_ngspice_uses_save() {
        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "ng_save".into(),
                ports: vec![], parameters: vec![],
                components: vec![], instances: vec![], models: vec![],
                raw_spice: vec![], includes: vec![], libs: vec![],
                osdi_loads: vec![], verilog_blocks: vec![],
            },
            testbench: Some(Testbench { prints: vec![],
                dut: "ng_save".into(), stimulus: vec![],
                analyses: vec![Analysis::Op],
                options: SimOptions::default(),
                saves: vec!["V(a)".into()],
                measures: vec![], temperature: None, nominal_temperature: None,
                initial_conditions: vec![], node_sets: vec![],
                step_params: vec![], extra_lines: vec![],
            }),
            subcircuit_defs: vec![], model_libraries: vec![],
        };

        let ng = Dialect::Ngspice;
        let netlist = netlist(ng, &ir).unwrap();
        assert!(netlist.contains(".save V(a)"), "ngspice uses .save: {}", netlist);
    }

    #[test]
    fn test_pz_disto_sens_error_on_ltspice() {
        let cg_xy = Dialect::Ltspice;
        let pz = Analysis::PoleZero {
            node1: "1".into(), node2: "0".into(),
            node3: "3".into(), node4: "0".into(),
            tf_type: "vol".into(), pz_type: "pz".into(),
        };
        assert!(analysis(cg_xy, &pz).is_err(), "pz should error on ltspice");

        let disto = Analysis::Distortion {
            variation: "dec".into(), points: 10,
            start: 1e3, stop: 1e6, f2overf1: None,
        };
        assert!(analysis(cg_xy, &disto).is_err(), "disto should error on ltspice");

        let sens = Analysis::Sensitivity { output: "V(out)".into(), ac: None };
        assert!(analysis(cg_xy, &sens).is_err(), "sens should error on ltspice");
    }

    #[test]
    fn test_pz_disto_ok_on_ngspice() {
        let cg_ng = Dialect::Ngspice;
        let pz = Analysis::PoleZero {
            node1: "1".into(), node2: "0".into(),
            node3: "3".into(), node4: "0".into(),
            tf_type: "vol".into(), pz_type: "pz".into(),
        };
        assert!(analysis(cg_ng, &pz).is_ok(), "pz should work on ngspice");
    }

    #[test]
    fn test_voltage_source_ac_magnitude_and_phase() {
        let cg = Dialect::Ngspice;

        // AC magnitude only
        let v_ac = Component::VoltageSource {
            name: "in".into(), np: "inp".into(), nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0), ac_phase: None,
            waveform: None,
        };
        let s = component(cg, &v_ac).unwrap();
        assert_eq!(s, "Vin inp 0 0 AC 1", "ac mag only: {}", s);

        // AC magnitude + phase
        let v_ac_phase = Component::VoltageSource {
            name: "in".into(), np: "inp".into(), nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0), ac_phase: Some(45.0),
            waveform: None,
        };
        let s = component(cg, &v_ac_phase).unwrap();
        assert_eq!(s, "Vin inp 0 0 AC 1 45", "ac mag+phase: {}", s);

        // No AC — regression check
        let v_dc = Component::VoltageSource {
            name: "dd".into(), np: "vdd".into(), nm: "0".into(),
            value: IrValue::Numeric { value: 3.3 },
            ac_magnitude: None, ac_phase: None,
            waveform: None,
        };
        let s = component(cg, &v_dc).unwrap();
        assert_eq!(s, "Vdd vdd 0 3.3", "dc only: {}", s);
        assert!(!s.contains("AC"), "no AC in dc-only source");
    }

    #[test]
    fn test_current_source_ac_magnitude() {
        let cg = Dialect::Ngspice;
        let i_ac = Component::CurrentSource {
            name: "ac".into(), np: "a".into(), nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1e-3), ac_phase: Some(90.0),
            waveform: None,
        };
        let s = component(cg, &i_ac).unwrap();
        assert!(s.contains("AC 0.001 90"), "current AC: {}", s);
    }

    #[test]
    fn test_pdk_netlist_lib_before_subckt_and_components() {
        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "cs_amp_tb".into(),
                ports: vec![], parameters: vec![],
                components: vec![],
                instances: vec![Instance {
                    name: "dut".into(),
                    subcircuit: "cs_amp".into(),
                    port_mapping: vec!["vdd".into(), "vin".into(), "vout".into()],
                    parameters: vec![],
                }],
                models: vec![], raw_spice: vec![],
                includes: vec![], libs: vec![], osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: Some(Testbench { prints: vec![],
                dut: "cs_amp_tb".into(), stimulus: vec![
                    Component::VoltageSource {
                        name: "dd".into(), np: "vdd".into(), nm: "0".into(),
                        value: IrValue::Numeric { value: 1.8 },
                        ac_magnitude: None, ac_phase: None,
                        waveform: None,
                    },
                ],
                analyses: vec![Analysis::Op],
                options: SimOptions::default(), saves: vec![],
                measures: vec![], temperature: None, nominal_temperature: None,
                initial_conditions: vec![], node_sets: vec![],
                step_params: vec![], extra_lines: vec![],
            }),
            subcircuit_defs: vec![Subcircuit {
                name: "cs_amp".into(),
                ports: vec![
                    Port { name: "vdd".into(), direction: PortDirection::InOut },
                    Port { name: "vin".into(), direction: PortDirection::Input },
                    Port { name: "vout".into(), direction: PortDirection::Output },
                ],
                parameters: vec![], models: vec![],
                components: vec![
                    Component::Resistor {
                        name: "drain".into(), n1: "vdd".into(), n2: "vout".into(),
                        value: IrValue::Numeric { value: 5e3 }, model: None, params: vec![],
                    },
                    Component::Mosfet {
                        name: "n1".into(), nd: "vout".into(), ng: "vin".into(),
                        ns: "0".into(), nb: Some("0".into()), extra_nodes: vec![],
                        model: "sky130_fd_pr__nfet_01v8".into(),
                        params: vec![],
                    },
                ],
                instances: vec![], raw_spice: vec![],
                includes: vec![], libs: vec![], osdi_loads: vec![],
                verilog_blocks: vec![],
            }],
            model_libraries: vec![ModelLibrary {
                name: "sky130".into(),
                path: "/pdk/sky130.lib.spice".into(),
                corner: Some("tt".into()),
                backend_paths: BTreeMap::new(),
                setup_includes: vec![],
            }],
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();

        // Title must be line 1
        let lines: Vec<&str> = netlist.lines().collect();
        assert!(lines[0].starts_with("*"), "first line is title: {}", lines[0]);

        // .lib must come before .subckt
        let lib_pos = netlist.find(".lib").expect("must have .lib");
        let subckt_pos = netlist.find(".subckt").expect("must have .subckt");
        assert!(lib_pos < subckt_pos,
            ".lib ({}) must come before .subckt ({}):\n{}", lib_pos, subckt_pos, netlist);

        // .lib must come before components
        let mosfet_pos = netlist.find("Mn1").expect("must have Mn1");
        assert!(lib_pos < mosfet_pos,
            ".lib must come before MOSFET:\n{}", netlist);

        // Verify the .lib line itself
        assert!(netlist.contains(".lib /pdk/sky130.lib.spice tt"),
            "lib line: {}", netlist);
    }

    #[test]
    fn test_setup_includes_emitted_before_lib() {
        let ir = CircuitIR { raw_dialect: None, globals: vec![], functions: vec![],
            top: Subcircuit {
                name: "gf180_test".into(),
                ports: vec![], parameters: vec![], components: vec![],
                instances: vec![], models: vec![], raw_spice: vec![],
                includes: vec![], libs: vec![], osdi_loads: vec![],
                verilog_blocks: vec![],
            },
            testbench: Some(Testbench { prints: vec![],
                dut: "gf180_test".into(), stimulus: vec![],
                analyses: vec![Analysis::Op],
                options: SimOptions::default(), saves: vec![],
                measures: vec![], temperature: None, nominal_temperature: None,
                initial_conditions: vec![], node_sets: vec![],
                step_params: vec![], extra_lines: vec![],
            }),
            subcircuit_defs: vec![],
            model_libraries: vec![ModelLibrary {
                name: "gf180".into(),
                path: "/pdk/gf180/sm141064.ngspice".into(),
                corner: Some("typical".into()),
                backend_paths: BTreeMap::new(),
                setup_includes: vec!["/pdk/gf180/design.ngspice".into()],
            }],
        };

        let cg = Dialect::Ngspice;
        let netlist = netlist(cg, &ir).unwrap();

        // setup_includes emitted as .include
        assert!(netlist.contains(".include /pdk/gf180/design.ngspice"),
            "setup include present: {}", netlist);

        // setup_includes comes BEFORE the .lib
        let setup_pos = netlist.find(".include /pdk/gf180/design.ngspice").unwrap();
        let lib_pos = netlist.find(".lib /pdk/gf180/sm141064.ngspice typical").unwrap();
        assert!(setup_pos < lib_pos,
            "setup ({}) before lib ({}): {}", setup_pos, lib_pos, netlist);
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
        assert!(n.starts_with("* Voltage Divider\n.hdl \"/m/vres.va\"\n"), "{n}");
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
            fundamental: 1e9, stabilization: 1e-7, observe_node: "v(out)".into(),
            points_per_period: 128, harmonics: 10,
        };
        let sp = Analysis::SPar { variation: "dec".into(), points: 10, start: 1e6, stop: 1e10 };
        assert_eq!(analysis(Dialect::Ngspice, &pss).unwrap(), ".pss 1000000000 0.0000001 v(out) 128 10");
        assert_eq!(analysis(Dialect::Ngspice, &sp).unwrap(), ".sp dec 10 1000000 10000000000");
        for a in [&pss, &sp] {
            assert!(analysis(Dialect::Ltspice, a).is_err(), "{a:?}");
        }
        for a in [
            Analysis::HarmonicBalance { frequencies: vec![1e6], harmonics: vec![7] },
            Analysis::TransientNoise { step: 1e-9, stop: 1e-6 },
        ] {
            assert!(matches!(analysis(Dialect::Ngspice, &a), Err(CodeGenError::UnsupportedAnalysis { .. })));
        }
    }
}
