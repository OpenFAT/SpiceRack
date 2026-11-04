//! HSPICE's lexicon, also read inside a Spectre `simulator lang=spice`
//! region.
//!
//! Cites (the rule ids the old HSPICE grammar labels its rules with,
//! `git show daa0ad0:grammar/hspice.peg`; UG08 = HSPICE User Guide 2008, CR10 =
//! Command Reference 2010): L1 title; L2 comment lines; L3 the `$` rule; L5
//! `+` continuation; L6 `\` and `\\` breaks; L9 `.END`; T1 separators; T2
//! quotes (`$` is text inside); T3 numbers (`X` is mega, `D` an exponent);
//! X2 operators (UG08 pp.279-283). ESPice `src/frontend/lines_hspice.zig`
//! (`stripComment`, `isNumberToken`: the `$` hook).
//!
//! Contents:
//! - `pub const HSPICE: Lexicon`

use super::{
    Arithmetic, Backslash, Caret, Dollar, End, Join, Lexicon, Numbers, Ops, Quote, Quoting,
    SPICE_KEYWORDS, Scale, Tail,
};
use crate::codegen::Dialect;
use crate::syntax::device;
use crate::syntax::token::Tag;

/// HSPICE: case-blind, `*`/`$` comment lines, the `$` hook, `+`, `\` and
/// `\\` continuation, `'…'` expressions, braces as name bytes (#203).
pub const HSPICE: Lexicon = Lexicon {
    dialect: Dialect::Hspice,
    title: true,
    fold_case: true,
    comment_lines: &["*", "$"],
    comment_col1: &[],
    blank_led_comment: false,
    cont_skip: &["*", "$"],
    comment_tails: &[],
    block_comment: false,
    long_strings: false,
    dollar: Dollar::Hspice,
    plus: true,
    backslash: Backslash::Hspice,
    // L9: `.END` ends the run; what follows is a further run, not read.
    end: Some(End {
        col1: false,
        then_blank: false,
        stops: true,
    }),
    stop: None,
    join: Join::None,
    quote: Quote::Expr(QUOTED),
    string: QUOTED,
    string_expr: true,
    name_escape: false,
    ident_start: b"$",
    ident_continue: b"$.",
    directive: Some(b'.'),
    head_words: false,
    heads_after: &[],
    operators: OPERATORS,
    expr_braces: false,
    caret: Caret::Pow,
    ops: Ops {
        // X2 (UG08 pp.279-283, Table 19): no `%`, `\\` or unary `!`.
        infix: &[
            (Tag::PipePipe, 2),
            (Tag::AmpAmp, 3),
            (Tag::EqualEqual, 4),
            (Tag::BangEqual, 4),
            (Tag::LessEqual, 4),
            (Tag::GreaterEqual, 4),
            (Tag::Less, 4),
            (Tag::Greater, 4),
            (Tag::Plus, 5),
            (Tag::Minus, 5),
            (Tag::Star, 6),
            (Tag::Slash, 6),
            (Tag::Caret, 7),
            (Tag::StarStar, 7),
        ],
        right: &[],
        prefix: &[Tag::Plus, Tag::Minus],
        prefix_bp: 6,
        probes: &["v", "i"],
        index: false,
    },
    lib_sections: true,
    numbers: Numbers {
        exponent: b"eEdD",
        scales: SCALES,
        tail: Tail::Spice { dot: true },
        arithmetic: Arithmetic::Decimal,
    },
    keywords: &[SPICE_KEYWORDS, OWN_KEYWORDS],
    named: false,
    devices: &[device::SPICE],
    masters: &[],
    analyses: &[],
    lang_spice: None,
};

/// T2, L6: `'…'` and `"…"` protect `$`, have no escapes, and continue across
/// a `\` or `\\` break.
const QUOTED: Quoting = Quoting {
    escapes: false,
    protects: true,
    across_breaks: true,
};

/// T3: T G MEG X K MIL M U N P F A.
const SCALES: &[(&str, Scale)] = &[
    ("meg", Scale::Exp(6)),
    ("x", Scale::Exp(6)),
    ("mil", Scale::Mil),
    ("t", Scale::Exp(12)),
    ("g", Scale::Exp(9)),
    ("k", Scale::Exp(3)),
    ("m", Scale::Exp(-3)),
    ("u", Scale::Exp(-6)),
    ("n", Scale::Exp(-9)),
    ("p", Scale::Exp(-12)),
    ("f", Scale::Exp(-15)),
    ("a", Scale::Exp(-18)),
];

/// X2: no `<>`, `%` or `\` (UG08 Table 19).
const OPERATORS: &[(&str, Tag)] = &[
    ("**", Tag::StarStar),
    ("==", Tag::EqualEqual),
    ("!=", Tag::BangEqual),
    ("<=", Tag::LessEqual),
    (">=", Tag::GreaterEqual),
    ("&&", Tag::AmpAmp),
    ("||", Tag::PipePipe),
];

/// `.probe` selects outputs, as `.save` does elsewhere.
const OWN_KEYWORDS: &[(&str, Tag)] = &[(".probe", Tag::KwProbe)];

#[cfg(test)]
mod tests {
    use crate::syntax::lexicon::HSPICE;
    use crate::syntax::{test_numbers as numbers, test_words as words};

    #[test]
    fn dollar_is_a_comment_after_a_separator_or_a_number() {
        // L3: `$` after a blank, `=` or a number (`1k$c` is `1k`, `-1w$c` is
        // `-1w`); inside a name it is a name byte (`a$b`).
        let w = words(
            "t\nR1 a$b 0 1k$c\nR2 a b r=$c\nV1 a b -1w$c\nR3 a b r1k_x$c\n",
            &HSPICE,
        );
        assert_eq!(
            w,
            ["R1 a$b 0 1k", "R2 a b r =", "V1 a b - 1w", "R3 a b r1k_x$c"]
        );
    }

    #[test]
    fn dollar_is_text_inside_quotes() {
        // T2.
        let w = words("t\n.param a='1 $2' $ c\n", &HSPICE);
        assert_eq!(w, [".param a = ' 1 $2 '"]);
    }

    #[test]
    fn backslash_breaks_glue_or_keep_the_blank() {
        // L6: `\\` joins with nothing between, a blank and `\` keeps it;
        // neither needs a `+` on the next line.
        let src = "t\nR1 ab\\\\\ncd 1\nR2 a \\\nb 2\n";
        let t = crate::syntax::tokenize(src, &HSPICE);
        assert_eq!(words(src, &HSPICE), ["R1 ab cd 1", "R2 a b 2"]);
        // `ab` and `cd` are two tokens with only the glue between them.
        assert_eq!(&src[t.end(3) as usize..t.start(4) as usize], "\\\\\n");
    }

    #[test]
    fn end_stops_the_run() {
        // L9.
        assert_eq!(
            words("t\nR1 a b 1\n.END\nR2 a b 2\n", &HSPICE),
            ["R1 a b 1"]
        );
    }

    #[test]
    fn x_is_mega_and_d_an_exponent() {
        // T3.
        assert_eq!(
            numbers("t\nR1 a b 1x 2.5D-3 1M 10pf\n", &HSPICE),
            [1e6, 2.5e-3, 1e-3, 1e-11]
        );
    }
}
