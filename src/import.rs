//! SPICE-family deck text -> [`CircuitIR`].
//!
//! Reads the card syntax ngspice, HSPICE, LTspice and Xyce share: `+`
//! continuations, `*` comments, `$`/`;`/`//` tail comments, `{expr}`/`'expr'`
//! expressions. A card the IR has no field for (POLY, TABLE, 4-terminal BJTs,
//! `.pz`, `.step`, ...) is kept verbatim as a raw line, so it reaches the
//! SPICE-family outputs unchanged. `tools/spice2rack.rs` is the CLI.

use crate::ir::*;

// ── Line layer ──

/// Analysis commands a `.control` block runs; ngspice reads `tran 1n 1u`
/// there as it reads `.tran 1n 1u`.
const CONTROL_ANALYSES: &[&str] = &["op", "dc", "ac", "tran", "noise", "tf", "sens", "pz", "disto", "four", "meas"];

/// Logical cards: title dropped, comments stripped, `+` lines joined. A
/// `.control` block keeps only its analysis commands, as dot cards; the rest
/// is ngspice scripting no other dialect has.
fn cards(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut control = false;
    for raw in src.lines().skip(1) {
        let line = strip_comment(raw.trim());
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if control {
            control = !lower.starts_with(".endc");
            let word = lower.split_whitespace().next().unwrap_or("");
            if CONTROL_ANALYSES.contains(&word) {
                out.push(format!(".{line}"));
            }
            continue;
        }
        if lower.starts_with(".control") {
            control = true;
            continue;
        }
        if lower == ".end" || lower.starts_with(".end ") {
            break;
        }
        match (line.strip_prefix('+'), out.last_mut()) {
            (Some(rest), Some(last)) => {
                last.push(' ');
                last.push_str(rest.trim());
            }
            _ => out.push(line.to_string()),
        }
    }
    out
}

/// `*` lines are comments; a tail comment starts at `$` or `;` after a blank
/// (inside a name `$` is a name byte) or at `//`. Quotes and braces protect.
fn strip_comment(line: &str) -> &str {
    if line.starts_with('*') {
        return "";
    }
    let b = line.as_bytes();
    let (mut depth, mut quote) = (0i32, None::<u8>);
    for i in 0..b.len() {
        match (quote, b[i]) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, b'\'' | b'"') => quote = Some(b[i]),
            (None, b'{') => depth += 1,
            (None, b'}') => depth -= 1,
            (None, b'$' | b';') if depth == 0 && i > 0 && b[i - 1].is_ascii_whitespace() => return line[..i].trim_end(),
            (None, b'/') if depth == 0 && b.get(i + 1) == Some(&b'/') => return line[..i].trim_end(),
            _ => {}
        }
    }
    line
}

/// Whitespace-separated tokens; `{..}`, `'..'`, `".."` and `(..)` stay whole,
/// `a = b` becomes `a=b`, and a `(` group after a blank joins its word
/// (`sin (0 1 1k)` -> `sin(0 1 1k)`).
fn tokens(card: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let (mut depth, mut quote) = (0i32, None::<char>);
    let mut glue = false; // next token joins the previous one
    let flush = |cur: &mut String, out: &mut Vec<String>, glue: &mut bool| {
        if cur.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if *glue || cur.starts_with('(') || cur.starts_with('=') || last.ends_with('=') => last.push_str(cur),
            _ => out.push(cur.clone()),
        }
        *glue = cur.ends_with('=');
        cur.clear();
    };
    for c in card.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => {
                quote = None;
                cur.push(c);
            }
            (Some(_), _) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                cur.push(c);
            }
            (None, '{' | '(') => {
                depth += 1;
                cur.push(c);
            }
            (None, '}' | ')') => {
                depth -= 1;
                cur.push(c);
            }
            (None, c) if depth == 0 && c.is_whitespace() => flush(&mut cur, &mut out, &mut glue),
            (None, '=') if depth == 0 => {
                cur.push('=');
                flush(&mut cur, &mut out, &mut glue);
                glue = true;
            }
            _ => cur.push(c),
        }
    }
    flush(&mut cur, &mut out, &mut glue);
    out
}

