# Dialect grammars

`src/peg.rs` interprets these; `src/import.rs` (with `import_spectre.rs`,
`import_vacask.rs`) lowers their capture trees to `CircuitIR`.

| files | source |
|---|---|
| `spice_cards.peg`, `ngspice*.peg`, `hspice*.peg`, `spectre*.peg` (not `spectre_one.peg`), `include.peg`, the rest | ESPice's grammar oracle (`tests/frontend/grammar/`), unchanged at the commit in `UPSTREAM`; edit them in ESPice and re-copy |
| `ltspice.peg`, `ltspice_lines.peg` | SpiceRack, from ADI's ltspice-reference (`dev/research/dialect-readers.md` 4) |
| `xyce.peg`, `xyce_lines.peg` | SpiceRack, from the Xyce Reference Guide (ibid. 3) |
| `vacask.peg`, `vacask_lines.peg` | SpiceRack, from VACASK's lexer, parser and docs (ibid. 2) |
| `spicerack.peg`, `spectre_one.peg`, `vacask_one.peg` | SpiceRack's one-statement entry rules |
