//! SPICE-family deck text -> [`CircuitIR`].
//!
//! Reads the card syntax ngspice, HSPICE, LTspice and Xyce share: `+`
//! continuations, `*` comments, `$`/`;`/`//` tail comments, `{expr}`/`'expr'`
//! expressions. A card the IR has no field for (POLY, TABLE, 4-terminal BJTs,
//! `.pz`, `.step`, ...) is kept verbatim as a raw line, so it reaches the
//! SPICE-family outputs unchanged. `tools/spice2rack.rs` is the CLI.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::codegen::Dialect;
use crate::ir::*;
use crate::peg::Grammar;

// ── Grammars ──

/// One dialect's grammars, compiled once: physical lines -> logical lines,
/// one logical line -> its card, and include lines.
struct Grammars {
    lines: Grammar,
    cards: Grammar,
    one_card: u32,
}

macro_rules! peg {
    ($($f:literal),+) => { concat!($(include_str!(concat!("../grammar/", $f)), "\n"),+) };
}

fn grammars(d: Dialect) -> Result<&'static Grammars, String> {
    static NG: OnceLock<Grammars> = OnceLock::new();
    static HS: OnceLock<Grammars> = OnceLock::new();
    let (cell, lines, cards) = match d {
        Dialect::Ngspice | Dialect::Espice => (&NG, peg!("ngspice_lines.peg"), peg!("spice_cards.peg", "ngspice.peg", "spicerack.peg")),
        Dialect::Hspice => (&HS, peg!("hspice_lines.peg"), peg!("spice_cards.peg", "hspice_ops.peg", "hspice.peg", "spicerack.peg")),
        other => return Err(format!("no {other:?} reader yet")),
    };
    Ok(cell.get_or_init(|| {
        let cards = Grammar::compile(cards).expect("card grammar");
        Grammars { lines: Grammar::compile(lines).expect("line grammar"), one_card: cards.rule("OneCard").unwrap(), cards }
    }))
}

fn include_grammar() -> &'static Grammar {
    static G: OnceLock<Grammar> = OnceLock::new();
    G.get_or_init(|| Grammar::compile(peg!("include.peg")).expect("include grammar"))
}

// ── Line layer ──

/// The title and the logical lines, original case: continuations joined
/// (`Piece` with a blank, HSPICE's `\\` `Glue` with nothing), comments and
/// blank lines gone, as the dialect's `*_lines.peg` reads them.
fn logical_lines(src: &str, g: &Grammars) -> Result<(String, Vec<String>), usize> {
    let tree = g.lines.parse(g.lines.rule("File").unwrap(), src)?;
    let (mut title, mut out) = (String::new(), Vec::new());
    // Row 0 is `File` itself; its children are the title and the lines.
    for row in tree.children(Some(0)) {
        match g.lines.name(tree.rows[row].rule) {
            "Title" => title = tree.text(row).trim().to_string(),
            "Line" => {
                let mut line = String::new();
                for part in tree.children(Some(row)) {
                    let text = tree.text(part).trim();
                    if !line.is_empty() && !text.is_empty() && g.lines.name(tree.rows[part].rule) == "Piece" {
                        line.push(' ');
                    }
                    line.push_str(text);
                }
                if !line.is_empty() {
                    out.push(line);
                }
            }
            _ => {} // HSPICE `EndRun`: a further run, not read
        }
    }
    Ok((title, out))
}

/// A card's tokens as the card grammar splits it: the dot keyword, then each
/// captured field (`Name`, `Word`, `Pair`, `Group`, `Expr`, `Str`, `Assign`)
/// in original case. Two fields with a bare `=` between them (`v(a) = 1`)
/// are one pair. `None` when the grammar refuses the card.
fn card_tokens(line: &str, g: &Grammars) -> Option<Vec<String>> {
    let lower = format!("{}\n", line.to_ascii_lowercase());
    let tree = g.cards.parse(g.one_card, &lower).ok()?;
    let orig = |i: usize| &line[tree.rows[i].start as usize..(tree.rows[i].end as usize).min(line.len())];
    // Row 0 is `OneCard`; its child is the card rule that matched.
    let card = tree.children(Some(0)).next()?;
    let mut out: Vec<String> = Vec::new();
    if line.starts_with('.') {
        out.push(line.split(|c: char| c.is_whitespace() || c == '(').next()?.to_string());
    }
    let mut last_end = None;
    for f in tree.children(Some(card)) {
        let text = match g.cards.name(tree.rows[f].rule) {
            "Name" | "Word" | "Group" | "Expr" | "Str" => orig(f).trim().to_string(),
            "Pair" | "Assign" | "LPair" => {
                let (k, v) = orig(f).split_once('=')?;
                format!("{}={}", k.trim(), v.trim())
            }
            _ => continue, // Kind (the keyword), Ends
        };
        let between = last_end.map(|e| &lower[e..tree.rows[f].start as usize]);
        match (between, out.last_mut()) {
            (Some(gap), Some(prev)) if gap.trim() == "=" => {
                prev.push('=');
                prev.push_str(&text);
            }
            _ => out.push(text),
        }
        last_end = Some(tree.rows[f].end as usize);
    }
    Some(out)
}