// ── Values ──

/// A SPICE number with an optional scale factor and unit tail: `10k`,
/// `1.5meg`, `2mil`, `1e-12f`, `10pF`. `None` for anything else.
fn num(tok: &str) -> Option<f64> {
    let t = tok.to_ascii_lowercase();
    let end = t
        .char_indices()
        .take_while(|&(i, c)| c.is_ascii_digit() || c == '.' || (i == 0 && (c == '+' || c == '-')) || ((c == '+' || c == '-') && t[..i].ends_with('e')) || (c == 'e' && t[i + 1..].starts_with(|n: char| n.is_ascii_digit() || n == '+' || n == '-')))
        .last()
        .map(|(i, c)| i + c.len_utf8())?;
    let mantissa: f64 = t[..end].parse().ok()?;
    let tail = &t[end..];
    if !tail.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let scale = if tail.starts_with("meg") {
        1e6
    } else if tail.starts_with("mil") {
        25.4e-6
    } else {
        match tail.chars().next() {
            Some('t') => 1e12,
            Some('g') => 1e9,
            Some('k') => 1e3,
            Some('m') => 1e-3,
            Some('u') => 1e-6,
            Some('n') => 1e-9,
            Some('p') => 1e-12,
            Some('f') => 1e-15,
            Some('a') => 1e-18,
            _ => 1.0,
        }
    };
    Some(mantissa * scale)
}

fn unquote(tok: &str) -> &str {
    let t = tok.trim();
    for (o, c) in [('{', '}'), ('\'', '\''), ('"', '"')] {
        if let Some(inner) = t.strip_prefix(o).and_then(|s| s.strip_suffix(c)) {
            return inner.trim();
        }
    }
    t
}

fn value(tok: &str) -> IrValue {
    match num(tok) {
        Some(v) => IrValue::Numeric { value: v },
        None if tok.starts_with(['{', '\'', '"']) || tok.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') => {
            IrValue::Expression { expr: unquote(tok).to_string() }
        }
        None => IrValue::Raw { text: tok.to_string() },
    }
}

