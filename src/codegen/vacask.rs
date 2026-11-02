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

use std::collections::BTreeSet;
use std::fmt::Write as _;
use crate::ir::*;
use super::{bare_node, model_refuse, CodeGenError};


/// A device module the deck needs before it can be elaborated.
///
/// SPICE carries R/C/L values on the instance and never writes a `.model` for
/// them; VACASK has no anonymous instances, so every device needs a model. The
/// synthesised ones are named after the device, which is what upstream's own
/// decks do (`model resistor resistor` in `test/test_op.sim`); a user `.model`
/// wearing one of these names is a collision and is refused.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum Need {
    /// SPICE device set: `load "spice/<m>.osdi"` + `model <m> sp_<m>`.
    Sp(&'static str),
    /// Built into the simulator: no load, but still a `model` line.
    Builtin(&'static str),
}

impl Need {
    fn auto_model(&self) -> &'static str {
        match self {
            Need::Sp(m) | Need::Builtin(m) => m,
        }
    }
}

fn unsupported_comp(what: impl Into<String>) -> CodeGenError {
    CodeGenError::UnsupportedComponent { backend: "vacask".into(), component: what.into() }
}

fn unsupported_analysis(what: impl Into<String>) -> CodeGenError {
    CodeGenError::UnsupportedAnalysis { backend: "vacask".into(), analysis: what.into() }
}

// ── Numbers ──

/// A plain number VACASK cannot misread. SPICE scale suffixes are
/// case-INSENSITIVE and VACASK's are not (`M` is milli in SPICE and MEGA in
/// VACASK: a factor of 1e9 with nothing to notice it), so nothing computed here
/// is ever re-spelled with a suffix.
fn num(v: f64) -> String {
    if v == 0.0 { "0".to_string() } else { format!("{:e}", v) }
}

/// A whole token that is one signed SPICE number-with-suffix, as `(value,
/// vacask_spelling)`. A trailing unit (`5V`, `1ohm`) is ignored, as SPICE
/// ignores it.
pub(super) fn parse_spice_num(t: &str) -> Option<(f64, String)> {
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    let (v, len, scale) = crate::unit::parse_spice_number(body)?;
    let lead = t.len() - body.len() + len; // sign and mantissa
    // A scale and a unit, nothing else: `1k*(1+x)` is an expression, and
    // reading it as `1k` would drop the rest.
    if !t[lead..].bytes().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    // VACASK spells tera and giga in capitals (`t` is not a scale there).
    let vacask = match scale { "t" => "T", "g" => "G", s => s };
    let sign = if t.starts_with('-') { -1.0 } else { 1.0 };
    Some((sign * v, format!("{}{vacask}", &t[..lead])))
}

/// A value written as text: a number gets re-spelled in VACASK's suffixes,
/// anything else (a parameter name, an arithmetic expression — VACASK has
/// those) passes through minus the ngspice `{}` wrapper it has no use for.
fn respell(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix('{').and_then(|r| r.strip_suffix('}')).unwrap_or(t).trim();
    match parse_spice_num(t) {
        Some((_, spelled)) => spelled,
        // Case is settled by `prepared` (lowercased for SPICE-origin text).
        None => t.to_string(),
    }
}

/// Numeric value of a text field, when one is needed to compute with.
fn value_of(s: &str) -> Option<f64> {
    parse_spice_num(s.trim().trim_start_matches('{').trim_end_matches('}').trim()).map(|(v, _)| v)
}

// ── Model kind mapping ──

/// SPICE MOSFET LEVEL -> VACASK module. A level that is not here is refused:
/// substituting "the nearest model" compares two different sets of equations.
fn mos_module(level: u32) -> Option<&'static str> {
    Some(match level {
        1 => "mos1",
        2 => "mos2",
        3 => "mos3",
        6 => "mos6",
        9 => "mos9",
        49 | 8 => "bsim3v3",
        54 | 14 => "bsim4v8",
        _ => return None,
    })
}

/// `.model` TYPE -> (module, polarity), for the types whose parameter sets
/// transcribe name-for-name AT LEVEL 1.
///
/// `nmos`/`pmos` are absent on purpose: their LEVEL selects between seven
/// modules. R/C/L are absent too, and they are the one place the "same
/// parameter names" premise fails — `sp_resistor` renames the model card's
/// `r`/`tc1`/`tc2` to `model_r`/`model_tc1`/`model_tc2` to keep them apart from
/// the instance parameters of the same name, so a name-for-name transcription
/// would set the instance parameter and quietly change the device.
fn model_type(kind: &str) -> Option<(&'static str, i8)> {
    Some(match kind {
        "d" => ("diode", 0),
        "npn" => ("bjt", 1),
        "pnp" => ("bjt", -1),
        "njf" => ("jfet1", 1),
        "pjf" => ("jfet1", -1),
        "nmf" => ("mes1", 1),
        "pmf" => ("mes1", -1),
        "vdmos" => ("vdmos", 1),
        _ => return None,
    })
}

// ── Options ──

/// `.options` keys that only steer a simulator's own bookkeeping or printing.
/// Dropping one cannot change a number in the raw file, which is the only
/// reason a key is on this list.
const COSMETIC_OPTIONS: &[&str] = &[
    "noacct", "acct", "list", "node", "post", "trans", "nopage", "nomod",
    "lvlcod", "klu", "sparse", "savecurrents", "numdgt",
];

/// `.options KEY=VALUE` whose VACASK spelling is the same word and whose
/// meaning is the same quantity. Probed against the binary; `itl1` and `maxord`
/// are absent because VACASK rejects them.
const NUMERIC_OPTIONS: &[&str] = &[
    "reltol", "abstol", "vntol", "chgtol", "gmin", "temp", "tnom",
];

