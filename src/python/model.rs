//! What a Python circuit holds: each call spelled as its ngspice card at
//! once (`Card`), kept in the blocks the deck writes them in (`Body`, the
//! testbench's `Bench`), plus the spelling of a number or value as an
//! ngspice token reads it back. `deck.rs` puts the blocks in deck order and
//! the parser reads that text (TESTING.md, "Golden gate": the builder's
//! rows are the parser's).
//!
//! Cites: ngspice manual §2.1.3 (numbers and scale factors), ch.3-4 (the
//! element cards).
//!
//! Contents:
//! - `pub(super) struct Card`, `Card::element(name: &str, text: String) -> Card`,
//!   `Card::source(name: &str, head: String, value: &Value, tail: String) -> Card`,
//!   `Card::set_value(&mut self, v: f64) -> bool`
//! - `pub(super) struct Body`, `pub(super) struct Bench`, `pub(super) struct Step`,
//!   `pub(super) struct ModelLibrary`, `pub(super) struct Func`
//! - `pub(super) enum Value`, `Value::text(&self) -> String`
//! - `pub(super) fn num(v: f64) -> String`, `pub(super) fn dec(v: f64) -> String`,
//!   `pub(super) fn params(ps: &[(String, String)]) -> String`
//! - `pub(super) mod need`: the feature bits a card adds (`crate::backend::feature`)

use crate::veriloga::VerilogBlock;

/// One element card as written (`R1 a b 1k m=2`), the name the API finds it
/// by, and what it needs from a simulator.
#[derive(Clone, Debug)]
pub(super) struct Card {
    /// The name without its letter (`R("1", …)` is `1`).
    pub(super) name: String,
    pub(super) text: String,
    /// A V or I source: the card is `head value tail`, so its value can
    /// be set again.
    source: Option<(String, String)>,
    /// `crate::backend::feature` bits.
    pub(super) needs: u8,
}

/// A value argument: a number, or an expression over parameters (written
/// `{e}`), or text kept as given (`raw_spice=`).
#[derive(Clone, Debug)]
pub(super) enum Value {
    Num(f64),
    Expr(String),
    Raw(String),
}

/// A subcircuit's (or the top's) cards, in the blocks a deck writes them in.
#[derive(Clone, Debug, Default)]
pub(super) struct Body {
    pub(super) name: String,
    pub(super) ports: Vec<String>,
    /// `PARAMS:` defaults (the top's: its `.param` cards).
    pub(super) params: Vec<(String, String)>,
    pub(super) elements: Vec<Card>,
    /// `X` cards, written after the elements.
    pub(super) instances: Vec<Card>,
    /// `.model` cards.
    pub(super) models: Vec<String>,
    /// Lines given as text (`raw_spice`), written after the instances.
    pub(super) raw: Vec<String>,
    pub(super) includes: Vec<String>,
    pub(super) libs: Vec<(String, String)>,
    /// Verilog-A sources and OSDI binaries to load.
    pub(super) osdi: Vec<String>,
    pub(super) verilog: Vec<VerilogBlock>,
}

/// A testbench's settings, stimulus and analyses.
#[derive(Clone, Debug, Default)]
pub(super) struct Bench {
    pub(super) stimulus: Vec<Card>,
    pub(super) analyses: Vec<super::analysis::Analysis>,
    pub(super) options: Vec<(String, String)>,
    pub(super) saves: Vec<String>,
    pub(super) measures: Vec<String>,
    pub(super) temperature: Option<f64>,
    pub(super) nominal_temperature: Option<f64>,
    pub(super) ics: Vec<(String, f64)>,
    pub(super) nodesets: Vec<(String, f64)>,
    pub(super) steps: Vec<Step>,
    /// Lines in the target's own dialect, given as text.
    pub(super) extra_lines: Vec<String>,
    /// `.print` cards: the analysis (maybe empty) and its outputs.
    pub(super) prints: Vec<(String, Vec<String>)>,
    /// ngspice's compatibility mode, which the backend passes on its
    /// command line (`backend::ngspice::NGBEHAVIOR`).
    pub(super) ngbehavior: Option<String>,
}

