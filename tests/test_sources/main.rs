//! Waveform sources beyond SPICE3's six: PWL `r=`/`td=`, TRNOISE and
//! TRRANDOM, and what each dialect writes for them or refuses
//! (`syntax::render`); ngspice PWL `r=`/`td=`, RF ports and `.sp` read and
//! written back, and Spectre PWL `delay`/`pwlperiod` written as ngspice.
//!
//! Oracle: real ngspice running the deck the renderer wrote, checked against
//! the waveform's closed form (ngspice 45 manual §4.1.4) or, for the random
//! sources, against their distribution with a stated statistical tolerance
//! (§4.1.7, §4.1.8). An ngspice deck read and written back: real ngspice on
//! the original and on the written deck, every vector equal to 1e-12. A
//! Spectre deck (no Spectre here): the Spectre Reference's semantics as the
//! closed form, on ngspice running the deck written from it. A missing
//! ngspice is a failure, not a skip.
//!
//! Contents: none (tests only).
#![cfg(feature = "backend")]

use spicerack::result::RawData;
use spicerack::syntax::Dialect;
use spicerack::syntax::diag::Code;
use spicerack::syntax::render::{Diags, render_text};

mod imported;
mod pwl;
mod random;

// ---- private ----

/// `PWL(0 0 1m 1 2m 0)`: a 1 V triangle, 2 ms long, then ` r=<repeat>`
/// and ` td=<delay>` as given.
fn triangle(repeat: Option<&str>, delay: Option<&str>) -> String {
    let mut w = String::from("PWL(0 0 1m 1 2m 0)");
    if let Some(r) = repeat {
        w.push_str(&format!(" r={r}"));
    }
    if let Some(td) = delay {
        w.push_str(&format!(" td={td}"));
    }
    w
}

/// A `V1` (or, with `current`, an `I1`) card from `a` to `0` carrying
/// `wave`, at DC 0 (the waveform's own t=0 value for every one used here).
fn source(current: bool, wave: &str) -> String {
    let letter = if current { 'I' } else { 'V' };
    format!("{letter}1 a 0 0 {wave}")
}

/// `src` into 1 Ω at `a`, under `.tran` to `stop` with steps of at most
/// `step`, and `extra` cards before the analysis.
fn bench(src: &str, step: f64, stop: f64, extra: &str) -> String {
    let n = spicerack::unit::format_spice_number;
    let (step, stop) = (n(step), n(stop));
    format!("* sources\n{src}\nRl a 0 1\n{extra}.tran {step} {stop} 0 {step}\n.end\n")
}

/// The ngspice deck `deck` written for `d`.
fn netlist(deck: &str, d: Dialect) -> Result<String, Diags> {
    render_text(deck, Dialect::Ngspice, "sources", d).map(|w| w.text)
}

/// Whether the refusal names `code`.
fn refuses(r: Result<String, Diags>, code: Code) -> bool {
    r.is_err_and(|e| e.diags.iter().any(|d| d.code == code))
}

fn ngspice(deck: &str) -> RawData {
    Dialect::Ngspice
        .run(deck)
        .unwrap_or_else(|e| panic!("ngspice: {e}\n{deck}"))
}

fn col<'a>(raw: &'a RawData, name: &str) -> &'a [f64] {
    let wrapped = format!("v({name})");
    let i = raw
        .variables
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case(name) || v.name.eq_ignore_ascii_case(&wrapped));
    let i = i.unwrap_or_else(|| {
        panic!(
            "no {name} in {:?}",
            raw.variables.iter().map(|v| &v.name).collect::<Vec<_>>()
        )
    });
    &raw.real_data[i]
}

/// The output at time `t`, linear between timepoints: exact for a PWL
/// source, whose corners ngspice lands on as breakpoints.
fn at(time: &[f64], v: &[f64], t: f64) -> f64 {
    let k = time.partition_point(|&x| x < t).clamp(1, time.len() - 1);
    let (t0, t1) = (time[k - 1], time[k]);
    v[k - 1] + (v[k] - v[k - 1]) * (t - t0) / (t1 - t0)
}