/// `.include path` / `.lib path section`, read by `include.peg`.
enum IncludeLine {
    Include(String),
    Lib(String, String),
}

fn include_line(line: &str) -> Option<IncludeLine> {
    let g = include_grammar();
    let tree = g.parse(g.rule("SpiceFile").unwrap(), line).ok()?;
    let row = tree.children(Some(0)).next()?; // under `SpiceFile`
    let args: Vec<String> = tree
        .children(Some(row))
        .filter(|&a| matches!(g.name(tree.rows[a].rule), "Quoted" | "Word"))
        .map(|a| unquote(tree.text(a)).to_string())
        .collect();
    match (g.name(tree.rows[row].rule), args.as_slice()) {
        ("Include", [p]) => Some(IncludeLine::Include(p.clone())),
        ("Lib", [p, sec]) => Some(IncludeLine::Lib(p.clone(), sec.clone())),
        _ => None,
    }
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
    let tail = &t[end..];
    if !tail.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if tail.starts_with("mil") {
        return Some(t[..end].parse::<f64>().ok()? * 25.4e-6);
    }
    let scale = if tail.starts_with("meg") {
        6
    } else {
        match tail.chars().next() {
            Some('t') => 12,
            Some('g') => 9,
            Some('k') => 3,
            Some('m') => -3,
            Some('u') => -6,
            Some('n') => -9,
            Some('p') => -12,
            Some('f') => -15,
            Some('a') => -18,
            _ => 0,
        }
    };
    // One decimal, parsed once: `0.1n` is `0.1e-9`, exactly the f64 nearest
    // 1e-10, where `0.1 * 1e-9` is an ulp off.
    let (base, exp) = match t[..end].split_once('e') {
        Some((b, e)) => (b, e.parse::<i32>().ok()?),
        None => (&t[..end], 0),
    };
    format!("{base}e{}", exp + scale).parse().ok()
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
/// `k=v` pairs where a bare word is a flag: `(word, "")` (`off`, `pchan`).
fn flag_pairs(toks: &[String]) -> Vec<(String, String)> {
    toks.iter().map(|t| t.split_once('=').map_or((t.clone(), String::new()), |(k, v)| (k.to_string(), v.to_string()))).collect()
}

fn split_params(toks: &[String]) -> (&[String], Option<Vec<(String, String)>>) {
    let at = toks.iter().position(|t| t.contains('=')).unwrap_or(toks.len());
    (&toks[..at], Some(flag_pairs(&toks[at..])))
}

/// `name(a b, c)` -> numbers, or `None` if any is not a plain number.
fn args(tok: &str) -> Option<Vec<f64>> {
    let inner = tok.split_once('(')?.1.strip_suffix(')')?;
    inner.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).map(num).collect()
}

/// Words an element card may carry after its model as flags.
const INSTANCE_FLAGS: &[&str] = &["off", "on", "thermal"];

/// Independent-source waveform keywords.
const WAVEFORMS: &[&str] = &["pulse", "sin", "sine", "pwl", "exp", "sffm", "am"];

