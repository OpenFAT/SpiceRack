//! Spectre deck text -> [`CircuitIR`]: the inverse of `codegen::spectre`,
//! plus the master and source tables of ESPice's `src/frontend/spectre.zig`
//! (dev/research/dialect-readers.md 1.2-1.3).
//!
//! Statements are read one logical line at a time and checked against
//! `spectre.peg` (entry `OneStmt`, `grammar/spectre_one.peg`); a statement the
//! IR has no field for, or the grammar refuses, stays a raw line. Spectre is
//! case-sensitive, so statements are parsed in their own case.

use std::collections::HashMap;

use super::{FuncDef, Report, close_subckt, logical_lines, sub, testbench, unquote};
use crate::ir::*;

/// A Spectre number (`1M` is mega, `1m` milli, `%` and `c` 1e-2, `a` atto):
/// the value, read as one decimal so `0.1n` is exactly 1e-10.
fn num(tok: &str) -> Option<f64> {
    let t = tok.trim();
    let end = t
        .char_indices()
        .take_while(|&(i, c)| c.is_ascii_digit() || c == '.' || (i == 0 && matches!(c, '+' | '-')) || (matches!(c, '+' | '-') && t[..i].ends_with(['e', 'E'])) || (matches!(c, 'e' | 'E') && t[i + 1..].starts_with(|n: char| n.is_ascii_digit() || n == '+' || n == '-')))
        .last()
        .map(|(i, c)| i + c.len_utf8())?;
    // The scale letter, then an ignored unit (`1MHz`, `2pf`, `0.3ns`).
    let tail = &t[end..];
    let unit = tail.char_indices().nth(1).map_or("", |(i, _)| &tail[i..]);
    if !unit.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let exp = match tail.chars().next().map_or("", |_| &tail[..tail.chars().next().map_or(0, char::len_utf8)]) {
        "" => 0,
        "T" => 12,
        "G" => 9,
        "M" => 6,
        "K" | "k" => 3,
        "%" | "c" => -2,
        "m" => -3,
        "u" => -6,
        "n" => -9,
        "p" => -12,
        "f" => -15,
        "a" => -18,
        _ => return None,
    };
    let (base, e) = match t[..end].split_once(['e', 'E']) {
        Some((b, e)) => (b, e.parse::<i32>().ok()?),
        None => (&t[..end], 0),
    };
    format!("{base}e{}", e + exp).parse().ok()
}

/// A parameter value in the IR's SPICE spelling: a Spectre number becomes a
/// plain SPICE number (`1M` must not reach a SPICE reader as milli).
fn spice_text(v: &str) -> String {
    num(v).map_or_else(|| v.to_string(), crate::codegen::format_spice_number)
}

fn value(v: &str) -> IrValue {
    match num(v) {
        Some(value) => IrValue::Numeric { value },
        None => IrValue::Expression { expr: v.to_string() },
    }
}

