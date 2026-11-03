# Tools and the Rust crate

## spice2rack: convert a deck to every dialect

`spice2rack` reads existing SPICE decks and writes them out for every backend,
plus the circuit as JSON and, optionally, as SpiceRack Python.

```bash
cargo run --features json --bin spice2rack -- -o out/ deck.sp [more.sp ...]
```

For `deck.sp` it writes `out/deck.json` and `deck.ngspice.sp`,
`deck.hspice.sp`, `deck.ltspice.cir`, `deck.spectre.scs`, `deck.vacask.sim`
and `deck.espice.sp`. A dialect that cannot express the circuit is reported and
skipped; the rest are still written.

| Flag | Effect |
| --- | --- |
| `--python` | also write `deck.py`, the circuit as SpiceRack Python; constructs the API cannot express go through `raw_spice()` under a `# gap:` comment |
| `--check` | run the generated Python and compare its netlist with the converted deck, and check each deck reads back unchanged in its own dialect |
| `--dialect hspice` | read HSPICE decks (default: ngspice) |

A PDK model file converts the same way. Its library sections (SPICE
`.lib tt` ... `.endl tt`, Spectre and VACASK `section tt` ... `endsection`,
in Spectre bare or inside a `library`) are read into the IR's `sections`, each
holding its `.param`s, models, `.subckt`s, `.include`s and `.lib` calls, and
are written back as sections in every dialect that has them: ngspice, HSPICE,
Xyce and ESPice as `.lib`/`.endl`, Spectre and VACASK as `section` (inside
its `library` when it had one). SPICE bins (`nch.1`, `nch.2`) become a Spectre model group and
back. LTspice reads `.lib tt` as a file, so it has no section form and is
skipped.

Monte Carlo statistics convert too (`docs/research/monte-carlo-spectre.md`).
SPICE's `agauss`, `gauss`, `aunif`, `unif` and `limit` reach Spectre as
`rack_*` functions of normalized variables varied in a `statistics` block
(`process` at a file's or section's top, `mismatch` inside a subcircuit), and
read back as the calls; VACASK writes them as they are (`limit` spelled out).
A Spectre `statistics` block is read into the IR and written to SPICE as
`agauss`/`aunif` draws. LTspice has none of the functions and refuses them.

## The JSON circuit format

Every circuit has a JSON form (`to_json()`, `save_json()`, `load_json()`, see
[Circuits](circuits.md#saving-circuits)). Its schema is
[`schema/circuit-ir.schema.json`](https://github.com/OpenFAT/SpiceRack/blob/main/schema/circuit-ir.schema.json),
so other tools can generate circuits for SpiceRack without Python.

## Using the Rust crate

```toml
[dependencies]
spicerack = { git = "https://github.com/OpenFAT/SpiceRack", default-features = false, features = ["backend"] }
```

| Feature | Adds |
| --- | --- |
| (none) | the circuit IR, the netlist generators and the `.raw` readers; no I/O, builds for `wasm32-unknown-unknown` |
| `backend` | running simulators, Verilog tools, backend checks |
| `json` | IR to and from JSON |
| `python` (default) | the Python bindings; implies `backend` and `json` |
