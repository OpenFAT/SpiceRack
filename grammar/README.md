# Dialect grammars

One file per dialect; `spice.peg` is the SPICE family's shared card
structure, which `ngspice.peg`, `hspice.peg`, `ltspice.peg` and `xyce.peg`
are layered over (a rule there replaces the rule here). `spectre.peg` and
`vacask.peg` stand alone.

Each file holds two stages: `file` splits physical lines into the title and
logical lines (`Title`, `Line`, `Piece`, `Glue`), and `deck` parses the
logical lines into the AST. A captured rule (capitalized, or `Foo_x` for a
second spelling of `Foo`) must be a tag of `src/import/mod.rs`'s `Tag`, the
one schema every dialect captures into; `src/import/lower.rs` lowers it to
`CircuitIR`. `cargo run --example ast_dump -- <dialect> <deck>` prints the
AST a deck parses to.

Sources: the SPICE rules started as ESPice's grammar oracle
(`tests/frontend/grammar/`); LTspice from ADI's ltspice-reference, Xyce from
its Reference Guide, VACASK from its lexer, parser and docs
(`dev/research/dialect-readers.md`).
