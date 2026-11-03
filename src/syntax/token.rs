//! The tokenizer's output: one row per token, `{tag: u8, start: u32}` SoA,
//! plus the side tables a later stage reads instead of re-lexing (number
//! values, the physical line starts, the lexicon switches).
//!
//! A token's end is not stored: [`Tokens::end`] re-lexes it from `start`, as
//! Zig's `Ast.tokenSlice` does. Comments, blank lines and continuation marks
//! are not tokens: they stay in the gaps between token spans, where a
//! renderer finds them (Zig `Render.zig:3074-3099`). Statement structure is
//! tokens ([`Tag::EndOfStatement`], [`Tag::LangSpice`]).
//!
//! Cites: `docs/research/transpiler-architecture.md` §2.1;
//! `compiler-architecture-reference.md` §1.1-§1.5 (Zig `tokenizer.zig`,
//! `Ast.zig:28-31`, `279-295`).
//!
//! Contents:
//! - `pub enum Tag`, `Tag::lexeme(self) -> Option<&'static str>`
//! - `pub struct Tokens<'s>`
//! - `pub struct Location { pub line: u32, pub column: u32 }`
//! - `Tokens::source(&self) -> &'s str`, `Tokens::tags(&self) -> &[Tag]`,
//!   `Tokens::len(&self) -> usize`, `Tokens::is_empty(&self) -> bool`,
//!   `Tokens::start(&self, i: u32) -> u32`, `Tokens::end(&self, i: u32) -> u32`,
//!   `Tokens::text(&self, i: u32) -> &'s str`, `Tokens::value(&self, i: u32) -> f64`,
//!   `Tokens::location(&self, i: u32) -> Location`,
//!   `Tokens::lexicon_at(&self, i: u32) -> &'static Lexicon`

use crate::syntax::lexicon::Lexicon;
use crate::syntax::scan;

