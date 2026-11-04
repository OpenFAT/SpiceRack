# Tools and the Rust crate

## spice2rack: convert a deck to every dialect

`spice2rack` reads existing SPICE decks and writes them out for every backend.

```bash
cargo run --no-default-features --bin spice2rack -- -o out/ deck.sp [more.sp ...]
```

For `deck.sp` it writes `deck.ngspice.sp`, `deck.hspice.sp`,
`deck.ltspice.cir`, `deck.spectre.scs`, `deck.vacask.sim` and
`deck.espice.sp`. A dialect that cannot express the circuit is reported with
its diagnostic codes and skipped; the rest are still written, and what each
kept or left out is printed beside it.

| Flag | Effect |
| --- | --- |
| `--check` | check each deck is a fixpoint: written in its own dialect and read back, it writes the same text |
| `--dialect hspice` | read HSPICE decks (default: ngspice; also ltspice, xyce, spectre, vacask) |

A PDK model file converts the same way. Its library sections (SPICE
`.lib tt` ... `.endl tt`, Spectre and VACASK `section tt` ... `endsection`,
in Spectre bare or inside a `library`) are read with what each holds (its
`.param`s, models, `.subckt`s, `.include`s and `.lib` calls), and
are written back as sections in every dialect that has them: ngspice, HSPICE,
Xyce and ESPice as `.lib`/`.endl`, Spectre and VACASK as `section` (inside
its `library` when it had one). SPICE bins (`nch.1`, `nch.2`) become a Spectre model group and
back. LTspice reads `.lib tt` as a file, so it has no section form and is
skipped.

Monte Carlo statistics convert too.
SPICE's `agauss`, `gauss`, `aunif`, `unif` and `limit` reach Spectre as
`rack_*` functions of normalized variables varied in a `statistics` block
(`process` at a file's or section's top, `mismatch` inside a subcircuit), and
read back as the calls; VACASK writes them as they are (`limit` spelled out).
A Spectre `statistics` block is read and written to SPICE as
`agauss`/`aunif` draws. LTspice has none of the functions and refuses them.

## Using the Rust crate

```toml
[dependencies]
spicerack = { git = "https://github.com/OpenFAT/SpiceRack", default-features = false, features = ["backend"] }
```

| Feature | Adds |
| --- | --- |
| (none) | the transpiler (`syntax`: read any dialect, write any other, coded diagnostics) and the `.raw` readers; no I/O, builds for `wasm32-unknown-unknown` |
| `backend` | running simulators, Verilog tools, backend checks |
| `python` (default) | the Python bindings; implies `backend` |

`spicerack::syntax`'s module notes show how to load a kit, write a file in
another dialect and read its diagnostics.