/// `k=v` tokens into pairs (value text kept as written).
fn pairs(toks: &[String]) -> Option<Vec<(String, String)>> {
    toks.iter().map(|t| t.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

/// Splits `toks` into positional tokens and trailing `k=v` pairs.
fn split_params(toks: &[String]) -> (&[String], Option<Vec<(String, String)>>) {
    let at = toks.iter().position(|t| t.contains('=')).unwrap_or(toks.len());
    (&toks[..at], pairs(&toks[at..]))
}

/// `name(a b, c)` -> numbers, or `None` if any is not a plain number.
fn args(tok: &str) -> Option<Vec<f64>> {
    let inner = tok.split_once('(')?.1.strip_suffix(')')?;
    inner.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).map(num).collect()
}

fn waveform(tok: &str) -> Option<IrWaveform> {
    let kind = tok.split_once('(')?.0.to_ascii_lowercase();
    let a = args(tok)?;
    let g = |i: usize, d: f64| a.get(i).copied().unwrap_or(d);
    Some(match kind.as_str() {
        "sin" => IrWaveform::Sin { offset: g(0, 0.0), amplitude: g(1, 0.0), frequency: g(2, 0.0), delay: g(3, 0.0), damping: g(4, 0.0), phase: g(5, 0.0) },
        "pulse" => IrWaveform::Pulse { initial: g(0, 0.0), pulsed: g(1, 0.0), delay: g(2, 0.0), rise_time: g(3, 0.0), fall_time: g(4, 0.0), pulse_width: g(5, 0.0), period: g(6, 0.0) },
        "pwl" if a.len() % 2 == 0 => IrWaveform::Pwl { values: a.chunks(2).map(|p| (p[0], p[1])).collect() },
        "exp" => IrWaveform::Exp { initial: g(0, 0.0), pulsed: g(1, 0.0), rise_delay: g(2, 0.0), rise_tau: g(3, 0.0), fall_delay: g(4, 0.0), fall_tau: g(5, 0.0) },
        "sffm" => IrWaveform::Sffm { offset: g(0, 0.0), amplitude: g(1, 0.0), carrier_freq: g(2, 0.0), modulation_index: g(3, 0.0), signal_freq: g(4, 0.0) },
        "am" => IrWaveform::Am { amplitude: g(0, 0.0), offset: g(1, 0.0), modulating_freq: g(2, 0.0), carrier_freq: g(3, 0.0), delay: g(4, 0.0) },
        _ => return None,
    })
}

// ── Elements ──

/// One element card, or `None` when the IR has no faithful form for it.
fn element(card: &str) -> Option<Component> {
    let t = tokens(card);
    let name = t[0][1..].to_string();
    let s = |i: usize| t.get(i).cloned();
    let letter = t[0].chars().next()?.to_ascii_lowercase();
    Some(match letter {
        'r' | 'c' | 'l' => {
            let (pos, params) = split_params(&t[3..]);
            let mut params = params?;
            let v = match pos {
                [v] => value(v),
                // ngspice `R1 a b r=1k`
                [] => {
                    let key = letter.to_string();
                    let i = params.iter().position(|(k, _)| k.eq_ignore_ascii_case(&key))?;
                    value(&params.remove(i).1)
                }
                _ => return None,
            };
            let (n1, n2) = (s(1)?, s(2)?);
            match letter {
                'r' => Component::Resistor { name, n1, n2, value: v, params },
                'c' => Component::Capacitor { name, n1, n2, value: v, params },
                _ => Component::Inductor { name, n1, n2, value: v, params },
            }
        }
        'k' if t.len() == 4 => Component::MutualInductor {
            name,
            inductor1: t[1].get(1..)?.to_string(),
            inductor2: t[2].get(1..)?.to_string(),
            coupling: num(&t[3])?,
        },
        'v' | 'i' => {
            let (np, nm) = (s(1)?, s(2)?);
            let (mut v, mut ac_magnitude, mut ac_phase, mut wf) = (IrValue::Numeric { value: 0.0 }, None, None, None);
            let mut dc_given = false;
            let mut i = 3;
            while i < t.len() {
                let k = t[i].to_ascii_lowercase();
                if k == "dc" {
                    v = value(t.get(i + 1)?);
                    dc_given = true;
                    i += 2;
                } else if k == "ac" {
                    ac_magnitude = Some(t.get(i + 1).and_then(|x| num(x)).unwrap_or(1.0));
                    i += if ac_magnitude.is_some() && t.get(i + 1).and_then(|x| num(x)).is_some() { 2 } else { 1 };
                    if let Some(p) = t.get(i).and_then(|x| num(x)) {
                        ac_phase = Some(p);
                        i += 1;
                    }
                } else if t[i].contains('(') {
                    wf = Some(waveform(&t[i])?);
                    i += 1;
                } else if i == 3 {
                    v = value(&t[i]);
                    dc_given = true;
                    i += 1;
                } else {
                    return None;
                }
            }
            // No DC value: SPICE takes the waveform's t=0 value for the
            // operating point, so an explicit 0 would move it.
            if !dc_given && let Some(w) = &wf {
                let at0 = match w {
                    IrWaveform::Sin { offset, .. } | IrWaveform::Sffm { offset, .. } => *offset,
                    IrWaveform::Pulse { initial, .. } | IrWaveform::Exp { initial, .. } => *initial,
                    IrWaveform::Pwl { values } => values.first().map_or(0.0, |p| p.1),
                    IrWaveform::Am { .. } => 0.0,
                };
                v = IrValue::Numeric { value: at0 };
            }
            if letter == 'v' {
                Component::VoltageSource { name, np, nm, value: v, ac_magnitude, ac_phase, waveform: wf }
            } else {
                Component::CurrentSource { name, np, nm, value: v, ac_magnitude, ac_phase, waveform: wf }
            }
        }
        'e' | 'g' => {
            let (np, nm) = (s(1)?, s(2)?);
            let rest = &t[3..];
            // `value={expr}` / `vol='expr'` / `cur='expr'`
            if let [kv] = rest
                && let Some((k, e)) = kv.split_once('=')
                && matches!(k.to_ascii_lowercase().as_str(), "value" | "vol" | "cur")
            {
                let expression = unquote(e).to_string();
                return Some(if letter == 'e' {
                    Component::BehavioralVoltage { name: format!("_{name}"), np, nm, expression }
                } else {
                    Component::BehavioralCurrent { name: format!("_{name}"), np, nm, expression }
                });
            }
            let [ncp, ncm, k] = rest else { return None };
            let k = num(k)?;
            let (ncp, ncm) = (ncp.clone(), ncm.clone());
            if letter == 'e' {
                Component::Vcvs { name, np, nm, ncp, ncm, gain: k }
            } else {
                Component::Vccs { name, np, nm, ncp, ncm, transconductance: k }
            }
        }
        'f' | 'h' if t.len() == 5 => {
            let (np, nm, vsense, k) = (s(1)?, s(2)?, s(3)?, num(&t[4])?);
            if letter == 'f' {
                Component::Cccs { name, np, nm, vsense, gain: k }
            } else {
                Component::Ccvs { name, np, nm, vsense, transresistance: k }
            }
        }
        'b' => {
            let (np, nm) = (s(1)?, s(2)?);
            let rest = t[3..].join(" ");
            let (k, e) = rest.split_once('=')?;
            let expression = unquote(e).to_string();
            match k.trim().to_ascii_lowercase().as_str() {
                "v" => Component::BehavioralVoltage { name, np, nm, expression },
                "i" => Component::BehavioralCurrent { name, np, nm, expression },
                _ => return None,
            }
        }
        'd' | 'q' | 'm' | 'j' | 'z' => {
            let (pos, params) = split_params(&t[1..]);
            let params = params?;
            let model = pos.last()?.clone();
            let n: Vec<String> = pos[..pos.len() - 1].to_vec();
            match (letter, n.as_slice()) {
                ('d', [np, nm]) => Component::Diode { name, np: np.clone(), nm: nm.clone(), model, params },
                ('q', [nc, nb, ne]) => Component::Bjt { name, nc: nc.clone(), nb: nb.clone(), ne: ne.clone(), model, params },
                ('m', [nd, ng, ns, nb]) => Component::Mosfet { name, nd: nd.clone(), ng: ng.clone(), ns: ns.clone(), nb: nb.clone(), model, params },
                ('j', [nd, ng, ns]) => Component::Jfet { name, nd: nd.clone(), ng: ng.clone(), ns: ns.clone(), model, params },
                ('z', [nd, ng, ns]) => Component::Mesfet { name, nd: nd.clone(), ng: ng.clone(), ns: ns.clone(), model, params },
                _ => return None,
            }
        }
        's' if t.len() == 6 => Component::VSwitch { name, np: s(1)?, nm: s(2)?, ncp: s(3)?, ncm: s(4)?, model: s(5)? },
        'w' if t.len() == 5 => Component::ISwitch { name, np: s(1)?, nm: s(2)?, vcontrol: s(3)?, model: s(4)? },
        't' => {
            let (pos, params) = split_params(&t[1..]);
            let p = params?;
            let get = |k: &str| p.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).and_then(|(_, v)| num(v));
            let [inp, inm, outp, outm] = pos else { return None };
            if p.len() != 2 {
                return None;
            }
            Component::TLine { name, inp: inp.clone(), inm: inm.clone(), outp: outp.clone(), outm: outm.clone(), z0: get("z0")?, td: get("td")? }
        }
        _ => return None,
    })
}

