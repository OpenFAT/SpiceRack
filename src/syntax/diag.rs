//! The diagnostic catalogue: one [`Code`] per refusal or drop, each with its
//! title and the manual clause it cites, and the [`Diag`] row a stage
//! appends (a code and the token it anchors to). Messages are rendered
//! late, from the row, never stored.
//!
//! Codes are never renumbered or reused. The first letter is the stage:
//! `L` tokenize, `P` parse, `R` resolve, `W` render. Phase 2 of the rewrite
//! starts the `P` class; phase 5 completes the catalogue
//! (`docs/research/transpiler-architecture.md` §2.6, §5).
//!
//! Cites: `transpiler-architecture.md` §2.6; `compiler-architecture-reference.md`
//! §3.5 (Zig `Ast.Error`), §8.1 (VerA `lib/diag/code.zig`).
//!
//! Contents:
//! - `pub enum Code`, `pub struct Info`, `pub const fn info(c: Code) -> Info`
//! - `pub struct Diag { pub code: Code, pub token: u32 }`

/// A diagnostic's code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    /// A statement no rule reads; kept as a `Verbatim` node.
    P0001,
    /// An expression that does not parse; kept as a `Bad` node.
    P0002,
    /// A block the file ends inside (no `.ends`, `.endl`, `.endif`, `}` …).
    P0003,
    /// A block closer with no block open (`.ends` at the top).
    P0004,
    /// A number whose spelling the dialect's number grammar does not read
    /// (`1k5` in Spectre, `0x1F` in VACASK): its token's value is NaN.
    P0005,
    /// Nesting deeper than the parser's cap: the rest is kept as tokens.
    P0006,
    /// A byte no token rule reads (the tokenizer's `Invalid`).
    P0007,
    /// A delimiter with no closer on its statement: `{`, `'`, `(`.
    P0008,
    /// A section opened inside a section: sections do not nest.
    P0009,
}

/// A code's fixed text.
pub struct Info {
    /// What went wrong, in a few words.
    pub title: &'static str,
    /// The manual clause, or the reason no clause applies.
    pub cite: &'static str,
}

/// One diagnostic: `code` at token `token` of the file's [`Tokens`].
///
/// [`Tokens`]: crate::syntax::token::Tokens
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Diag {
    /// What it is.
    pub code: Code,
    /// The token it anchors to.
    pub token: u32,
}

/// Code `c`'s title and citation.
pub const fn info(c: Code) -> Info {
    let (title, cite) = match c {
        Code::P0001 => (
            "a statement no rule reads",
            "the dialect's statement grammar (ngspice manual ch.2; Spectre UG ch.3)",
        ),
        Code::P0002 => (
            "an expression that does not parse",
            "ngspice manual §2.8.5; HSPICE UG08 pp.279-283; Spectre UG p.87",
        ),
        Code::P0003 => (
            "a block the file ends inside",
            "ngspice manual §2.5 (.subckt ... .ends), §2.7 (.lib ... .endl)",
        ),
        Code::P0004 => ("a block closer with no block open", "ngspice manual §2.5"),
        Code::P0005 => (
            "a number the dialect does not read",
            "ngspice manual §2.1.3; Spectre UG §2.4 (dev/research/spectre-syntax.md)",
        ),
        Code::P0006 => (
            "nesting deeper than the parser's cap",
            "no clause: the parser's own limit, syntax::parse DEPTH_MAX",
        ),
        Code::P0007 => ("a byte no token reads", "the dialect's lexical grammar"),
        Code::P0008 => (
            "a delimiter with no closer",
            "ngspice manual §2.1 ({...}, '...'); Spectre UG p.87",
        ),
        Code::P0009 => (
            "a section inside a section",
            "ngspice manual §2.7; Spectre Reference 19.1 p.498",
        ),
    };
    Info { title, cite }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_title_and_a_cite() {
        use Code::*;
        for c in [
            P0001, P0002, P0003, P0004, P0005, P0006, P0007, P0008, P0009,
        ] {
            let i = info(c);
            assert!(!i.title.is_empty());
            assert!(!i.cite.is_empty());
        }
        assert_eq!(size_of::<Diag>(), 8);
    }
}