// ── Codegen ──

/// The module a component needs loaded, if any.
fn need(comp: &Component) -> Option<Need> {
    Some(match comp {
        Component::Resistor { .. } => Need::Sp("resistor"),
        Component::Capacitor { .. } => Need::Sp("capacitor"),
        Component::Inductor { .. } => Need::Sp("inductor"),
        Component::MutualInductor { .. } => Need::Builtin("mutual"),
        Component::VoltageSource { .. } => Need::Builtin("vsource"),
        Component::CurrentSource { .. } => Need::Builtin("isource"),
        Component::Vcvs { .. } => Need::Builtin("vcvs"),
        Component::Vccs { .. } => Need::Builtin("vccs"),
        Component::Cccs { .. } => Need::Builtin("cccs"),
        Component::Ccvs { .. } => Need::Builtin("ccvs"),
        _ => return None,
    })
}

/// An expression from the IR, lowercased when `low` (a SPICE-origin IR:
/// SPICE names are case-blind, `Rload` and `rload` are one parameter, and
/// VACASK's are not) unless it is a number; a VACASK-origin IR keeps its
/// case (`M_PI` is not `m_pi`). Then [`respell`]ed.
fn expr(s: &str, low: bool) -> String {
    if low && parse_spice_num(s.trim()).is_none() { respell(&s.to_ascii_lowercase()) } else { respell(s) }
}

fn val(v: &IrValue, low: bool) -> String {
    match v {
        IrValue::Numeric { value } => num(*value),
        IrValue::Expression { expr: e } => expr(e, low),
        IrValue::Raw { text } => respell(text),
    }
}

/// A waveform number: [`num`], or an [`expr`].
fn wave_num(x: &IrNum, low: bool) -> String {
    match x {
        IrNum::Num(v) => num(*v),
        IrNum::Expr(e) => expr(e, low),
    }
}

/// Instance parameters, written onto `out`. `m` is SPICE's device
/// multiplier, which VACASK spells with the Verilog-A builtin.
fn inst_params(out: &mut String, params: &[(String, String)], low: bool) {
    for (k, v) in params {
        let k = k.to_ascii_lowercase();
        let k = if k == "m" { "$mfactor" } else { &k };
        w!(out, " {}={}", k, expr(v, low));
    }
}

/// `mag=`/`phase=` on an independent source.
fn ac_spec(out: &mut String, mag: &Option<f64>, phase: &Option<f64>) {
    if let Some(m) = mag {
        w!(out, " mag={}", num(*m));
    }
    if let Some(p) = phase {
        w!(out, " phase={}", num(*p));
    }
}

/// A source's waveform parameters; `name` (the instance) only names it in an error.
fn wave(name: &dyn std::fmt::Display, wf: &IrWaveform, low: bool) -> Result<String, CodeGenError> {
    let n = |x: &IrNum| wave_num(x, low);
    Ok(match wf {
        IrWaveform::Sin { offset, amplitude, frequency, delay, damping, phase } => {
            // VACASK accepts `sinephase` and does not apply it: the waveform
            // is byte-identical for 0, 45 and 90 degrees. `delay` and
            // `theta` beside it ARE honoured, which is what makes dropping
            // this one plausible and wrong — a 90-degree SIN would run and
            // sit 1.414 V away from what the deck asked for.
            if *phase != 0.0 || phase.num().is_none() {
                return Err(unsupported_comp(format!(
                    "{}: SIN phase ({} deg) is accepted but ignored by VACASK",
                    name, n(phase)
                )));
            }
            format!(
                "type=\"sine\" sinedc={} ampl={} freq={} delay={} theta={}",
                n(offset), n(amplitude), n(frequency), n(delay), n(damping),
            )
        }
        IrWaveform::Pulse { initial, pulsed, delay, rise_time, fall_time, pulse_width, period } => {
            // VACASK rejects a period that is not strictly greater than
            // rise+fall+width, while SPICE's own PER=PW default violates
            // exactly that. A zero period means "single pulse" to VACASK,
            // which is what SPICE means by PER >= TSTOP anyway.
            let period = match (period.num(), rise_time.num(), fall_time.num(), pulse_width.num()) {
                (Some(p), Some(r), Some(f), Some(w)) if p > 0.0 && p <= r + f + w => IrNum::Num(0.0),
                // ponytail: an expression period passes as written; VACASK
                // itself rejects one that turns out too short.
                _ => period.clone(),
            };
            format!(
                "type=\"pulse\" val0={} val1={} delay={} rise={} fall={} width={} period={}",
                n(initial), n(pulsed), n(delay),
                n(rise_time), n(fall_time), n(pulse_width), n(&period),
            )
        }
        IrWaveform::Exp { initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau } => {
            // SPICE's TD2 is an ABSOLUTE time ("start falling at t = TD2");
            // VACASK measures its `td2` from `delay`. Passed straight
            // through the fall starts TD1 late, which reads as an
            // integration disagreement rather than a different waveform.
            let (Some(fd), Some(rd)) = (fall_delay.num(), rise_delay.num()) else {
                return Err(unsupported_comp(format!(
                    "{}: EXP delays must be numbers (VACASK counts td2 from delay)",
                    name
                )));
            };
            if fd < rd {
                return Err(unsupported_comp(format!(
                    "{}: EXP fall delay ({}) precedes rise delay ({})",
                    name, fd, rd
                )));
            }
            format!(
                "type=\"exp\" val0={} val1={} delay={} tau1={} td2={} tau2={}",
                n(initial), n(pulsed), n(rise_delay),
                n(rise_tau), num(fd - rd), n(fall_tau),
            )
        }
        IrWaveform::Sffm { offset, amplitude, carrier_freq, modulation_index, signal_freq } => {
            // The FM pedestal is `sinedc`, same as the sine's. `offset` is
            // also a real vsource parameter and is silently ignored on an
            // FM source, which is what makes the wrong spelling dangerous.
            format!(
                "type=\"fm\" sinedc={} ampl={} freq={} modindex={} modfreq={}",
                n(offset), n(amplitude), n(carrier_freq),
                n(modulation_index), n(signal_freq),
            )
        }
        IrWaveform::Am { amplitude, offset, modulating_freq, carrier_freq, delay } => {
            format!(
                "type=\"am\" sinedc={} ampl={} freq={} modfreq={} delay={}",
                n(offset), n(amplitude), n(carrier_freq),
                n(modulating_freq), n(delay),
            )
        }
        // `type="pwl" wave=[t0, v0, ...]` is the documented spelling and
        // VACASK does parse the list (`print instance` reads it back
        // correctly), but the transient it then runs is identically zero
        // with uninitialised memory at t=0 — measured 6.9e-310 V on a deck
        // whose source should have ramped to 1 V. A named refusal beats a
        // plausible wrong answer.
        IrWaveform::Pwl { .. } => {
            return Err(unsupported_comp(format!(
                "{}: PWL sources are broken in VACASK (the transient comes back identically zero)",
                name
            )));
        }
        // VACASK's trannoise injects device noise; it has no noise or
        // random source (dev/research/ir-gaps.md §2).
        IrWaveform::TrNoise { .. } => return Err(unsupported_comp(format!("{name}: TRNOISE source (ngspice only)"))),
        IrWaveform::TrRandom { .. } => return Err(unsupported_comp(format!("{name}: TRRANDOM source (ngspice only)"))),
    })
}

