//! Raw and extra lines -> whether an ngspice deck runs from a `.control`
//! block, and the `fourier` commands it runs there.

use super::values::num;
use crate::codegen::Dialect;
use crate::ir::*;

/// Control commands that start an analysis: a `.control` block issuing one
/// owns the run. A block that runs nothing (the `pre_osdi` loads above) does
/// not; ngspice accepts several blocks per deck (44.2).
const RUN_COMMANDS: [&str; 5] = ["run", "op", "ac", "tran", "dc"];

/// True if a `.control`/`.endc` block among `lines` issues a run command.
pub(crate) fn runs_in_control<'a>(lines: impl IntoIterator<Item = &'a str>) -> bool {
    let mut inside = false;
    for line in lines {
        let word = line.split_whitespace().next().unwrap_or("");
        if word.eq_ignore_ascii_case(".control") {
            inside = true;
        } else if word.eq_ignore_ascii_case(".endc") {
            inside = false;
        } else if inside && RUN_COMMANDS.iter().any(|c| word.eq_ignore_ascii_case(c)) {
            return true;
        }
    }
    false
}

/// True if `line` is a `.meas`/`.measure` card (any case).
pub(crate) fn is_meas(line: &str) -> bool {
    line.split_whitespace()
        .next()
        .and_then(|w| w.get(..5))
        .is_some_and(|w| w.eq_ignore_ascii_case(".meas"))
}

/// True if `line` is a `.four` card (any case).
pub(crate) fn is_four(line: &str) -> bool {
    line.split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case(".four"))
}

/// Whether an ngspice deck runs its analyses from a `.control` block, and
/// the `fourier` commands to run there, one per `.four` (an
/// `Analysis::Fourier`, or a raw or extra line, with its `+` continuations).
///
/// ngspice refuses `.measure` ("No .measure possible in batch mode (-b) with
/// -r rawfile set!") and ignores `.four` (".fourier line ignored since
/// rawfile was produced") under `-b -r`, so a deck with either (from the
/// testbench or a raw line) runs every analysis from one block, in card
/// order, where `.four` has to be the `fourier` command (there is no `four`
/// command); `.meas` cards stay where they are. Every analysis goes in: one
/// left as a dot card would run after the block, past the backend's
/// `write`, and its plot would be lost. The backend appends the `write` of
/// every plot: only it knows the rawfile's path. `None` when nothing needs a
/// block, the deck has no `.ac`/`.tran`/`.dc` for a `.measure` to read, or a
/// raw control block already runs the deck (the backend refuses a deck left
/// with a `.meas` or `.four` and no block that runs it, rather than lose its
/// results).
pub(super) fn control_run(x: bool, ir: &CircuitIR, tb: &Testbench) -> Option<Vec<String>> {
    let raw = || {
        ir.subcircuit_defs
            .iter()
            .chain(std::iter::once(&ir.top))
            .flat_map(|s| &s.raw_spice)
            .chain(&tb.extra_lines)
            .map(String::as_str)
    };
    let mut fourier = fourier_commands(raw());
    for a in &tb.analyses {
        if let Analysis::Fourier {
            fundamental,
            outputs,
            ..
        } = a
        {
            let mut cmd = format!("fourier {}", num(Dialect::Ngspice, x, *fundamental));
            outputs.iter().for_each(|o| w!(cmd, " {o}"));
            fourier.push(cmd);
        }
    }
    if (tb.measures.is_empty() && !raw().any(is_meas) && fourier.is_empty())
        || runs_in_control(raw())
    {
        return None;
    }
    tb.analyses
        .iter()
        .any(|a| {
            matches!(
                a,
                Analysis::Ac { .. } | Analysis::Transient { .. } | Analysis::Dc { .. }
            )
        })
        .then_some(fourier)
}

/// Raw lines, one each; with `drop_four`, every `.four` card and its `+`
/// continuation lines are left out (the deck runs them as `fourier`
/// commands in its `.control` block instead).
pub(super) fn raw_lines(out: &mut String, lines: &[String], drop_four: bool) {
    let mut four = false;
    for l in lines {
        if !is_continuation(l) {
            four = drop_four && is_four(l);
        }
        if !four {
            w!(out, "{l}\n");
        }
    }
}

// ---- private ----

/// True if `line` continues the card above it (`+ ...`).
fn is_continuation(line: &str) -> bool {
    line.trim_start().starts_with('+')
}

/// The `.four` cards among `lines`, each joined with its `+` continuation
/// lines, as `fourier` commands: `.four 1k v(a)` / `+ v(b)` is
/// `fourier 1k v(a) v(b)`.
fn fourier_commands<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut open = false;
    for l in lines {
        if is_continuation(l) {
            if let (true, Some(cmd)) = (open, out.last_mut()) {
                l.trim_start()[1..]
                    .split_whitespace()
                    .for_each(|t| w!(cmd, " {t}"));
            }
            continue;
        }
        open = is_four(l);
        if open {
            let mut cmd = String::from("fourier");
            l.split_whitespace().skip(1).for_each(|t| w!(cmd, " {t}"));
            out.push(cmd);
        }
    }
    out
}
