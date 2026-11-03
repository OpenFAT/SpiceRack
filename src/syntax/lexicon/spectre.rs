//! Spectre's lexicon, with the `simulator lang=spice` hook.
//!
//! Cites (as `grammar/spectre.peg` does; § = `dev/research/spectre-syntax.md`,
//! UG = Spectre User Guide 5.1.41, R19 = Spectre Reference 19.1): §1.2 title
//! (UG p.29, p.50); §1.3 `*` in column 1 and `//` comments (UG p.59); §1.4
//! `\` and `+` continuation (UG p.59); §2.1 case kept; §2.2 names (`\`
//! escapes, `!`); §2.4 numbers (case-sensitive scales, `M` mega, `m` milli,
//! units after the scale); §2.5 strings; §3.1 the `simulator lang=` switch
//! (UG p.53); §6.1 operators (UG p.87).
//!
//! Contents:
//! - `pub const SPECTRE: Lexicon`

use super::{
    Arithmetic, Backslash, Caret, Dollar, HSPICE, Lexicon, Numbers, Ops, Quote, Quoting, Scale,
    Tail,
};
use crate::codegen::Dialect;
use crate::syntax::device;
use crate::syntax::statement::{Analysis, Command, Statement};
use crate::syntax::token::Tag;

/// Spectre: case kept, `*` (column 1) and `//` comments, `\` and `+`
/// continuation, keywords at a statement's head, `M` mega; a
/// `simulator lang=spice` region reads as HSPICE.
pub const SPECTRE: Lexicon = Lexicon {
    dialect: Dialect::Spectre,
    title: true,
    fold_case: false,
    comment_lines: &["//"],
    comment_col1: b"*",
    blank_led_comment: false,
    cont_skip: &["//"],
    comment_tails: &["//"],
    block_comment: false,
    long_strings: false,
    dollar: Dollar::Text,
    plus: true,
    backslash: Backslash::Join,
    end: None,
    stop: None,
    brackets_join: false,
    quote: Quote::Plain,
    string: Quoting {
        escapes: true,
        protects: true,
        across_breaks: false,
    },
    string_expr: false,
    name_escape: true,
    ident_start: b"$",
    ident_continue: b"!$",
    directive: None,
    head_words: true,
    heads_after: &[Tag::LBrace, Tag::RBrace, Tag::KwInline, Tag::KwElse],
    operators: &[
        ("**", Tag::StarStar),
        ("==", Tag::EqualEqual),
        ("!=", Tag::BangEqual),
        ("<=", Tag::LessEqual),
        (">=", Tag::GreaterEqual),
        ("&&", Tag::AmpAmp),
        ("||", Tag::PipePipe),
    ],
    expr_braces: false,
    caret: Caret::None,
    ops: Ops {
        // §6.1 (UG p.87), decreasing: unary, `**` (right), `* / %`, `+ -`,
        // relational, equality, `&&`, `||`, `?:`.
        infix: &[
            (Tag::PipePipe, 2),
            (Tag::AmpAmp, 3),
            (Tag::EqualEqual, 4),
            (Tag::BangEqual, 4),
            (Tag::LessEqual, 5),
            (Tag::GreaterEqual, 5),
            (Tag::Less, 5),
            (Tag::Greater, 5),
            (Tag::Plus, 6),
            (Tag::Minus, 6),
            (Tag::Star, 7),
            (Tag::Slash, 7),
            (Tag::Percent, 7),
            (Tag::StarStar, 8),
        ],
        right: &[Tag::StarStar],
        prefix: &[Tag::Plus, Tag::Minus, Tag::Bang],
        prefix_bp: 9,
        probes: &[],
        index: false,
    },
    lib_sections: true,
    numbers: Numbers {
        exponent: b"eE",
        scales: SCALES,
        tail: Tail::Letters {
            after_exponent: true,
            hex: false,
        },
        arithmetic: Arithmetic::Decimal,
    },
    keywords: &[KEYWORDS],
    named: true,
    devices: &[device::SPECTRE],
    masters: MASTERS,
    analyses: &[],
    lang_spice: Some(&HSPICE),
};