/// `ic=[...]` / `nodeset=[...]`: a flat list, semicolon-separated, node
/// names quoted and values bare.
fn node_list(key: &str, items: &[(String, f64)]) -> String {
    let body: Vec<String> = items
        .iter()
        .map(|(n, v)| format!("\"{}\"; {}", n.to_ascii_lowercase(), num(*v)))
        .collect();
    format!(" {}=[{}]", key, body.join("; "))
}

/// One `sweep` line. `points` counts INTERVALS, not points, and the
/// endpoint is RECOMPUTED rather than copied: SPICE walks from START by
/// STEP and stops before passing STOP, while VACASK divides `from`..`to`
/// into equal intervals and always lands ON `to`. Copying STOP across puts
/// every sample on a different grid.
fn sweep_line(scale: &str, target: &str, name: &str, param: &str, start: f64, stop: f64, step: f64)
    -> Result<String, CodeGenError>
{
    if step == 0.0 {
        return Err(unsupported_analysis("sweep with a zero step"));
    }
    let span = ((stop - start) / step).abs();
    let n = (span + 1e-9 * span.max(1.0)).floor();
    if n < 1.0 || n.is_nan() || n > 1e7 {
        return Err(unsupported_analysis(format!(
            "sweep of '{}' covers {} intervals", name, n
        )));
    }
    let last = start + (stop - start).signum() * n * step.abs();
    // Within rounding of the stop, the stop is the endpoint (a VACASK-origin
    // sweep's step is (to - from) / points, which multiplies back an ulp off).
    let last = if (last - stop).abs() <= 1e-9 * step.abs() { stop } else { last };
    Ok(format!(
        "sweep {} {}=\"{}\" parameter=\"{}\" from={} to={} mode=\"lin\" points={}",
        scale, target, name.to_ascii_lowercase(), param, num(start), num(last), n as u64,
    ))
}

/// `.step param` -> a control variable plus a sweep over it. A `var`
/// declared in the control block IS visible to the netlist body, verified
/// against the binary, so `r1 (a b) resistor r=rval` tracks the sweep.
fn step_sweep(sp: &StepParam) -> Result<String, CodeGenError> {
    let name = sp.param.to_ascii_lowercase();
    let scale = format!("step_{}", sanitize(&name));
    match sp.sweep_type.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("dec") | Some("oct") => {
            let mode = sp.sweep_type.as_deref().unwrap().to_ascii_lowercase();
            Ok(format!(
                "sweep {} variable=\"{}\" from={} to={} mode=\"{}\" points={}",
                scale, name, num(sp.start), num(sp.stop), mode, sp.step as u64,
            ))
        }
        _ => Ok(format!(
            "sweep {} variable=\"{}\" from={} to={} step={}",
            scale, name, num(sp.start), num(sp.stop), num(sp.step),
        )),
    }
}

/// Frequency-swept analyses. For `dec`/`oct` SPICE's N is points per
/// decade/octave and so is VACASK's; for `lin` SPICE's N is the TOTAL point
/// count while VACASK counts intervals, hence the `- 1`.
fn freq_sweep(variation: Sweep, points: u32, start: f64, stop: f64)
    -> Result<String, CodeGenError>
{
    let mode = variation.name();
    let n = match variation {
        Sweep::Dec | Sweep::Oct => points,
        Sweep::Lin => points.saturating_sub(1),
        Sweep::Log => return Err(unsupported_analysis("frequency sweep type 'log'")),
    };
    if n == 0 {
        return Err(unsupported_analysis(format!("frequency sweep with {} points", points)));
    }
    Ok(format!(
        "from={} to={} mode=\"{}\" points={}",
        num(start), num(stop), mode, n
    ))
}

