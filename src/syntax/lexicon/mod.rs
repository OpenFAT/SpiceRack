//! A dialect's lexical facts as one `const` table: what the tokenizer
//! (`syntax::tokenize`) reads per dialect, and what the parser looks up after
//! it (keywords, element letters, operator meaning). One file per dialect
//! holds its table and the tests that pin each rule to its source.
//!
//! The table is data. The two rules data cannot say are the hooks of
//! `transpiler-architecture.md` §2.1: HSPICE's `$` ([`Dollar::Hspice`]) and
//! Spectre's `simulator lang=` switch ([`Lexicon::lang_spice`]).
//!
//! Cites: `docs/research/transpiler-architecture.md` §2.1;
//! `compiler-architecture-reference.md` §7 and §12.1 (ESPice's per-dialect
//! constants, `ESPice/src/frontend/lines.zig:45-51`); the line stage and the
//! `Num` rule of each `grammar/*.peg`, which this replaces and whose manual
//! citations each dialect file repeats.
//!
//! Contents:
//! - `pub struct Lexicon`, `Lexicon::keyword(&self, word: &str) -> Option<Tag>`,
//!   `Lexicon::element(&self, letter: u8) -> Option<Element>`
//! - `pub enum Dollar`, `pub enum Backslash`, `pub struct End`, `pub enum Quote`,
//!   `pub struct Quoting`, `pub enum Caret`
//! - `pub struct Numbers`, `pub enum Scale`, `pub enum Tail`, `pub enum Arithmetic`
//! - `pub enum Element`
//! - `pub const NGSPICE`, `HSPICE`, `LTSPICE`, `XYCE`, `SPECTRE`, `VACASK: Lexicon`
//! - `pub fn of(d: Dialect) -> Option<&'static Lexicon>`

use crate::codegen::Dialect;
use crate::syntax::token::Tag;

mod hspice;
mod ltspice;
mod ngspice;
mod spectre;
mod vacask;
mod xyce;

pub use hspice::HSPICE;
pub use ltspice::LTSPICE;
pub use ngspice::NGSPICE;
pub use spectre::SPECTRE;
pub use vacask::VACASK;
pub use xyce::XYCE;

/// One dialect's lexical rules. Line structure first, then words, then the
/// tables the parser reads.
pub struct Lexicon {
    /// The dialect whose text this reads (ESPice reads [`NGSPICE`]).
    pub dialect: Dialect,
    /// Line 1 of a top-level file is the title, whatever it holds.
    pub title: bool,
    /// Words compare case-blind: keywords and suffixes match lowercased.
    pub fold_case: bool,

