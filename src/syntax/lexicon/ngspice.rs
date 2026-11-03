//! ngspice's lexicon (ESPice reads the same text).
//!
//! Cites (the § are `dev/research/ngspice-syntax.md`'s, as
//! `grammar/ngspice.peg` cites them): §1.2 title; §1.3 `@` ends the file,
//! blank lines; §1.5 comment lines and `$` `;` `//` tails; §1.6 `+` and `\\`
//! continuation; §1.7 `.end`; §1.9 quotes; §1.10 numbers (`INPevaluate`,
//! inpeval.c); §2.2 operators; ngspice manual §2.1.3 (scale factors). ESPice
//! `src/frontend/lines_ngspice.zig`.
//!
//! Contents:
//! - `pub const NGSPICE: Lexicon`

use super::{
    Arithmetic, Backslash, Caret, Dollar, End, Lexicon, Numbers, Quote, Quoting, SPICE_KEYWORDS,
    SPICE_LETTERS, SPICE_OPERATORS, Scale, Tail,
};
use crate::codegen::Dialect;
use crate::syntax::token::Tag;

/// ngspice: case-blind, `*`/`$`/`;`/`//` comments, `+` and `\\` continuation,
/// `'…'` and `{…}` expressions, `M` milli.
pub const NGSPICE: Lexicon = Lexicon {
    dialect: Dialect::Ngspice,
    title: true,
    fold_case: true,
    // §1.5: `*`, `#`, `$`, `;` are comments; the punctuation is turned into
    // one with a warning; so is a line of only a `//` comment.
    comment_lines: &[
        "*", "#", "$", ";", "=", "[", "]", "?", "(", ")", "&", "%", "\"", "!", ":", ",", "\x0c",
        "//",
    ],
    comment_col1: &[],
    blank_led_comment: false,
    // §1.6: a `+` continues past `*`, `#`, `$` comment lines and blank lines.
    cont_skip: &["*", "#", "$"],
    comment_tails: &[";", "//"],
    block_comment: false,
    long_strings: false,
    dollar: Dollar::AfterBlank,
    plus: true,
    backslash: Backslash::Glue,
    // §1.7: `.end` alone or before a blank is dropped, and reading goes on.
    end: Some(End {
        col1: false,
        then_blank: true,
        stops: false,
    }),
    stop: Some(b'@'),
    brackets_join: false,
    quote: Quote::Expr(QUOTED),
    string: QUOTED,
    name_escape: false,
    ident_start: b"$",
    ident_continue: b"$.",
    directive: Some(b'.'),
    head_words: false,
    heads_after: &[],
    operators: SPICE_OPERATORS,
    expr_braces: true,
    caret: Caret::Pow,
    numbers: Numbers {
        exponent: b"eEdD",
        scales: SCALES,
        tail: Tail::Spice { dot: true },
        arithmetic: Arithmetic::Ngspice,
    },
    keywords: &[SPICE_KEYWORDS, OWN_KEYWORDS],
    letters: SPICE_LETTERS,
    lang_spice: None,
};

/// §1.9: quotes protect comment bytes and take backslash escapes; an
/// unclosed one runs to the line's end or a `\\` break.
const QUOTED: Quoting = Quoting {
    escapes: true,
    protects: true,
    across_breaks: false,
};

/// §1.10: T G MEG K MIL M U N P F A, `meg` and `mil` before `m`.
const SCALES: &[(&str, Scale)] = &[
    ("meg", Scale::Exp(6)),
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

/// ngspice's own dot cards (`grammar/ngspice.peg`).
const OWN_KEYWORDS: &[(&str, Tag)] = &[
    (".control", Tag::KwControl),
    (".endc", Tag::KwEndc),
    (".sp", Tag::KwSp),
];

#[cfg(test)]
mod tests {
    use crate::syntax::lexicon::NGSPICE;
    use crate::syntax::token::Tag;
    use crate::syntax::tokenize;
    use crate::syntax::{test_numbers as numbers, test_words as words};

    #[test]
    fn comment_lines_and_tails() {
        // §1.5: `*` and the punctuation lines are comments; `$` after a
        // blank, `;` and `//` anywhere end the line; `a$b` is a name.
        let w = words(
            "t\n* c\n= c\nR1 a$b 0 1k $ c\nR2 a b 2k;c\nR3 a b 3k//c\n",
            &NGSPICE,
        );
        assert_eq!(w, ["R1 a$b 0 1k", "R2 a b 2k", "R3 a b 3k"]);
    }

    #[test]
    fn plus_reads_past_star_lines_but_not_semicolon_lines() {
        // §1.6 (`l_skip_cont`): `*`, `#`, `$` and blank lines; a `;` line
        // ends the statement, and the `+` after it continues nothing.
        let w = words("t\nR1 a\n* c\n\n+ b 1\nR2 a\n; c\n+ b 2\n", &NGSPICE);
        assert_eq!(w, ["R1 a b 1", "R2 a"]);
    }

    #[test]
    fn glue_continues_even_after_a_comment() {
        // §1.6: a trailing `\\` continues the statement with no `+`, past
        // an end-of-line comment.
        let w = words("t\nR1 a b \\\\\n 1k\nR2 a $ c \\\\\nb 2k\n", &NGSPICE);
        assert_eq!(w, ["R1 a b 1k", "R2 a b 2k"]);
    }

    #[test]
    fn end_is_dropped_and_reading_goes_on_and_at_stops() {
        // §1.7, §1.3.
        let t = tokenize("t\n.end\nR1 a b 1\n@\nR2 a b 2\n", &NGSPICE);
        let w = words("t\n.end\nR1 a b 1\n@\nR2 a b 2\n", &NGSPICE);
        assert_eq!(w, ["R1 a b 1"]);
        assert_eq!(t.tags()[2], Tag::KwEnd);
    }

    #[test]
    fn quotes_protect_comments() {
        // §1.9: `$` and `;` inside `'…'` and `"…"` are text.
        let w = words("t\n.param a='1 $2' b=\"x;y\"\n", &NGSPICE);
        assert_eq!(w, [".param a = ' 1 $2 ' b = \"x;y\""]);
    }

    #[test]
    fn numbers_read_as_inpevaluate() {
        // §1.10: `M` is milli, `meg` mega, the tail is ignored (`1k5`),
        // `d` is an exponent letter.
        let v = numbers("t\nR1 a b 1M 1meg 1k5 10v 1d3 0.1n\n", &NGSPICE);
        assert_eq!(v, [1e-3, 1e6, 1e3, 10.0, 1e3, 1e-10]);
        let mil = numbers("t\nR1 a b 1mil\n", &NGSPICE)[0];
        assert!((mil - 25.4e-6).abs() < 1e-20);
    }

    #[test]
    fn keywords_fold_and_letters_map() {
        let t = tokenize("t\n.PARAM a=1\n.Control\n", &NGSPICE);
        assert_eq!(t.tags()[2], Tag::KwParam);
        assert_eq!(t.tags()[7], Tag::KwControl);
        assert_eq!(
            NGSPICE.element(b'X'),
            Some(crate::syntax::lexicon::Element::Instance)
        );
    }
}