/// The `analysis` statement plus any `sweep` lines that must precede it.
/// `native`: the IR was read from a VACASK deck, whose `tran` without a
/// `maxstep` means VACASK's own default, not SPICE's TMAX.
fn analysis_block(idx: usize, a: &Analysis, native: bool) -> Result<Vec<String>, CodeGenError> {
    let name = format!("{}{}", a.kind_str(), idx + 1);
    let mut out = Vec::new();
    match a {
        Analysis::Op => out.push(format!("analysis {} op", name)),

        // VACASK has no `.dc`: a DC sweep is a `sweep` block wrapped around
        // an operating point. SPICE's FIRST source is the fast one and
        // VACASK's LAST `sweep` is, so the blocks emit in the opposite
        // order.
        Analysis::Dc { sweeps } => {
            if sweeps.is_empty() {
                return Err(unsupported_analysis("dc with no swept source"));
            }
            for (i, sw) in sweeps.iter().enumerate().rev() {
                let scale = if i == 0 { "vsweep".to_string() } else { format!("sweep{}", i) };
                out.push(sweep_line(
                    &scale, "instance", &sw.source, "dc", sw.start, sw.stop, sw.step,
                )?);
            }
            out.push(format!("  analysis {} op", name));
        }

        Analysis::Ac { sweep: FreqSweep { variation, points, start, stop } } => {
            out.push(format!(
                "analysis {} ac {}",
                name, freq_sweep(*variation, *points, *start, *stop)?
            ));
        }

        // `step` is only a STARTING step in VACASK; `maxstep` is the
        // separate bound. SPICE's TSTEP also caps the internal step because
        // TMAX defaults to min(TSTEP, (TSTOP-TSTART)/50). Emitting `step`
        // alone lets VACASK take strides SPICE never would.
        Analysis::Transient { step, stop, start, max_step, uic } => {
            if start.unwrap_or(0.0) != 0.0 {
                return Err(unsupported_analysis(
                    "transient with a nonzero start time (VACASK always saves from t=0)",
                ));
            }
            let tmax = max_step.or((!native).then(|| step.min(stop / 50.0)));
            let mut s = format!("analysis {} tran step={} stop={}", name, num(*step), num(*stop));
            if let Some(t) = tmax {
                w!(s, " maxstep={}", num(t));
            }
            s.push_str(if *uic { " icmode=\"uic\"" } else { " icmode=\"op\"" });
            out.push(s);
        }

        Analysis::Noise { output, reference, source, sweep: FreqSweep { variation, points, start, stop }, .. } => {
            if !reference.is_empty() && reference != "0" {
                return Err(unsupported_analysis(
                    "noise with a non-ground reference node (VACASK's `out=` names one node)",
                ));
            }
            out.push(format!(
                "analysis {} noise out=\"{}\" in=\"{}\" {}",
                name,
                bare_node(output).to_ascii_lowercase(),
                source.to_ascii_lowercase(),
                freq_sweep(*variation, *points, *start, *stop)?,
            ));
        }

        // VACASK's `dcxf` reports the transfer function from EVERY source
        // to `out` in one plot, so the input source is implicit.
        Analysis::Tf { output, .. } => {
            out.push(format!("analysis {} dcxf out=\"{}\"", name, bare_node(output).to_ascii_lowercase()));
        }

        other => {
            return Err(unsupported_analysis(other.kind_str().to_string()));
        }
    }
    Ok(out)
}

fn emit_model(m: &ModelDef, loads: &mut BTreeSet<String>, low: bool) -> Result<String, CodeGenError> {
    let kind = m.kind.to_ascii_lowercase();
    let name = q(&m.name);

    // LEVEL is the module SELECTOR, not a parameter, and defaults to 1.
    let mut level = 1u32;
    for (k, v) in &m.parameters {
        if k.eq_ignore_ascii_case("level") {
            level = value_of(v).map(|f| f as u32).ok_or_else(|| {
                unsupported_comp(format!("model {}: LEVEL={}", name, v))
            })?;
        }
    }

    let (module, sign) = if kind == "nmos" || kind == "pmos" {
        let module = mos_module(level).ok_or_else(|| {
            unsupported_comp(format!("model {}: {} LEVEL={} has no VACASK module", name, kind, level))
        })?;
        (module, if kind.starts_with('n') { 1i8 } else { -1 })
    } else if let Some((module, sign)) = model_type(&kind) {
        // Every other type here is the LEVEL 1 device. `.model x NPN
        // (LEVEL=4)` is VBIC and `LEVEL=8` is HICUM: different equations
        // wearing the same keyword, and mapping on the keyword alone would
        // run them as Gummel-Poon.
        if level != 1 {
            return Err(unsupported_comp(format!(
                "model {}: {} LEVEL={} has no VACASK module", name, kind, level
            )));
        }
        (module, sign)
    } else if matches!(kind.as_str(), "r" | "c" | "l" | "res" | "cap" | "ind" | "sw" | "csw") {
        // See `model_type`: the passive modules rename their model-card
        // parameters, so a name-for-name transcription would silently set
        // the instance parameter instead. Switches have no VACASK module.
        return Err(unsupported_comp(format!(
            "model {}: '{}' model cards have no faithful VACASK spelling", name, kind
        )));
    } else {
        // A module the user loaded themselves (`osdi_loads`). Pass it
        // through: VACASK errors loudly with "Model not found" if it is not
        // there, which beats guessing at a substitute.
        return Ok(format!("model {} {}{}", name, kind, model_params(&m.parameters, None, low)));
    };

    loads.insert(format!("spice/{}.osdi", module));
    // `type=` is the polarity selector, not a `.model` parameter, and it
    // also keeps an otherwise-empty list non-empty: `model x sp_diode ( )`
    // is a VACASK syntax error.
    let lead = (sign != 0).then_some(sign);
    Ok(format!("model {} sp_{}{}", name, module, model_params(&m.parameters, lead, low)))
}

