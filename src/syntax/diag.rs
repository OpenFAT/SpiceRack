//! The diagnostic catalogue: one [`Code`] per refusal or drop, each with its
//! title and the manual clause it cites, and the [`Diag`] row a stage
//! appends (a code and the token it anchors to). Messages are rendered
//! late, from the row, never stored.
//!
//! Codes are never renumbered or reused. The first letter is the stage:
//! `L` tokenize, `P` parse, `R` resolve, `W` render. Phase 2 of the rewrite
//! starts the `P` class, phase 3 the `R` class (`syntax::resolve`); phase 5
//! completes the catalogue (`docs/research/transpiler-architecture.md` §2.6,
//! §5).
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
    /// An instance whose master names nothing the kit declares: no
    /// subcircuit, model or primitive. The instance keeps its name.
    R0001,
    /// A name declared twice in one scope (two subcircuits, two models, two
    /// instances of one block); the first is the one a lookup finds.
    R0002,
    /// A master that names declarations of different kinds equally near
    /// (a subcircuit and a model); the first in read order is bound.
    R0003,
    /// An instance whose node count differs from its subcircuit's ports.
    R0004,
    /// A master that names a parameter or a function, not a device.
    R0005,
    /// An include, `.lib` call or Verilog-A source the kit lacks.
    R0006,
    /// An include the loader refused (a path outside the kit's root).
    R0007,
    /// An include past the kit's nesting cap (`resolve::DEPTH_MAX`): not
    /// read.
    R0008,
    /// An include past the kit's file cap (`resolve::FILES_MAX`): not read.
    R0009,
    /// A `.lib file section` or `include … section=` naming a section the
    /// file does not hold.
    R0010,
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
        Code::R0001 => (
            "a master the kit does not declare",
            "ngspice manual §2.4.3 (X ... SUBNAM), ch.7-ch.11 (mname); Spectre UG p.66",
        ),
        Code::R0002 => (
            "a name declared twice in one scope",
            "ngspice manual §2.5; Spectre UG p.94 (names unique in a subcircuit)",
        ),
        Code::R0003 => (
            "a master naming declarations of different kinds",
            "Spectre UG p.66 (a master is a model, a subcircuit or a primitive)",
        ),
        Code::R0004 => (
            "an instance's node count differs from its subcircuit's ports",
            "ngspice manual §2.4.3; Spectre UG p.94",
        ),
        Code::R0005 => (
            "a master naming a parameter or a function",
            "ngspice manual §2.4.3, §2.8; Spectre UG p.66",
        ),
        Code::R0006 => (
            "an included file the kit lacks",
            "ngspice manual §2.6 (.include), §2.7 (.lib); Spectre UG p.98 (include)",
        ),
        Code::R0007 => (
            "an include the loader refused",
            "no clause: the kit's loader confines paths (syntax::resolve::Kit::load)",
        ),
        Code::R0008 => (
            "an include past the nesting cap",
            "no clause: the kit's own limit, syntax::resolve::DEPTH_MAX",
        ),
        Code::R0009 => (
            "an include past the file cap",
            "no clause: the kit's own limit, syntax::resolve::FILES_MAX",
        ),
        Code::R0010 => (
            "a section the file does not hold",
            "ngspice manual §2.7; Spectre Reference 19.1 p.498 (include ... section=)",
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
            P0001, P0002, P0003, P0004, P0005, P0006, P0007, P0008, P0009, R0001, R0002, R0003,
            R0004, R0005, R0006, R0007, R0008, R0009, R0010,
        ] {
            let i = info(c);
            assert!(!i.title.is_empty());
            assert!(!i.cite.is_empty());
        }
        assert_eq!(size_of::<Diag>(), 8);
    }
}
