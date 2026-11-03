//! LTspice's lexicon.
//!
//! Cites (as the old LTspice grammar did): ADI ltspice-reference `ai_ref/`
//! SPICE-SYNTAX-REFERENCE.md (SSR) "Netlist Structure" (title), "Comments"
//! (`*` lines, `;` anywhere), "Engineering Multipliers" (T G Meg K mil m u
//! n p f, `µ`, `6K34`); SIMULATION-COMMANDS-REFERENCE.md `.END`;
//! CIRCUIT-ELEMENTS-REFERENCE.md "Supported Operators" (`& | ^` Boolean,
//! `**`); LTspice XVII help DotParam.htm (`{expr}`).
//!
//! Contents:
//! - `pub const LTSPICE: Lexicon`

use super::{
    Arithmetic, Backslash, Caret, Dollar, End, Join, Lexicon, Numbers, Ops, Quote, Quoting,
    SPICE_KEYWORDS, Scale, Tail,
};
use crate::codegen::Dialect;
use crate::syntax::device;
use crate::syntax::token::Tag;

/// LTspice: case-blind, `*`/`;` comments, `+` continuation, `{…}`
/// expressions, `"…"` strings, no `$` or `//` comments.
pub const LTSPICE: Lexicon = Lexicon {
    dialect: Dialect::Ltspice,
    title: true,
    fold_case: true,
    comment_lines: &["*", ";"],
    comment_col1: &[],
    blank_led_comment: false,
    cont_skip: &["*", ";"],
    comment_tails: &[";"],
    block_comment: false,
    long_strings: false,
    dollar: Dollar::Text,
    plus: true,
    backslash: Backslash::None,
    end: Some(End {
        col1: false,
        then_blank: false,
        stops: true,
    }),
    stop: None,
    join: Join::None,
    quote: Quote::Plain,
    // A `;` inside double quotes (a path) is no comment (unverified).
    string: Quoting {
        escapes: false,
        protects: true,
        across_breaks: false,
    },
    string_expr: false,
    name_escape: false,
    ident_start: b"$",
    ident_continue: b"$.",
    directive: Some(b'.'),
    head_words: false,
    heads_after: &[],
    operators: OPERATORS,
    expr_braces: true,
    caret: Caret::Xor,
    ops: Ops {
        // CER "Supported Operators (ascending precedence)": `& | ^`,
        // comparisons, `+ -`, `* / %`, `**`, `! ~`.
        infix: &[
            (Tag::Amp, 2),
            (Tag::Pipe, 2),
            (Tag::Caret, 2),
            (Tag::EqualEqual, 3),
            (Tag::BangEqual, 3),
            (Tag::LessEqual, 3),
            (Tag::GreaterEqual, 3),
            (Tag::Less, 3),
            (Tag::Greater, 3),
            (Tag::Plus, 4),
            (Tag::Minus, 4),
            (Tag::Star, 5),
            (Tag::Slash, 5),
            (Tag::Percent, 5),
            (Tag::StarStar, 6),
        ],
        right: &[],
        prefix: &[Tag::Plus, Tag::Minus, Tag::Bang, Tag::Tilde],
        prefix_bp: 7,
        probes: &["v", "i"],
        index: false,
    },
    lib_sections: false,
    numbers: Numbers {
        exponent: b"eE",
        scales: SCALES,
        tail: Tail::Ltspice,
        arithmetic: Arithmetic::Decimal,
    },
    // `.fourier` is LTspice's own card, not `.four`.
    keywords: &[OWN_KEYWORDS, SPICE_KEYWORDS],
    named: false,
    devices: &[device::SPICE],
    masters: &[],
    analyses: &[],
    lang_spice: None,
};

/// SSR "Engineering Multipliers": no atto.
const SCALES: &[(&str, Scale)] = &[
    ("meg", Scale::Exp(6)),
    ("mil", Scale::Mil),
    ("µ", Scale::Exp(-6)),
    ("t", Scale::Exp(12)),
    ("g", Scale::Exp(9)),
    ("k", Scale::Exp(3)),
    ("m", Scale::Exp(-3)),
    ("u", Scale::Exp(-6)),
    ("n", Scale::Exp(-9)),
    ("p", Scale::Exp(-12)),
    ("f", Scale::Exp(-15)),
];

const OPERATORS: &[(&str, Tag)] = &[
    ("**", Tag::StarStar),
    ("==", Tag::EqualEqual),
    ("!=", Tag::BangEqual),
    ("<=", Tag::LessEqual),
    (">=", Tag::GreaterEqual),
];

const OWN_KEYWORDS: &[(&str, Tag)] = &[
    (".params", Tag::KwParam),
    (".step", Tag::KwStep),
    (".fourier", Tag::Directive),
];

#[cfg(test)]
mod tests {
    use crate::syntax::lexicon::LTSPICE;
    use crate::syntax::{test_numbers as numbers, test_words as words};

    #[test]
    fn semicolon_comments_and_dollar_is_text() {
        // SSR "Comments": `*` lines and `;` anywhere; `$` and `//` are not.
        let w = words(
            "t\n* c\nR1 a$b 0 1k ; c\nR2 a b 2k $ x // y\n;c\n",
            &LTSPICE,
        );
        assert_eq!(w, ["R1 a$b 0 1k", "R2 a b 2k $ x / / y"]);
    }

    #[test]
    fn plus_reads_past_semicolon_lines() {
        let w = words("t\nR1 a\n; c\n+ b 1\n", &LTSPICE);
        assert_eq!(w, ["R1 a b 1"]);
    }

    #[test]
    fn multipliers_mu_and_fractions() {
        // SSR "Engineering Multipliers": `6K34` is 6.34k, `µ` micro, no atto.
        assert_eq!(
            numbers("t\nR1 a b 6K34 1µ 1Meg 2a\n", &LTSPICE),
            [6340.0, 1e-6, 1e6, 2.0]
        );
    }
}
