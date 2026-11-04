//! A library section's meaning, for comparing sections across dialects
//! (`test_sections.rs`, `test_sections_pdk.rs`), read from a deck SpiceRack
//! wrote as ngspice (`.lib name` … `.endl`, one card a line): values as
//! numbers when they are one, else without SPICE's delimiters; names
//! lowercased; model parameters sorted, without LEVEL and `type`; a
//! definition's own `.param`s after its parameters (Spectre and VACASK
//! have no other kind).
//!
//! Contents: `pub struct Meaning`, `pub type Pairs`,
//! `pub fn meaning(ngspice: &str) -> Vec<Meaning>`.

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

/// Every section of `ngspice`, a deck SpiceRack wrote for ngspice.
pub fn meaning(ngspice: &str) -> Vec<Meaning> {
    let mut out: Vec<Meaning> = Vec::new();
    let mut sub: Option<(String, Vec<String>, Pairs, usize)> = None;
    let mut open = false;
    for line in ngspice.lines() {
        let words = split(line);
        let Some(head) = words.first().map(|w| w.to_ascii_lowercase()) else {
            continue;
        };
        if let Some(s) = &mut sub {
            match head.as_str() {
                ".ends" => out
                    .last_mut()
                    .expect("a subcircuit in a section")
                    .subckts
                    .push(sub.take().expect("open")),
                ".param" => s.2.extend(pairs(&words[1..], false)),
                h if !h.starts_with(['.', '*', '+']) => s.3 += 1,
                _ => {}
            }
            continue;
        }
        if !open {
            if head == ".lib" && words.len() == 2 {
                open = true;
                out.push(Meaning {
                    name: words[1].clone(),
                    params: vec![],
                    includes: vec![],
                    libs: vec![],
                    models: vec![],
                    subckts: vec![],
                });
            }
            continue;
        }
        let m = out.last_mut().expect("an open section");
        match head.as_str() {
            ".endl" => open = false,
            ".param" => m.params.extend(pairs(&words[1..], false)),
            ".include" => m.includes.push(words[1].trim_matches(['"', '\'']).into()),
            ".lib" => m
                .libs
                .push((words[1].trim_matches(['"', '\'']).into(), words[2].clone())),
            ".model" => {
                let rest = words[3..].join(" ");
                let rest = rest.trim();
                let inner = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')'));
                m.models.push((
                    words[1].to_ascii_lowercase(),
                    words[2].to_ascii_lowercase(),
                    pairs(&split(inner.unwrap_or(rest)), true),
                ));
            }
            ".subckt" => {
                let at = words.iter().position(|w| w.eq_ignore_ascii_case("params:"));
                let end = at.unwrap_or(words.len());
                let ps = at.map_or(vec![], |k| pairs(&words[k + 1..], false));
                sub = Some((words[1].clone(), words[2..end].to_vec(), ps, 0));
            }
            _ => {}
        }
    }
    out
}

// ---- private ----

/// A card's words, split at blanks outside quotes, braces and parentheses.
fn split(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut quote) = (Vec::new(), String::new(), 0i32, false);
    for c in line.chars() {
        match c {
            '\'' => quote = !quote,
            '{' | '(' if !quote => depth += 1,
            '}' | ')' if !quote => depth -= 1,
            c if c.is_whitespace() && !quote && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `k=v` words as normalized pairs; `model` drops LEVEL and `type` and
/// sorts them.
fn pairs(words: &[String], model: bool) -> Pairs {
    let mut v: Pairs = words
        .iter()
        .filter_map(|w| w.split_once('='))
        .filter(|(k, _)| {
            !model || !(k.eq_ignore_ascii_case("level") || k.eq_ignore_ascii_case("type"))
        })
        .map(|(k, x)| (k.to_ascii_lowercase(), value(x)))
        .collect();
    if model {
        v.sort();
    }
    v
}

/// A value as a number when it is one, else without SPICE's delimiters.
fn value(v: &str) -> String {
    let t = v.trim().trim_matches(|c| c == '\'' || c == '{' || c == '}');
    let (sign, mag) = t.strip_prefix('-').map_or((1.0, t), |m| (-1.0, m));
    match spicerack::unit::parse_spice_number(mag) {
        Some((x, end, scale)) if end + scale.len() == mag.len() => format!("{:e}", sign * x),
        _ => t.to_ascii_lowercase(),
    }
}
