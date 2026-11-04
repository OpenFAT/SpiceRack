//! The diagnostic catalogue: one [`Code`] per refusal or drop, each a row
//! of [`Info`] (its title, the manual clause it cites, its explanation and
//! its [`Kind`]), and the [`Diag`] row a stage appends (a code and the
//! token it anchors to). Messages are rendered late, from the rows
//! ([`messages`]: line and column from the token, sorted by source
//! position), never stored.
//!
//! Codes are never renumbered or reused; a retired code keeps its row. The
//! first letter is the stage class and the number its range:
//!
//! | Range | Stage |
//! |---|---|
//! | L0001–L0100 | tokenize |
//! | P0001–P0100 | parse |
//! | R0001–R0100 | resolve |
//! | W0001–W0100 | render: the SPICE-family writer and the rules both writers share |
//! | W0101–W0200 | render: the named writer (Spectre, VACASK) |
//!
//! The catalogue is one table (`catalogue!`): a code's variant and its
//! row are one entry, so no code lacks a row; `tests` checks each row and
//! the ranges.
//!
//! Cites: Zig 0.17 `lib/std/zig/Ast.zig` (`Ast.Error`); VerA
//! `lib/diag/code.zig`.
//!
//! Contents:
//! - `pub enum Code`, `Code::ALL: &[Code]`, `pub enum Kind`, `pub struct Info`,
//!   `pub const fn info(c: Code) -> Info`
//! - `pub struct Diag { pub code: Code, pub token: u32 }`
//! - `pub fn sort(diags: &mut [Diag])`
//! - `pub fn messages(diags: &[Diag], t: &Tokens, file: &str) -> String`

use std::fmt::Write as _;

use crate::syntax::token::Tokens;

/// What a diagnostic means for the file it is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The stage kept going and kept the meaning (text kept as written, a
    /// binding left open): the deck is still written.
    Note,
    /// The target cannot say this with its meaning: the deck is not written.
    Refusal,
}

/// A code's fixed text.
pub struct Info {
    /// What went wrong, in a few words.
    pub title: &'static str,
    /// The manual clause, or the reason no clause applies.
    pub cite: &'static str,
    /// What triggers it and what the user can do (`--explain`).
    pub explain: &'static str,
    /// Whether the deck is still written.
    pub kind: Kind,
}

/// One diagnostic: `code` at token `token` of the file's [`Tokens`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Diag {
    /// What it is.
    pub code: Code,
    /// The token it anchors to.
    pub token: u32,
}

