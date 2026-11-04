//! The testbench → VACASK's `control` block: one `options` line, the
//! stepped parameters as control variables swept ahead of each analysis,
//! and the analyses, initial conditions on each `analysis` line.
//!
//! Cites: VACASK `share/doc/vacask/demo` (`analysis`, `sweep`, `options`,
//! `var`), `docs/cmd-analysis-mc.md` (as the old writer,
//! `codegen::vacask::analyses`, probed them).
//!
//! Contents:
//! - `W::control(&mut self, tb: &Tb)`

use super::W;
use super::bench::{Step, Tb, bare_node};
use super::vacask::{num, respell};
use crate::codegen::Dialect;
use crate::syntax::diag::Code;
use crate::syntax::render::capability::Construct;
use crate::syntax::render::read::{Ana, Sweep};

/// `.options` keys that only steer a simulator's bookkeeping or printing:
/// dropping one changes no number in the raw file.
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

/// `.options KEY=VALUE` whose VACASK spelling is the same word and the same
/// quantity (`itl1`, `maxord` are rejected there).
const NUMERIC_OPTIONS: &[&str] = &[
    "reltol", "abstol", "vntol", "chgtol", "gmin", "temp", "tnom",
];

impl W<'_, '_> {
    /// The `control` block: options, then per analysis the stepped sweeps
    /// and the analysis, its initial conditions on its `analysis` line.
    pub(super) fn control(&mut self, tb: &Tb) {
        self.unwritten_outputs(tb);
        self.line("");
        self.line("control");
        let mut opts = String::from("  options rawfile=\"binary\"");
        for (k, v, tok) in &tb.options {
            let k = k.to_ascii_lowercase();
            if NUMERIC_OPTIONS.contains(&k.as_str()) {
                opts.push_str(&format!(" {k}={}", respell(v)));
            } else if k == "method" {
                match v.to_ascii_lowercase().as_str() {
                    "trap" | "trapezoidal" => opts.push_str(" tran_method=\"trap\""),
                    "gear" | "gear2" => opts.push_str(" tran_method=\"gear2\""),
                    _ => {
                        self.ask(Construct::UnmappedOption, *tok);
                    }
                }
            } else if !COSMETIC_OPTIONS.contains(&k.as_str()) {
                self.ask(Construct::UnmappedOption, *tok);
            } else {
                self.note(Code::W0024, *tok);
            }
        }
        for (k, v, _) in &tb.vacask {
            opts.push_str(&format!(" {k}={v}"));
        }
        if let Some(t) = tb.temp {
            opts.push_str(&format!(" temp={}", num(t)));
        }
        if let Some(t) = tb.tnom {
            opts.push_str(&format!(" tnom={}", num(t)));
        }
        self.line(&opts);
        if let Some((_, tok)) = tb.measures.first() {
            self.ask(Construct::Measure, *tok);
        }
        for (_, p, [start, ..], _) in &tb.steps {
            self.line(&format!("  var {}={}", p.to_ascii_lowercase(), num(*start)));
        }
        let sweeps: Vec<String> = tb.steps.iter().filter_map(|s| self.step_sweep(s)).collect();
        let mut ic = String::new();
        if !tb.ics.is_empty() {
            ic.push_str(&node_list("ic", &tb.ics));
        }
        if !tb.nodesets.is_empty() {
            ic.push_str(&node_list("nodeset", &tb.nodesets));
        }
        // `Construct::Save` is `Spell(Omit)`: VACASK saves every node.
        for l in &tb.extra {
            self.line(&format!("  {l}"));
        }
        for (idx, (a, tok)) in tb.analyses.iter().enumerate() {
            // A `sweep` binds to the next `analysis`: the stepped sweeps
            // are repeated ahead of each.
            for s in &sweeps {
                self.line(&format!("  {s}"));
            }
            for l in self.vacask_analysis(idx, a, *tok) {
                let ic = match l.trim_start().starts_with("analysis") {
                    true => ic.as_str(),
                    false => "",
                };
                self.line(&format!("  {l}{ic}"));
            }
        }
        self.line("endc");
    }