/// §2.4: P T G M K/k `_` %/c m u n p f a.
const SCALES: &[(&str, Scale)] = &[
    ("P", Scale::Exp(15)),
    ("T", Scale::Exp(12)),
    ("G", Scale::Exp(9)),
    ("M", Scale::Exp(6)),
    ("K", Scale::Exp(3)),
    ("k", Scale::Exp(3)),
    ("_", Scale::Exp(0)),
    ("%", Scale::Exp(-2)),
    ("c", Scale::Exp(-2)),
    ("m", Scale::Exp(-3)),
    ("u", Scale::Exp(-6)),
    ("n", Scale::Exp(-9)),
    ("p", Scale::Exp(-12)),
    ("f", Scale::Exp(-15)),
    ("a", Scale::Exp(-18)),
];

/// The statement words of `grammar/spectre.peg` (R19 pp.175-179, p.491,
/// p.498, p.506; UG p.92, p.94).
const KEYWORDS: &[(&str, Tag)] = &[
    ("simulator", Tag::KwSimulator),
    ("include", Tag::KwInclude),
    ("ahdl_include", Tag::KwAhdlInclude),
    ("library", Tag::KwLibrary),
    ("endlibrary", Tag::KwEndlibrary),
    ("section", Tag::KwSection),
    ("endsection", Tag::KwEndsection),
    ("inline", Tag::KwInline),
    ("subckt", Tag::KwSubckt),
    ("ends", Tag::KwEnds),
    ("model", Tag::KwModel),
    ("parameters", Tag::KwParam),
    ("statistics", Tag::KwStatistics),
    ("process", Tag::KwProcess),
    ("mismatch", Tag::KwMismatch),
    ("vary", Tag::KwVary),
    ("correlate", Tag::KwCorrelate),
    ("truncate", Tag::KwTruncate),
    ("if", Tag::KwIf),
    ("else", Tag::KwElse),
    ("real", Tag::KwReal),
    ("return", Tag::KwReturn),
    ("global", Tag::KwGlobal),
    ("save", Tag::KwSave),
    ("ic", Tag::KwIc),
    ("nodeset", Tag::KwNodeset),
];

/// The analysis and control-statement masters (`statement.rs` cites the
/// `[REF]` pages), in today's reader's order.
const MASTERS: &[(&str, Statement)] = {
    use Analysis as A;
    use Statement::{Analysis as An, Command as C};
    &[
        ("montecarlo", An(A::Montecarlo)),
        ("acmatch", An(A::Acmatch)),
        ("dcmatch", An(A::Dcmatch)),
        ("hbnoise", An(A::Hbnoise)),
        ("qpnoise", An(A::Qpnoise)),
        ("fourier", An(A::Fourier)),
        ("pdisto", An(A::Pdisto)),
        ("pnoise", An(A::Pnoise)),
        ("hbstb", An(A::Hbstb)),
        ("envlp", An(A::Envlp)),
        ("hbsp", An(A::Hbsp)),
        ("hbac", An(A::Hbac)),
        ("hbxf", An(A::Hbxf)),
        ("noise", An(A::Noise)),
        ("qpac", An(A::Qpac)),
        ("qpsp", An(A::Qpsp)),
        ("qpss", An(A::Qpss)),
        ("qpxf", An(A::Qpxf)),
        ("sweep", An(A::Sweep)),
        ("pac", An(A::Pac)),
        ("psp", An(A::Psp)),
        ("pss", An(A::Pss)),
        ("pstb", An(A::Pstb)),
        ("pxf", An(A::Pxf)),
        ("sens", An(A::Sens)),
        ("tran", An(A::Tran)),
        ("tdr", An(A::Tdr)),
        ("stb", An(A::Stb)),
        ("ac", An(A::Ac)),
        ("dc", An(A::Dc)),
        ("hb", An(A::Hb)),
        ("pz", An(A::Pz)),
        ("sp", An(A::Sp)),
        ("xf", An(A::Xf)),
        ("altergroup", C(Command::Altergroup)),
        ("checklimit", C(Command::Checklimit)),
        ("paramtest", C(Command::Paramtest)),
        ("paramset", C(Command::Paramset)),
        ("options", Statement::Options),
        ("assert", C(Command::Assert)),
        ("alter", C(Command::Alter)),
        ("check", C(Command::Check)),
        ("shell", C(Command::Shell)),
        ("info", C(Command::Info)),
        ("set", C(Command::Set)),
    ]
};