/// A `.step` sweep: `lin`, `dec`, `oct` or `log`, `None` linear.
#[derive(Clone, Debug)]
pub(super) struct Step {
    pub(super) param: String,
    pub(super) start: f64,
    pub(super) stop: f64,
    pub(super) step: f64,
    pub(super) sweep: Option<super::analysis::Sweep>,
}

/// A PDK model file and corner, with a path per backend family.
#[derive(Clone, Debug)]
pub(super) struct ModelLibrary {
    pub(super) name: String,
    pub(super) path: String,
    pub(super) corner: Option<String>,
    pub(super) backend_paths: std::collections::BTreeMap<String, String>,
    pub(super) setup_includes: Vec<String>,
}

/// A `.func name(args) {body}`.
#[derive(Clone, Debug)]
pub(super) struct Func {
    pub(super) name: String,
    pub(super) args: Vec<String>,
    pub(super) body: String,
}

/// The feature bits a card can add.
pub(super) mod need {
    pub(in super::super) use crate::backend::feature::{LAPLACE, XSPICE};
}

impl Card {
    /// An element or instance card, needing nothing special.
    pub(super) fn element(name: &str, text: String) -> Card {
        Card {
            name: name.into(),
            text,
            source: None,
            needs: 0,
        }
    }

    /// A V or I card, `head value tail` (`tail` empty or starting with a
    /// blank).
    pub(super) fn source(name: &str, head: String, value: &Value, tail: String) -> Card {
        let text = format!("{head} {}{tail}", value.text());
        Card {
            name: name.into(),
            text,
            source: Some((head, tail)),
            needs: 0,
        }
    }

    /// Sets a source's DC value; `false` (and nothing changed) when the
    /// card is no source.
    pub(super) fn set_value(&mut self, v: f64) -> bool {
        let Some((head, tail)) = &self.source else {
            return false;
        };
        self.text = format!("{head} {}{tail}", num(v));
        true
    }
}

impl Value {
    /// The value as an ngspice card field.
    pub(super) fn text(&self) -> String {
        match self {
            Value::Num(v) => num(*v),
            Value::Expr(e) => format!("{{{e}}}"),
            Value::Raw(t) => t.clone(),
        }
    }
}

/// `v` in SI form (`10k`, `2.5n`), which the deck's lexicon
/// (`lexicon::NGSPICE_GIVEN`) reads back as exactly `v`.
pub(super) fn num(v: f64) -> String {
    crate::unit::format_spice_number(v)
}

/// `v` as plain decimal (`0.5`, `1000`), read back as [`num`]'s is.
pub(super) fn dec(v: f64) -> String {
    format!("{v}")
}

/// ` k=v k=v`: instance or model parameters, a flag (empty value) bare.
pub(super) fn params(ps: &[(String, String)]) -> String {
    let mut s = String::new();
    for (k, v) in ps {
        s.push(' ');
        s.push_str(k);
        if !v.is_empty() {
            s.push('=');
            s.push_str(v);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_reads_back_as_the_same_double() {
        use crate::syntax::{lexicon::NGSPICE_GIVEN, token::Tag, tokenize};
        for v in [1e3, 3.3, 0.1 + 0.2, 1e-9, 2.5e-6, -4.7e-12, 1e-4, 1e300_f64] {
            for s in [num(v.abs()), dec(v.abs())] {
                let deck = format!("t\nR1 a b {s}\n");
                let t = tokenize(&deck, &NGSPICE_GIVEN);
                let i = t.tags().iter().position(|&g| g == Tag::Number).unwrap();
                assert_eq!(t.value(i as u32), v.abs(), "{s}");
            }
        }
        assert_eq!(num(1e-4), "100u");
        assert_eq!(dec(0.5), "0.5");
    }

    #[test]
    fn a_source_value_is_set_again() {
        let mut c = Card::source("1", "V1 a 0".into(), &Value::Num(1.0), " AC 1".into());
        assert_eq!(c.text, "V1 a 0 1 AC 1");
        assert!(c.set_value(2e-3));
        assert_eq!(c.text, "V1 a 0 2m AC 1");
        let mut r = Card::element("1", "R1 a b 1k".into());
        assert!(!r.set_value(1.0));
        assert_eq!(r.text, "R1 a b 1k");
    }
}
