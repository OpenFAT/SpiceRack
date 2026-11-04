//! VACASK's lexicon.
//!
//! Cites (as the old VACASK grammar did; L = `lib/dfllexer.l` line, P =
//! `lib/dflparser.y` line): L159 title; L176-L187 `//` and `/* */`
//! comments; L913-L921 a newline inside brackets is no statement end, `\`
//! before the newline joins; L594-L693 numbers (`M` `x` `X` `meg` mega, `m`
//! milli, `mil`, letters after the scale, `1e3k` an error); L695 long
//! strings; L705 strings; L869 names (`'…'` parts); L783-L868 keywords at a
//! statement's head; P216-P230 and docs/expr-operators.md operators.
//!
//! Contents:
//! - `pub const VACASK: Lexicon`

use super::{
    Arithmetic, Backslash, Caret, Dollar, Join, Lexicon, Numbers, Ops, Quote, Quoting, Scale, Tail,
};
use crate::syntax::Dialect;
use crate::syntax::statement::Analysis;
use crate::syntax::token::Tag;

/// VACASK: case kept, `//` and `/* */` comments, statements across lines
/// inside brackets, `@if` directives, keywords at a statement's head.
pub const VACASK: Lexicon = Lexicon {
    dialect: Dialect::Vacask,
    title: true,
    fold_case: false,
    comment_lines: &[],
    comment_col1: &[],
    blank_led_comment: false,
    cont_skip: &[],
    comment_tails: &["//"],
    block_comment: true,
    long_strings: true,
    dollar: Dollar::Text,
    plus: false,
    backslash: Backslash::Tight,
    end: None,
    stop: None,
    join: Join::Brackets,
    quote: Quote::Name,
    string: Quoting {
        escapes: true,
        protects: true,
        across_breaks: false,
    },
    string_expr: false,
    name_escape: false,
    ident_start: b"$",
    ident_continue: b"$",
    directive: Some(b'@'),
    head_words: true,
    heads_after: &[],
    operators: &[
        ("**", Tag::StarStar),
        ("==", Tag::EqualEqual),
        ("!=", Tag::BangEqual),
        ("<=", Tag::LessEqual),
        (">=", Tag::GreaterEqual),
        ("<<", Tag::LessLess),
        (">>", Tag::GreaterGreater),
        ("&&", Tag::AmpAmp),
        ("||", Tag::PipePipe),
    ],
    expr_braces: false,
    caret: Caret::Xor,
    ops: Ops {
        // P216-P230, docs/expr-operators.md.
        infix: &[
            (Tag::PipePipe, 2),
            (Tag::AmpAmp, 3),
            (Tag::Pipe, 4),
            (Tag::Caret, 5),
            (Tag::Amp, 6),
            (Tag::EqualEqual, 7),
            (Tag::BangEqual, 7),
            (Tag::LessEqual, 8),
            (Tag::GreaterEqual, 8),
            (Tag::Less, 8),
            (Tag::Greater, 8),
            (Tag::LessLess, 9),
            (Tag::GreaterGreater, 9),
            (Tag::Plus, 10),
            (Tag::Minus, 10),
            (Tag::Star, 11),
            (Tag::Slash, 11),
            (Tag::StarStar, 12),
        ],
        right: &[Tag::StarStar],
        prefix: &[Tag::Plus, Tag::Minus, Tag::Bang, Tag::Tilde],
        prefix_bp: 13,
        probes: &[],
        index: true,
    },
    lib_sections: true,
    numbers: Numbers {
        exponent: b"eE",
        scales: SCALES,
        tail: Tail::Letters {
            after_exponent: false,
            hex: true,
        },
        arithmetic: Arithmetic::Decimal,
    },
    keywords: &[KEYWORDS],
    named: true,
    // An instance's master is a model or a subcircuit, never a device word;
    // the modules a `model` names are `device::VACASK`, which resolve reads.
    devices: &[],
    masters: &[],
    analyses: ANALYSES,
    lang_spice: None,
};