#[cfg(test)]
mod tests {
    use crate::syntax::lexicon::{HSPICE, SPECTRE};
    use crate::syntax::token::Tag;
    use crate::syntax::tokenize;
    use crate::syntax::{test_numbers as numbers, test_words as words};

    #[test]
    fn star_only_in_column_one_and_slashes() {
        // §1.3: `*` in column 1; ` *` is text; `//` anywhere.
        let w = words("t\n* c\n *x\nr1 (a b) resistor r=1k // c\n// c\n", &SPECTRE);
        assert_eq!(w, ["* x", "r1 ( a b ) resistor r = 1k"]);
    }

    #[test]
    fn backslash_and_plus_continue() {
        // §1.4 (UG p.59).
        let w = words("t\nr1 (a b) \\\n resistor\n+ r=1\n", &SPECTRE);
        assert_eq!(w, ["r1 ( a b ) resistor r = 1"]);
    }

    #[test]
    fn escaped_names_keep_their_backslash() {
        // §2.2: `\2N2222` is a name.
        let t = tokenize("t\nq1 (c b e) \\2N2222\n", &SPECTRE);
        assert_eq!(t.tags()[8], Tag::Ident);
        assert_eq!(t.text(8), "\\2N2222");
    }

    #[test]
    fn keywords_at_heads_only() {
        // `section=tt` is a parameter, `} else {` a head again.
        let t = tokenize("t\ninclude \"a\" section=tt\n} else {\n", &SPECTRE);
        let tags = t.tags();
        assert_eq!(tags[2], Tag::KwInclude);
        assert_eq!(tags[4], Tag::Ident);
        assert_eq!(tags[9], Tag::KwElse);
    }

    #[test]
    fn scales_are_case_sensitive() {
        // §2.4: `M` mega, `m` milli, units after the scale; `1k5` is none.
        let v = numbers("t\np a=1M b=1m c=1pf d=5% e=50_Ohms f=1k5\n", &SPECTRE);
        assert_eq!(&v[..5], [1e6, 1e-3, 1e-12, 0.05, 50.0]);
        assert!(v[5].is_nan());
    }

    #[test]
    fn lang_spice_region_reads_as_hspice() {
        // §3.1 (UG p.53): the region's `.param` is a keyword, `$` a comment,
        // `+` a continuation; `simulator lang=spectre` (any case) goes back.
        let src =
            "t\nsimulator lang=spice\n.param a=1M $ c\n+ b=2\nSIMULATOR LANG=SPECTRE\np c=1M\n";
        let t = tokenize(src, &SPECTRE);
        assert_eq!(
            words(src, &SPECTRE),
            [
                "simulator lang = spice",
                ".param a = 1M b = 2",
                "SIMULATOR LANG = SPECTRE",
                "p c = 1M"
            ]
        );
        let at = |tag| t.tags().iter().position(|&x| x == tag).unwrap() as u32;
        assert!(std::ptr::eq(t.lexicon_at(at(Tag::KwParam)), &HSPICE));
        assert_eq!(numbers(src, &SPECTRE), [1e-3, 2.0, 1e6]);
        assert!(at(Tag::LangSpice) < at(Tag::KwParam) && at(Tag::KwParam) < at(Tag::LangSpectre));
    }
}