/// A token's kind. Words are re-lexed from their start; every other tag but
/// the zero-width structure tags spells itself ([`Tag::lexeme`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Tag {
    // ── structure ──
    /// Line 1 of a top-level file, the whole physical line, maybe empty.
    Title,
    /// Zero width, at the end of a logical statement's last token line.
    EndOfStatement,
    /// Zero width, after a Spectre `simulator lang=spice` statement: the
    /// tokens up to the next [`Tag::LangSpectre`] are lexed by the
    /// lexicon's [`Lexicon::lang_spice`].
    LangSpice,
    /// Zero width, after a `simulator lang=spectre` statement in a SPICE
    /// region: back to the file's own lexicon.
    LangSpectre,
    /// One byte no rule reads (a control byte); the parser reports it.
    Invalid,

    // ── words ──
    /// A name: letters, digits, `_`, non-ASCII, the lexicon's extra bytes.
    Ident,
    /// A number with its scale and unit tail; its value is
    /// [`Tokens::value`].
    Number,
    /// `"…"` (or VACASK `<<<MARK … >>>MARK`), quotes included.
    String,
    /// A directive word (`.foo`, VACASK `@foo`) that is no keyword.
    Directive,

    // ── keywords: one tag per meaning, whatever the dialect's spelling ──
    /// `.param`, `parameters`.
    KwParam,
    /// `.func`.
    KwFunc,
    /// `.subckt`, `subckt`.
    KwSubckt,
    /// `.ends`, `ends`.
    KwEnds,
    /// `.model`, `model`.
    KwModel,
    /// `.include`, `include`.
    KwInclude,
    /// `.lib`.
    KwLib,
    /// `.endl`.
    KwEndl,
    /// `.end`.
    KwEnd,
    /// `.global`, `global`.
    KwGlobal,
    /// `.options`.
    KwOptions,
    /// `.temp`.
    KwTemp,
    /// `.ic`, `ic`.
    KwIc,
    /// `.nodeset`, `nodeset`.
    KwNodeset,
    /// `.save`, `save`.
    KwSave,
    /// `.print`, `.plot`.
    KwPrint,
    /// `.probe` (HSPICE).
    KwProbe,
    /// `.meas`.
    KwMeas,
    /// `.op`.
    KwOp,
    /// `.dc`.
    KwDc,
    /// `.ac`.
    KwAc,
    /// `.tran`.
    KwTran,
    /// `.noise`.
    KwNoise,
    /// `.tf`.
    KwTf,
    /// `.sens`.
    KwSens,
    /// `.pz`.
    KwPz,
    /// `.four`.
    KwFour,
    /// `.sp` (ngspice).
    KwSp,
    /// `.step` (LTspice, Xyce).
    KwStep,
    /// `.data`.
    KwData,
    /// `.enddata`.
    KwEnddata,
    /// `.variation`.
    KwVariation,
    /// `.end_variation`.
    KwEndVariation,
    /// `.control`, VACASK `control`.
    KwControl,
    /// `.endc`, VACASK `endc`.
    KwEndc,
    /// `.if`, Spectre `if`, VACASK `@if`.
    KwIf,
    /// `.elseif`, VACASK `@elseif`.
    KwElseif,
    /// `.else`, Spectre `else`, VACASK `@else`.
    KwElse,
    /// `.endif`, VACASK `@end`.
    KwEndif,
    /// Spectre `simulator`.
    KwSimulator,
    /// Spectre `ahdl_include`.
    KwAhdlInclude,
    /// Spectre `library`.
    KwLibrary,
    /// Spectre `endlibrary`.
    KwEndlibrary,
    /// Spectre and VACASK `section`.
    KwSection,
    /// Spectre and VACASK `endsection`.
    KwEndsection,
    /// Spectre `inline`.
    KwInline,
    /// Spectre `statistics`.
    KwStatistics,
    /// Spectre `process`.
    KwProcess,
    /// Spectre `mismatch`.
    KwMismatch,
    /// Spectre `vary`.
    KwVary,
    /// Spectre `correlate`.
    KwCorrelate,
    /// Spectre `truncate`.
    KwTruncate,
    /// Spectre `real`.
    KwReal,
    /// Spectre `return`.
    KwReturn,
    /// VACASK `load`.
    KwLoad,
    /// VACASK `embed`.
    KwEmbed,
    /// VACASK `ground`.
    KwGround,
    /// VACASK `analysis`.
    KwAnalysis,
    /// VACASK `sweep`.
    KwSweep,

    // ── punctuation and operators: each spells itself ──
    /// `(`.
    LParen,
    /// `)`.
    RParen,
    /// `[`.
    LBracket,
    /// `]`.
    RBracket,
    /// `{`.
    LBrace,
    /// `}`.
    RBrace,
    /// `,`.
    Comma,
    /// `=`.
    Equal,
    /// `'`, an expression quote where the lexicon says so.
    Quote,
    /// `:`.
    Colon,
    /// `;`.
    Semicolon,
    /// `?`.
    Question,
    /// `+`.
    Plus,
    /// `-`.
    Minus,
    /// `*`.
    Star,
    /// `**`.
    StarStar,
    /// `/`.
    Slash,
    /// `\`.
    Backslash,
    /// `%`.
    Percent,
    /// `^`.
    Caret,
    /// `!`.
    Bang,
    /// `!=`.
    BangEqual,
    /// `==`.
    EqualEqual,
    /// `<>`.
    LessGreater,
    /// `<`.
    Less,
    /// `<=`.
    LessEqual,
    /// `<<`.
    LessLess,
    /// `>`.
    Greater,
    /// `>=`.
    GreaterEqual,
    /// `>>`.
    GreaterGreater,
    /// `&`.
    Amp,
    /// `&&`.
    AmpAmp,
    /// `|`.
    Pipe,
    /// `||`.
    PipePipe,
    /// `~`.
    Tilde,
    /// `.`.
    Dot,
    /// `@`.
    At,
    /// `#`.
    Hash,
    /// `` ` ``.
    Backtick,
}

/// A file's tokens.
///
/// Invariants: `tags` and `starts` have one row per token, `starts` never
/// decreases; `number_tokens` lists every [`Tag::Number`] token in order,
/// `number_values` its value; `switches` lists every [`Tag::LangSpice`] and
/// [`Tag::LangSpectre`] token in order.
pub struct Tokens<'s> {
    pub(super) source: &'s str,
    pub(super) lexicon: &'static Lexicon,
    pub(super) tags: Vec<Tag>,
    pub(super) starts: Vec<u32>,
    /// The byte offset of each physical line's first byte.
    pub(super) line_starts: Vec<u32>,
    pub(super) number_tokens: Vec<u32>,
    /// The value of each `number_tokens` entry, NaN for a spelling the
    /// lexicon's number grammar does not read (`1k5` in Spectre).
    pub(super) number_values: Vec<f64>,
    pub(super) switches: Vec<u32>,
}

/// A position for a diagnostic, both 1-based; the column counts bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Location {
    /// The physical line, counting every `\n`.
    pub line: u32,
    /// The byte offset in that line, plus one.
    pub column: u32,
}