// ── Deck ──

fn sub(name: &str) -> Subcircuit {
    Subcircuit {
        name: name.into(),
        ports: vec![],
        parameters: vec![],
        components: vec![],
        instances: vec![],
        models: vec![],
        raw_spice: vec![],
        includes: vec![],
        libs: vec![],
        osdi_loads: vec![],
        verilog_blocks: vec![],
    }
}

fn testbench(dut: &str) -> Testbench {
    Testbench {
        dut: dut.into(),
        stimulus: vec![],
        analyses: vec![],
        options: SimOptions::default(),
        saves: vec![],
        measures: vec![],
        temperature: None,
        nominal_temperature: None,
        initial_conditions: vec![],
        node_sets: vec![],
        step_params: vec![],
        extra_lines: vec![],
    }
}

/// `v(n)=x` pairs of `.ic`/`.nodeset`.
fn node_values(toks: &[String]) -> Option<Vec<(String, f64)>> {
    toks.iter()
        .map(|t| {
            let (k, v) = t.split_once('=')?;
            let node = k.strip_prefix(['v', 'V'])?.strip_prefix('(')?.strip_suffix(')')?;
            Some((node.to_string(), num(v)?))
        })
        .collect()
}

/// A dot card that is an analysis or testbench setting; `false` if not.
fn control(tb: &mut Testbench, card: &str, t: &[String]) -> bool {
    let n = |i: usize| t.get(i).and_then(|x| num(x));
    let kind = t[0].to_ascii_lowercase();
    let parsed = match kind.as_str() {
        ".op" => Some(Analysis::Op),
        ".dc" if t.len() >= 5 && (t.len() - 1) % 4 == 0 => t[1..]
            .chunks(4)
            .map(|c| Some(DcSweep { source: c[0].clone(), start: num(&c[1])?, stop: num(&c[2])?, step: num(&c[3])? }))
            .collect::<Option<Vec<_>>>()
            .map(|sweeps| Analysis::Dc { sweeps }),
        ".ac" if t.len() == 5 => (|| Some(Analysis::Ac { variation: t[1].to_ascii_lowercase(), points: t[2].parse().ok()?, start: n(3)?, stop: n(4)? }))(),
        ".tran" => (|| {
            let uic = t.last()?.eq_ignore_ascii_case("uic");
            let p: Vec<f64> = t[1..t.len() - uic as usize].iter().map(|x| num(x)).collect::<Option<_>>()?;
            match p.as_slice() {
                [step, stop, rest @ ..] if rest.len() <= 2 => Some(Analysis::Transient { step: *step, stop: *stop, start: rest.first().copied(), max_step: rest.get(1).copied(), uic }),
                _ => None,
            }
        })(),
        ".noise" if t.len() >= 7 => (|| {
            let inner = t[1].strip_prefix(['v', 'V'])?.strip_prefix('(')?.strip_suffix(')')?;
            let (output, reference) = inner.split_once(',').map(|(a, b)| (a.trim().to_string(), b.trim().to_string())).unwrap_or((inner.to_string(), "0".into()));
            Some(Analysis::Noise {
                output,
                reference,
                source: t[2].clone(),
                variation: t[3].to_ascii_lowercase(),
                points: t[4].parse().ok()?,
                start: n(5)?,
                stop: n(6)?,
                points_per_summary: t.get(7).and_then(|x| x.parse().ok()),
            })
        })(),
        ".tf" if t.len() == 3 => Some(Analysis::Tf { output: t[1].clone(), source: t[2].clone() }),
        ".four" | ".fourier" if t.len() >= 3 => n(1).map(|f| Analysis::Fourier { fundamental: f, outputs: t[2..].to_vec(), num_harmonics: None }),
        _ => None,
    };
    if let Some(a) = parsed {
        tb.analyses.push(a);
        return true;
    }
    match kind.as_str() {
        ".option" | ".options" | ".opt" if t[1..].iter().all(|x| x.contains('=')) => {
            tb.options.portable.extend(pairs(&t[1..]).unwrap_or_default());
        }
        ".temp" | ".temperature" if t.len() == 2 && n(1).is_some() => tb.temperature = n(1),
        ".ic" | ".nodeset" => match node_values(&t[1..]) {
            Some(v) if kind == ".ic" => tb.initial_conditions.extend(v),
            Some(v) => tb.node_sets.extend(v),
            None => return false,
        },
        ".save" => tb.saves.extend(t[1..].iter().cloned()),
        // `.print tran v(a)` / `.probe v(a)`: the analysis word is not an output.
        ".print" | ".probe" | ".plot" => tb.saves.extend(t[1..].iter().filter(|x| !matches!(x.to_ascii_lowercase().as_str(), "op" | "dc" | "ac" | "tran" | "noise")).cloned()),
        ".meas" | ".measure" => tb.measures.push(card.to_string()),
        _ => return false,
    }
    true
}