/// Defines [`Code`], [`Code::ALL`] and [`info`] from one list of rows, so
/// a code cannot lack its row nor be missing from `ALL`.
macro_rules! catalogue {
    ($(
        $(#[$doc:meta])*
        $code:ident $kind:ident {
            title: $title:literal,
            cite: $cite:literal,
            explain: $explain:literal $(,)?
        }
    )*) => {
        /// A diagnostic's code.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub enum Code {
            $($(#[$doc])* $code,)*
        }

        impl Code {
            /// Every code, in catalogue order.
            pub const ALL: &[Code] = &[$(Code::$code),*];
        }

        /// Code `c`'s row.
        pub const fn info(c: Code) -> Info {
            match c {
                $(Code::$code => Info {
                    title: $title,
                    cite: $cite,
                    explain: $explain,
                    kind: Kind::$kind,
                },)*
            }
        }
    };
}

catalogue! {
    /// A statement no rule reads; kept as a `Verbatim` node.
    P0001 Note {
        title: "a statement no rule reads",
        cite: "the dialect's statement grammar (ngspice manual ch.2; Spectre UG ch.3)",
        explain: "The parser has no rule for this statement's shape, so it keeps the statement's text as written. A writer for the same dialect family copies it; another family refuses it (W0001).",
    }
    /// An expression that does not parse; kept as a `Bad` node.
    P0002 Note {
        title: "an expression that does not parse",
        cite: "ngspice manual §2.8.5; HSPICE UG08 pp.279-283; Spectre UG p.87",
        explain: "The value's text is no expression of the dialect's grammar (a stray operator, an unbalanced parenthesis). Its text is kept as written; check the value's spelling against the manual.",
    }
    /// A block the file ends inside (no `.ends`, `.endl`, `.endif`, `}` …).
    P0003 Note {
        title: "a block the file ends inside",
        cite: "ngspice manual §2.5 (.subckt ... .ends), §2.7 (.lib ... .endl)",
        explain: "A subcircuit, library section, conditional or brace block is still open at the end of the file. Add its closer; the block's statements are read as if it closed at the end.",
    }
    /// A block closer with no block open (`.ends` at the top).
    P0004 Note {
        title: "a block closer with no block open",
        cite: "ngspice manual §2.5",
        explain: "A `.ends`, `.endl`, `.endif` or `}` closes nothing. It is ignored; remove it, or open the block it was meant to close.",
    }
    /// A number whose spelling the dialect's number grammar does not read
    /// (`1k5` in Spectre, `0x1F` in VACASK): its token's value is NaN.
    P0005 Note {
        title: "a number the dialect does not read",
        cite: "ngspice manual §2.1.3; the old Spectre grammar's §2.4 number rule (`9d1d6bb:grammar/spectre.peg`)",
        explain: "The token starts like a number but the dialect's suffix table does not read it. Spell it with the dialect's suffixes (Spectre has no `1k5`; SPICE's `M` is milli, `meg` mega).",
    }
    /// Nesting deeper than the parser's cap: the rest is kept as tokens.
    P0006 Note {
        title: "nesting deeper than the parser's cap",
        cite: "no clause: the parser's own limit, syntax::parse DEPTH_MAX",
        explain: "Expressions or blocks nest deeper than the parser walks (`DEPTH_MAX`), so the part below the cap is kept as tokens, unread. No real deck nests this deep; split the expression.",
    }
    /// A byte no token rule reads (the tokenizer's `Invalid`).
    P0007 Note {
        title: "a byte no token reads",
        cite: "the dialect's lexical grammar",
        explain: "A character the dialect's lexicon has no token for (a control byte, a stray symbol). The statement holding it is kept as text; remove the character or check the file's encoding.",
    }
    /// A delimiter with no closer on its statement: `{`, `'`, `(`.
    P0008 Note {
        title: "a delimiter with no closer",
        cite: "ngspice manual §2.1 ({...}, '...'); Spectre UG p.87",
        explain: "An expression delimiter (`{`, `'`, `(`) is not closed before the statement ends. Close it on the same logical line (continuations count).",
    }
    /// A section opened inside a section: sections do not nest.
    P0009 Note {
        title: "a section inside a section",
        cite: "ngspice manual §2.7; Spectre Reference 19.1 p.498",
        explain: "A library section opens before the previous one closed. Close the first section (`.endl`, `endsection`) before opening the next.",
    }
    /// An instance whose master names nothing the kit declares: no
    /// subcircuit, model or primitive. The instance keeps its name.
    R0001 Note {
        title: "a master the kit does not declare",
        cite: "ngspice manual §2.4.3 (X ... SUBNAM), ch.7-ch.11 (mname); Spectre UG p.66",
        explain: "No file of the kit declares the subcircuit or model this instance names. The instance keeps the name and the dialect's default letter; read the file in its kit (the files that define the master), or check the name's spelling.",
    }
    /// A name declared twice in one scope (two subcircuits, two models, two
    /// instances of one block); the first is the one a lookup finds.
    R0002 Note {
        title: "a name declared twice in one scope",
        cite: "ngspice manual §2.5; Spectre UG p.94 (names unique in a subcircuit)",
        explain: "Two declarations of one kind share a name in one scope. A reference binds the first in read order; rename or remove the other.",
    }
    /// A master that names declarations of different kinds equally near
    /// (a subcircuit and a model); the first in read order is bound.
    R0003 Note {
        title: "a master naming declarations of different kinds",
        cite: "Spectre UG p.66 (a master is a model, a subcircuit or a primitive)",
        explain: "The master names both a subcircuit and a model at the same distance. The first in read order is bound, which picks the element's letter; rename one of them.",
    }
    /// An instance whose node count differs from its subcircuit's ports.
    R0004 Note {
        title: "an instance's node count differs from its subcircuit's ports",
        cite: "ngspice manual §2.4.3; Spectre UG p.94",
        explain: "The instance connects more or fewer nodes than its subcircuit has ports. A simulator refuses it; connect one node per port.",
    }
    /// A master that names a parameter or a function, not a device.
    R0005 Note {
        title: "a master naming a parameter or a function",
        cite: "ngspice manual §2.4.3, §2.8; Spectre UG p.66",
        explain: "The instance's master is declared, but as a parameter or a function, not a subcircuit or model. Check which name the instance means.",
    }
    /// An include, `.lib` call or Verilog-A source the kit lacks.
    R0006 Note {
        title: "an included file the kit lacks",
        cite: "ngspice manual §2.6 (.include), §2.7 (.lib); Spectre UG p.98 (include)",
        explain: "The included path does not exist under the kit's root as the loader resolves it. The statement is written as is; the names it would declare stay unbound (R0001).",
    }
    /// An include the loader refused (a path outside the kit's root).
    R0007 Note {
        title: "an include the loader refused",
        cite: "no clause: the kit's loader confines paths (syntax::resolve::Kit::load)",
        explain: "The loader does not read files outside the kit's root. Move the file into the kit, or load the kit from a root that holds it.",
    }
    /// An include past the kit's nesting cap (`resolve::DEPTH_MAX`): not
    /// read.
    R0008 Note {
        title: "an include past the nesting cap",
        cite: "no clause: the kit's own limit, syntax::resolve::DEPTH_MAX",
        explain: "Includes nest deeper than the kit reads (`DEPTH_MAX`), usually an include cycle. The file below the cap is not read; break the cycle.",
    }
    /// An include past the kit's file cap (`resolve::FILES_MAX`): not read.
    R0009 Note {
        title: "an include past the file cap",
        cite: "no clause: the kit's own limit, syntax::resolve::FILES_MAX",
        explain: "The kit holds more files than it reads (`FILES_MAX`). The rest are not read; load a smaller entry set.",
    }
    /// A `.lib file section` or `include … section=` naming a section the
    /// file does not hold.
    R0010 Note {
        title: "a section the file does not hold",
        cite: "ngspice manual §2.7; Spectre Reference 19.1 p.498 (include ... section=)",
        explain: "The file is read but declares no section of that name. Check the section's spelling (corner names such as `tt` are case-sensitive in Spectre).",
    }
    /// A statement kept as text from another dialect family: only its own family reads it.
    W0001 Refusal {
        title: "a statement kept as another dialect's text",
        cite: "no clause: the text is the source dialect's (Pandoc's RawBlock rule)",
        explain: "The reader kept this statement as its source dialect's text (P0001, or a construct the reader keeps raw). Only a target of the same family can copy it; translate the statement by hand, or write it for its own family.",
    }
    /// A `^` whose meaning differs between the source and the target (power in one, XOR in the other).
    W0002 Refusal {
        title: "a `^` that means power in one dialect and XOR in the other",
        cite: "ngspice manual §2.8.5 (`^` power); LTspice help (`^` XOR); Xyce RG §2.1.18 (`^` XOR); VACASK Verilog-A operators",
        explain: "The deck's dialect reads `^` as power and the target as XOR, or the reverse, so the same text would compute another value. Spell the power as `**` or `pwr()` in the source.",
    }
    /// A statistical function (`agauss`, `gauss`, `aunif`, `unif`, `limit`) the target has no function for.
    W0003 Refusal {
        title: "a statistical function the target lacks",
        cite: "ngspice manual §2.11.5; LTspice help (`gauss(x)` takes one argument)",
        explain: "The target has no function with this statistical call's meaning (LTspice's `gauss` takes one argument). Write the variation in the target's own Monte Carlo form.",
    }
    /// A Verilog-A load the target loads another way (an OSDI binary where it compiles the source, or a command-line plugin).
    W0004 Refusal {
        title: "a Verilog-A load the target cannot spell",
        cite: "ngspice manual §13 (`pre_osdi`); HSPICE `.hdl`; Xyce RG (`-plugin`)",
        explain: "The target loads compiled models in a way a deck line cannot name (Xyce takes a `-plugin` on its command line). Load the model through the simulator's own mechanism.",
    }
    /// A `.lib file` with no section: LTspice reads the whole file; another target reads the card as a section definition.
    W0005 Refusal {
        title: "a `.lib` call with no section",
        cite: "ngspice manual §2.7; LTspice help (`.lib`)",
        explain: "A `.lib file` with no section is LTspice's include; other SPICE targets read `.lib name` as the start of a section. Spell it `.include file`.",
    }
    /// An R, C or L card naming a model the target's element line has no field for.
    W0006 Refusal {
        title: "an R/C/L model reference the target lacks",
        cite: "ngspice manual §3.3.1-§3.3.10; LTspice help (R, C, L)",
        explain: "The element names a `.model` for its value, which the target's R, C or L line has no field for. Write the value on the element.",
    }
    /// A source's RF port (`portnum n z0 Z`) the target has no form for.
    W0007 Refusal {
        title: "an RF port the target lacks",
        cite: "ngspice manual §4.1.11",
        explain: "The source is an ngspice RF port (`portnum`, `z0`), which the target has no source for. Model the port as a source with a series resistor.",
    }
    /// A PWL source's repeat or delay (`r=`, `td=`) the target does not read.
    W0008 Refusal {
        title: "a PWL repeat or delay the target lacks",
        cite: "ngspice manual §4.1.4",
        explain: "The PWL waveform repeats (`r=`) or starts late (`td=`), which the target's PWL does not read. Write out the repeated points.",
    }
    /// A TRNOISE or TRRANDOM waveform (ngspice only).
    W0009 Refusal {
        title: "a TRNOISE or TRRANDOM waveform the target lacks",
        cite: "ngspice manual §4.1.7, §4.1.8",
        explain: "Transient noise sources are ngspice's own. Use the target's noise analysis, or a PWL of sampled noise.",
    }
    /// A Laplace source whose expression has no transfer-function reading.
    W0010 Refusal {
        title: "a Laplace expression that does not read",
        cite: "ngspice manual §12.2.16 (`s_xfer`); HSPICE `LAPLACE`",
        explain: "The Laplace expression is not a proper rational function of `s` with numeric coefficients (a symbolic coefficient, an improper ratio, arithmetic around the call). Write the coefficients as numbers.",
    }
    /// A Laplace source the target has no form for (Xyce).
    W0011 Refusal {
        title: "a Laplace source the target lacks",
        cite: "Xyce RG §6.1.5",
        explain: "The target has no Laplace or transfer-function source. Realize the transfer function as a network of controlled sources.",
    }
    /// A library section on a target that reads `.lib name` as a file (LTspice).
    W0012 Refusal {
        title: "a library section the target reads as a file",
        cite: "LTspice help (`.lib`); ngspice manual §2.7",
        explain: "LTspice reads `.lib name` as a file to include, so it has no library sections. Split the sections into files and include the one wanted.",
    }
    /// A conditional block on a target with no `.if` (Xyce, LTspice).
    W0013 Refusal {
        title: "a conditional block the target lacks",
        cite: "ngspice manual §2.15; Xyce RG Table 6-1",
        explain: "The target has no `.if`. Keep the branch wanted, or select it with a library section.",
    }
    /// A `.param` or a load inside a conditional branch, which a SPICE `.if` may not hold.
    W0014 Refusal {
        title: "a `.param` or load inside a SPICE `.if` branch",
        cite: "ngspice manual §2.15",
        explain: "A SPICE `.if` branch may hold elements and models only. Move the `.param` or load out of the branch, or choose between branches with a ternary parameter.",
    }
    /// An analysis the target has no card for.
    W0015 Refusal {
        title: "an analysis the target lacks",
        cite: "ngspice manual ch.11; HSPICE UG; Xyce RG Table 6-1; LTspice help",
        explain: "The target has no analysis of this kind. Run it on a simulator that has one, or drop it from the deck.",
    }
    /// A `.step` sweep the target has no card for (it would run once).
    W0016 Refusal {
        title: "a `.step` the target lacks",
        cite: "ngspice manual (no `.step`; `.dc` sweeps); HSPICE `.alter`; Xyce RG p.155",
        explain: "The target has no `.step`, so the deck would run once. Sweep with the target's own form (`.dc` over a parameter, `.alter`, a script loop).",
    }
    /// An option the target has no spelling for.
    W0017 Refusal {
        title: "an option the target lacks",
        cite: "Xyce RG pp.96-104 (`.options <pkg>`)",
        explain: "The target groups options by package and has none of this name. Look up the target's option for the same setting.",
    }
    /// A Spectre `statistics` block a SPICE or VACASK deck cannot hold.
    W0018 Refusal {
        title: "a statistics block the target cannot hold",
        cite: "Spectre Reference 19.1 pp.175-179",
        explain: "The `statistics` block uses a form the target's statistical functions cannot draw (a correlated device list, a truncation under 4 sigma, a percent lognormal, a block inside a subcircuit or branch). Rewrite the block without them.",
    }
    /// A Spectre `vary` of a parameter no `parameters` of its scope declares.
    W0019 Refusal {
        title: "a vary of an undeclared parameter",
        cite: "Spectre Reference 19.1 p.176 (vary names a parameter)",
        explain: "The `vary` names a parameter no `parameters` statement of its scope declares, so there is no nominal value to draw around. Declare the parameter.",
    }
    /// A Spectre `correlate` whose correlation matrix is not positive definite.
    W0020 Refusal {
        title: "a correlation matrix that is not positive definite",
        cite: "Spectre Reference 19.1 p.177 (correlate); the Cholesky factor of the correlation matrix",
        explain: "The correlations given have no Cholesky factor (a coefficient outside [-1, 1], or an inconsistent set), so no draw has them. Check the `cc=` values.",
    }
    /// An element CDL's layout writes as an `* error:` comment: its ngspice
    /// card has no form (the writer's refusal codes), or a Verilog-A load.
    W0021 Note {
        title: "an element written as an error comment",
        cite: "no clause: the xschem layout's own form (codegen::xschem)",
        explain: "The CDL netlist is still written, with this element as an `* error:` comment naming why (the ngspice card's refusal codes, or a Verilog-A load CDL cannot hold). An LVS run sees no device there; fix the element for ngspice first.",
    }
    /// A Verilog-A or OSDI load not written: on LTspice (no OSDI loader),
    /// or inside a subcircuit (the writers load the top's alone).
    W0022 Note {
        title: "a Verilog-A load not written",
        cite: "LTspice help (no OSDI or Verilog-A loader)",
        explain: "The writer leaves the load out, as the old writer did: LTspice has no way to load a compiled model (phase 7 refuses it), and a load inside a subcircuit is not written on any target but ESPice. An instance of the model then names nothing the simulator knows; load it at the top.",
    }
    /// An output request (`.print`, `.save`, `.probe`) not written.
    W0023 Note {
        title: "an output request not written",
        cite: "LTspice help, Xyce RG §2.1.21, VACASK docs (every vector saved); Spectre Reference 19.1 (no `.print` mapped)",
        explain: "The target saves every vector anyway (LTspice, Xyce, VACASK), or the writer has no form for the request (Spectre's and VACASK's `.print`, HSPICE's `.probe` under a noise run). Select the outputs in the target's post-processing.",
    }
    /// An option not written: another simulator's own, or one with no effect
    /// on the numbers.
    W0024 Note {
        title: "an option not written",
        cite: "Xyce RG pp.96-104 (`.options <pkg>`); VACASK docs/cmd-options.md",
        explain: "The option is another simulator's own (VACASK's on a SPICE target, Xyce's packaged options on Spectre or VACASK) or only cosmetic, so it is left out. Set the target's own option if one has the same effect.",
    }
    /// A `.control` line other than an analysis: not written.
    W0025 Note {
        title: "a control-block line not written",
        cite: "ngspice manual ch.13 (the control language is a script, not a netlist)",
        explain: "Only the analyses of an ngspice `.control` block are read; its script lines (`let`, `alter`, `write`, `plot` …) are not translated. Run the script on ngspice, or rewrite it for the target's own scripting.",
    }
    /// A waveform's arguments past the count the target's form takes.
    W0026 Note {
        title: "a waveform argument past the target's count not written",
        cite: "ngspice manual §4.1.1 (PULSE's NP), §4.1.2 (SIN)",
        explain: "The source's waveform has more arguments than the form written takes (ngspice's PULSE pulse count NP), and the extra ones are left out. Check that the waveform does not depend on them.",
    }
    /// A statement kept as text, copied as written into a target of its own
    /// family (or CDL's layout).
    W0027 Note {
        title: "a statement kept as text, copied as written",
        cite: "no clause: the text is the source dialect's (Pandoc's RawBlock rule)",
        explain: "The reader kept this statement as its source's text (P0001, or a form the reader keeps raw), and the target reads that text, so it is copied unchecked: no capability or `^` check looks inside it.",
    }
    /// A statement the CDL layout (an LVS netlist) does not write.
    W0028 Note {
        title: "a statement the CDL layout does not write",
        cite: "Cadence CDL (no analyses); the xschem layout (codegen::xschem)",
        explain: "CDL is a netlist for LVS: analyses, library sections, conditionals, includes, functions and a subcircuit's models are not written. Netlist the deck for a simulator to keep them.",
    }
    // W0101–W0200: the named writer's (Spectre, VACASK).
    /// A flag (a parameter with no value) a target taking only `name=value` cannot spell.
    W0101 Refusal {
        title: "a flag the target has no spelling for",
        cite: "Spectre Reference 19.1 p.506 (`name=value`); VACASK docs/input-instances.md",
        explain: "The element carries a bare flag (`off`, `ic`) and the target takes only `name=value`. Write the flag's parameter form, if the device has one.",
    }
    /// A behavioural source whose SPICE expression the target's grammar cannot be trusted to read.
    W0102 Refusal {
        title: "a behavioural source the target cannot spell",
        cite: "Spectre Reference 5.0 p.856 (bsource: `v(a,b)`, `i(\"inst:idx\")`, `$time`)",
        explain: "The behavioural source's expression uses forms (a current probe of an element, `time`, a function) the target's bsource spells otherwise. Rewrite the expression in the target's form.",
    }
    /// A charge expression reading a branch current, a braced value, `time`, `temper` or `hertz`.
    W0103 Refusal {
        title: "a charge expression the target cannot spell",
        cite: "Spectre Reference 19.1 p.459 (bsource `q=`)",
        explain: "The charge capacitor's expression reads something the target's `q=` form cannot (a branch current, `time`, `temper`, `hertz`). Model the capacitor with a behavioural current source.",
    }
    /// A `.model` whose kind (and level) has no master in the target.
    W0104 Refusal {
        title: "a model kind with no master in the target",
        cite: "Spectre Reference 19.1 p.498 (model masters); VACASK spice/*.osdi",
        explain: "No master of the target has this model kind and level's equations; the nearest master would compute something else. Use a model the target ships, or load the model as Verilog-A.",
    }
    /// A device the target has no master or terminal list for (a MOSFET without its bulk or with extra nodes, a current-controlled switch; in VACASK a POLY source, a charge capacitor, a switch, a line).
    W0105 Refusal {
        title: "a device the target has no master or terminals for",
        cite: "Spectre Reference 5.0 (mos, relay, vsource chapters)",
        explain: "The target has no device with this element's terminals (a MOSFET with three or five nodes, a current-controlled switch, a POLY source). Connect the missing terminal, or model the device another way.",
    }
    /// A function the target cannot define (a body over `v()`/`i()`; VACASK has none).
    W0106 Refusal {
        title: "a function the target cannot define",
        cite: "Spectre Reference R5 pp.870-871 (`real f(real x)`); VACASK has no user functions",
        explain: "The target's functions cannot read node voltages or currents (Spectre), or it has no user functions (VACASK). Inline the function where it is called.",
    }
    /// A statistical call with no form in the target (HSPICE's multiplier, a wrong argument count).
    W0107 Refusal {
        title: "a statistical call with no form in the target",
        cite: "HSPICE UG B-2008.09 App. A (`multiplier`); ngspice manual §2.11.5; Xyce RG Table 2-33",
        explain: "The call's arguments have no meaning in the target's statistical form (HSPICE's four-argument multiplier, a count no dialect reads). Write the call with ngspice's arguments.",
    }
    /// A source waveform the target cannot run as SPICE does (Spectre's AM; VACASK's PWL, a SIN phase, an EXP delay that is no number).
    W0108 Refusal {
        title: "a source waveform the target cannot run",
        cite: "Spectre Reference 5.0 p.685 (no AM equation); VACASK vsource (`sine` ignores a phase; PWL)",
        explain: "The target's source runs this waveform otherwise (Spectre has no AM; VACASK ignores a SIN phase). Write the waveform as a behavioural source or a PWL.",
    }
    /// A `.measure` the target has no statement for (VACASK).
    W0109 Refusal {
        title: "a measurement the target has no statement for",
        cite: "VACASK docs (no `measure`: post-process the raw file)",
        explain: "VACASK has no measurement statement. Compute the measurement from the raw file after the run.",
    }
    /// A `.lib file` with no section written as `include "file" section=`.
    W0110 Note {
        title: "a sectionless library call written with an empty section",
        cite: "Spectre Reference 19.1 p.493 (include ... section=)",
        explain: "The SPICE `.lib file` names no section; the writer writes `include \"file\" section=` with an empty section, as the old writer did (phase 7 refuses it). Name the section, or use `.include`.",
    }
    /// A `.model` level that is no number, mapped as level 1.
    W0111 Note {
        title: "a model level that is no number, read as level 1",
        cite: "Spectre Reference 19.1 p.498 (model masters)",
        explain: "The model's `level` is an expression, so the master is chosen as for level 1 (`mos1`, `bjt`). Write the level as a number so the master matches the model's equations.",
    }
}

/// Sorts `diags` by source position (token), then code: the order their
/// messages are rendered in.
pub fn sort(diags: &mut [Diag]) {
    diags.sort_by_key(|d| (d.token, d.code));
}

/// `diags` (one file's) as messages, one a line, sorted by source
/// position: `file:line:column: CODE title`, the line and column computed
/// here from each diagnostic's token in `t`.
///
/// # Panics
/// When a diagnostic's token is not one of `t`'s.
pub fn messages(diags: &[Diag], t: &Tokens, file: &str) -> String {
    let mut sorted = diags.to_vec();
    sort(&mut sorted);
    let mut out = String::new();
    for d in &sorted {
        assert!(
            (d.token as usize) < t.len(),
            "a diagnostic on a token of `t`"
        );
        let at = t.location(d.token);
        let i = info(d.code);
        writeln!(
            out,
            "{file}:{}:{}: {:?} {}",
            at.line, at.column, d.code, i.title
        )
        .expect("a String takes writes");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_row_with_a_title_a_cite_and_an_explanation() {
        assert!(!Code::ALL.is_empty());
        for (k, &c) in Code::ALL.iter().enumerate() {
            // `ALL` is in declaration order, so it holds every variant once.
            assert_eq!(c as usize, k, "{c:?}");
            let i = info(c);
            assert!(!i.title.is_empty(), "{c:?}");
            assert!(!i.cite.trim().is_empty(), "{c:?}");
            assert!(i.explain.len() > i.title.len(), "{c:?}");
        }
        assert_eq!(size_of::<Diag>(), 8);
    }

    #[test]
    fn codes_follow_their_stage_ranges() {
        // The module header's table: (letter, first, last, stage).
        const RANGES: [(char, u32, u32, &str); 5] = [
            ('L', 1, 100, "tokenize"),
            ('P', 1, 100, "parse"),
            ('R', 1, 100, "resolve"),
            ('W', 1, 100, "render: SPICE and shared"),
            ('W', 101, 200, "render: named"),
        ];
        let mut last: Option<(char, u32)> = None;
        for &c in Code::ALL {
            let name = format!("{c:?}");
            let (letter, digits) = name.split_at(1);
            let letter = letter.chars().next().expect("a code has a letter");
            assert_eq!(digits.len(), 4, "{name}");
            let n: u32 = digits.parse().expect("four digits");
            assert!(
                RANGES
                    .iter()
                    .any(|&(l, lo, hi, _)| l == letter && (lo..=hi).contains(&n)),
                "{name} is in no stage's range"
            );
            // Stage order (L, P, R, W), each class ascending: a new code is
            // appended to its range.
            let rank = |l: char| "LPRW".find(l).expect("a stage letter");
            if let Some((pl, pn)) = last {
                assert!((rank(pl), pn) < (rank(letter), n), "{name} out of order");
            }
            last = Some((letter, n));
            // Parse, tokenize and resolve keep going; only a writer refuses.
            if letter != 'W' {
                assert_eq!(info(c).kind, Kind::Note, "{name}");
            }
        }
    }
}
