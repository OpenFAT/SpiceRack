# Dialect grammars

ESPice's grammar files (`tests/frontend/grammar/`), copied unchanged at the
commit in `UPSTREAM`. `src/peg.rs` interprets them; `src/import.rs` lowers
the capture trees to `CircuitIR`. Edit them in ESPice and re-copy, so both
read decks the same way.