impl Tag {
    /// The fixed spelling of a punctuation tag; `None` for words and the
    /// structure tags, whose text is re-lexed.
    pub fn lexeme(self) -> Option<&'static str> {
        use Tag::*;
        Some(match self {
            LParen => "(",
            RParen => ")",
            LBracket => "[",
            RBracket => "]",
            LBrace => "{",
            RBrace => "}",
            Comma => ",",
            Equal => "=",
            Quote => "'",
            Colon => ":",
            Semicolon => ";",
            Question => "?",
            Plus => "+",
            Minus => "-",
            Star => "*",
            StarStar => "**",
            Slash => "/",
            Backslash => "\\",
            Percent => "%",
            Caret => "^",
            Bang => "!",
            BangEqual => "!=",
            EqualEqual => "==",
            LessGreater => "<>",
            Less => "<",
            LessEqual => "<=",
            LessLess => "<<",
            Greater => ">",
            GreaterEqual => ">=",
            GreaterGreater => ">>",
            Amp => "&",
            AmpAmp => "&&",
            Pipe => "|",
            PipePipe => "||",
            Tilde => "~",
            Dot => ".",
            At => "@",
            Hash => "#",
            Backtick => "`",
            Title | EndOfStatement | LangSpice | LangSpectre | Invalid | Ident | Number
            | String | Directive | KwParam | KwFunc | KwSubckt | KwEnds | KwModel | KwInclude
            | KwLib | KwEndl | KwEnd | KwGlobal | KwOptions | KwTemp | KwIc | KwNodeset
            | KwSave | KwPrint | KwProbe | KwMeas | KwOp | KwDc | KwAc | KwTran | KwNoise
            | KwTf | KwSens | KwPz | KwFour | KwSp | KwStep | KwData | KwEnddata | KwVariation
            | KwEndVariation | KwControl | KwEndc | KwIf | KwElseif | KwElse | KwEndif
            | KwSimulator | KwAhdlInclude | KwLibrary | KwEndlibrary | KwSection | KwEndsection
            | KwInline | KwStatistics | KwProcess | KwMismatch | KwVary | KwCorrelate
            | KwTruncate | KwReal | KwReturn | KwLoad | KwEmbed | KwGround | KwAnalysis
            | KwSweep => return None,
        })
    }
}

impl<'s> Tokens<'s> {
    /// The text the tokens index.
    pub fn source(&self) -> &'s str {
        self.source
    }

    /// Every token's tag, in source order.
    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    /// The number of tokens.
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// Whether there are no tokens.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// Token `i`'s first byte.
    pub fn start(&self, i: u32) -> u32 {
        self.starts[i as usize]
    }

    /// One past token `i`'s last byte, re-lexed from its start: O(token
    /// length), plus O(log switches) to find its lexicon.
    pub fn end(&self, i: u32) -> u32 {
        let start = self.start(i);
        match self.tags[i as usize] {
            Tag::EndOfStatement | Tag::LangSpice | Tag::LangSpectre => start,
            Tag::Title => scan::line_end(self.source.as_bytes(), start as usize) as u32,
            tag => match tag.lexeme() {
                Some(s) => start + s.len() as u32,
                None => {
                    scan::token_at(self.source.as_bytes(), start as usize, self.lexicon_at(i)).1
                        as u32
                }
            },
        }
    }

    /// Token `i`'s text, as written.
    pub fn text(&self, i: u32) -> &'s str {
        &self.source[self.start(i) as usize..self.end(i) as usize]
    }

    /// Number token `i`'s value, decoded once by its lexicon's suffix table;
    /// NaN when that grammar does not read its spelling.
    ///
    /// # Panics
    /// When token `i` is not a [`Tag::Number`].
    pub fn value(&self, i: u32) -> f64 {
        let row = self
            .number_tokens
            .binary_search(&i)
            .expect("value: token is a Number");
        self.number_values[row]
    }

    /// Where token `i` starts.
    pub fn location(&self, i: u32) -> Location {
        let at = self.start(i);
        let line = self.line_starts.partition_point(|&s| s <= at);
        Location {
            line: line as u32,
            column: at - self.line_starts[line - 1] + 1,
        }
    }

    /// The lexicon token `i` was lexed by: the file's, or its `lang_spice`
    /// inside a `simulator lang=spice` region.
    pub fn lexicon_at(&self, i: u32) -> &'static Lexicon {
        let before = self.switches.partition_point(|&s| s < i);
        match before
            .checked_sub(1)
            .map(|k| self.tags[self.switches[k] as usize])
        {
            Some(Tag::LangSpice) => self.lexicon.lang_spice.unwrap_or(self.lexicon),
            _ => self.lexicon,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_five_bytes_per_token_and_twelve_per_number() {
        // A token row is a tag byte plus a u32 start (Zig `Ast.TokenList`);
        // a number row is a u32 token index plus an f64 value.
        assert_eq!(size_of::<Tag>(), 1);
        assert_eq!(size_of::<Tag>() + size_of::<u32>(), 5);
        assert_eq!(size_of::<u32>() + size_of::<f64>(), 12);
        assert_eq!(size_of::<Location>(), 8);
    }
}
