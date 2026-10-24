//! VACASK deck text -> [`CircuitIR`]: the inverse of `codegen::vacask`, read
//! one logical line at a time and checked against `vacask.peg` (entry
//! `OneLine`, `grammar/vacask_one.peg`). VACASK is case-sensitive; names
//! are compared as written, and instance names lowercased as the codegen
//! writes them. A statement the IR has no field for stays VACASK text: in
//! the `control` block as `extra_lines`, elsewhere as raw lines.

use std::collections::HashMap;

use super::{FuncDef, Report, close_subckt, logical_lines, sub, testbench};
use crate::ir::*;

/// A VACASK number (`docs/input-numbers.md`): an SI prefix (`M`/`meg`/`x`
/// mega, `m` milli, `mil`), then an ignored unit; one decimal parse.
fn num(tok: &str) -> Option<f64> {
    let t = tok.trim();
    let end = t
        .char_indices()
        .take_while(|&(i, c)| c.is_ascii_digit() || c == '.' || (i == 0 && matches!(c, '+' | '-')) || (matches!(c, '+' | '-') && t[..i].ends_with(['e', 'E'])) || (matches!(c, 'e' | 'E') && t[i + 1..].starts_with(|n: char| n.is_ascii_digit() || n == '+' || n == '-')))
        .last()
        .map(|(i, c)| i + c.len_utf8())?;
    let tail = &t[end..];
    if !tail.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if tail.starts_with("mil") {
        return Some(t[..end].parse::<f64>().ok()? * 25.4e-6);
    }
    let exp = if tail.starts_with("meg") {
        6
    } else {
        match tail.chars().next() {
            None => 0,
            Some('T') => 12,
            Some('G') => 9,
            Some('M' | 'x' | 'X') => 6,
            Some('k' | 'K') => 3,
            Some('m') => -3,
            Some('u') => -6,
            Some('n') => -9,
            Some('p') => -12,
            Some('f') => -15,
            Some('a') => -18,
            Some(_) => return None,
        }
    };
    let (base, e) = match t[..end].split_once(['e', 'E']) {
        Some((b, e)) => (b, e.parse::<i32>().ok()?),
        None => (&t[..end], 0),
    };
    format!("{base}e{}", e + exp).parse().ok()
}

fn spice_text(v: &str) -> String {
    num(v).map_or_else(|| v.to_string(), crate::codegen::format_spice_number)
}

fn value(v: &str) -> IrValue {
    match num(v) {
        Some(value) => IrValue::Numeric { value },
        None => IrValue::Expression { expr: v.to_string() },
    }
}

/// `'quoted'` name parts unquoted (`''` is a quote).
fn unquote_name(n: &str) -> String {
    let mut out = String::new();
    let mut it = n.chars().peekable();
    let mut quoted = false;
    while let Some(c) = it.next() {
        match c {
            '\'' if quoted && it.peek() == Some(&'\'') => {
                it.next();
                out.push('\'');
            }
            '\'' => quoted = !quoted,
            _ => out.push(c),
        }
    }
    out
}

fn string(v: &str) -> Option<String> {
    Some(v.strip_prefix('"')?.strip_suffix('"')?.to_string())
}

