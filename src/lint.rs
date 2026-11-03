//! SPICE netlist linter: common mistakes plus backend-specific card warnings.
//!
//! The netlist is tokenised once into [`Row`]s; every check is a pass over them.
//! Netlist text in, [`LintResult`] out.
//!
//! Cites: none (the backend card table names each simulator's missing analyses).
//!
//! Contents:
//! - `pub struct LintResult`
//! - `pub struct Finding`
//! - `pub fn lint_netlist(netlist: &str, backend: Option<&str>) -> LintResult`

use std::collections::{HashMap, HashSet};

/// Warnings may still simulate; errors likely fail.
#[derive(Debug, Clone, Default)]
pub struct LintResult {
    pub warnings: Vec<Finding>,
    pub errors: Vec<Finding>,
}

/// One finding. Errors carry no suggestion and no backend list.
#[derive(Debug, Clone)]
pub struct Finding {
    /// 1-based; 0 means "the whole netlist".
    pub line: usize,
    pub message: String,
    pub suggestion: Option<String>,
    pub backends: &'static [&'static str],
}

struct Row<'a> {
    no: usize,
    text: &'a str,
    toks: Vec<&'a str>,
}

/// Backend family, card prefix, message, suggestion (`None` = error).
type BackendCard = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
);
const BACKEND_CARDS: &[BackendCard] = &[
    (
        "ngspice",
        ".MEAS",
        ".meas only produces output in batch mode (ngspice -b)",
        Some("Run with ngspice -b, not interactive mode"),
    ),
    (
        "ltspice",
        ".PZ",
        ".pz analysis is not supported by LTspice",
        Some("Use .ac analysis instead"),
    ),
    (
        "ltspice",
        ".DISTO",
        ".disto analysis is not supported by LTspice",
        Some("Use .tran with .four or .FFT instead"),
    ),
    (
        "ltspice",
        ".SENS",
        ".sens analysis is not supported by LTspice",
        Some("Use parametric sweeps with .step instead"),
    ),
    (
        "ltspice",
        ".CONTROL",
        ".control blocks are not supported by LTspice",
        None,
    ),
    (
        "spectre",
        ".CONTROL",
        ".control blocks are not supported by Spectre",
        None,
    ),
    (
        "spectre",
        ".PZ",
        ".pz analysis is not supported by Spectre",
        Some("Use stb or ac analysis for stability/poles"),
    ),
    (
        "spectre",
        ".DISTO",
        ".disto analysis is not supported by Spectre",
        Some("Use HB (harmonic balance) analysis for distortion"),
    ),
];