/// L594-L693, in the old VACASK grammar's order.
const SCALES: &[(&str, Scale)] = &[
    ("meg", Scale::Exp(6)),
    ("M", Scale::Exp(6)),
    ("mil", Scale::Mil),
    ("x", Scale::Exp(6)),
    ("X", Scale::Exp(6)),
    ("m", Scale::Exp(-3)),
    ("u", Scale::Exp(-6)),
    ("n", Scale::Exp(-9)),
    ("p", Scale::Exp(-12)),
    ("f", Scale::Exp(-15)),
    ("a", Scale::Exp(-18)),
    ("k", Scale::Exp(3)),
    ("K", Scale::Exp(3)),
    ("G", Scale::Exp(9)),
    ("T", Scale::Exp(12)),
];

/// L783-L868 and P462-P509, P1267-P1296.
const KEYWORDS: &[(&str, Tag)] = &[
    ("include", Tag::KwInclude),
    ("load", Tag::KwLoad),
    ("embed", Tag::KwEmbed),
    ("global", Tag::KwGlobal),
    ("ground", Tag::KwGround),
    ("parameters", Tag::KwParam),
    ("model", Tag::KwModel),
    ("subckt", Tag::KwSubckt),
    ("ends", Tag::KwEnds),
    ("section", Tag::KwSection),
    ("endsection", Tag::KwEndsection),
    ("control", Tag::KwControl),
    ("endc", Tag::KwEndc),
    ("analysis", Tag::KwAnalysis),
    ("options", Tag::KwOptions),
    ("sweep", Tag::KwSweep),
    ("save", Tag::KwSave),
    ("@if", Tag::KwIf),
    ("@elseif", Tag::KwElseif),
    ("@else", Tag::KwElse),
    ("@end", Tag::KwEndif),
];

/// The kinds after `analysis name` (`statement.rs` cites the decks).
const ANALYSES: &[(&str, Analysis)] = &[
    ("op", Analysis::Op),
    ("dcinc", Analysis::Dcinc),
    ("dcxf", Analysis::Dcxf),
    ("ac", Analysis::Ac),
    ("acxf", Analysis::Acxf),
    ("noise", Analysis::Noise),
    ("tran", Analysis::Tran),
    ("hb", Analysis::Hb),
];

#[cfg(test)]
mod tests {
    use crate::syntax::lexicon::VACASK;
    use crate::syntax::token::Tag;
    use crate::syntax::tokenize;
    use crate::syntax::{test_numbers as numbers, test_words as words};

    #[test]
    fn brackets_hold_newlines_and_comments_go_anywhere() {
        // L913-L919, L176-L187.
        let w = words("t\nr1 (a /* x\n y */ b) r // c\nr2 (a\n b) r\n", &VACASK);
        assert_eq!(w, ["r1 ( a b ) r", "r2 ( a b ) r"]);
    }

    #[test]
    fn quoted_name_parts_and_long_strings() {
        // L869, L695.
        let src = "t\nsave p('x1:m1', id)\nembed \"f\" <<<E\nline // kept\n>>>E\n";
        let t = tokenize(src, &VACASK);
        // Title, end, `save`, `p`, `(`, then the name.
        assert_eq!(t.text(5), "'x1:m1'");
        assert_eq!(t.tags()[5], Tag::Ident);
        assert_eq!(t.tags()[12], Tag::String);
        assert_eq!(t.text(12), "<<<E\nline // kept\n>>>E");
    }

    #[test]
    fn directives_and_head_keywords() {
        let t = tokenize("t\n@if a\nmodel m r\n@end\n", &VACASK);
        assert_eq!(t.tags()[2], Tag::KwIf);
        assert_eq!(t.tags()[5], Tag::KwModel);
        assert_eq!(t.tags()[9], Tag::KwEndif);
    }

    #[test]
    fn numbers_and_their_errors() {
        // L594-L693: `M` mega, `1kOhm`; `1e3k` and hex are no decimal.
        let v = numbers("t\np a=1M b=1x c=1kOhm d=1e3k e=0x1F\n", &VACASK);
        assert_eq!(&v[..3], [1e6, 1e6, 1e3]);
        assert!(v[3].is_nan() && v[4].is_nan());
    }
}