/// Fields: blanks separate; `(...)`, `[...]`, `{...}`, `"..."` and `'...'`
/// stay whole; a `(` opening a node or parameter list after a bare name
/// (`r1(a b)`, `sp_mos3(type=1)`) starts a field of its own.
fn fields(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let (mut depth, mut quote) = (0i32, None::<char>);
    for c in line.chars() {
        match (quote, c) {
            (Some(q), _) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            (None, '"' | '\'') => {
                quote = Some(c);
                cur.push(c);
            }
            (None, '(' | '[' | '{') => {
                // Only after a bare name: `||(1<4)` is an operator and a group.
                if depth == 0 && c == '(' && !cur.is_empty() && !cur.contains('=') && cur.chars().all(|x| x.is_ascii_alphanumeric() || matches!(x, '_' | '$' | '\'')) {
                    out.push(std::mem::take(&mut cur));
                }
                depth += 1;
                cur.push(c);
            }
            (None, ')' | ']' | '}') => {
                depth -= 1;
                cur.push(c);
            }
            (None, c) if depth == 0 && c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    let mut joined: Vec<String> = Vec::new();
    for f in out {
        match joined.last_mut() {
            Some(prev) if prev.ends_with('=') || f.starts_with('=') => prev.push_str(&f),
            _ => joined.push(f),
        }
    }
    joined
}

/// `k=v` fields, a `(k=v ...)` group opened up. A field with no `=` after a
/// value continues it: an expression may hold blanks (`dc=! 0.0`).
fn pairs(fs: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for f in fs {
        // A new pair's key is an identifier; `>=` in `dc=3 >= 2` is not one.
        let kv = f.split_once('=').filter(|(k, _)| is_key(k));
        match (f.strip_prefix('(').and_then(|g| g.strip_suffix(')')), kv, out.last_mut()) {
            // `(k=v ...)`: a parameter list; a group with no `=` is an operand.
            (Some(g), _, _) if g.contains('=') && f.split_once('=').is_none_or(|(k, _)| !is_key(k)) => out.extend(pairs(&fields(g))),
            (_, Some((k, v)), _) => out.push((k.to_string(), v.to_string())),
            (_, None, Some((_, prev))) => {
                prev.push(' ');
                prev.push_str(f);
            }
            _ => {}
        }
    }
    // `unif (2k)` and `unif(2k)` are one call; the line layer keeps no blank
    // before a bracket, so a value has none either.
    out.into_iter().map(|(k, v)| (k, v.replace(" (", "("))).collect()
}

fn is_key(k: &str) -> bool {
    k.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$') && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn get<'a>(p: &'a [(String, String)], k: &str) -> Option<&'a str> {
    p.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn nodes(f: &str) -> Option<Vec<String>> {
    let inner = f.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.split_whitespace().map(unquote_name).collect())
}

/// `sp_<module>` (and `type=`) -> SPICE kind and LEVEL: the inverse of
/// `codegen::vacask`'s `mos_module`/`model_type`.
fn spice_kind(module: &str, ty: Option<f64>) -> Option<(String, Option<&'static str>)> {
    let neg = ty.is_some_and(|t| t < 0.0);
    let mos = |l| Some((if neg { "pmos" } else { "nmos" }.to_string(), Some(l)));
    match module.strip_prefix("sp_")? {
        "mos1" => mos("1"),
        "mos2" => mos("2"),
        "mos3" => mos("3"),
        "mos6" => mos("6"),
        "mos9" => mos("9"),
        "bsim3v3" => mos("49"),
        "bsim4v8" => mos("54"),
        "diode" => Some(("d".into(), None)),
        "bjt" => Some((if neg { "pnp" } else { "npn" }.into(), None)),
        "jfet1" => Some((if neg { "pjf" } else { "njf" }.into(), None)),
        "mes1" => Some((if neg { "pmf" } else { "nmf" }.into(), None)),
        "vdmos" => Some(("vdmos".into(), None)),
        _ => None,
    }
}

/// The module names `codegen::vacask` declares itself (`model resistor
/// sp_resistor`, `model vsource vsource`): implied by the devices.
const AUTO_MODELS: &[&str] = &["resistor", "capacitor", "inductor", "vsource", "isource", "vcvs", "vccs", "cccs", "ccvs", "mutual"];

/// The device a `model NAME <module>` names when `<module>` is a built-in
/// or a SPICE R/C/L: `vsource`, `sp_resistor` -> `resistor`.
fn builtin(module: &str) -> Option<&'static str> {
    let m = module.strip_prefix("sp_").unwrap_or(module);
    AUTO_MODELS.iter().find(|a| **a == m).copied()
}

fn strip(name: &str, letter: char) -> String {
    let name = unquote_name(name).to_lowercase();
    match name.chars().next() {
        Some(c) if c == letter && name.len() > 1 => name[1..].to_string(),
        _ => name,
    }
}

const SOURCE_KEYS: &[&str] = &["dc", "mag", "phase", "type", "val0", "val1", "delay", "rise", "fall", "width", "period", "sinedc", "ampl", "freq", "theta", "modindex", "modfreq", "tau1", "td2", "tau2", "wave"];

fn waveform(p: &[(String, String)]) -> Result<Option<IrWaveform>, ()> {
    let f = |k: &str| get(p, k).map_or(Some(0.0), num).ok_or(());
    Ok(Some(match get(p, "type").and_then(string).as_deref() {
        None | Some("dc") => return Ok(None),
        Some("pulse") => IrWaveform::Pulse { initial: f("val0")?, pulsed: f("val1")?, delay: f("delay")?, rise_time: f("rise")?, fall_time: f("fall")?, pulse_width: f("width")?, period: f("period")? },
        Some("sine") => IrWaveform::Sin { offset: f("sinedc")?, amplitude: f("ampl")?, frequency: f("freq")?, delay: f("delay")?, damping: f("theta")?, phase: 0.0 },
        // VACASK's td2 counts from `delay`; SPICE's is absolute.
        Some("exp") => IrWaveform::Exp { initial: f("val0")?, pulsed: f("val1")?, rise_delay: f("delay")?, rise_tau: f("tau1")?, fall_delay: f("delay")? + f("td2")?, fall_tau: f("tau2")? },
        Some("fm") => IrWaveform::Sffm { offset: f("sinedc")?, amplitude: f("ampl")?, carrier_freq: f("freq")?, modulation_index: f("modindex")?, signal_freq: f("modfreq")? },
        Some("am") => IrWaveform::Am { offset: f("sinedc")?, amplitude: f("ampl")?, carrier_freq: f("freq")?, modulating_freq: f("modfreq")?, delay: f("delay")? },
        Some(_) => return Err(()),
    }))
}

fn instance(f: &[String], kinds: &HashMap<String, String>, subckts: &[String]) -> Option<Result<Component, Instance>> {
    let name = f.first()?.clone();
    let ns = nodes(f.get(1)?)?;
    let master = unquote_name(f.get(2)?);
    // `model v vsource` then `v1 (a 0) v ...`: an alias of a built-in.
    let master = kinds.get(&master).filter(|k| k.starts_with('=')).map_or(master, |k| k[1..].to_string());
    let rest = f.get(3..).unwrap_or(&[]);
    let p = pairs(rest);
    let kept = |used: &[&str]| -> Vec<(String, String)> { p.iter().filter(|(k, _)| !used.contains(&k.as_str())).map(|(k, v)| (k.clone(), spice_text(v))).collect() };
    let two = |ns: &[String]| -> Option<(String, String)> { match ns { [a, b] => Some((a.clone(), b.clone())), _ => None } };
    Some(Ok(match master.as_str() {
        "resistor" | "capacitor" | "inductor" => {
            let (n1, n2) = two(&ns)?;
            let (letter, key) = match master.as_str() { "resistor" => ('r', "r"), "capacitor" => ('c', "c"), _ => ('l', "l") };
            let (name, v, params) = (strip(&name, letter), value(get(&p, key)?), kept(&[key]));
            match letter {
                'r' => Component::Resistor { name, n1, n2, value: v, params },
                'c' => Component::Capacitor { name, n1, n2, value: v, params },
                _ => Component::Inductor { name, n1, n2, value: v, params },
            }
        }
        "vsource" | "isource" => {
            let (np, nm) = two(&ns)?;
            if p.iter().any(|(k, _)| !SOURCE_KEYS.contains(&k.as_str())) {
                return None;
            }
            let wf = waveform(&p).ok()?;
            let value = get(&p, "dc").map_or(IrValue::Numeric { value: 0.0 }, value);
            let (ac_magnitude, ac_phase) = (get(&p, "mag").and_then(num), get(&p, "phase").and_then(num));
            if master == "vsource" {
                Component::VoltageSource { name: strip(&name, 'v'), np, nm, value, ac_magnitude, ac_phase, waveform: wf }
            } else {
                Component::CurrentSource { name: strip(&name, 'i'), np, nm, value, ac_magnitude, ac_phase, waveform: wf }
            }
        }
        "vcvs" | "vccs" => {
            let [np, nm, ncp, ncm] = ns.as_slice() else { return None };
            let gain = num(get(&p, "gain")?)?;
            let (np, nm, ncp, ncm) = (np.clone(), nm.clone(), ncp.clone(), ncm.clone());
            if master == "vcvs" {
                Component::Vcvs { name: strip(&name, 'e'), np, nm, ncp, ncm, gain }
            } else {
                Component::Vccs { name: strip(&name, 'g'), np, nm, ncp, ncm, transconductance: gain }
            }
        }
        "cccs" | "ccvs" => {
            let (np, nm) = two(&ns)?;
            let (vsense, gain) = (string(get(&p, "ctlinst")?)?, num(get(&p, "gain")?)?);
            if master == "cccs" {
                Component::Cccs { name: strip(&name, 'f'), np, nm, vsense, gain }
            } else {
                Component::Ccvs { name: strip(&name, 'h'), np, nm, vsense, transresistance: gain }
            }
        }
        "mutual" if ns.is_empty() => Component::MutualInductor {
            name: strip(&name, 'k'),
            inductor1: strip(&string(get(&p, "ind1")?)?, 'l'),
            inductor2: strip(&string(get(&p, "ind2")?)?, 'l'),
            coupling: num(get(&p, "k")?)?,
        },
        m if subckts.iter().any(|s| s == m) => {
            return Some(Err(Instance { name: strip(&name, 'x'), subcircuit: m.to_string(), port_mapping: ns, parameters: kept(&[]) }));
        }
        m => {
            let kind = kinds.get(m)?.as_str();
            let (model, params) = (m.to_string(), kept(&[]));
            match (kind, ns.as_slice()) {
                ("d", [np, nm]) => Component::Diode { name: strip(&name, 'd'), np: np.clone(), nm: nm.clone(), model, params },
                ("npn" | "pnp", [nc, nb, ne, extra @ ..]) if extra.len() <= 2 => Component::Bjt { name: strip(&name, 'q'), nc: nc.clone(), nb: nb.clone(), ne: ne.clone(), extra_nodes: extra.to_vec(), model, params },
                ("nmos" | "pmos", [nd, ng, s, nb]) => Component::Mosfet { name: strip(&name, 'm'), nd: nd.clone(), ng: ng.clone(), ns: s.clone(), nb: Some(nb.clone()), extra_nodes: vec![], model, params },
                ("njf" | "pjf", [nd, ng, s]) => Component::Jfet { name: strip(&name, 'j'), nd: nd.clone(), ng: ng.clone(), ns: s.clone(), model, params },
                ("nmf" | "pmf", [nd, ng, s]) => Component::Mesfet { name: strip(&name, 'z'), nd: nd.clone(), ng: ng.clone(), ns: s.clone(), model, params },
                _ => return None,
            }
        }
    }))
}

/// `ic=["n"; v; ...]` (or `{"n", v, ...}`) -> node values.
fn node_list(v: &str) -> Option<Vec<(String, f64)>> {
    let inner = v.trim().get(1..v.trim().len().checked_sub(1)?)?;
    let items: Vec<&str> = inner.split([';', ',']).map(str::trim).filter(|s| !s.is_empty()).collect();
    items.chunks(2).map(|c| Some((string(c.first()?)?, num(c.get(1)?)?))).collect()
}

/// One `control` block statement the IR models, or `false`.
fn control(tb: &mut Testbench, f: &[String], pending: &mut Vec<DcSweep>) -> bool {
    let p = pairs(f.get(1..).unwrap_or(&[]));
    match f.first().map(String::as_str) {
        Some("options") => {
            for (k, v) in &p {
                match (k.as_str(), num(v)) {
                    ("rawfile", _) => {}
                    ("temp", Some(x)) => tb.temperature = Some(x),
                    ("tnom", Some(x)) => tb.nominal_temperature = Some(x),
                    // VACASK's own option names stay VACASK's, written back as
                    // they read (the portable names are SPICE's).
                    _ => tb.options.backend_specific.entry("vacask".into()).or_default().push((k.clone(), v.clone())),
                }
            }
            true
        }
        // `sweep NAME instance="v1" parameter="dc" from= to= mode="lin" points=n`
        Some("sweep") => {
            let (Some(src), Some("dc"), Some(from), Some(to), Some(n)) = (get(&p, "instance").and_then(string), get(&p, "parameter").and_then(string).as_deref(), get(&p, "from").and_then(num), get(&p, "to").and_then(num), get(&p, "points").and_then(num)) else {
                return false;
            };
            pending.push(DcSweep { source: src, start: from, stop: to, step: (to - from) / n });
            true
        }
        Some("analysis") => {
            let kind = f.get(2).map(String::as_str).unwrap_or("");
            let p = pairs(f.get(3..).unwrap_or(&[]));
            let n = |k: &str| get(&p, k).and_then(num);
            let freq = || -> Option<(String, u32, f64, f64)> {
                let mode = string(get(&p, "mode")?)?;
                let pts = n("points")? as u32 + u32::from(mode == "lin");
                Some((mode, pts, n("from")?, n("to")?))
            };
            let a = match kind {
                "op" if !pending.is_empty() => Some(Analysis::Dc { sweeps: pending.drain(..).rev().collect() }),
                "op" => Some(Analysis::Op),
                "ac" => freq().map(|(variation, points, start, stop)| Analysis::Ac { variation, points, start, stop }),
                "tran" => (|| {
                    if let Some(ic) = get(&p, "ic") {
                        tb.initial_conditions.extend(node_list(ic)?);
                    }
                    if let Some(ns) = get(&p, "nodeset") {
                        tb.node_sets.extend(node_list(ns)?);
                    }
                    Some(Analysis::Transient { step: n("step")?, stop: n("stop")?, start: None, max_step: n("maxstep"), uic: get(&p, "icmode").and_then(string).as_deref() == Some("uic") })
                })(),
                "noise" => freq().and_then(|(variation, points, start, stop)| {
                    Some(Analysis::Noise { output: string(get(&p, "out")?)?, reference: "0".into(), source: string(get(&p, "in")?)?, variation, points, start, stop, points_per_summary: None })
                }),
                "dcxf" => get(&p, "out").and_then(string).map(|o| Analysis::Tf { output: o, source: String::new() }),
                _ => None,
            };
            match a {
                Some(a) if pending.is_empty() => {
                    tb.analyses.push(a);
                    true
                }
                _ => false,
            }
        }
        _ => false,
    }
}

pub(super) fn parse(src: &str, name: &str, g: &super::Grammars) -> Result<(CircuitIR, Report), String> {
    let (_title, lines) = logical_lines(src, g).map_err(|at| format!("VACASK line grammar stops at byte {at}"))?;
    let mut kinds: HashMap<String, String> = HashMap::new();
    let mut subckts: Vec<String> = Vec::new();
    for l in &lines {
        let f = fields(l);
        match f.first().map(String::as_str) {
            Some("model") if f.len() >= 3 => {
                let ty = get(&pairs(&f[3..]), "type").and_then(num);
                if let Some((k, _)) = spice_kind(&f[2], ty) {
                    kinds.insert(unquote_name(&f[1]), k);
                } else if let Some(m) = builtin(&f[2]) {
                    // An alias of a built-in device: `=<module>`.
                    kinds.insert(unquote_name(&f[1]), format!("={m}"));
                }
            }
            Some("subckt") => subckts.extend(f.get(1).map(|n| unquote_name(n))),
            _ => {}
        }
    }

    let mut report = Report::default();
    let (mut top, mut tb) = (sub(name), testbench(name));
    let (mut defs, mut open): (Vec<Subcircuit>, Vec<(Subcircuit, usize, Vec<(String, String)>)>) = (Vec::new(), Vec::new());
    let mut globals = Vec::new();
    let functions: Vec<FuncDef> = Vec::new();
    let (mut in_control, mut pending, mut at_if) = (false, Vec::new(), 0usize);

    for line in lines {
        let inside = !open.is_empty();
        let here = open.last_mut().map_or(&mut top, |f| &mut f.0);
        let f = fields(&line);
        let w0 = f.first().map(String::as_str).unwrap_or("");
        if in_control {
            if w0 == "endc" {
                in_control = false;
            } else if !control(&mut tb, &f, &mut pending) {
                tb.extra_lines.push(line);
            }
            continue;
        }
        // `@if` ... `@end`: kept whole, nested.
        if at_if > 0 || w0 == "@if" {
            at_if = match w0 { "@if" => at_if + 1, "@end" => at_if - 1, _ => at_if };
            report.raw += 1;
            here.raw_spice.push(line);
            continue;
        }
        if g.cards.parse(g.one_card, &format!("{line}\n")).is_err() {
            report.unparsed.push(line.clone());
            here.raw_spice.push(line);
            continue;
        }
        let p = pairs(f.get(1..).unwrap_or(&[]));
        let mut raw = |here: &mut Subcircuit, l: String| {
            report.raw += 1;
            here.raw_spice.push(l);
        };
        match w0 {
            "control" => in_control = true,
            "ground" if f.get(1).map(String::as_str) == Some("0") && f.len() == 2 => {}
            "load" => match f.get(1).and_then(|x| string(x)) {
                // `spice/<m>.osdi` is what the codegen loads for its devices.
                Some(path) if path.starts_with("spice/") => {}
                Some(path) => here.osdi_loads.push(path),
                None => raw(here, line),
            },
            "include" => match (f.get(1).and_then(|x| string(x)), get(&p, "section")) {
                (Some(path), Some(sec)) => here.libs.push((path, sec.to_string())),
                (Some(path), None) => here.includes.push(path),
                _ => raw(here, line),
            },
            "global" => globals.extend(f[1..].iter().map(|n| unquote_name(n))),
            "parameters" => here.parameters.extend(p.iter().map(|(k, v)| ParamDef { name: k.clone(), default: Some(spice_text(v)) })),
            "model" if f.len() >= 3 => {
                let mname = unquote_name(&f[1]);
                let ty = get(&pairs(&f[3..]), "type").and_then(num);
                match spice_kind(&f[2], ty) {
                    // `model resistor sp_resistor` (the codegen's own) or an
                    // alias such as `model v vsource`: implied by the devices.
                    _ if builtin(&f[2]).is_some() => {}
                    Some((kind, level)) => {
                        let mut parameters: Vec<(String, String)> = pairs(&f[3..]).into_iter().filter(|(k, _)| k != "type").map(|(k, v)| (k, spice_text(&v))).collect();
                        if let Some(l) = level {
                            parameters.insert(0, ("level".into(), l.into()));
                        }
                        here.models.push(ModelDef { name: mname, kind, parameters });
                    }
                    None => raw(here, line),
                }
            }
            "subckt" => {
                let Some(sname) = f.get(1) else { raw(here, line); continue };
                let mut sc = sub(&unquote_name(sname));
                sc.ports = f.get(2).and_then(|x| nodes(x)).unwrap_or_default().into_iter().map(|n| Port { name: n, direction: PortDirection::InOut }).collect();
                open.push((sc, defs.len(), Vec::new()));
            }
            "ends" if inside => close_subckt(&mut open, &mut defs),
            _ => match instance(&f, &kinds, &subckts) {
                Some(Ok(c)) => here.components.push(c),
                Some(Err(i)) => here.instances.push(i),
                None => raw(here, line),
            },
        }
    }
    while !open.is_empty() {
        close_subckt(&mut open, &mut defs);
    }
    // VACASK loads are order-free; codegen::vacask writes them sorted.
    top.osdi_loads.sort();
    let ir = CircuitIR { raw_dialect: Some("vacask".into()), globals, functions, top, testbench: Some(tb), subcircuit_defs: defs, model_libraries: vec![] };
    Ok((ir, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vacask_numbers_and_fields() {
        assert_eq!(num("1M"), Some(1e6));
        assert_eq!(num("1meg"), Some(1e6));
        assert_eq!(num("2m"), Some(2e-3));
        assert_eq!(fields("r1 (a b) resistor r=1k"), ["r1", "(a b)", "resistor", "r=1k"]);
        assert_eq!(fields("model nn sp_mos3 (type=1 vto=1)"), ["model", "nn", "sp_mos3", "(type=1 vto=1)"]);
        assert_eq!(node_list(r#"["48"; 0; "50"; 1]"#), Some(vec![("48".into(), 0.0), ("50".into(), 1.0)]));
    }
}