    /// Analysis `idx`'s lines (`analysis <kind><idx+1> …`, a DC sweep's
    /// `sweep` lines first); empty when it is refused.
    fn vacask_analysis(&mut self, idx: usize, a: &Ana, tok: u32) -> Vec<String> {
        let kind = match a {
            Ana::Op => "op",
            Ana::Dc(_) => "dc",
            Ana::Ac(_) => "ac",
            Ana::Tran { .. } => "tran",
            Ana::Noise { .. } => "noise",
            Ana::Tf(..) => "tf",
            Ana::Sens(..) | Ana::Pz(..) | Ana::Sp(..) | Ana::Four(..) => {
                self.refuse(Code::W0015, tok);
                return vec![];
            }
        };
        let name = format!("{kind}{}", idx + 1);
        let mut out = Vec::new();
        match a {
            Ana::Op => out.push(format!("analysis {name} op")),
            Ana::Dc(sweeps) => {
                if sweeps.is_empty() {
                    self.refuse(Code::W0015, tok);
                    return vec![];
                }
                for (i, (src, start, stop, step)) in sweeps.iter().enumerate().rev() {
                    let scale = match i {
                        0 => "vsweep".to_string(),
                        i => format!("sweep{i}"),
                    };
                    match sweep_line(&scale, src, *start, *stop, *step) {
                        Some(l) => out.push(l),
                        None => {
                            self.refuse(Code::W0015, tok);
                            return vec![];
                        }
                    }
                }
                out.push(format!("  analysis {name} op"));
            }
            Ana::Ac(sw) => match freq_sweep(sw) {
                Some(f) => out.push(format!("analysis {name} ac {f}")),
                None => {
                    self.refuse(Code::W0015, tok);
                    return vec![];
                }
            },
            Ana::Tran {
                step,
                stop,
                start,
                max_step,
                uic,
            } => {
                // VACASK always saves from t=0.
                if start.unwrap_or(0.0) != 0.0 {
                    self.refuse(Code::W0015, tok);
                    return vec![];
                }
                let native = self.rd.d == Dialect::Vacask;
                let tmax = max_step.or((!native).then(|| step.min(stop / 50.0)));
                let mut s = format!(
                    "analysis {name} tran step={} stop={}",
                    num(*step),
                    num(*stop)
                );
                if let Some(t) = tmax {
                    s.push_str(&format!(" maxstep={}", num(t)));
                }
                s.push_str(match uic {
                    true => " icmode=\"uic\"",
                    false => " icmode=\"op\"",
                });
                out.push(s);
            }
            // `out=` names one node: a non-ground reference has no form.
            Ana::Noise {
                output,
                reference,
                source,
                sweep,
                ..
            } => {
                let f = freq_sweep(sweep);
                if !reference.is_empty() && reference != "0" || f.is_none() {
                    self.refuse(Code::W0015, tok);
                    return vec![];
                }
                out.push(format!(
                    "analysis {name} noise out=\"{}\" in=\"{}\" {}",
                    bare_node(output).to_ascii_lowercase(),
                    source.to_ascii_lowercase(),
                    f.expect("checked")
                ));
            }
            Ana::Tf(output, _) => out.push(format!(
                "analysis {name} dcxf out=\"{}\"",
                bare_node(output).to_ascii_lowercase()
            )),
            Ana::Sens(..) | Ana::Pz(..) | Ana::Sp(..) | Ana::Four(..) => {
                unreachable!("refused above")
            }
        }
        out
    }

    /// A `.step`'s `sweep` line; `None` when it is refused (VACASK sweeps
    /// lin, dec or oct).
    fn step_sweep(&mut self, (sweep, param, [start, stop, step], tok): &Step) -> Option<String> {
        let name = param.to_ascii_lowercase();
        let scale = format!("step_{}", sanitize(&name));
        Some(match sweep {
            Some(mode @ ("dec" | "oct")) => format!(
                "sweep {scale} variable=\"{name}\" from={} to={} mode=\"{mode}\" points={}",
                num(*start),
                num(*stop),
                *step as u64
            ),
            Some("log") => {
                self.refuse(Code::W0016, *tok);
                return None;
            }
            _ => format!(
                "sweep {scale} variable=\"{name}\" from={} to={} step={}",
                num(*start),
                num(*stop),
                num(*step)
            ),
        })
    }
}

// ---- private ----

/// ` ic=["n"; v; …]`.
fn node_list(key: &str, items: &[(String, f64)]) -> String {
    let body: Vec<String> = items
        .iter()
        .map(|(n, v)| format!("\"{}\"; {}", n.to_ascii_lowercase(), num(*v)))
        .collect();
    format!(" {key}=[{}]", body.join("; "))
}

/// A DC sweep of instance `src`'s `dc` from `start` by `step` over the
/// whole intervals that fit (its last point `stop` within rounding);
/// `None` for a zero step or a count out of range.
fn sweep_line(scale: &str, src: &str, start: f64, stop: f64, step: f64) -> Option<String> {
    if step == 0.0 {
        return None;
    }
    let span = ((stop - start) / step).abs();
    let n = (span + 1e-9 * span.max(1.0)).floor();
    if n < 1.0 || n.is_nan() || n > 1e7 {
        return None;
    }
    let last = start + (stop - start).signum() * n * step.abs();
    let last = match (last - stop).abs() <= 1e-9 * step.abs() {
        true => stop,
        false => last,
    };
    Some(format!(
        "sweep {scale} instance=\"{}\" parameter=\"dc\" from={} to={} mode=\"lin\" points={}",
        src.to_ascii_lowercase(),
        num(start),
        num(last),
        n as u64
    ))
}

/// `from= to= mode="dec" points=n` (a `lin` sweep counts intervals, not
/// points); `None` for `log` or no interval.
fn freq_sweep(sw: &Sweep) -> Option<String> {
    let n = match sw.variation {
        "dec" | "oct" => sw.points,
        "lin" => sw.points.saturating_sub(1),
        _ => return None,
    };
    (n != 0).then(|| {
        format!(
            "from={} to={} mode=\"{}\" points={n}",
            num(sw.start),
            num(sw.stop),
            sw.variation
        )
    })
}

/// VACASK identifiers carry no punctuation.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c.is_ascii_alphanumeric() {
            true => c.to_ascii_lowercase(),
            false => '_',
        })
        .collect()
}
