//! How each target spells what every target writes: one [`Spelling`] row
//! per dialect, read by both structural writers. A row holds the target's
//! lexicon (its element letters and its number suffixes, read here in
//! reverse: primitive → letter, exponent → suffix), its comment leader, the
//! quotes around a path and an expression, the form of a function, of a
//! subcircuit's parameter list, and whether it reads numbers with ngspice's
//! arithmetic. What a target cannot write at all is
//! [`super::capability`]'s, never this table's.
//!
//! Cites: `docs/research/transpiler-architecture.md` §2.5 (`Spelling`);
//! ngspice manual §2.1.3 (scale factors), §2.8.5 (`{…}`), §2.4.3
//! (`PARAMS:`); HSPICE UG08 pp.279-283 (`'…'`, `.param f(x)=`); the
//! lexicons' own citations for each suffix table.
//!
//! Contents:
//! - `pub struct Spelling`, `pub enum FuncForm`, `pub const fn spelling(target: Dialect) -> &'static Spelling`
//! - `Spelling::letter(&self, p: Primitive) -> char`, `Spelling::number(&self, v: f64, exact: bool) -> String`,
//!   `Spelling::decimal(&self, v: f64, exact: bool) -> String`, `Spelling::format(&self, v: f64) -> String`

use crate::codegen::Dialect;
use crate::syntax::device::{self, Primitive};
use crate::syntax::lexicon::{self, Lexicon, Scale};

/// One target's spelling.
pub struct Spelling {
    pub target: Dialect,
    /// The lexicon the target's reader reads: its element letters and its
    /// number suffixes are the writer's too.
    pub lexicon: &'static Lexicon,
    /// What starts a comment line (the title, a note).
    pub comment: &'static str,
    /// The quote around a file path (HSPICE `'file'`).
    pub path_quote: &'static str,
    /// The delimiters around an expression value.
    pub expr: (&'static str, &'static str),
    /// How a function is defined.
    pub func: FuncForm,
    /// The word before a subcircuit's parameter defaults (`PARAMS:`); `None`
    /// where they follow the ports bare (HSPICE).
    pub subckt_params: Option<&'static str>,
    /// The target reads numbers with ngspice's arithmetic (`INPevaluate`),
    /// so a number read the same way is spelled to land on the same double.
    pub ngspice_numbers: bool,
}

/// How a target defines a function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FuncForm {
    /// `.func f(a, b) {body}`.
    Func,
    /// `.param f(a, b)='body'` (HSPICE).
    Param,
}

/// The writer's scale ladder, largest first: (multiplier, exponent). The
/// multipliers are literals, as `unit::scale` needs them exact.
const LADDER: [(f64, i32); 10] = [
    (1e12, 12),
    (1e9, 9),
    (1e6, 6),
    (1e3, 3),
    (1.0, 0),
    (1e-3, -3),
    (1e-6, -6),
    (1e-9, -9),
    (1e-12, -12),
    (1e-15, -15),
];

const NGSPICE: Spelling = Spelling {
    target: Dialect::Ngspice,
    lexicon: &lexicon::NGSPICE,
    comment: "*",
    path_quote: "",
    expr: ("{", "}"),
    func: FuncForm::Func,
    subckt_params: Some("PARAMS:"),
    ngspice_numbers: true,
};

const ESPICE: Spelling = Spelling {
    target: Dialect::Espice,
    ..NGSPICE
};

const HSPICE: Spelling = Spelling {
    target: Dialect::Hspice,
    lexicon: &lexicon::HSPICE,
    path_quote: "'",
    expr: ("'", "'"),
    func: FuncForm::Param,
    subckt_params: None,
    ngspice_numbers: false,
    ..NGSPICE
};

const XYCE: Spelling = Spelling {
    target: Dialect::Xyce,
    lexicon: &lexicon::XYCE,
    ngspice_numbers: false,
    ..NGSPICE
};

/// CDL (the xschem layout): its elements are ngspice cards, its numbers
/// spelled as a deck read with no ngspice arithmetic.
const CDL: Spelling = Spelling {
    target: Dialect::Cdl,
    ..NGSPICE
};

const LTSPICE: Spelling = Spelling {
    target: Dialect::Ltspice,
    lexicon: &lexicon::LTSPICE,
    ngspice_numbers: false,
    ..NGSPICE
};

/// Target `target`'s row.
///
/// # Panics
/// For Spectre, VACASK and Verilog, which have no row yet.
pub const fn spelling(target: Dialect) -> &'static Spelling {
    match target {
        Dialect::Ngspice => &NGSPICE,
        Dialect::Espice => &ESPICE,
        Dialect::Hspice => &HSPICE,
        Dialect::Xyce => &XYCE,
        Dialect::Ltspice => &LTSPICE,
        Dialect::Cdl => &CDL,
        Dialect::Spectre | Dialect::Vacask | Dialect::Verilog => {
            panic!("spelling: no row yet for this target")
        }
    }
}

impl Spelling {
    /// The element letter of `p`, uppercase: the device table read
    /// primitive → letter, the direction the lexicon reads it the other way.
    ///
    /// # Panics
    /// When the target has no letter for `p`.
    pub fn letter(&self, p: Primitive) -> char {
        let d = device::spelling(self.lexicon.devices, p).expect("a letter for the primitive");
        d.word.as_bytes()[0].to_ascii_uppercase() as char
    }