    /// A line whose first non-blank text starts with one of these is a
    /// comment line.
    pub comment_lines: &'static [&'static str],
    /// A line whose first byte (column 1) is one of these is a comment line.
    pub comment_col1: &'static [u8],
    /// A line that starts with a blank, unless its first non-blank is `+`, is
    /// a comment line (Xyce).
    pub blank_led_comment: bool,
    /// The comment lines a `+` continuation reads past (with blank lines).
    /// [`Lexicon::comment_col1`] and [`Lexicon::blank_led_comment`] lines
    /// are always read past.
    pub cont_skip: &'static [&'static str],
    /// Starters of a comment that runs to the end of the line, anywhere
    /// outside a protecting quote.
    pub comment_tails: &'static [&'static str],
    /// `/* … */` comments, across lines (VACASK).
    pub block_comment: bool,
    /// `<<<MARK`, a newline, text, `>>>MARK` is one string (VACASK).
    pub long_strings: bool,
    /// When `$` starts a comment.
    pub dollar: Dollar,
    /// `+` as a line's first non-blank continues the statement before it;
    /// one with nothing to continue is dropped.
    pub plus: bool,
    /// A trailing backslash's meaning.
    pub backslash: Backslash,
    /// The `.end` card, when the dialect has one.
    pub end: Option<End>,
    /// A line whose first non-blank is this byte ends the file (ngspice `@`).
    pub stop: Option<u8>,
    /// A newline inside `()`, `[]` or `{}` does not end the statement.
    pub brackets_join: bool,

    /// What `'` delimits.
    pub quote: Quote,
    /// How a `"…"` string reads.
    pub string: Quoting,
    /// `\` before a printable byte is part of a name (`\2N2222`).
    pub name_escape: bool,
    /// Bytes past `[A-Za-z_]` and non-ASCII that start a name.
    pub ident_start: &'static [u8],
    /// Bytes past `[A-Za-z0-9_]` and non-ASCII that continue a name.
    pub ident_continue: &'static [u8],
    /// The sigil of a directive word (`.param`, `@if`), always a keyword
    /// lookup.
    pub directive: Option<u8>,
    /// A bare word at a statement head is a keyword lookup too.
    pub head_words: bool,
    /// Tokens after which the next word is at a head again (Spectre's
    /// `} else {`, `inline subckt`).
    pub heads_after: &'static [Tag],
    /// Multi-byte operators, each one token; any other punctuation byte is
    /// its own one-byte token.
    pub operators: &'static [(&'static str, Tag)],
    /// `{…}` delimits an expression.
    pub expr_braces: bool,
    /// What `^` means in an expression.
    pub caret: Caret,

    /// The number grammar and its suffix table.
    pub numbers: Numbers,
    /// Keyword spellings, lowercase where [`Lexicon::fold_case`]; the first
    /// table holding a word wins.
    pub keywords: &'static [&'static [(&'static str, Tag)]],
    /// The SPICE element letters, lowercase; empty for named dialects.
    pub letters: &'static [(u8, Element)],
    /// The lexicon a `simulator lang=spice` statement switches to, and the
    /// one `simulator lang=spectre` switches back from: Spectre's hook.
    pub lang_spice: Option<&'static Lexicon>,
}

/// When `$` starts a comment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dollar {
    /// Never: `$` is a name byte.
    Text,
    /// At a statement or continuation's start, or after a blank, tab or
    /// comma; inside a name, or right after a quote, it is a name byte
    /// (ngspice).
    AfterBlank,
    /// HSPICE's hook: after a separator (blank, `,` `=` `(` `)`, a closing
    /// quote) or right after a number that follows one (`1k$c` is `1k`).
    Hspice,
}

/// A trailing backslash's meaning.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backslash {
    /// Not special.
    None,
    /// `\\`, blanks, end of line: the next line continues the statement,
    /// even after an end-of-line comment (ngspice).
    Glue,
    /// `\\` joins the next line with nothing between; a blank then `\`
    /// joins it keeping the blank; both continue quotes (HSPICE).
    Hspice,
    /// `\`, blanks, end of line joins the next line (Spectre).
    Join,
    /// `\` right before the newline joins the next line (VACASK).
    Tight,
}

/// The `.end` card.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct End {
    /// Only in column 1 (Xyce).
    pub col1: bool,
    /// Only a blank or the line's end may follow; else any non-word byte.
    pub then_blank: bool,
    /// Reading stops after it; else the card is dropped and reading goes on.
    pub stops: bool,
}

/// What `'` delimits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quote {
    /// Nothing: a one-byte [`Tag::Quote`].
    Plain,
    /// An expression, as [`Tag::Quote`] tokens around ordinary tokens.
    Expr(Quoting),
    /// A quoted part of a name, any non-blank bytes, `''` a quote (VACASK).
    Name,
}

/// How a quoted run reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Quoting {
    /// A backslash escapes the next byte.
    pub escapes: bool,
    /// Comment starters inside are text.
    pub protects: bool,
    /// The quote continues across a backslash break; else the break ends it.
    pub across_breaks: bool,
}

/// What `^` means in an expression.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Caret {
    /// Power, as `**`.
    Pow,
    /// Exclusive or.
    Xor,
    /// No operator.
    None,
}