/// Lint a SPICE netlist; `backend` adds that simulator's card checks.
pub fn lint_netlist(netlist: &str, backend: Option<&str>) -> LintResult {
    let rows: Vec<Row> = netlist
        .lines()
        .enumerate()
        .map(|(i, l)| {
            let text = l.trim();
            Row {
                no: i + 1,
                text,
                toks: text.split_whitespace().collect(),
            }
        })
        .collect();
    let elems = || rows.iter().filter(|r| r.is_elem());
    let mut out = LintResult::default();
    let err = |out: &mut LintResult, line, message| {
        out.errors.push(Finding {
            line,
            message,
            suggestion: None,
            backends: &[],
        })
    };
    let warn = |out: &mut LintResult, line, message, suggestion: String, backends| {
        out.warnings.push(Finding {
            line,
            message,
            suggestion: Some(suggestion),
            backends,
        })
    };

    if !rows.iter().any(|r| r.text.eq_ignore_ascii_case(".end")) {
        err(
            &mut out,
            rows.len(),
            "Netlist does not end with .end".into(),
        );
    }

    // Ground may sit on a `+` continuation line too.
    let ground = rows
        .iter()
        .filter(|r| !r.text.is_empty() && !r.text.starts_with(['*', '.']))
        .any(|r| r.toks.iter().any(|t| is_ground(t)));
    if !ground {
        err(
            &mut out,
            0,
            "No ground node (0 or gnd) found in netlist".into(),
        );
    }

    let mut seen: HashMap<String, usize> = HashMap::new();
    for r in elems().filter(|r| r.text.starts_with(|c: char| c.is_ascii_alphabetic())) {
        match seen.get(&r.toks[0].to_lowercase()) {
            Some(prev) => err(
                &mut out,
                r.no,
                format!(
                    "Duplicate element name '{}' (first defined on line {})",
                    r.toks[0], prev
                ),
            ),
            None => {
                seen.insert(r.toks[0].to_lowercase(), r.no);
            }
        }
    }

    // Floating nodes: (node, first line, terminal count) in first-seen order.
    let mut nodes: Vec<(String, usize, usize)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for r in elems() {
        let t = &r.toks;
        let n = match r.kind() {
            'R' | 'C' | 'L' | 'V' | 'I' | 'D' | 'F' | 'H' | 'B' | 'W' => 2,
            'Q' | 'J' | 'Z' => 3,
            'M' | 'E' | 'G' | 'S' | 'T' => 4,
            // Subcircuit: tokens before the first `k=v`, minus the subckt name.
            'X' => {
                t.iter()
                    .skip(1)
                    .take_while(|p| !p.contains('='))
                    .count()
                    .max(1)
                    - 1
            }
            _ => 0,
        };
        for node in t
            .get(1..=n)
            .unwrap_or(&[])
            .iter()
            .filter(|n| !is_ground(n))
            .map(|n| n.to_lowercase())
        {
            match index.get(&node) {
                Some(&i) => nodes[i].2 += 1,
                None => {
                    index.insert(node.clone(), nodes.len());
                    nodes.push((node, r.no, 1));
                }
            }
        }
    }
    for (node, line, _) in nodes.into_iter().filter(|n| n.2 == 1) {
        let msg = format!(
            "Node '{}' is connected to only one element (floating node)",
            node
        );
        warn(
            &mut out,
            line,
            msg,
            "Add another connection or remove the node".into(),
            &[],
        );
    }

    for r in elems().filter(|r| r.toks.get(3).is_some_and(|v| is_zero_value(v))) {
        match r.kind() {
            'R' => warn(
                &mut out,
                r.no,
                format!("Resistor '{}' has zero resistance", r.toks[0]),
                "Use a small value (e.g., 1m) or a voltage source instead".into(),
                &["ngspice", "ltspice"],
            ),
            'C' => warn(
                &mut out,
                r.no,
                format!("Capacitor '{}' has zero capacitance", r.toks[0]),
                "Remove the capacitor or use a small value".into(),
                &["ngspice"],
            ),
            _ => {}
        }
    }

    // Model references; any .include/.lib may define them, so skip then.
    if !rows.iter().any(|r| r.card(".INCLUDE") || r.card(".LIB")) {
        let models: HashSet<String> = rows
            .iter()
            .filter(|r| r.card(".MODEL"))
            .filter_map(|r| r.toks.get(1))
            .map(|m| m.to_lowercase())
            .collect();
        for r in elems() {
            let pos = match r.kind() {
                'D' => 3,
                'Q' | 'J' | 'Z' => 4,
                'M' => 5,
                _ => continue,
            };
            if let Some(m) = r.toks.get(pos)
                && !models.contains(&m.to_lowercase())
            {
                let msg = format!(
                    "Element '{}' references model '{}' which is not defined in this netlist",
                    r.toks[0], m
                );
                warn(
                    &mut out,
                    r.no,
                    msg,
                    "Add a .model definition or .include the model file".into(),
                    &[],
                );
            }
        }
    }

    // `{name}` with no `.param name=...`; expressions are not checked.
    let params: HashSet<String> = rows
        .iter()
        .filter(|r| r.card(".PARAM"))
        .flat_map(|r| r.text[6..].split_whitespace()) // card() matched 6 ASCII bytes
        .filter_map(|p| p.split_once('=').map(|(n, _)| n.trim().to_lowercase()))
        .collect();
    for r in rows.iter().filter(|r| !r.text.starts_with(['*', '.'])) {
        let mut rest = r.text;
        while let Some(s) = rest.find('{') {
            let Some(e) = rest[s..].find('}') else { break };
            let expr = &rest[s + 1..s + e];
            let name = expr.trim().to_lowercase();
            if !name.is_empty()
                && !name.contains(['+', '-', '*', '/', '('])
                && !params.contains(&name)
                && name.parse::<f64>().is_err()
            {
                let msg = format!("Parameter '{{{}}}' used but not defined with .param", expr);
                warn(
                    &mut out,
                    r.no,
                    msg,
                    format!("Add: .param {} = <value>", expr),
                    &[],
                );
            }
            rest = &rest[s + e + 1..];
        }
    }

    // A one-layer random `.param` read more than once: ngspice, HSPICE and
    // Xyce draw each read anew, Spectre and VACASK once per scope
    // (monte-carlo-spectre.md §1.2, §5.1, §10.5).
    let random: Vec<(String, usize)> = rows
        .iter()
        .filter(|r| r.card(".PARAM"))
        .flat_map(|r| {
            param_assigns(&r.text[6..])
                .into_iter()
                .map(move |(n, v)| (n, v, r.no))
        })
        .filter(|(_, v, _)| crate::codegen::statistics::has_statistical(v))
        .map(|(n, _, no)| (n, no))
        .collect();
    for (name, line) in random {
        // Every occurrence but the definition's own name is a read.
        let reads = rows
            .iter()
            .filter(|r| !r.text.starts_with('*'))
            .map(|r| {
                idents(&r.text.to_lowercase())
                    .filter(|w| *w == name)
                    .count()
            })
            .sum::<usize>()
            - 1;
        if reads > 1 {
            warn(
                &mut out,
                line,
                format!(
                    "random .param '{name}' is read {reads} times: ngspice, HSPICE and Xyce draw each read anew, Spectre and VACASK once"
                ),
                format!(
                    "Draw it once everywhere: .param {name}_r=<call> and .param {name}={name}_r"
                ),
                &["spectre", "vacask"],
            );
        }
    }

    // An alias (`ngspice-shared`) checks as the dialect it names.
    let family = backend.map_or("", |b| {
        crate::ir::Dialect::from_backend_name(b).map_or(b, |d| d.name())
    });
    for r in &rows {
        for (fam, _, msg, sugg) in BACKEND_CARDS
            .iter()
            .filter(|c| c.0 == family && r.card(c.1))
        {
            match sugg {
                Some(s) => warn(
                    &mut out,
                    r.no,
                    msg.to_string(),
                    s.to_string(),
                    std::slice::from_ref(fam),
                ),
                None => err(&mut out, r.no, msg.to_string()),
            }
        }
    }
    out
}