    /// `v` as the target reads it back: with `exact` (the number was read
    /// with the target's own arithmetic), the fewest digits ngspice reads as
    /// exactly `v`; else [`Spelling::format`].
    pub fn number(&self, v: f64, exact: bool) -> String {
        if exact && self.ngspice_numbers {
            crate::unit::ngspice_spelling(v)
        } else {
            self.format(v)
        }
    }

    /// `v` as plain decimal, unless an `exact` number would read back as
    /// another double (then [`Spelling::number`]'s exact spelling).
    pub fn decimal(&self, v: f64, exact: bool) -> String {
        let s = format!("{v}");
        if !(exact && self.ngspice_numbers) || crate::unit::ngspice_value(&s) == Some(v) {
            s
        } else {
            crate::unit::ngspice_spelling(v)
        }
    }

    /// `v` with the scale suffix that keeps its mantissa in [1, 1000)
    /// (clamped to tera..femto), when that spelling reads back as exactly
    /// `v` (`10k`, `2.5n`), else the shortest exact exponent form
    /// (`2.0000000000000002e-7`): rounding the mantissa would hand the
    /// simulator another number.
    pub fn format(&self, v: f64) -> String {
        let (mult, exp) = scale_of(v);
        let suffix = self.suffix(exp);
        let m = crate::unit::unscale(v, mult);
        let rounded = m.round();
        let whole = (rounded.abs() < 1e15).then(|| format!("{}", rounded as i64));
        for mantissa in whole.into_iter().chain([format!("{m}")]) {
            if format!("{mantissa}e{exp}").parse::<f64>() == Ok(v) {
                return mantissa + suffix;
            }
        }
        format!("{v:e}")
    }
}

// ---- private ----

/// The ladder rung keeping `|v| / multiplier` in [1, 1000), clamped to the
/// ends; `(1.0, 0)` for zero.
fn scale_of(v: f64) -> (f64, i32) {
    if v == 0.0 {
        return (1.0, 0);
    }
    LADDER
        .into_iter()
        .find(|p| v.abs() >= p.0)
        .unwrap_or(LADDER[9])
}

impl Spelling {
    /// The target lexicon's suffix for `10^exp`: its first ASCII spelling
    /// (`meg` before HSPICE's `x`, `u` past LTspice's `µ`), lowercase.
    fn suffix(&self, exp: i32) -> &'static str {
        if exp == 0 {
            return "";
        }
        let scales = self.lexicon.numbers.scales;
        let hit = scales
            .iter()
            .find(|(s, k)| *k == Scale::Exp(exp) && s.is_ascii());
        hit.map(|(s, _)| *s).expect("a suffix for every rung")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPICE: [Dialect; 5] = [
        Dialect::Ngspice,
        Dialect::Espice,
        Dialect::Hspice,
        Dialect::Xyce,
        Dialect::Ltspice,
    ];

    #[test]
    fn letters_invert_the_lexicon_device_table() {
        // Writing a primitive's letter and reading it back gives the
        // primitive: the reader's table, read both ways.
        for d in SPICE {
            let s = spelling(d);
            for p in Primitive::ALL {
                let Some(row) = device::spelling(s.lexicon.devices, p) else {
                    continue;
                };
                let letter = s.letter(p).to_ascii_lowercase().to_string();
                assert_eq!(row.word, letter);
                let back = device::find(s.lexicon.devices, &letter, true).map(|r| r.primitive);
                assert_eq!(back, Some(p), "{d:?} {p:?}");
            }
        }
    }

    #[test]
    fn numbers_read_back_as_the_same_double() {
        // The oracle is the target's own number grammar: what is written
        // reads back, through the lexicon's suffix table, as `v`.
        let values = [
            0.0,
            1.0,
            -1.0,
            10e3,
            2.5e-9,
            1e-15,
            3.3,
            2.0000000000000002e-7,
            1e15,
            4.7e-18,
            123456.789,
            0.1,
            1e6,
            -2.2e-12,
        ];
        for d in SPICE {
            let s = spelling(d);
            for v in values {
                // An ngspice reader's arithmetic is not decimal: its exact
                // spelling is the one that reads back.
                let text = s.number(v, true);
                let (sign, body) = text
                    .strip_prefix('-')
                    .map_or((1.0, &text[..]), |b| (-1.0, b));
                let got = sign * crate::syntax::number::value(body, s.lexicon);
                assert_eq!(got, v, "{d:?} wrote {v} as {text}");
                assert_eq!(s.format(v), crate::codegen::format_spice_number(v), "{d:?}");
            }
        }
    }

    #[test]
    fn exact_numbers_are_ngspice_spellings_only_for_ngspice_readers() {
        let v = 5e-7;
        assert_eq!(
            spelling(Dialect::Ngspice).number(v, true),
            crate::unit::ngspice_spelling(v)
        );
        assert_eq!(spelling(Dialect::Hspice).number(v, true), "500n");
        assert_eq!(spelling(Dialect::Ngspice).decimal(0.5, true), "0.5");
    }
}