/// A dialect's number grammar: a mantissa, an exponent, a scale suffix from
/// [`Numbers::scales`], then a unit tail, decoded by
/// [`Numbers::arithmetic`].
#[derive(Clone, Copy, Debug)]
pub struct Numbers {
    /// Letters that start an exponent.
    pub exponent: &'static [u8],
    /// Scale suffixes, the first match wins, lowercase where the dialect
    /// folds case.
    pub scales: &'static [(&'static str, Scale)],
    /// What may follow the mantissa and scale.
    pub tail: Tail,
    /// How the digits become an `f64`.
    pub arithmetic: Arithmetic,
}

/// A scale suffix's factor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scale {
    /// `10^n`, applied as a decimal shift so `0.1n` is exactly 1e-10.
    Exp(i32),
    /// A thousandth of an inch, 25.4e-6.
    Mil,
}

/// What may follow the mantissa and the scale.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tail {
    /// Letters, digits, `_` (and `.` when `dot`): a unit SPICE ignores.
    Spice { dot: bool },
    /// LTspice: digits right after the scale are its fraction (`6K34` is
    /// 6.34k); else letters, digits, `_`, `.` and `µ`.
    Ltspice,
    /// Letters and `_` only after a scale; nothing after a bare mantissa.
    /// `after_exponent`: a scale may follow an exponent (`1e3k`); `hex`:
    /// `0x…` is a number this does not decode.
    Letters { after_exponent: bool, hex: bool },
}

/// How a number's digits become an `f64`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arithmetic {
    /// ngspice's `INPevaluate`, digit for digit (`unit::ngspice_value`).
    Ngspice,
    /// The mantissa and the shifted exponent as one decimal, parsed once.
    Decimal,
}

/// The element a SPICE card's first letter names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Element {
    /// `R`.
    Resistor,
    /// `C`.
    Capacitor,
    /// `L`.
    Inductor,
    /// `K`, mutual inductance.
    Coupling,
    /// `V`.
    VoltageSource,
    /// `I`.
    CurrentSource,
    /// `E`.
    Vcvs,
    /// `G`.
    Vccs,
    /// `F`.
    Cccs,
    /// `H`.
    Ccvs,
    /// `B`, the behavioural source.
    Behavioural,
    /// `D`.
    Diode,
    /// `Q`.
    Bjt,
    /// `M`.
    Mosfet,
    /// `J`.
    Jfet,
    /// `Z`.
    Mesfet,
    /// `S`.
    VoltageSwitch,
    /// `W`.
    CurrentSwitch,
    /// `T`.
    TransmissionLine,
    /// `X`, a subcircuit instance.
    Instance,
}

/// The dot cards every SPICE dialect shares, with their aliases: the
/// spellings `grammar/spice.peg` reads (`.macro`/`.eom` are HSPICE's
/// `.subckt`/`.ends`, `.dcvolt` its `.ic`).
const SPICE_KEYWORDS: &[(&str, Tag)] = &[
    (".param", Tag::KwParam),
    (".parameter", Tag::KwParam),
    (".parameters", Tag::KwParam),
    (".func", Tag::KwFunc),
    (".subckt", Tag::KwSubckt),
    (".macro", Tag::KwSubckt),
    (".ends", Tag::KwEnds),
    (".eom", Tag::KwEnds),
    (".model", Tag::KwModel),
    (".include", Tag::KwInclude),
    (".inc", Tag::KwInclude),
    (".lib", Tag::KwLib),
    (".endl", Tag::KwEndl),
    (".end", Tag::KwEnd),
    (".global", Tag::KwGlobal),
    (".options", Tag::KwOptions),
    (".option", Tag::KwOptions),
    (".opts", Tag::KwOptions),
    (".opt", Tag::KwOptions),
    (".temp", Tag::KwTemp),
    (".temperature", Tag::KwTemp),
    (".ic", Tag::KwIc),
    (".dcvolt", Tag::KwIc),
    (".nodeset", Tag::KwNodeset),
    (".save", Tag::KwSave),
    (".print", Tag::KwPrint),
    (".plot", Tag::KwPrint),
    (".meas", Tag::KwMeas),
    (".measure", Tag::KwMeas),
    (".op", Tag::KwOp),
    (".dc", Tag::KwDc),
    (".ac", Tag::KwAc),
    (".tran", Tag::KwTran),
    (".noise", Tag::KwNoise),
    (".tf", Tag::KwTf),
    (".sens", Tag::KwSens),
    (".pz", Tag::KwPz),
    (".four", Tag::KwFour),
    (".fourier", Tag::KwFour),
    (".data", Tag::KwData),
    (".enddata", Tag::KwEnddata),
    (".variation", Tag::KwVariation),
    (".end_variation", Tag::KwEndVariation),
    (".if", Tag::KwIf),
    (".elseif", Tag::KwElseif),
    (".else", Tag::KwElse),
    (".endif", Tag::KwEndif),
];

