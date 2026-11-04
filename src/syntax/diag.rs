//! The diagnostic catalogue: one [`Code`] per refusal or drop, each with its
//! title and the manual clause it cites, and the [`Diag`] row a stage
//! appends (a code and the token it anchors to). Messages are rendered
//! late, from the row, never stored.
//!
//! Codes are never renumbered or reused. The first letter is the stage:
//! `L` tokenize, `P` parse, `R` resolve, `W` render. Phase 2 of the rewrite
//! starts the `P` class, phase 3 the `R` class (`syntax::resolve`), phase 4
//! the `W` class (`syntax::render`, each a `Capability::Refuse`); phase 5
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
    /// A statement kept as text from another dialect family: only its own family reads it.
    W0001,
    /// A `^` whose meaning differs between the source and the target (power in one, XOR in the other).
    W0002,
    /// A statistical function (`agauss`, `gauss`, `aunif`, `unif`, `limit`) the target has no function for.
    W0003,
    /// A Verilog-A load the target loads another way (an OSDI binary where it compiles the source, or a command-line plugin).
    W0004,
    /// A `.lib file` with no section: LTspice reads the whole file; another target reads the card as a section definition.
    W0005,
    /// An R, C or L card naming a model the target's element line has no field for.
    W0006,
    /// A source's RF port (`portnum n z0 Z`) the target has no form for.
    W0007,
    /// A PWL source's repeat or delay (`r=`, `td=`) the target does not read.
    W0008,
    /// A TRNOISE or TRRANDOM waveform (ngspice only).
    W0009,
    /// A Laplace source whose expression has no transfer-function reading.
    W0010,
    /// A Laplace source the target has no form for (Xyce).
    W0011,
    /// A library section on a target that reads `.lib name` as a file (LTspice).
    W0012,
    /// A conditional block on a target with no `.if` (Xyce, LTspice).
    W0013,
    /// A `.param` or a load inside a conditional branch, which a SPICE `.if` may not hold.
    W0014,
    /// An analysis the target has no card for.
    W0015,
    /// A `.step` sweep the target has no card for (it would run once).
    W0016,
    /// An option the target has no spelling for.
    W0017,
    /// A Spectre `statistics` block a SPICE or VACASK deck cannot hold.
    W0018,
    /// A Spectre `vary` of a parameter no `parameters` of its scope declares.
    W0019,
    /// A Spectre `correlate` whose correlation matrix is not positive definite.
    W0020,
    // The named family's writer (Spectre, VACASK) numbers from W0101, so
    // the two writers' codes never collide while they land apart.
    /// A flag (a parameter with no value) a target taking only `name=value` cannot spell.
    W0101,
    /// A behavioural source whose SPICE expression the target's grammar cannot be trusted to read.
    W0102,
    /// A charge expression reading a branch current, a braced value, `time`, `temper` or `hertz`.
    W0103,
    /// A `.model` whose kind (and level) has no master in the target.
    W0104,
    /// A device the target has no master or terminal list for (a MOSFET without its bulk or with extra nodes, a current-controlled switch; in VACASK a POLY source, a charge capacitor, a switch, a line).
    W0105,
    /// A function the target cannot define (a body over `v()`/`i()`; VACASK has none).
    W0106,
    /// A statistical call with no form in the target (HSPICE's multiplier, a wrong argument count, a call inside a function).
    W0107,
    /// A source waveform the target cannot run as SPICE does (Spectre's AM; VACASK's PWL, a SIN phase, an EXP delay that is no number).
    W0108,
    /// A `.measure` the target has no statement for (VACASK).
    W0109,
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
        Code::W0001 => (
            "a statement kept as another dialect's text",
            "no clause: the text is the source dialect's (Pandoc's RawBlock rule)",
        ),
        Code::W0002 => (
            "a `^` that means power in one dialect and XOR in the other",
            "ngspice manual §2.8.5 (`^` power); LTspice help (`^` XOR); Xyce RG §2.1.18 (`^` XOR); VACASK Verilog-A operators",
        ),
        Code::W0003 => (
            "a statistical function the target lacks",
            "ngspice manual §2.11.5; LTspice help (`gauss(x)` takes one argument)",
        ),
        Code::W0004 => (
            "a Verilog-A load the target cannot spell",
            "ngspice manual §13 (`pre_osdi`); HSPICE `.hdl`; Xyce RG (`-plugin`)",
        ),
        Code::W0005 => (
            "a `.lib` call with no section",
            "ngspice manual §2.7; LTspice help (`.lib`)",
        ),
        Code::W0006 => (
            "an R/C/L model reference the target lacks",
            "ngspice manual §3.3.1-§3.3.10; LTspice help (R, C, L)",
        ),
        Code::W0007 => ("an RF port the target lacks", "ngspice manual §4.1.11"),
        Code::W0008 => (
            "a PWL repeat or delay the target lacks",
            "ngspice manual §4.1.4",
        ),
        Code::W0009 => (
            "a TRNOISE or TRRANDOM waveform the target lacks",
            "ngspice manual §4.1.7, §4.1.8",
        ),
        Code::W0010 => (
            "a Laplace expression that does not read",
            "ngspice manual §12.2.16 (`s_xfer`); HSPICE `LAPLACE`",
        ),
        Code::W0011 => ("a Laplace source the target lacks", "Xyce RG §6.1.5"),
        Code::W0012 => (
            "a library section the target reads as a file",
            "LTspice help (`.lib`); ngspice manual §2.7",
        ),
        Code::W0013 => (
            "a conditional block the target lacks",
            "ngspice manual §2.15; Xyce RG Table 6-1",
        ),
        Code::W0014 => (
            "a `.param` or load inside a SPICE `.if` branch",
            "ngspice manual §2.15",
        ),
        Code::W0015 => (
            "an analysis the target lacks",
            "ngspice manual ch.11; HSPICE UG; Xyce RG Table 6-1; LTspice help",
        ),
        Code::W0016 => (
            "a `.step` the target lacks",
            "ngspice manual (no `.step`; `.dc` sweeps); HSPICE `.alter`; Xyce RG p.155",
        ),
        Code::W0017 => (
            "an option the target lacks",
            "Xyce RG pp.96-104 (`.options <pkg>`)",
        ),
        Code::W0018 => (
            "a statistics block the target cannot hold",
            "docs/research/monte-carlo-spectre.md §10.5; Spectre Reference 19.1 pp.175-179",
        ),
        Code::W0019 => (
            "a vary of an undeclared parameter",
            "Spectre Reference 19.1 p.176 (vary names a parameter)",
        ),
        Code::W0020 => (
            "a correlation matrix that is not positive definite",
            "Spectre Reference 19.1 p.177 (correlate); docs/research/monte-carlo-spectre.md §6b",
        ),
        Code::W0101 => (
            "a flag the target has no spelling for",
            "Spectre Reference 19.1 p.506 (`name=value`); VACASK docs/input-instances.md",
        ),
        Code::W0102 => (
            "a behavioural source the target cannot spell",
            "Spectre Reference 5.0 p.856 (bsource: `v(a,b)`, `i(\"inst:idx\")`, `$time`)",
        ),
        Code::W0103 => (
            "a charge expression the target cannot spell",
            "Spectre Reference 19.1 p.459 (bsource `q=`)",
        ),
        Code::W0104 => (
            "a model kind with no master in the target",
            "Spectre Reference 19.1 p.498 (model masters); VACASK spice/*.osdi",
        ),
        Code::W0105 => (
            "a device the target has no master or terminals for",
            "Spectre Reference 5.0 (mos, relay, vsource chapters)",
        ),
        Code::W0106 => (
            "a function the target cannot define",
            "Spectre Reference R5 pp.870-871 (`real f(real x)`); VACASK has no user functions",
        ),
        Code::W0107 => (
            "a statistical call with no form in the target",
            "docs/research/monte-carlo-spectre.md §5.3, §7, §10.2",
        ),
        Code::W0108 => (
            "a source waveform the target cannot run",
            "Spectre Reference 5.0 p.685 (no AM equation); VACASK vsource (`sine` ignores a phase; PWL)",
        ),
        Code::W0109 => (
            "a measurement the target has no statement for",
            "VACASK docs (no `measure`: post-process the raw file)",
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
            R0004, R0005, R0006, R0007, R0008, R0009, R0010, W0001, W0002, W0003, W0004, W0005,
            W0006, W0007, W0008, W0009, W0010, W0011, W0012, W0013, W0014, W0015, W0016, W0017,
            W0018, W0019, W0020, W0101, W0102, W0103, W0104, W0105, W0106, W0107, W0108, W0109,
        ] {
            let i = info(c);
            assert!(!i.title.is_empty());
            assert!(!i.cite.is_empty());
        }
        assert_eq!(size_of::<Diag>(), 8);
    }
}