fn waveform(tok: &str) -> Option<IrWaveform> {
    let kind = tok.split_once('(')?.0.trim().to_ascii_lowercase(); // `pwl (0 0 ...)`: one group, blank before `(`
    let a = args(tok)?;
    let g = |i: usize, d: f64| a.get(i).copied().unwrap_or(d);
    Some(match kind.as_str() {
        "sin" | "sine" => IrWaveform::Sin { offset: g(0, 0.0), amplitude: g(1, 0.0), frequency: g(2, 0.0), delay: g(3, 0.0), damping: g(4, 0.0), phase: g(5, 0.0) },
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
/// `models` holds the deck's `.model` names, lowercased: a bare word in an
/// R/C/L value position that names one is a model reference, which the IR's
/// R/C/L cannot hold, so the card stays raw rather than become `{rtest}`.
fn element(t: &[String], models: &HashSet<String>) -> Option<Component> {
    let name = t[0][1..].to_string();
    let s = |i: usize| t.get(i).cloned();
    let letter = t[0].chars().next()?.to_ascii_lowercase();
    Some(match letter {
        'r' | 'c' | 'l' => {
            let (pos, params) = split_params(&t[3..]);
            let mut params = params?;
            let v = match pos {
                [v] if models.contains(&v.to_ascii_lowercase()) => return None,
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
            // `dc=1` / `ac=1` are the keyword forms spelled as pairs.
            let t: Vec<String> = t[..3]
                .iter()
                .cloned()
                .chain(t[3..].iter().flat_map(|x| match x.split_once('=') {
                    Some((k, v)) if matches!(k.to_ascii_lowercase().as_str(), "dc" | "ac") => vec![k.to_string(), v.to_string()],
                    _ => vec![x.clone()],
                }))
                .collect::<Vec<String>>();
            // `pulse 0 5 1n ...`: the parentheses are optional (ngspice ch.3),
            // so a bare waveform word takes the numbers after it as its group.
            let mut t2: Vec<String> = Vec::new();
            let mut k = 0;
            while k < t.len() {
                let w = t[k].to_ascii_lowercase();
                if k >= 3 && WAVEFORMS.contains(&w.as_str()) {
                    let args: Vec<String> = t[k + 1..].iter().take_while(|x| num(x).is_some()).cloned().collect();
                    k += 1 + args.len();
                    t2.push(format!("{w}({})", args.join(" ")));
                } else {
                    t2.push(t[k].clone());
                    k += 1;
                }
            }
            let t = t2;
            // A DC value: a number, an expression or a parameter, not a group.
            let is_value = |x: &String| !x.contains('(') || x.starts_with(['{', '\'']);
            let mut i = 3;
            while i < t.len() {
                let k = t[i].to_ascii_lowercase();
                if k == "dc" {
                    match t.get(i + 1).filter(|x| is_value(x)) {
                        Some(x) => {
                            v = value(x);
                            dc_given = true;
                            i += 2;
                        }
                        None => i += 1, // a bare `DC`
                    }
                } else if k == "ac" {
                    ac_magnitude = Some(t.get(i + 1).and_then(|x| num(x)).unwrap_or(1.0));
                    i += if ac_magnitude.is_some() && t.get(i + 1).and_then(|x| num(x)).is_some() { 2 } else { 1 };
                    if let Some(p) = t.get(i).and_then(|x| num(x)) {
                        ac_phase = Some(p);
                        i += 1;
                    }
                } else if i == 3 && is_value(&t[i]) {
                    v = value(&t[i]);
                    dc_given = true;
                    i += 1;
                } else if t[i].contains('(') {
                    wf = Some(waveform(&t[i])?);
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
        // `E o 0 POLY(2) a 0 b 0 c0 c1 ...` / `F o 0 POLY(2) v1 v2 c0 ...`
        'e' | 'g' | 'f' | 'h' if t.get(3).is_some_and(|x| x.to_ascii_lowercase().starts_with("poly")) => {
            let dims = count(t[3].split_once('(')?.1.strip_suffix(')')?.trim())? as usize;
            let n_controls = if matches!(letter, 'e' | 'g') { 2 * dims } else { dims };
            let controls = t.get(4..4 + n_controls)?.to_vec();
            let coeffs: Vec<f64> = t[4 + n_controls..].iter().map(|x| num(x)).collect::<Option<_>>()?;
            Component::Poly { name, letter: letter.to_ascii_uppercase(), np: s(1)?, nm: s(2)?, controls, coeffs }
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
                    Component::BehavioralVoltage { name, np, nm, expression }
                } else {
                    Component::BehavioralCurrent { name, np, nm, expression }
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
            // Instance flags after the model (`off`, VDMOS `thermal`) are
            // parameters with no value.
            let mut params = params;
            let mut pos = pos.to_vec();
            while pos.len() > 2 && pos.last().is_some_and(|w| INSTANCE_FLAGS.contains(&w.to_ascii_lowercase().as_str())) {
                params.insert(0, (pos.pop()?, String::new()));
            }
            let model = pos.last()?.clone();
            let n: Vec<String> = pos[..pos.len() - 1].to_vec();
            match (letter, n.as_slice()) {
                ('d', [np, nm]) => Component::Diode { name, np: np.clone(), nm: nm.clone(), model, params },
                // `Q c b e [s [t]] model`: substrate, then thermal node.
                ('q', [nc, nb, ne, extra @ ..]) if extra.len() <= 2 => {
                    Component::Bjt { name, nc: nc.clone(), nb: nb.clone(), ne: ne.clone(), extra_nodes: extra.to_vec(), model, params }
                }
                // `M d g s [b [more...]] model`: 3 nodes (VDMOS) to 7 (SOI).
                ('m', [nd, ng, ns, rest @ ..]) if rest.len() <= 4 => Component::Mosfet {
                    name,
                    nd: nd.clone(),
                    ng: ng.clone(),
                    ns: ns.clone(),
                    nb: rest.first().cloned(),
                    extra_nodes: rest.iter().skip(1).cloned().collect(),
                    model,
                    params,
                },
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
    Testbench { prints: vec![],
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
/// The analyses a `.print` card may name.
const PRINT_KINDS: &[&str] = &["op", "dc", "ac", "tran", "noise", "disto", "tf", "sens", "pz", "four"];

/// `.func name(a, b) {body}`, `= body` or `'body'`; also HSPICE's
/// `.param name(a, b)='body'`, which is the same definition.
fn func(card: &str) -> Option<FuncDef> {
    let rest = card.split_once(char::is_whitespace)?.1.trim();
    // A `.param` holds a function only when a `(` comes before its `=`.
    if rest.find('(').is_none_or(|p| rest.find('=').is_some_and(|e| e < p)) {
        return None;
    }
    let (name, rest) = rest.split_once('(')?;
    let (args, body) = rest.split_once(')')?;
    let body = unquote(body.trim().trim_start_matches('=').trim());
    let args: Vec<String> = args.split(',').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    (!name.trim().is_empty() && !body.is_empty()).then(|| FuncDef { name: name.trim().to_string(), args, body: body.to_string() })
}

/// A point count: `10`, or a number written as one (`1e2`).
fn count(tok: &str) -> Option<u32> {
    num(tok).filter(|v| *v >= 0.0 && v.fract() == 0.0).map(|v| v as u32)
}

fn control_card(tb: &mut Testbench, card: &str, t: &[String], d: Dialect) -> bool {
    let n = |i: usize| t.get(i).and_then(|x| num(x));
    let kind = t[0].to_ascii_lowercase();
    let parsed = match kind.as_str() {
        ".op" => Some(Analysis::Op),
        ".dc" if t.len() >= 5 && (t.len() - 1) % 4 == 0 => t[1..]
            .chunks(4)
            .map(|c| Some(DcSweep { source: c[0].clone(), start: num(&c[1])?, stop: num(&c[2])?, step: num(&c[3])? }))
            .collect::<Option<Vec<_>>>()
            .map(|sweeps| Analysis::Dc { sweeps }),
        ".ac" if t.len() == 5 => (|| Some(Analysis::Ac { variation: t[1].to_ascii_lowercase(), points: count(&t[2])?, start: n(3)?, stop: n(4)? }))(),
        // `.pz n1 n2 n3 n4 vol|cur pol|zer|pz`
        ".pz" if t.len() == 7 => Some(Analysis::PoleZero {
            node1: t[1].clone(),
            node2: t[2].clone(),
            node3: t[3].clone(),
            node4: t[4].clone(),
            tf_type: t[5].to_ascii_lowercase(),
            pz_type: t[6].to_ascii_lowercase(),
        }),
        // `.sens out [dc]` / `.sens out ac dec|oct|lin n f1 f2`
        ".sens" if t.len() == 2 || (t.len() == 3 && t[2].eq_ignore_ascii_case("dc")) => Some(Analysis::Sensitivity { output: t[1].clone(), ac: None }),
        ".sens" if t.len() == 7 && t[2].eq_ignore_ascii_case("ac") => (|| {
            let ac = AcSweepParams { variation: t[3].to_ascii_lowercase(), points: count(&t[4])?, start: n(5)?, stop: n(6)? };
            Some(Analysis::Sensitivity { output: t[1].clone(), ac: Some(ac) })
        })(),
        ".tran" => (|| {
            let uic = t.last()?.eq_ignore_ascii_case("uic");
            let body = &t[1..t.len() - uic as usize];
            // HSPICE: `START=` names the start, and further positional
            // fields are more step/stop segments, which the IR cannot hold.
            let start_pair = body.iter().find_map(|x| x.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case("start")).map(|(_, v)| v));
            let p: Vec<f64> = body.iter().filter(|x| !x.contains('=')).map(|x| num(x)).collect::<Option<_>>()?;
            if body.iter().filter(|x| x.contains('=')).count() > start_pair.is_some() as usize {
                return None;
            }
            match (d, p.as_slice()) {
                (Dialect::Hspice, [step, stop]) => Some(Analysis::Transient { step: *step, stop: *stop, start: start_pair.and_then(num), max_step: None, uic }),
                (Dialect::Hspice, _) => None,
                (_, [step, stop, rest @ ..]) if rest.len() <= 2 && start_pair.is_none() => Some(Analysis::Transient { step: *step, stop: *stop, start: rest.first().copied(), max_step: rest.get(1).copied(), uic }),
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
                points: count(&t[4])?,
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
        // A bare word is a flag (`noacct`): an empty value.
        ".option" | ".options" | ".opt" | ".opts" => {
            tb.options.portable.extend(t[1..].iter().map(|x| match x.split_once('=') {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => (x.clone(), String::new()),
            }));
        }
        ".temp" | ".temperature" if t.len() == 2 && n(1).is_some() => tb.temperature = n(1),
        ".ic" | ".nodeset" => match node_values(&t[1..]) {
            Some(v) if kind == ".ic" => tb.initial_conditions.extend(v),
            Some(v) => tb.node_sets.extend(v),
            None => return false,
        },
        // A lone `.save all` is what no save list means.
        ".save" if t[1..].iter().all(|x| x.eq_ignore_ascii_case("all")) => {}
        ".save" => tb.saves.extend(t[1..].iter().cloned()),
        // HSPICE `.probe v(a)` selects outputs, as `.save` does elsewhere.
        // ngspice's `.probe` inserts current probes instead, and `.print`/
        // `.plot` only print: no IR field, so they stay raw rather than vanish.
        // `.print [tran|dc|...] out...`; `.plot` asks the same of a plotter.
        ".print" | ".plot" => {
            let mut w = tokens(card).into_iter().skip(1).peekable();
            let analysis = w.next_if(|x| PRINT_KINDS.contains(&x.to_ascii_lowercase().as_str())).map(|x| x.to_ascii_lowercase()).unwrap_or_default();
            tb.prints.push(PrintCard { analysis, outputs: w.collect() });
        }
        ".probe" if d == Dialect::Hspice => tb.saves.extend(card.split_whitespace().skip(1).filter(|x| !matches!(x.to_ascii_lowercase().as_str(), "op" | "dc" | "ac" | "tran" | "noise")).map(String::from)),
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
    // A bare number continues the previous value: a CPL/W model's matrices
    // are blank-separated (`R=0.03 0 0 0.03`). A bare word is a flag.
    let mut parameters: Vec<(String, String)> = Vec::new();
    for (k, v) in flag_pairs(&tokens(&format!("x {body}"))[1..]) {
        match parameters.last_mut() {
            Some((_, prev)) if v.is_empty() && !prev.is_empty() && num(&k).is_some() => {
                prev.push(' ');
                prev.push_str(&k);
            }
            _ => parameters.push((k, v)),
        }
    }
    Some(ModelDef { name, kind, parameters })
}

/// Analysis commands a `.control` block runs; ngspice reads `tran 1n 1u`
/// there as it reads `.tran 1n 1u`.
const CONTROL_ANALYSES: &[&str] = &["op", "dc", "ac", "tran", "noise", "tf", "sens", "pz", "disto", "four", "meas"];

/// Multi-line blocks no IR field holds, kept whole as raw lines: opener,
/// closer.
const BLOCKS: &[(&str, &str)] = &[(".if", ".endif"), (".data", ".enddata"), (".variation", ".end_variation")];

/// What a deck lost on the way to IR.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    /// Cards the dialect grammar refused, kept as raw lines.
    pub unparsed: Vec<String>,
    /// Cards the grammar read that no IR field holds, kept as raw lines.
    pub raw: usize,
}

/// Closes the innermost open `.subckt`. A definition nested in another is
/// local to it in SPICE, while IR subcircuit names are global: it is hoisted
/// as `<parent>__<name>` and the parent's references to it renamed, so the
/// two `sub`s of `sub1` and `sub2` stay two.
fn close_subckt(open: &mut Vec<(Subcircuit, usize, Vec<(String, String)>)>, defs: &mut Vec<Subcircuit>) {
    let Some((mut sc, first, renames)) = open.pop() else { return };
    for inst in sc.instances.iter_mut().chain(defs[first..].iter_mut().flat_map(|d| d.instances.iter_mut())) {
        if let Some((_, to)) = renames.iter().find(|(from, _)| from.eq_ignore_ascii_case(&inst.subcircuit)) {
            inst.subcircuit = to.clone();
        }
    }
    if let Some((parent, _, parent_renames)) = open.last_mut() {
        let scoped = format!("{}__{}", parent.name, sc.name);
        parent_renames.push((std::mem::replace(&mut sc.name, scoped.clone()), scoped));
    }
    defs.push(sc);
}

/// Parses deck text in dialect `d` (line 1 is the title) into IR named `name`.
pub fn parse(src: &str, name: &str, d: Dialect) -> Result<(CircuitIR, Report), String> {
    let g = grammars(d)?;
    let (_title, lines) = logical_lines(src, g).map_err(|at| format!("line grammar stops at byte {at}"))?;
    let mut report = Report::default();
    let models: HashSet<String> = lines
        .iter()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            w.next().filter(|k| k.eq_ignore_ascii_case(".model"))?;
            w.next().map(str::to_ascii_lowercase)
        })
        .collect();
    let mut top = sub(name);
    let mut tb = testbench(name);
    let mut globals: Vec<String> = Vec::new();
    let mut functions: Vec<FuncDef> = Vec::new();
    let mut defs: Vec<Subcircuit> = Vec::new();
    // Open `.subckt`s, innermost last: the definition, where its descendants
    // start in `defs`, and its local names renamed on hoisting.
    let mut open: Vec<(Subcircuit, usize, Vec<(String, String)>)> = Vec::new();
    let mut block: Option<(&str, &str, usize)> = None; // an open raw block: opener, closer, depth
    let mut control = false;

    for card in lines {
        let word = card.split(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("").to_ascii_lowercase();
        let inside = !open.is_empty();
        let here = open.last_mut().map_or(&mut top, |f| &mut f.0);
        if control {
            control = word != ".endc";
            // ngspice runs a `.control` block's analyses; the rest is scripting.
            let cmd = word.trim_start_matches('.');
            if control && CONTROL_ANALYSES.contains(&cmd) {
                if let Some(t) = card_tokens(&format!(".{card}"), g) {
                    let _ = control_card(&mut tb, &format!(".{card}"), &t, d);
                }
            }
            continue;
        }
        if let Some((open_w, close, depth)) = block {
            here.raw_spice.push(card.clone());
            // Blocks nest (`.if` inside `.if`): only the matching closer ends one.
            block = match (word == open_w, word == close) {
                (true, _) => Some((open_w, close, depth + 1)),
                (_, true) if depth == 0 => None,
                (_, true) => Some((open_w, close, depth - 1)),
                _ => block,
            };
            continue;
        }
        if word == ".control" && d == Dialect::Ngspice {
            control = true;
            continue;
        }
        if let Some(&(open_w, close)) = BLOCKS.iter().find(|(o, _)| *o == word) {
            here.raw_spice.push(card.clone());
            report.raw += 1;
            block = Some((open_w, close, 0));
            continue;
        }
        // An include stays in its block: a `.lib` in a `.subckt` is local.
        match include_line(&card) {
            Some(IncludeLine::Include(p)) => {
                here.includes.push(p);
                continue;
            }
            Some(IncludeLine::Lib(p, sec)) => {
                here.libs.push((p, sec));
                continue;
            }
            None if word == ".lib" => {
                // `.lib name` ... `.endl`: a section definition.
                here.raw_spice.push(card.clone());
                report.raw += 1;
                block = Some(("", ".endl", 0)); // sections do not nest
                continue;
            }
            None => {}
        }
        let Some(t) = card_tokens(&card, g) else {
            report.unparsed.push(card.clone());
            here.raw_spice.push(card);
            continue;
        };
        let kind = t[0].to_ascii_lowercase();
        if !kind.starts_with('.') {
            if kind.starts_with('x') {
                let (pos, params) = split_params(&t[1..]);
                if let (Some(params), Some((sc, ports))) = (params, pos.split_last()) {
                    here.instances.push(Instance { name: t[0][1..].to_string(), subcircuit: sc.clone(), port_mapping: ports.to_vec(), parameters: params });
                    continue;
                }
            } else if let Some(c) = element(&t, &models) {
                here.components.push(c);
                continue;
            }
            report.raw += 1;
            here.raw_spice.push(card);
            continue;
        }
        match kind.as_str() {
            ".subckt" | ".macro" if t.len() >= 2 => {
                let mut sc = sub(&t[1]);
                for x in t[2..].iter().filter(|x| !x.eq_ignore_ascii_case("params:")) {
                    match x.split_once('=') {
                        Some((k, v)) => sc.parameters.push(ParamDef { name: k.into(), default: Some(v.into()) }),
                        None => sc.ports.push(Port { name: x.clone(), direction: PortDirection::InOut }),
                    }
                }
                open.push((sc, defs.len(), Vec::new()));
            }
            ".ends" | ".eom" if inside => close_subckt(&mut open, &mut defs),
            ".param" | ".parameter" | ".parameters" if !inside && func(&card).is_some() => functions.push(func(&card).unwrap()),
            ".param" | ".parameter" | ".parameters" if !inside => match pairs(&t[1..]) {
                Some(p) => top.parameters.extend(p.into_iter().map(|(k, v)| ParamDef { name: k, default: Some(v) })),
                None => {
                    report.raw += 1;
                    top.raw_spice.push(card);
                }
            },
            // `.global` nodes name one net everywhere, wherever the card sits.
            ".global" => globals.extend(t[1..].iter().filter(|g| !globals.contains(g)).cloned().collect::<Vec<_>>()),
            // `.func name(a, b) {body}` / `= body`, at the top level.
            ".func" if !inside && func(&card).is_some() => functions.push(func(&card).unwrap()),
            ".model" => match model(&t) {
                Some(m) => here.models.push(m),
                None => {
                    report.raw += 1;
                    here.raw_spice.push(card);
                }
            },
            _ if !inside && control_card(&mut tb, &card, &t, d) => {}
            // SPICE text the IR has no form for (.pz, .step, .hb, ...);
            // `extra_lines` is for target-native text, not this.
            _ => {
                report.raw += 1;
                here.raw_spice.push(card);
            }
        }
    }
    while let Some((sc, ..)) = open.last() {
        eprintln!("warning: .subckt {} has no .ends", sc.name);
        close_subckt(&mut open, &mut defs);
    }
    let raw_dialect = Some(if d == Dialect::Hspice { "hspice" } else { "ngspice" }.to_string());
    Ok((CircuitIR { raw_dialect, globals, functions, top, testbench: Some(tb), subcircuit_defs: defs, model_libraries: vec![] }, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{Dialect, netlist};

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
        let ir = parse(DECK, "divider", Dialect::Ngspice).unwrap().0;
        // A bare word naming a `.model` is a model reference: raw, not `{rtest}`.
        let m = parse("t\n.model rtest r r=1k\nR1 1 0 rtest\nR2 1 0 rv\n", "t", Dialect::Ngspice).unwrap().0;
        assert_eq!(m.top.raw_spice, vec!["R1 1 0 rtest".to_string()]);
        assert!(matches!(&m.top.components[0], Component::Resistor { value: IrValue::Expression { expr }, .. } if expr == "rv"));
        // Nested definitions are local: two `sub`s, hoisted under their parents.
        let sc = parse("t\n.subckt a n\n.subckt sub n\nR1 n 0 1k\n.ends\nX1 n sub\n.ends\n.subckt b n\n.subckt sub n\nR1 n 0 2k\n.ends\nX1 n sub\n.ends\n", "t", Dialect::Ngspice).unwrap().0;
        let names: Vec<&str> = sc.subcircuit_defs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a__sub", "a", "b__sub", "b"]);
        assert_eq!(sc.subcircuit_defs[1].instances[0].subcircuit, "a__sub");
        assert_eq!(sc.subcircuit_defs[3].instances[0].subcircuit, "b__sub");
        // Nested `.if` blocks stay one raw block.
        let n = parse("t\n.if (a == 1)\n.if (b == 1)\nR1 1 0 1\n.endif\n.else\nR1 1 0 2\n.endif\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(n.top.components.is_empty() && n.top.raw_spice.len() == 7, "{:?}", n.top);
        assert_eq!(ir.subcircuit_defs[0].parameters[0], ParamDef { name: "r".into(), default: Some("1k".into()) });
        assert_eq!(ir.top.instances[0].parameters, vec![("r".into(), "2k".into())]);
        assert!(ir.top.components.iter().any(|c| matches!(c, Component::Resistor { value: IrValue::Expression { expr }, .. } if expr == "rv")));
        assert!(ir.top.components.iter().any(|c| matches!(c, Component::VoltageSource { waveform: Some(IrWaveform::Pulse { period, .. }), .. } if *period == 10e-9)));
        assert!(ir.top.components.iter().any(|c| matches!(c, Component::BehavioralVoltage { expression, .. } if expression == "2*v(mid)+1")));
        assert_eq!(ir.top.models[1].parameters.len(), 3);
        // POLY is a component now; nothing in this deck stays raw.
        assert!(ir.top.raw_spice.is_empty(), "{:?}", ir.top.raw_spice);
        // `.control` analyses become dot cards; the scripting goes.
        let c = parse("t\nR1 a 0 1k\n.control\nTRAN 1n 10n\nplot v(a)\n.endc\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(matches!(c.testbench.as_ref().unwrap().analyses[..], [Analysis::Transient { stop, .. }] if stop == 10e-9));
        // `.func`, as ngspice and HSPICE spell it.
        let f = parse("t\n.func dbl(x) {x*2}\nR1 a 0 {dbl(500)}\n", "t", Dialect::Ngspice).unwrap().0;
        assert_eq!(f.functions, vec![FuncDef { name: "dbl".into(), args: vec!["x".into()], body: "x*2".into() }]);
        assert!(netlist(&f, Dialect::Ngspice).unwrap().contains(".func dbl(x) {x*2}"));
        assert!(netlist(&f, Dialect::Hspice).unwrap().contains(".param dbl(x)='x*2'"));
        // POLY sources, voltage- and current-controlled.
        let p = parse("t\nE1 o 0 poly(2) a 0 b 0 1 2 3 4 5 6\nF1 f 0 poly(1) vs 0 1m\nVs s 0 0\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(p.top.raw_spice.is_empty(), "{:?}", p.top.raw_spice);
        assert!(matches!(&p.top.components[0], Component::Poly { letter: 'E', controls, coeffs, .. } if controls.len() == 4 && coeffs.len() == 6));
        assert!(netlist(&p, Dialect::Ngspice).unwrap().contains("E1 o 0 POLY(2) a 0 b 0 1 2 3 4 5 6"));
        // A 3-terminal VDMOS, a model flag and an instance flag.
        let v = parse("t\n.model vdm vdmos pchan vto=1\n.model nch nmos level=1\nM1 d g s vdm\nM2 d g s b nch w=1u off\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(v.top.raw_spice.is_empty(), "{:?}", v.top.raw_spice);
        assert_eq!(v.top.models[0].parameters[0], ("pchan".to_string(), String::new()));
        assert!(matches!(&v.top.components[0], Component::Mosfet { nb: None, .. }));
        assert!(matches!(&v.top.components[1], Component::Mosfet { params, .. } if params.contains(&("off".into(), String::new()))));
        let text = netlist(&v, Dialect::Ngspice).unwrap();
        assert!(text.contains("M1 d g s vdm\n") && text.contains("M2 d g s b nch w=1u off") && text.contains("vdmos(pchan vto=1)"), "{text}");
        assert!(netlist(&v, Dialect::Spectre).is_err());
        // Optional waveform parentheses, `sine`, `.pz`, `.sens`.
        let w = parse("t\nV1 a 0 pulse 0 5 1n 1n 1n 5n 10n\nV2 b 0 dc 0 Sine(0 1 1k)\nR1 a 0 1k\n.pz a 0 b 0 vol pz\n.sens v(a) ac dec 10 1 1meg\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(w.top.raw_spice.is_empty(), "{:?}", w.top.raw_spice);
        assert!(matches!(&w.top.components[0], Component::VoltageSource { waveform: Some(IrWaveform::Pulse { pulsed, .. }), .. } if *pulsed == 5.0));
        assert!(matches!(&w.top.components[1], Component::VoltageSource { waveform: Some(IrWaveform::Sin { frequency, .. }), .. } if *frequency == 1e3));
        let a = &w.testbench.as_ref().unwrap().analyses;
        assert!(matches!(a[..], [Analysis::PoleZero { .. }, Analysis::Sensitivity { ac: Some(_), .. }]), "{a:?}");
        // A blank before the waveform's `(` is still the waveform.
        let b = parse("t\nV1 a 0 pwl (0 0 1n 1)\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(matches!(&b.top.components[0], Component::VoltageSource { waveform: Some(IrWaveform::Pwl { .. }), .. }), "{:?}", b.top);
        // A waveform-only source keeps its t=0 value as the DC value.
        let w = parse("t\nV2 x 0 pulse(3 0 1n 1n 1n 5n 10n)\n", "t", Dialect::Ngspice).unwrap().0;
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
        let converted = netlist(&parse(DECK, "divider", Dialect::Ngspice).unwrap().0, Dialect::Ngspice).unwrap();
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