// ---- private ----

/// `.param` assignments, lowercased names and their value text: `a=1
/// b = 'x*2'` is `[("a", "1"), ("b", "'x*2'")]`.
fn param_assigns(s: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut rest = s;
    while let Some(eq) = rest.find('=') {
        let head = rest[..eq].trim_end();
        let name = head
            .rsplit(|c: char| c.is_whitespace() || c == ',')
            .next()
            .unwrap_or("")
            .to_lowercase();
        if let Some(last) = out.last_mut() {
            let v = head[..head.len() - name.len()].trim();
            last.1 = v.to_string();
        }
        out.push((name, String::new()));
        rest = &rest[eq + 1..];
    }
    if let Some(last) = out.last_mut() {
        last.1 = rest.trim().to_string();
    }
    out
}

/// The identifiers of `s`.
fn idents(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
}

fn is_ground(t: &str) -> bool {
    t == "0" || t.eq_ignore_ascii_case("gnd")
}

fn is_zero_value(s: &str) -> bool {
    s.parse::<f64>() == Ok(0.0)
}

impl Row<'_> {
    /// Element card: not blank, comment, dot-card or `+` continuation.
    fn is_elem(&self) -> bool {
        !self.text.is_empty() && !self.text.starts_with(['*', '.', '+'])
    }
    fn kind(&self) -> char {
        self.text.chars().next().unwrap_or(' ').to_ascii_uppercase()
    }
    /// Case-insensitive prefix match on a dot-card (`.MODEL`, `.PZ`, ...).
    fn card(&self, card: &str) -> bool {
        self.text
            .get(..card.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(card))
    }
}