/// ` (type=1 k=v ...)`, or nothing for no `type=` and no parameters.
fn model_params(params: &[(String, String)], sign: Option<i8>, low: bool) -> String {
    let mut s = String::new();
    if let Some(t) = sign {
        w!(s, " type={t}");
    }
    for (k, v) in params.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("level")) {
        w!(s, " {}={}", k.to_ascii_lowercase(), expr(v, low));
    }
    if !s.is_empty() {
        s.replace_range(..1, " (");
        s.push(')');
    }
    s
}

/// VACASK identifiers cannot carry punctuation.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

/// A name as a VACASK identifier: `[a-zA-Z_$][0-9a-zA-Z_$]*`, a number, or
/// else quoted, `''` for a quote (docs/input-identifiers.md). SPICE names
/// carry `#` `/` `-` `+` freely (`mnd#/mp1`, `Net-_R1-Pad2_`). Lowercased
/// when `.1`: SPICE is case-insensitive, VACASK is not. Written as each name
/// is, so `netlist` renames nothing in a copy of the IR.
#[derive(Clone, Copy)]
struct Q<'a>(&'a str, bool);

/// [`Q`], lowercased: how every name reaches the deck.
fn q(name: &str) -> Q<'_> {
    Q(name, true)
}

impl std::fmt::Display for Q<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.0;
        let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
            && (name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
                // A number node is `0` or has no leading zero (`06` is not one).
                || (name.chars().all(|c| c.is_ascii_digit()) && (name == "0" || !name.starts_with('0'))));
        let quoted = !plain && !name.is_empty();
        if quoted {
            f.write_char('\'')?;
        }
        for c in name.chars() {
            if quoted && c == '\'' {
                f.write_char('\'')?;
            }
            f.write_char(if self.1 { c.to_ascii_lowercase() } else { c })?;
        }
        if quoted {
            f.write_char('\'')?;
        }
        Ok(())
    }
}

pub(super) fn netlist(ir: &CircuitIR) -> Result<String, CodeGenError> {
    super::refuse_raw_spice(ir, "vacask")?;
    if let Some(f) = ir.functions.first() {
        return Err(unsupported_comp(format!(".func {}: VACASK has no user functions", f.name)));
    }
    // A SPICE-origin IR's expressions are lowercased as they are written.
    let low = ir.raw_dialect != Some(Dialect::Vacask);
    let mut loads: BTreeSet<String> = BTreeSet::new();
    let mut needs: BTreeSet<Need> = BTreeSet::new();

    let subs: Vec<&Subcircuit> = std::iter::once(&ir.top).chain(ir.subcircuit_defs.iter()).collect();
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
        sub_models.push(sc.models.iter().map(|m| emit_model(m, &mut loads, low)).collect::<Result<_, _>>()?);
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
    let title = if title.trim().is_empty() { "circuit" } else { &title };
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
        if subs.iter().any(|s| s.models.iter().any(|m| m.name.eq_ignore_ascii_case(n.auto_model()))) {
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

        w!(out, "  options rawfile=\"binary\"{}", emit_options(&tb.options)?);
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
            w!(out, "  var {}={}\n", sp.param.to_ascii_lowercase(), num(sp.start));
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
            for l in analysis_block(idx, a, ir.raw_dialect == Some(Dialect::Vacask))? {
                let ic = if l.trim_start().starts_with("analysis") { ic.as_str() } else { "" };
                w!(out, "  {l}{ic}\n");
            }
        }
        out.push_str("endc\n");
    }

    Ok(out)
}

/// ` a=1 b=0`: parameter declarations, a missing default as 0.
fn param_list<'a>(out: &mut String, defs: impl Iterator<Item = &'a ParamDef>, low: bool) {
    for p in defs {
        match &p.default {
            Some(d) => w!(out, " {}={}", p.name.to_ascii_lowercase(), expr(d, low)),
            None => w!(out, " {}=0", p.name.to_ascii_lowercase()),
        }
    }
}