/// Fields of one statement: blanks separate, `(...)`, `[...]`, `{...}` and
/// `"..."` stay whole, and `a = b` is one `a=b`.
fn fields(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let (mut depth, mut quote) = (0i32, false);
    for c in line.chars() {
        match c {
            '"' => {
                quote = !quote;
                cur.push(c);
            }
            _ if quote => cur.push(c),
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            c if depth == 0 && c.is_whitespace() => {
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
    // `a = b` / `a= b` / `a =b` -> `a=b`; `\x` is the name byte `x`
    // (codegen::spectre escapes what Spectre's names cannot hold).
    let out: Vec<String> = out.into_iter().map(|f| if f.starts_with('"') { f } else { unescape(&f) }).collect();
    let mut joined: Vec<String> = Vec::new();
    for f in out {
        match joined.last_mut() {
            Some(prev) if prev.ends_with('=') || f.starts_with('=') => prev.push_str(&f),
            _ => joined.push(f),
        }
    }
    joined
}

fn unescape(f: &str) -> String {
    let mut out = String::with_capacity(f.len());
    let mut it = f.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => out.extend(it.next()),
            _ => out.push(c),
        }
    }
    out
}

fn pairs(fs: &[String]) -> Vec<(String, String)> {
    fs.iter().filter_map(|f| f.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

fn get<'a>(p: &'a [(String, String)], k: &str) -> Option<&'a str> {
    p.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

/// `(a b c)` -> nodes.
fn nodes(f: &str) -> Option<Vec<String>> {
    let inner = f.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.split_whitespace().map(String::from).collect())
}

/// Spectre model master (and `type=`) -> SPICE `.model` kind and LEVEL; the
/// inverse of `codegen::spectre`'s map (ESPice `spectre.zig:433-451`). A
/// master not here is a Spectre-native model and is kept as its kind.
fn spice_kind(master: &str, ty: Option<&str>) -> (String, Option<&'static str>) {
    let ty = ty.map(str::to_ascii_lowercase);
    let mos = |l: &'static str| (if ty.as_deref() == Some("p") { "pmos" } else { "nmos" }.to_string(), Some(l));
    match master {
        "mos1" => mos("1"),
        "mos2" => mos("2"),
        "mos3" => mos("3"),
        "bsim3v3" => mos("49"),
        "bsim4" => mos("54"),
        "bjt" => (if ty.as_deref() == Some("pnp") { "pnp" } else { "npn" }.to_string(), None),
        "diode" => ("d".to_string(), None),
        other => (other.to_string(), None),
    }
}

/// The SPICE letter a primitive's instance name starts with, when it is the
/// one `codegen::spectre` adds back: `r1 resistor` reads as `1`, which
/// writes as `r1` again; `load resistor` stays `load` (written `rload`).
/// Lowercased: codegen::spectre writes instance names lowercase, since the
/// IR's SPICE names are case-blind (an F card may sense `Vin` as `vin`).
fn strip(name: &str, letter: char) -> String {
    let name = name.to_lowercase();
    match name.chars().next() {
        Some(c) if c.eq_ignore_ascii_case(&letter) && name.len() > 1 => name[c.len_utf8()..].to_string(),
        _ => name,
    }
}

/// A source's waveform from its `type=` and keys (`codegen::spectre`'s
/// `emit_waveform_params`, inverted). `None`: no waveform; `Err`: one the IR
/// cannot hold.
fn waveform(p: &[(String, String)]) -> Result<Option<IrWaveform>, ()> {
    let f = |k: &str| get(p, k).map_or(Some(0.0), num).ok_or(());
    Ok(Some(match get(p, "type") {
        None | Some("dc") => return Ok(None),
        Some("pulse") => IrWaveform::Pulse { initial: f("val0")?, pulsed: f("val1")?, delay: f("delay")?, rise_time: f("rise")?, fall_time: f("fall")?, pulse_width: f("width")?, period: f("period")? },
        Some("sine") if get(p, "fmmodindex").is_some() => IrWaveform::Sffm { offset: f("sinedc")?, amplitude: f("ampl")?, carrier_freq: f("freq")?, modulation_index: f("fmmodindex")?, signal_freq: f("fmmodfreq")? },
        Some("sine") => IrWaveform::Sin { offset: f("sinedc")?, amplitude: f("ampl")?, frequency: f("freq")?, delay: f("delay")?, damping: f("damp")?, phase: f("sinephase")? },
        Some("exp") => IrWaveform::Exp { initial: f("val0")?, pulsed: f("val1")?, rise_delay: f("td1")?, rise_tau: f("tau1")?, fall_delay: f("td2")?, fall_tau: f("tau2")? },
        Some("pwl") => {
            let w = get(p, "wave").and_then(|w| w.strip_prefix('[')?.strip_suffix(']')).ok_or(())?;
            let xs: Vec<f64> = w.split_whitespace().map(num).collect::<Option<_>>().ok_or(())?;
            if xs.len() % 2 != 0 {
                return Err(());
            }
            IrWaveform::Pwl { values: xs.chunks(2).map(|c| (c[0], c[1])).collect() }
        }
        Some(_) => return Err(()),
    }))
}

/// Keys a source's waveform and value consume; the rest would be lost.
const SOURCE_KEYS: &[&str] = &["dc", "mag", "phase", "type", "val0", "val1", "delay", "rise", "fall", "width", "period", "sinedc", "ampl", "freq", "damp", "sinephase", "fmmodindex", "fmmodfreq", "td1", "tau1", "td2", "tau2", "wave"];

/// One instance statement as a component or subcircuit instance, or `None`.
fn instance(f: &[String], kinds: &HashMap<String, String>, subckts: &[String]) -> Option<Result<Component, Instance>> {
    let name = f.first()?.clone();
    let (ns, master, rest): (Vec<String>, &str, &[String]) = match f.get(1).and_then(|x| nodes(x)) {
        Some(ns) => (ns, f.get(2)?, f.get(3..)?),
        None => {
            let at = f.iter().position(|x| x.contains('=')).unwrap_or(f.len());
            let master = f.get(at.checked_sub(1)?)?;
            (f[1..at - 1].to_vec(), master, &f[at..])
        }
    };
    let p = pairs(rest);
    if p.len() != rest.len() {
        return None;
    }
    let kept = |used: &[&str]| -> Vec<(String, String)> { p.iter().filter(|(k, _)| !used.contains(&k.as_str())).map(|(k, v)| (k.clone(), spice_text(v))).collect() };
    let two = |ns: &[String]| -> Option<(String, String)> { match ns { [a, b] => Some((a.clone(), b.clone())), _ => None } };
    Some(Ok(match master {
        "resistor" | "capacitor" | "inductor" => {
            let (n1, n2) = two(&ns)?;
            let (letter, key) = match master { "resistor" => ('r', "r"), "capacitor" => ('c', "c"), _ => ('l', "l") };
            let v = value(get(&p, key)?);
            let (name, params) = (strip(&name, letter), kept(&[key]));
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
            let (np, nm, ncp, ncm) = (np.clone(), nm.clone(), ncp.clone(), ncm.clone());
            if master == "vcvs" {
                Component::Vcvs { name: strip(&name, 'e'), np, nm, ncp, ncm, gain: num(get(&p, "gain")?)? }
            } else {
                Component::Vccs { name: strip(&name, 'g'), np, nm, ncp, ncm, transconductance: num(get(&p, "gm")?)? }
            }
        }
        "cccs" | "ccvs" => {
            let (np, nm) = two(&ns)?;
            let vsense = get(&p, "probe")?.to_string();
            if master == "cccs" {
                Component::Cccs { name: strip(&name, 'f'), np, nm, vsense, gain: num(get(&p, "gain")?)? }
            } else {
                Component::Ccvs { name: strip(&name, 'h'), np, nm, vsense, transresistance: num(get(&p, "rm")?)? }
            }
        }
        "mutual_inductor" => Component::MutualInductor {
            name: strip(&name, 'k'),
            inductor1: strip(get(&p, "ind1")?, 'l'),
            inductor2: strip(get(&p, "ind2")?, 'l'),
            coupling: num(get(&p, "coupling")?)?,
        },
        "pvcvs" | "pvccs" | "pcccs" | "pccvs" => {
            let letter = match master { "pvcvs" => 'E', "pvccs" => 'G', "pcccs" => 'F', _ => 'H' };
            let list = |k: &str| get(&p, k).and_then(|v| v.strip_prefix('[')?.strip_suffix(']')).map(|v| v.split_whitespace().map(String::from).collect::<Vec<_>>());
            let coeffs: Vec<f64> = list("coeffs")?.iter().map(|c| num(c)).collect::<Option<_>>()?;
            let (np, nm, controls) = if matches!(letter, 'E' | 'G') { (ns.first()?.clone(), ns.get(1)?.clone(), ns[2..].to_vec()) } else { (ns.first()?.clone(), ns.get(1)?.clone(), list("probes")?) };
            Component::Poly { name: strip(&name, letter.to_ascii_lowercase()), letter, np, nm, controls, coeffs }
        }
        m if subckts.iter().any(|s| s == m) => {
            return Some(Err(Instance { name: strip(&name, 'x'), subcircuit: m.to_string(), port_mapping: ns, parameters: kept(&[]) }));
        }
        m => {
            let kind = kinds.get(m)?.as_str();
            let model = m.to_string();
            let params = kept(&[]);
            match (kind, ns.as_slice()) {
                ("d", [np, nm]) => Component::Diode { name: strip(&name, 'd'), np: np.clone(), nm: nm.clone(), model, params },
                ("npn" | "pnp", [nc, nb, ne, extra @ ..]) if extra.len() <= 2 => Component::Bjt { name: strip(&name, 'q'), nc: nc.clone(), nb: nb.clone(), ne: ne.clone(), extra_nodes: extra.to_vec(), model, params },
                ("nmos" | "pmos", [nd, ng, ns_, nb]) => Component::Mosfet { name: strip(&name, 'm'), nd: nd.clone(), ng: ng.clone(), ns: ns_.clone(), nb: Some(nb.clone()), extra_nodes: vec![], model, params },
                _ => return None,
            }
        }
    }))
}

/// `name options k=v`: options, `temp`/`tnom`, and the inverse of
/// `codegen::spectre::map_option_name`.
fn options(tb: &mut Testbench, p: &[(String, String)]) {
    for (k, v) in p {
        match (k.as_str(), num(v)) {
            ("temp", Some(x)) => tb.temperature = Some(x),
            ("tnom", Some(x)) => tb.nominal_temperature = Some(x),
            _ => {
                let k = match k.as_str() { "iabstol" => "abstol", "vabstol" => "vntol", "dcmaxiters" => "max_iterations", other => other };
                tb.options.portable.push((k.to_string(), spice_text(v)));
            }
        }
    }
}

/// `name [(p n)] kind k=v`: the analyses `codegen::spectre` writes.
fn analysis(f: &[String]) -> Option<Analysis> {
    let (pn, kind, rest) = match f.get(1).and_then(|x| nodes(x)) {
        Some(ns) => (Some(ns), f.get(2)?.as_str(), f.get(3..)?),
        None => (None, f.get(1)?.as_str(), f.get(2..)?),
    };
    let p = pairs(rest);
    if p.len() != rest.len() {
        return None;
    }
    let n = |k: &str| get(&p, k).and_then(num);
    let sweep = || -> Option<(String, u32)> {
        ["dec", "lin", "log"].iter().find_map(|v| get(&p, v).and_then(num).map(|x| (v.to_string(), x as u32)))
    };
    Some(match (kind, pn) {
        ("dc", None) if p.is_empty() => Analysis::Op,
        ("dc", None) => {
            let source = match (get(&p, "dev"), get(&p, "param")) {
                (Some(dev), Some("dc")) => dev.to_string(),
                (None, Some(param)) => param.to_string(),
                _ => return None,
            };
            Analysis::Dc { sweeps: vec![DcSweep { source, start: n("start")?, stop: n("stop")?, step: n("step")? }] }
        }
        ("ac", None) => {
            let (variation, points) = sweep()?;
            Analysis::Ac { variation, points, start: n("start")?, stop: n("stop")? }
        }
        ("tran", None) => Analysis::Transient { step: n("step")?, stop: n("stop")?, start: n("start"), max_step: n("maxstep"), uic: get(&p, "skipdc") == Some("yes") },
        ("noise", Some(ns)) => {
            let [out, reference] = ns.as_slice() else { return None };
            let (variation, points) = sweep()?;
            Analysis::Noise { output: out.clone(), reference: reference.clone(), source: get(&p, "iprobe")?.to_string(), variation, points, start: n("start")?, stop: n("stop")?, points_per_summary: None }
        }
        _ => return None,
    })
}

/// `real f(real a, real b) { return body; }` on one line.
fn function(line: &str) -> Option<FuncDef> {
    let rest = line.strip_prefix("real ")?.trim();
    let (name, rest) = rest.split_once('(')?;
    let (args, rest) = rest.split_once(')')?;
    let body = rest.trim().strip_prefix('{')?.trim().strip_prefix("return")?.trim().strip_suffix('}')?.trim().strip_suffix(';')?.trim();
    let args = args.split(',').map(|a| a.trim().trim_start_matches("real").trim().to_string()).filter(|a| !a.is_empty()).collect();
    Some(FuncDef { name: name.trim().to_string(), args, body: body.to_string() })
}

pub(super) fn parse(src: &str, name: &str, g: &super::Grammars) -> Result<(CircuitIR, Report), String> {
    let (_title, lines) = logical_lines(src, g).map_err(|at| format!("Spectre line grammar stops at byte {at}"))?;
    // Declare pass: instances may come before their model or subckt.
    let mut kinds: HashMap<String, String> = HashMap::new();
    let mut subckts: Vec<String> = Vec::new();
    for l in &lines {
        let f = fields(l);
        match f.first().map(String::as_str) {
            Some("model") if f.len() >= 3 => {
                kinds.insert(f[1].clone(), spice_kind(&f[2], pairs(&f[3..]).iter().find(|(k, _)| k == "type").map(|(_, v)| v.as_str())).0);
            }
            Some("subckt") => subckts.extend(f.get(1).cloned()),
            Some("inline") if f.get(1).map(String::as_str) == Some("subckt") => subckts.extend(f.get(2).cloned()),
            _ => {}
        }
    }

    let mut report = Report::default();
    let mut top = sub(name);
    let mut tb = testbench(name);
    let (mut defs, mut open): (Vec<Subcircuit>, Vec<(Subcircuit, usize, Vec<(String, String)>)>) = (Vec::new(), Vec::new());
    let (mut globals, mut functions) = (Vec::new(), Vec::new());
    let (mut spice, mut brace_depth) = (false, 0i32);
    let mut func_head: Option<String> = None;

    for line in lines {
        let inside = !open.is_empty();
        let here = open.last_mut().map_or(&mut top, |f| &mut f.0);
        let raw = |here: &mut Subcircuit, report: &mut Report, l: String| {
            report.raw += 1;
            here.raw_spice.push(l);
        };
        let f = fields(&line);
        let w0 = f.first().map(String::as_str).unwrap_or("");
        if w0 == "simulator" {
            spice = line.contains("spice");
            continue;
        }
        // A `simulator lang=spice` region: SPICE text. `.meas` is what
        // codegen::spectre puts there; anything else stays raw.
        if spice {
            if line.to_ascii_lowercase().starts_with(".meas") {
                tb.measures.push(line);
            } else {
                raw(here, &mut report, line);
            }
            continue;
        }
        // `real f(real a) {` / `return body;` / `}`: codegen::spectre's
        // function, read back as a .func.
        if let Some(head) = func_head.take() {
            func_head = Some(format!("{head} {}", line.trim()));
            if line.trim() == "}" {
                match function(func_head.as_deref().unwrap_or("")) {
                    Some(fd) if !inside => functions.push(fd),
                    _ => raw(here, &mut report, func_head.clone().unwrap_or_default()),
                }
                func_head = None;
            }
            continue;
        }
        if line.trim_start().starts_with("real ") && line.trim_end().ends_with('{') {
            func_head = Some(line.trim().to_string());
            continue;
        }
        // Brace blocks (model groups, if, statistics, sweeps) stay raw whole.
        if brace_depth > 0 || (line.trim_end().ends_with('{') && !line.trim_start().starts_with("real ")) {
            brace_depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            raw(here, &mut report, line);
            continue;
        }
        if g.cards.parse(g.one_card, &format!("{line}\n")).is_err() {
            report.unparsed.push(line.clone());
            here.raw_spice.push(line);
            continue;
        }
        let p = pairs(f.get(1..).unwrap_or(&[]));
        match w0 {
            "include" => match (f.get(1).map(|x| unquote(x).to_string()), get(&p, "section")) {
                (Some(path), Some(sec)) => here.libs.push((path, sec.to_string())),
                (Some(path), None) => here.includes.push(path),
                _ => raw(here, &mut report, line),
            },
            "ahdl_include" => match f.get(1) {
                Some(path) => here.osdi_loads.push(unquote(path).to_string()),
                None => raw(here, &mut report, line),
            },
            "global" => globals.extend(f[1..].iter().filter(|n| *n != "0").cloned()),
            "parameters" => here.parameters.extend(p.iter().map(|(k, v)| ParamDef { name: k.clone(), default: Some(spice_text(v)) })),
            "model" if f.len() >= 3 => {
                let ty = get(&pairs(&f[3..]), "type").map(str::to_string);
                let (kind, level) = spice_kind(&f[2], ty.as_deref());
                let mut parameters: Vec<(String, String)> = pairs(&f[3..]).into_iter().filter(|(k, _)| !(level.is_some() || kind != f[2]) || k != "type").map(|(k, v)| (k, spice_text(&v))).collect();
                if let Some(l) = level {
                    parameters.insert(0, ("level".into(), l.into()));
                }
                here.models.push(ModelDef { name: f[1].clone(), kind, parameters });
            }
            "subckt" | "inline" => {
                let at = if w0 == "inline" { 2 } else { 1 };
                let Some(sname) = f.get(at) else { raw(here, &mut report, line); continue };
                let ports = f.get(at + 1).and_then(|x| nodes(x)).unwrap_or_else(|| f[at + 1..].to_vec());
                let mut sc = sub(sname);
                sc.ports = ports.into_iter().map(|n| Port { name: n, direction: PortDirection::InOut }).collect();
                open.push((sc, defs.len(), Vec::new()));
            }
            "ends" if inside => close_subckt(&mut open, &mut defs),
            "real" => match function(&line) {
                Some(fd) if !inside => functions.push(fd),
                _ => raw(here, &mut report, line),
            },
            "save" => tb.saves.extend(f[1..].iter().map(|s| match s.split_once(':') {
                Some((dev, "p")) => format!("i({})", dev.to_lowercase()),
                _ => s.clone(),
            })),
            "ic" | "nodeset" => {
                let v: Option<Vec<(String, f64)>> = p.iter().map(|(k, v)| Some((k.clone(), num(v)?))).collect();
                match v {
                    Some(v) if w0 == "ic" => tb.initial_conditions.extend(v),
                    Some(v) => tb.node_sets.extend(v),
                    None => raw(here, &mut report, line),
                }
            }
            _ if f.get(1).map(String::as_str) == Some("options") => options(&mut tb, &pairs(&f[2..])),
            _ => {
                if !inside
                    && let Some(a) = analysis(&f)
                {
                    tb.analyses.push(a);
                    continue;
                }
                match instance(&f, &kinds, &subckts) {
                    Some(Ok(c)) => here.components.push(c),
                    Some(Err(i)) => here.instances.push(i),
                    None => raw(here, &mut report, line),
                }
            }
        }
    }
    while !open.is_empty() {
        close_subckt(&mut open, &mut defs);
    }
    let ir = CircuitIR { raw_dialect: Some("spectre".into()), globals, functions, top, testbench: Some(tb), subcircuit_defs: defs, model_libraries: vec![] };
    Ok((ir, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_spectre_numbers() {
        assert_eq!(num("1M"), Some(1e6));
        assert_eq!(num("1m"), Some(1e-3));
        assert_eq!(num("0.1n"), Some(1e-10));
        assert_eq!(num("5%"), Some(0.05));
        assert_eq!(spice_text("30M"), "30meg");
    }

    #[test]
    fn fields_keep_groups() {
        assert_eq!(fields("r1 (a b) resistor r = 1k"), ["r1", "(a b)", "resistor", "r=1k"]);
        assert_eq!(fields("v1 (in 0) vsource type=pwl wave=[0 0 1n 1]"), ["v1", "(in 0)", "vsource", "type=pwl", "wave=[0 0 1n 1]"]);
    }
}