fn model(t: &[String]) -> Option<ModelDef> {
    let name = t.get(1)?.clone();
    let (kind, mut rest) = match t.get(2)?.split_once('(') {
        Some((k, p)) => (k.to_string(), vec![format!("({p}")]),
        None => (t[2].clone(), vec![]),
    };
    rest.extend(t[3..].iter().cloned());
    let body = rest.join(" ");
    let body = body.trim().trim_start_matches('(').trim_end_matches(')');
    let parameters = pairs(&tokens(&format!("x {body}"))[1..])?;
    Some(ModelDef { name, kind, parameters })
}

/// Parses deck text (line 1 is the title) into IR named `name`.
pub fn parse(src: &str, name: &str) -> CircuitIR {
    let mut top = sub(name);
    let mut tb = testbench(name);
    let mut defs: Vec<Subcircuit> = Vec::new();
    let mut open: Option<Subcircuit> = None; // ponytail: one level; nested .subckt goes raw
    let mut lib_block = false;

    for card in cards(src) {
        let t = tokens(&card);
        let kind = t[0].to_ascii_lowercase();
        let inside = open.is_some();
        let here = open.as_mut().unwrap_or(&mut top);
        if lib_block {
            here.raw_spice.push(card.clone());
            lib_block = !kind.starts_with(".endl");
            continue;
        }
        if !kind.starts_with('.') {
            if kind.starts_with('x') {
                let (pos, params) = split_params(&t[1..]);
                if let (Some(params), Some((sc, ports))) = (params, pos.split_last()) {
                    here.instances.push(Instance { name: t[0][1..].to_string(), subcircuit: sc.clone(), port_mapping: ports.to_vec(), parameters: params });
                    continue;
                }
            } else if let Some(c) = element(&card) {
                here.components.push(c);
                continue;
            }
            here.raw_spice.push(card);
            continue;
        }
        match kind.as_str() {
            ".subckt" | ".macro" if !inside && t.len() >= 2 => {
                let mut sc = sub(&t[1]);
                let body: Vec<&String> = t[2..].iter().filter(|x| !x.eq_ignore_ascii_case("params:")).collect();
                for x in body {
                    match x.split_once('=') {
                        Some((k, v)) => sc.parameters.push(ParamDef { name: k.into(), default: Some(v.into()) }),
                        None => sc.ports.push(Port { name: x.clone(), direction: PortDirection::InOut }),
                    }
                }
                open = Some(sc);
            }
            ".ends" | ".eom" if inside => defs.push(open.take().unwrap()),
            ".param" | ".parameter" | ".parameters" if !inside => match pairs(&t[1..]) {
                Some(p) => top.parameters.extend(p.into_iter().map(|(k, v)| ParamDef { name: k, default: Some(v) })),
                None => top.raw_spice.push(card),
            },
            ".model" => match model(&t) {
                Some(m) => here.models.push(m),
                None => here.raw_spice.push(card),
            },
            ".include" | ".inc" if t.len() == 2 => here.includes.push(unquote(&t[1]).to_string()),
            ".lib" if t.len() == 3 => here.libs.push((unquote(&t[1]).to_string(), t[2].clone())),
            ".lib" if t.len() == 2 => {
                here.raw_spice.push(card);
                lib_block = true;
            }
            _ if !inside && control(&mut tb, &card, &t) => {}
            // Analyses this IR has no form for (.pz, .step, .hb, ...) stay
            // with the testbench so they keep their place after the circuit.
            _ if !inside => tb.extra_lines.push(card),
            _ => here.raw_spice.push(card),
        }
    }
    if let Some(sc) = open {
        eprintln!("warning: .subckt {} has no .ends", sc.name);
        defs.push(sc);
    }
    CircuitIR { top, testbench: Some(tb), subcircuit_defs: defs, model_libraries: vec![] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::CodeGen;
    use crate::codegen::spice3::{Spice3CodeGen, Spice3Dialect};
    const NG: Spice3CodeGen = Spice3CodeGen { dialect: Spice3Dialect::Ngspice };

    const DECK: &str = "\
divider bench
* a comment
.param rv=2k
.model dm D(is=1e-14 n=1.5)
.model nch nmos level=1 vto=0.7 kp=100u
.subckt load n params: r=1k
R1 n 0 {r}
.ends load
V1 in 0 dc 1 ac 1
+ pulse(0 1 1n 1p 1p 5n 10n)
R1 in mid {rv} $ tail comment
X1 mid load r=2k
D1 mid dn dm
R2 dn 0 10k
E1 b 0 mid 0 2
B1 c 0 V = 2*v(mid)+1
M1 dd mid 0 0 nch w=10u l=1u
Rd in dd 10k
Gp 0 p poly(1) mid 0 0 1m
Rp p 0 1k
.option reltol=1e-4
.op
.end
";

    #[test]
    fn deck_to_ir() {
        let ir = parse(DECK, "divider");
        assert_eq!(ir.subcircuit_defs[0].parameters[0], ParamDef { name: "r".into(), default: Some("1k".into()) });
        assert_eq!(ir.top.instances[0].parameters, vec![("r".into(), "2k".into())]);
        assert!(ir.top.components.iter().any(|c| matches!(c, Component::Resistor { value: IrValue::Expression { expr }, .. } if expr == "rv")));
        assert!(ir.top.components.iter().any(|c| matches!(c, Component::VoltageSource { waveform: Some(IrWaveform::Pulse { period, .. }), .. } if *period == 10e-9)));
        assert!(ir.top.components.iter().any(|c| matches!(c, Component::BehavioralVoltage { expression, .. } if expression == "2*v(mid)+1")));
        assert_eq!(ir.top.models[1].parameters.len(), 3);
        // POLY has no IR form: kept verbatim.
        assert_eq!(ir.top.raw_spice, vec!["Gp 0 p poly(1) mid 0 0 1m".to_string()]);
        // `.control` analyses become dot cards; the scripting goes.
        let c = parse("t\nR1 a 0 1k\n.control\nTRAN 1n 10n\nplot v(a)\n.endc\n", "t");
        assert!(matches!(c.testbench.as_ref().unwrap().analyses[..], [Analysis::Transient { stop, .. }] if stop == 10e-9));
        // A waveform-only source keeps its t=0 value as the DC value.
        let w = parse("t\nV2 x 0 pulse(3 0 1n 1n 1n 5n 10n)\n", "t");
        assert!(matches!(w.top.components[0], Component::VoltageSource { value: IrValue::Numeric { value: 3.0 }, .. }));
        assert_eq!(num("1.5meg"), Some(1.5e6));
        assert_eq!(num("10pF"), Some(10e-12));
        assert_eq!(num("abc"), None);
    }

    /// The converted ngspice deck must solve to the original deck's operating
    /// point. Skipped without ngspice on PATH.
    #[test]
    fn ngspice_round_trip_keeps_the_op() {
        let run = |deck: &str| -> Option<crate::result::RawData> {
            let dir = tempfile::TempDir::new().unwrap();
            let (p, r) = (dir.path().join("d.sp"), dir.path().join("d.raw"));
            std::fs::write(&p, deck).unwrap();
            let out = std::process::Command::new("ngspice").arg("-b").arg("-r").arg(&r).arg(&p).output().ok()?;
            assert!(out.status.success(), "{}\n{deck}", String::from_utf8_lossy(&out.stdout));
            Some(crate::rawfile::parse_raw(&std::fs::read(&r).unwrap()).unwrap())
        };
        let converted = NG.emit_netlist(&parse(DECK, "divider")).unwrap();
        let (Some(a), Some(b)) = (run(DECK), run(&converted)) else {
            eprintln!("ngspice not on PATH — skipping");
            return;
        };
        for node in ["v(mid)", "v(dn)", "v(b)", "v(c)", "v(dd)", "v(p)"] {
            let at = |r: &crate::result::RawData| r.variables.iter().position(|v| v.name.eq_ignore_ascii_case(node)).map(|i| r.real_data[i][0]);
            let (x, y) = (at(&a).unwrap(), at(&b).unwrap_or_else(|| panic!("{node} missing:\n{converted}")));
            assert!((x - y).abs() <= 1e-9 * x.abs().max(1.0), "{node}: {x} vs {y}\n{converted}");
        }
    }
}