/// A `subckt` ... `ends` block, with no line end after `ends`.
fn emit_subcircuit(out: &mut String, sc: &Subcircuit, models: &[String], low: bool) -> Result<(), CodeGenError> {
    w!(out, "subckt {} (", q(&sc.name));
    for (i, p) in sc.ports.iter().enumerate() {
        w!(out, "{}{}", if i == 0 { "" } else { " " }, q(&p.name));
    }
    out.push_str(")\n");
    for m in models {
        w!(out, "  {m}\n");
    }
    if !sc.parameters.is_empty() {
        out.push_str("parameters");
        param_list(out, sc.parameters.iter(), low);
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
    out.push_str("ends");
    Ok(())
}

/// `comp`'s instance line, written onto `out` with no line end. Nothing is
/// written when it errs.
fn emit_component(out: &mut String, comp: &Component, low: bool) -> Result<(), CodeGenError> {
    let auto = |n: Need| n.auto_model();
    match comp {
        Component::Resistor { name, n1, n2, value, model, params }
        | Component::Capacitor { name, n1, n2, value, model, params }
        | Component::Inductor { name, n1, n2, value, model, params } => {
            if model.is_some() {
                model_refuse("vacask", &Q(name, false).to_string(), model)?;
            }
            let (l, m) = match comp {
                Component::Resistor { .. } => ('r', "resistor"),
                Component::Capacitor { .. } => ('c', "capacitor"),
                _ => ('l', "inductor"),
            };
            w!(out, "{l}{} ({} {}) {} {l}={}", q(name), q(n1), q(n2), auto(Need::Sp(m)), val(value, low));
            inst_params(out, params, low);
        }
        // VACASK's mutual inductance is a device with NO nodes that names
        // the two inductors it couples.
        Component::MutualInductor { name, inductor1, inductor2, coupling } => w!(
            out,
            "k{} () {} k={} ind1=\"l{}\" ind2=\"l{}\"",
            q(name), auto(Need::Builtin("mutual")), num(*coupling), q(inductor1), q(inductor2),
        ),
        // VACASK's `acsp` ports are not ngspice's `portnum` (ir-gaps §2):
        // refused, not dropped.
        Component::VoltageSource { name, port: Some(_), .. } => return Err(unsupported_comp(format!("v{}: RF port", q(name)))),
        Component::VoltageSource { name, np, nm, value, ac_magnitude, ac_phase, waveform, port: None }
        | Component::CurrentSource { name, np, nm, value, ac_magnitude, ac_phase, waveform } => {
            let (l, m) = if matches!(comp, Component::VoltageSource { .. }) { ('v', "vsource") } else { ('i', "isource") };
            let wave = match waveform {
                Some(wf) => Some(wave(&format_args!("{l}{}", q(name)), wf, low)?),
                None => None,
            };
            w!(out, "{l}{} ({} {}) {} dc={}", q(name), q(np), q(nm), auto(Need::Builtin(m)), val(value, low));
            ac_spec(out, ac_magnitude, ac_phase);
            if let Some(wave) = wave {
                w!(out, " {wave}");
            }
        }
        Component::Vcvs { name, np, nm, ncp, ncm, gain } => w!(
            out,
            "e{} ({} {} {} {}) {} gain={}",
            q(name), q(np), q(nm), q(ncp), q(ncm), auto(Need::Builtin("vcvs")), num(*gain),
        ),
        Component::Vccs { name, np, nm, ncp, ncm, transconductance } => w!(
            out,
            "g{} ({} {} {} {}) {} gain={}",
            q(name), q(np), q(nm), q(ncp), q(ncm), auto(Need::Builtin("vccs")), num(*transconductance),
        ),
        // The current-controlled pair names its sensing device with
        // `ctlinst` instead of taking it as a node.
        Component::Cccs { name, np, nm, vsense, gain } => w!(
            out,
            "f{} ({} {}) {} ctlinst=\"{}\" gain={}",
            q(name), q(np), q(nm), auto(Need::Builtin("cccs")), q(vsense), num(*gain),
        ),
        Component::Ccvs { name, np, nm, vsense, transresistance } => w!(
            out,
            "h{} ({} {}) {} ctlinst=\"{}\" gain={}",
            q(name), q(np), q(nm), auto(Need::Builtin("ccvs")), q(vsense), num(*transresistance),
        ),
        Component::Diode { name, np, nm, model, params } => {
            w!(out, "d{} ({} {}) {}", q(name), q(np), q(nm), q(model));
            inst_params(out, params, low);
        }
        Component::Bjt { name, nc, nb, ne, extra_nodes, model, params } => {
            w!(out, "q{} ({} {} {}", q(name), q(nc), q(nb), q(ne));
            for x in extra_nodes {
                w!(out, " {}", q(x));
            }
            w!(out, ") {}", q(model));
            inst_params(out, params, low);
        }
        Component::Mosfet { nb: None, .. } => return Err(unsupported_comp("MOSFET without a bulk node (VDMOS): terminal list unverified")),
        Component::Mosfet { extra_nodes, .. } if !extra_nodes.is_empty() => return Err(unsupported_comp("MOSFET with extra nodes (VDMOS thermal, SOI): terminal list unverified")),
        Component::Mosfet { name, nd, ng, ns, nb: Some(nb), model, params, .. } => {
            w!(out, "m{} ({} {} {} {}) {}", q(name), q(nd), q(ng), q(ns), q(nb), q(model));
            inst_params(out, params, low);
        }
        Component::Jfet { name, nd, ng, ns, model, params } => {
            w!(out, "j{} ({} {} {}) {}", q(name), q(nd), q(ng), q(ns), q(model));
            inst_params(out, params, low);
        }
        Component::Mesfet { name, nd, ng, ns, model, params } => {
            w!(out, "z{} ({} {} {}) {}", q(name), q(nd), q(ng), q(ns), q(model));
            inst_params(out, params, low);
        }
        // SPICE text; native VACASK statements go in `Testbench::extra_lines`.
        Component::RawSpice { line } => return Err(unsupported_comp(format!("raw SPICE line '{}'", line.trim()))),

        Component::Poly { name, .. } => return Err(unsupported_comp(format!("{}: POLY source (VACASK has no polynomial builtin)", Q(name, false)))),
        Component::BehavioralVoltage { name, .. } | Component::BehavioralCurrent { name, .. } => {
            return Err(unsupported_comp(format!(
                "b{}: behavioural sources are an expression language VACASK does not share",
                q(name)
            )));
        }
        Component::VSwitch { name, .. } => {
            return Err(unsupported_comp(format!("s{}: VACASK has no voltage-controlled switch", Q(name, false))));
        }
        Component::ISwitch { name, .. } => {
            return Err(unsupported_comp(format!("w{}: VACASK has no current-controlled switch", Q(name, false))));
        }
        Component::TLine { name, .. } => {
            return Err(unsupported_comp(format!("t{}: VACASK has no lossless transmission line", Q(name, false))));
        }
        Component::Xspice { name, .. } => {
            return Err(unsupported_comp(format!("a{}: XSPICE A-elements are ngspice-only", Q(name, false))));
        }
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
            Component::Resistor { name: s("1"), n1: s("A"), n2: s("b"), value: IrValue::Expression { expr: s("Rload") }, model: None, params: vec![(s("tc1"), s("1M"))] },
            Component::Capacitor { name: s("1"), n1: s("a"), n2: s("b"), value: v(1e-9), model: None, params: vec![] },
            Component::Inductor { name: s("1"), n1: s("a"), n2: s("b"), value: v(1e-6), model: None, params: vec![] },
            Component::Inductor { name: s("2"), n1: s("a"), n2: s("b"), value: v(1e-6), model: None, params: vec![] },
            Component::MutualInductor { name: s("1"), inductor1: s("1"), inductor2: s("2"), coupling: 0.9 },
            Component::VoltageSource { name: s("1"), np: s("a"), nm: s("0"), value: v(1.0), ac_magnitude: None, ac_phase: None, waveform: None, port: None },
            Component::CurrentSource { name: s("1"), np: s("a"), nm: s("0"), value: v(1.0), ac_magnitude: None, ac_phase: None, waveform: None },
            Component::Vcvs { name: s("1"), np: s("a"), nm: s("b"), ncp: s("c"), ncm: s("d"), gain: 2.0 },
            Component::Vccs { name: s("1"), np: s("a"), nm: s("b"), ncp: s("c"), ncm: s("d"), transconductance: 2.0 },
            Component::Cccs { name: s("1"), np: s("a"), nm: s("b"), vsense: s("V1"), gain: 2.0 },
            Component::Ccvs { name: s("1"), np: s("a"), nm: s("b"), vsense: s("V1"), transresistance: 2.0 },
            Component::Diode { name: s("1"), np: s("a"), nm: s("b"), model: s("dm"), params: vec![] },
            Component::Bjt { name: s("1"), nc: s("a"), nb: s("b"), ne: s("c"), extra_nodes: vec![s("d")], model: s("qm"), params: vec![] },
            Component::Mosfet { name: s("1"), nd: s("a"), ng: s("b"), ns: s("c"), nb: Some(s("d")), extra_nodes: vec![], model: s("mm"), params: vec![] },
            Component::Jfet { name: s("1"), nd: s("a"), ng: s("b"), ns: s("c"), model: s("jm"), params: vec![] },
            Component::Mesfet { name: s("1"), nd: s("a"), ng: s("b"), ns: s("c"), model: s("zm"), params: vec![] },
        ];
        let dm = ModelDef { name: s("dm"), kind: s("d"), parameters: vec![] };
        let cell = Subcircuit { name: s("cell"), ports: vec![Port { name: s("p"), direction: PortDirection::InOut }], components: comps[..1].to_vec(), models: vec![dm.clone()], ..Default::default() };
        let x1 = Instance { name: s("1"), subcircuit: s("cell"), port_mapping: vec![s("a")], ..Default::default() };
        let top = Subcircuit { name: s("t"), components: comps, instances: vec![x1], models: vec![dm], ..Default::default() };
        let tb = Testbench { analyses: vec![Analysis::Op], ..Default::default() };
        let mut ir = CircuitIR { top, subcircuit_defs: vec![cell], testbench: Some(tb), globals: vec![s("g")], ..Default::default() };
        ir.names_mut(|n| if n != "0" { n.push_str("/x") });
        let deck = netlist(&ir).unwrap();
        for line in deck.lines().filter(|l| !l.starts_with("load ")) {
            let mut quoted = false;
            for c in line.chars() {
                quoted ^= c == '\'';
                assert!(c != '/' || quoted, "unquoted `/` in `{line}`");
            }
        }
        for want in ["r'1/x' ('a/x' 'b/x') resistor r=rload tc1=1m", "ind1=\"l'1/x'\"", "ctlinst=\"'v1/x'\"", "x'1/x' ('a/x') 'cell/x'", "subckt 'cell/x' ('p/x')", "model 'dm/x' sp_diode", "global 'g/x'"] {
            assert!(deck.contains(want), "no `{want}` in\n{deck}");
        }
    }

    fn emit_analysis(a: &Analysis) -> Result<String, CodeGenError> {
        Ok(analysis_block(0, a, false)?.join("\n"))
    }
    use std::collections::BTreeMap;

    fn sub(name: &str) -> Subcircuit {
        Subcircuit {
            name: name.into(), ..Default::default() }
    }

    fn tb(analyses: Vec<Analysis>) -> Testbench {
        Testbench {
            analyses, ..Default::default() }
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
        let a = Analysis::Transient { step: 1e-6, stop: 1e-3, start: None, max_step: None, uic: false };
        let s = emit_analysis(&a).unwrap();
        assert!(s.contains("maxstep="), "{}", s);
        // min(TSTEP, TSTOP/50) = min(1e-6, 2e-5) = 1e-6
        assert!(s.contains("maxstep=1e-6"), "{}", s);
    }

    #[test]
    fn dc_sweep_counts_intervals_and_recomputes_the_endpoint() {
        // 0..10 by 0.5 is 21 SPICE samples => 20 intervals.
        let a = Analysis::Dc { sweeps: vec![DcSweep { source: "V1".into(), start: 0.0, stop: 10.0, step: 0.5 }] };
        let s = emit_analysis(&a).unwrap();
        assert!(s.contains("points=20"), "{}", s);
        assert!(s.contains("instance=\"v1\" parameter=\"dc\""), "{}", s);

        // 0..1.1 by 0.2: SPICE stops at 1.0 without ever reaching 1.1.
        let a = Analysis::Dc { sweeps: vec![DcSweep { source: "Vgs".into(), start: 0.0, stop: 1.1, step: 0.2 }] };
        let s = emit_analysis(&a).unwrap();
        assert!(s.contains("points=5"), "{}", s);
        assert!(s.contains("to=1e0"), "{}", s);
    }

    #[test]
    fn two_source_dc_emits_blocks_in_reverse() {
        let a = Analysis::Dc {
            sweeps: vec![
                DcSweep { source: "Vds".into(), start: 0.0, stop: 1.0, step: 0.1 },
                DcSweep { source: "Vgs".into(), start: 0.0, stop: 1.0, step: 0.5 },
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
        let lin = emit_analysis(&Analysis::Ac { sweep: FreqSweep { variation: Sweep::Lin, points: 101, start: 1.0, stop: 1e3 } }).unwrap();
        assert!(lin.contains("points=100"), "{}", lin);
        let dec = emit_analysis(&Analysis::Ac { sweep: FreqSweep { variation: Sweep::Dec, points: 10, start: 1.0, stop: 1e3 } }).unwrap();
        assert!(dec.contains("points=10"), "{}", dec);
    }

    #[test]
    fn pwl_is_refused_rather_than_emitted() {
        let c = Component::VoltageSource {
            name: "1".into(), np: "a".into(), nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 }, ac_magnitude: None, ac_phase: None,
            waveform: Some(IrWaveform::Pwl { values: vec![((0.0).into(), (0.0).into()), ((1e-3).into(), (1.0).into())], repeat: None, delay: None }),
            port: None,
        };
        let err = emit_component(&mut String::new(), &c, true).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("pwl"), "{}", err);
    }

    #[test]
    fn measures_are_refused() {
        let mut t = tb(vec![Analysis::Op]);
        t.measures = vec!["tran vmax MAX V(out)".into()];
        let ir = CircuitIR { top: sub("m"), testbench: Some(t), ..Default::default() };
        assert!(netlist(&ir).is_err());
    }

    #[test]
    fn step_params_become_a_control_variable_and_a_sweep() {
        let mut t = tb(vec![Analysis::Op]);
        t.step_params = vec![StepParam { param: "rval".into(), start: 1e3, stop: 3e3, step: 1e3, sweep_type: None }];
        let mut top = sub("stepped");
        top.parameters = vec![ParamDef { name: "rval".into(), default: Some("1k".into()) }];
        let ir = CircuitIR { top, testbench: Some(t), ..Default::default() };
        let n = netlist(&ir).unwrap();
        assert!(n.contains("var rval=1e3"), "{}", n);
        assert!(n.contains("sweep step_rval variable=\"rval\""), "{}", n);
        // A stepped name must not also be a netlist parameter.
        assert!(!n.contains("parameters rval"), "{}", n);
    }

    #[test]
    fn model_library_uses_the_vacask_path() {
        let mut backend_paths = BTreeMap::new();
        backend_paths.insert("ngspice".into(), "/pdk/ngspice/sky130.lib.spice".into());
        backend_paths.insert("vacask".into(), "/pdk/vacask/sky130.sim".into());
        let ir = CircuitIR {
            top: sub("pdk_test"),
            model_libraries: vec![ModelLibrary {
                name: "sky130".into(), path: "/pdk/default.lib".into(), corner: Some("tt".into()),
                backend_paths, setup_includes: vec![],
            }], ..Default::default() };
        let n = netlist(&ir).unwrap();
        assert!(n.contains("include \"/pdk/vacask/sky130.sim\" section=tt"), "{}", n);
        assert!(!n.contains("ngspice"), "{}", n);
    }

    #[test]
    fn model_library_falls_back_to_the_default_path() {
        let ir = CircuitIR {
            top: sub("fallback"),
            model_libraries: vec![ModelLibrary {
                name: "custom".into(), path: "/models/custom.sim".into(), corner: None,
                backend_paths: BTreeMap::new(), setup_includes: vec![],
            }], ..Default::default() };
        let n = netlist(&ir).unwrap();
        assert!(n.contains("include \"/models/custom.sim\""), "{}", n);
    }

    #[test]
    fn bjt_model_carries_its_polarity_and_loads_its_module() {
        let mut top = sub("bjt");
        top.models = vec![ModelDef {
            name: "QN".into(), kind: "NPN".into(),
            parameters: vec![("IS".into(), "1e-15".into()), ("BF".into(), "150".into())],
        }];
        let ir = CircuitIR { top, ..Default::default() };
        let n = netlist(&ir).unwrap();
        assert!(n.contains("load \"spice/bjt.osdi\""), "{}", n);
        assert!(n.contains("model qn sp_bjt (type=1 is=1e-15 bf=150)"), "{}", n);
    }

    #[test]
    fn a_mosfet_level_without_a_module_is_refused() {
        let mut top = sub("m");
        top.models = vec![ModelDef {
            name: "nm".into(), kind: "NMOS".into(), parameters: vec![("LEVEL".into(), "77".into())],
        }];
        let ir = CircuitIR { top, ..Default::default() };
        assert!(netlist(&ir).is_err());
    }

    #[test]
    fn passive_model_cards_are_refused() {
        // sp_resistor renames the model card's parameters; transcribing them
        // name-for-name would set the instance parameters instead.
        let mut top = sub("r");
        top.models = vec![ModelDef { name: "rm".into(), kind: "R".into(), parameters: vec![("tc1".into(), "1e-3".into())] }];
        let ir = CircuitIR { top, ..Default::default() };
        assert!(netlist(&ir).is_err());
    }

    #[test]
    fn unknown_options_refuse_instead_of_being_dropped() {
        let mut t = tb(vec![Analysis::Op]);
        t.options.portable = vec![("itl1".into(), "100".into())];
        let ir = CircuitIR { top: sub("o"), testbench: Some(t), ..Default::default() };
        assert!(netlist(&ir).is_err());

        let mut t = tb(vec![Analysis::Op]);
        t.options.portable = vec![("reltol".into(), "1e-4".into()), ("noacct".into(), "".into())];
        let ir = CircuitIR { top: sub("o"), testbench: Some(t), ..Default::default() };
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