/// The element letters `grammar/spice.peg` reads (`element`, its §3.1
/// rules), shared by the SPICE dialects.
const SPICE_LETTERS: &[(u8, Element)] = &[
    (b'r', Element::Resistor),
    (b'c', Element::Capacitor),
    (b'l', Element::Inductor),
    (b'k', Element::Coupling),
    (b'v', Element::VoltageSource),
    (b'i', Element::CurrentSource),
    (b'e', Element::Vcvs),
    (b'g', Element::Vccs),
    (b'f', Element::Cccs),
    (b'h', Element::Ccvs),
    (b'b', Element::Behavioural),
    (b'd', Element::Diode),
    (b'q', Element::Bjt),
    (b'm', Element::Mosfet),
    (b'j', Element::Jfet),
    (b'z', Element::Mesfet),
    (b's', Element::VoltageSwitch),
    (b'w', Element::CurrentSwitch),
    (b't', Element::TransmissionLine),
    (b'x', Element::Instance),
];

/// The SPICE comparison and logic operators.
const SPICE_OPERATORS: &[(&str, Tag)] = &[
    ("**", Tag::StarStar),
    ("==", Tag::EqualEqual),
    ("!=", Tag::BangEqual),
    ("<>", Tag::LessGreater),
    ("<=", Tag::LessEqual),
    (">=", Tag::GreaterEqual),
    ("&&", Tag::AmpAmp),
    ("||", Tag::PipePipe),
];

impl Lexicon {
    /// The keyword `word` spells, `None` for any other word. Case-blind
    /// where the dialect folds case.
    pub fn keyword(&self, word: &str) -> Option<Tag> {
        self.keywords.iter().find_map(|table| {
            table.iter().find_map(|&(s, tag)| {
                let hit = if self.fold_case {
                    s.eq_ignore_ascii_case(word)
                } else {
                    s == word
                };
                hit.then_some(tag)
            })
        })
    }

    /// The element a SPICE card starting with `letter` names, either case.
    pub fn element(&self, letter: u8) -> Option<Element> {
        let letter = letter.to_ascii_lowercase();
        self.letters
            .iter()
            .find_map(|&(l, e)| (l == letter).then_some(e))
    }
}

/// The lexicon dialect `d`'s text is read with; `None` for the dialects
/// with no reader (CDL, Verilog).
pub fn of(d: Dialect) -> Option<&'static Lexicon> {
    match d {
        Dialect::Ngspice | Dialect::Espice => Some(&NGSPICE),
        Dialect::Hspice => Some(&HSPICE),
        Dialect::Ltspice => Some(&LTSPICE),
        Dialect::Xyce => Some(&XYCE),
        Dialect::Spectre => Some(&SPECTRE),
        Dialect::Vacask => Some(&VACASK),
        Dialect::Cdl | Dialect::Verilog => None,
    }
}
