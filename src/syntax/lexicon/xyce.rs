//! Xyce's lexicon.
//!
//! Cites (as the old Xyce grammar did; RG = Xyce Reference Guide 7.9, checked
//! on Xyce 7.10): §2.1.39 p.163 (line 1 the title); §2.1.39.1 (`*` comment
//! lines, a blank-led line is a comment); §2.1.39.2 (`;` anywhere, quotes
//! and braces included); §2.1.39.3 (`+` continuation); §2.1.6 (`.END`);
//! §3.1 p.837 and Table 6-2 p.852 (scale factors, `X` mega, no atto); §2.2
//! p.164 (`{expr}` and `'expr'`); Table 2-32 (operators); p.165 (`:` in
//! names).
//!
//! Contents:
//! - `pub const XYCE: Lexicon`

use super::{
    Arithmetic, Backslash, Caret, Dollar, End, Join, Lexicon, Numbers, Ops, Quote, Quoting,
    SPICE_KEYWORDS, SPICE_OPERATORS, Scale, Tail,
};
use crate::codegen::Dialect;
use crate::syntax::device;
use crate::syntax::token::Tag;

/// Xyce: case-blind, `*` and blank-led comment lines, `;` comments that no
/// quote protects, `+` continuation, `.END` in column 1.
pub const XYCE: Lexicon = Lexicon {
    dialect: Dialect::Xyce,
    title: true,
    fold_case: true,
    comment_lines: &["*"],
    comment_col1: &[],
    blank_led_comment: true,
    cont_skip: &["*"],
    comment_tails: &[";"],
    block_comment: false,
    long_strings: false,
    dollar: Dollar::Text,
    plus: true,
    backslash: Backslash::None,
    end: Some(End {
        col1: true,
        then_blank: true,
        stops: true,
    }),
    stop: None,
    join: Join::None,
    quote: Quote::Expr(UNPROTECTED),
    string: UNPROTECTED,
    string_expr: false,
    name_escape: false,
    ident_start: b"$",
    ident_continue: b"$.:",
    directive: Some(b'.'),
    head_words: false,
    heads_after: &[],
    operators: SPICE_OPERATORS,
    expr_braces: true,
    caret: Caret::Xor,
    ops: Ops {
        // Table 2-32 (RG p.164), the levels measured on Xyce 7.10.
        infix: &[
            (Tag::PipePipe, 2),
            (Tag::Pipe, 2),
            (Tag::AmpAmp, 2),
            (Tag::Amp, 2),
            (Tag::Caret, 3),
            (Tag::EqualEqual, 4),
            (Tag::BangEqual, 4),
            (Tag::LessGreater, 4),
            (Tag::LessEqual, 4),
            (Tag::GreaterEqual, 4),
            (Tag::Less, 4),
            (Tag::Greater, 4),
            (Tag::Plus, 5),
            (Tag::Minus, 5),
            (Tag::Percent, 6),
            (Tag::Star, 7),
            (Tag::Slash, 7),
            (Tag::StarStar, 9),
        ],
        right: &[Tag::StarStar],
        prefix: &[Tag::Plus, Tag::Minus, Tag::Tilde],
        prefix_bp: 8,
        // RG §2.1.31 pp.130-133.
        probes: &[
            "v", "vr", "vi", "vm", "vp", "vdb", "i", "ir", "ii", "im", "ip", "idb", "n", "p", "w",
        ],
        index: false,
    },
    lib_sections: true,
    numbers: Numbers {
        exponent: b"eE",
        scales: SCALES,
        tail: Tail::Spice { dot: false },
        arithmetic: Arithmetic::Decimal,
    },
    keywords: &[SPICE_KEYWORDS, OWN_KEYWORDS],
    named: false,
    devices: &[device::SPICE],
    masters: &[],
    analyses: &[],
    lang_spice: None,
};

/// §2.1.39.2: a `;` inside quotes still starts a comment.
const UNPROTECTED: Quoting = Quoting {
    escapes: false,
    protects: false,
    across_breaks: false,
};

const SCALES: &[(&str, Scale)] = &[
    ("meg", Scale::Exp(6)),
    ("mil", Scale::Mil),
    ("x", Scale::Exp(6)),
    ("t", Scale::Exp(12)),
    ("g", Scale::Exp(9)),
    ("k", Scale::Exp(3)),
    ("m", Scale::Exp(-3)),
    ("u", Scale::Exp(-6)),
    ("n", Scale::Exp(-9)),
    ("p", Scale::Exp(-12)),
    ("f", Scale::Exp(-15)),
];

/// RG p.49: `.incl` is `.include`.
const OWN_KEYWORDS: &[(&str, Tag)] = &[(".incl", Tag::KwInclude), (".step", Tag::KwStep)];

#[cfg(test)]
mod tests {
    use crate::syntax::lexicon::XYCE;
    use crate::syntax::{test_numbers as numbers, test_words as words};

    #[test]
    fn blank_led_lines_are_comments_and_semicolons_cut_quotes() {
        // §2.1.39.1, §2.1.39.2.
        let w = words(
            "t\n  R0 a b 1\nR1 a b 1 ; c\nR2 a b \"x;y\"\n  + 2\n",
            &XYCE,
        );
        assert_eq!(w, ["R1 a b 1", "R2 a b \"x 2"]);
    }

    #[test]
    fn end_only_in_column_one() {
        // §2.1.6: ` .end` is a blank-led comment line, `.end` ends the netlist.
        assert_eq!(
            words("t\nR1 a b 1\n .end\nR2 a b 2\n.END\nR3 a b 3\n", &XYCE),
            ["R1 a b 1", "R2 a b 2"]
        );
    }

    #[test]
    fn x_is_mega_and_a_is_no_scale() {
        // §3.1 p.837: `1a` is 1 with a unit.
        assert_eq!(numbers("t\nR1 a b 1x 1a 1M\n", &XYCE), [1e6, 1.0, 1e-3]);
    }
}
