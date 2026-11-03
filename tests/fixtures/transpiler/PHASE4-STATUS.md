# Phase 4 (renderers): status

Phase 4 of `docs/research/transpiler-architecture.md` §5: `src/syntax/render/`
writes the AST (and its `resolve::Kit`) in any dialect, gated byte for byte
against the `daa0ad0` baseline (TESTING.md, "Golden gate").

## Per target

| Target | Writer | Gate (`diff --refusal-status-only`) | Old codegen module |
|---|---|---|---|
| ngspice | `render::spice` | IDENTICAL (26 listed xfails, all pre-phase-4) | `codegen::spice3`: kept, `codegen::netlist` (the Python API's CircuitIR path) calls it |
| ESPice | `render::spice` (a `Spelling` row) | IDENTICAL | as ngspice |
| HSPICE | `render::spice` (a row) | IDENTICAL | as ngspice |
| Xyce | `render::spice` (a row) | IDENTICAL | as ngspice |
| LTspice | `render::spice` (a row) | IDENTICAL | as ngspice |
| Spectre | not started: `golden record --writer syntax` writes it with `codegen::netlist` | n/a | `codegen::spectre`, `codegen::statistics::lift` |
| VACASK | not started: as Spectre | n/a | `codegen::vacask`, `codegen::statistics::{lower, vacask_calls}` |
| CDL, Verilog | not started: the xschem layout writer | n/a | `codegen::xschem` (reads `CircuitIR` through `xschem::Deck`; it also lowers through `codegen::spectre::emit_component`) |

No written-versus-refused flip on any SPICE target. The refusal text report
(`--refusal-status-only <file>`) pairs each old free-text refusal with its
`W` codes: 26,368 pairs, all the same status.

Every input dialect reaches every SPICE target through the new reader
(`render::read`), SPICE and named alike (Spectre, VACASK: `read/named*.rs`).

The last `--writer syntax` record is `/tmp/claude-1000/spicerack-golden/p4`
(at `479c30c`), the baseline `/tmp/claude-1000/spicerack-golden/baseline-daa0ad0`.

## Run the gate

```sh
export CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=/tmp/claude-1000/spicerack-transpiler-target
cargo build -j 4 --release --no-default-features --features json --bin golden
G=$CARGO_TARGET_DIR/release/golden
nix develop .#testing -c env IHP_PDK=/home/omare/pdks/IHP-Open-PDK $G record --writer syntax <out>
$G diff <baseline-daa0ad0> <out> --xfail migration.xfail --refusal-status-only <report>
nix develop .#testing -c env IHP_PDK=... $G check-determinism --writer syntax   # IDENTICAL
```

`render::writes(target)` names the targets `--writer syntax` renders; the
others are recorded through `codegen::netlist`, so the whole tree diffs.

## Shape

- `render/mod.rs`: `render(&Source, target)`, `render_kit(kit, file, name,
  target)`, `render_text(text, d, name, target)` → `Result<String,
  Vec<Diag>>`; `writes(target)`.
- `render/out.rs`: the one token choke point (`Out::token(text, Space)`), the
  stream owns indentation (`indent`/`dedent`, used by the named writer).
- `render/spelling.rs`: the `Spelling` row per target (5 rows: the SPICE
  family): comment leader, path quote, expression delimiters, function form,
  `PARAMS:`, ngspice arithmetic; letters from the lexicon's device table
  (primitive → letter; test: inverts `device::find`); numbers from the
  lexicon's suffix table read in reverse (test: reads back to the same
  double, and equals `codegen::format_spice_number`).
- `render/capability.rs`: `const fn action(target, Construct) -> Action`, 28
  constructs × 9 targets, one `match`, no `_` arm. `Action::{Native,
  Spell(Alt), Lift(Rule), Refuse(Code)}`; 10 `Alt`s, 4 `Rule`s.
- `syntax/diag.rs`: W0001–W0018 added.
- `render/read/`: the reader. A port of `syntax::adapter`'s rules onto a
  plan of node ids (`Deck`, `Block`: which statement lands in which block and
  bucket, as the old reader placed it) and per-statement views (`Card`,
  `Val`, `Wave`, `BenchCard`, `Model`, `Head`), computed when written. The
  name map (`Rd::masters`: an `X` card's master after the hoisted
  `parent__name` renames) is the only made-up name table. `unlift.rs` is the
  Monte Carlo read-back as fixups over `Call` nodes (`Rd::etext` spells a
  value with them); no byte scanner.
- `render/spice/`: the SPICE-family writer: spine (`mod.rs`), cards
  (`cards.rs`), testbench (`bench.rs`), deck-wide refusals (`checks.rs`).

## Open (in the order the next agent meets them)

1. **Spectre writer** (target 3). Port `codegen::spectre` (mod, components,
   analyses, values, names, library) onto the plan as `render/named/` (the
   named structural writer), plus a `Spelling` row each for Spectre and
   VACASK. It needs the forward Monte Carlo lift (`codegen::statistics::lift`:
   `agauss(…)` → `rack_agauss(…, v)` + `parameters v=0` + a `statistics`
   block) as a `Lift` rule: write it as fixups over `Call` nodes, as
   `read/unlift.rs` does the reverse. The reader's `Section::library`,
   `Opt::Vacask` and `Out::{indent, dedent}` / `Space::Blank` are there for
   it (`expect(dead_code)` until then).
2. **VACASK writer** (target 4): `codegen::vacask` and
   `statistics::{lower(…, true), vacask_calls}`.
3. **CDL, Verilog** (target 5): `codegen::xschem` writes from `CircuitIR`
   through `xschem::Deck`, and borrows `spectre::emit_component`. Decide
   whether a third structural writer is natural; else keep it on the
   adapter → `CircuitIR` → old writer and say so.
4. **The statistics lowering for SPICE targets** (`Lift(StatisticsDraws)`,
   `codegen::statistics::lower`) is not ported: a Spectre deck whose
   `statistics` block survives the read-back is refused with `W0018`
   (`spice/checks.rs`, a `ponytail:`). No corpus deck reaches it (each such
   fixture is refused first for its `montecarlo` line). Port it as fixups:
   parameter replacements and insertions, `__mm` renames inside the
   subcircuits that read a mismatch parameter.
5. **Laplace** (`spice/cards.rs`) still calls `codegen::laplace::{is_laplace,
   parse, horner, coeff_array}` on the B source's text. The spec wants it a
   `Lift` over the expression tree; no corpus deck has one, so the unit
   tests (`tests/test_laplace_ngspice.rs` builds IR) are its only oracle:
   port it with a test against `codegen::laplace` on sample expressions.
6. **`check_functions` and `^`** (`spice/checks.rs`) scan the kept value
   text, as the old writer did (`calls`, `contains('^')`, a `ponytail:`);
   over `Call` nodes and the operator tokens they would be a walk of the
   value trees, with the same verdict on the corpus to be checked.
7. **Three capability cells reproduce a silent drop** (`// quirk:` in
   `capability.rs`): LTspice drops a Verilog-A/OSDI load (`Spell(Omit)`).
   Phase 7 decides (Refuse).
8. The reader duplicates `syntax::adapter`'s rules (copied, then typed to
   views). Phase 6 deletes the adapter; the rules live here after it.
9. **The file role** (`resolve::Role`) does not yet decide the title, `.end`
   and control block: the SPICE writer keeps the old rules (a title always;
   HSPICE drops `.end` when the deck has no analysis), which the gate holds.
   Moving them to `Role` changes bytes wherever the guess and the role
   disagree: a phase-7 behaviour change with its own xfail lines, or a
   decision for the named writer, whose VACASK library rules are the case
   the spec names.
10. **The name map** holds the hoisted-subcircuit renames (`parent__name`)
    of `X` masters. The SPICE letter policy is still the old reader's (the
    letter stripped on read, `Card::*::name`, and prefixed on write), so a
    Spectre `v1` reaches SPICE as `V1` and a reference to it (`probe=v1`)
    is spelled by the same `strip`; making the letter a rename in the map
    (declaration and every reference together) is the named writer's
    first use of it.
11. `Spelling` holds what the five SPICE rows differ in; the dot-card
    keywords the five share are literals in `spice/`. The named rows add
    them (`model`/`.model`, `subckt`/`.subckt`, `parameters`/`.param`).

## Decisions taken

- **The plan, not a lowering.** The old output reorders a deck (models
  before elements, raw lines after them, hoisted subcircuits, merged
  options); byte identity needs the old reader's placement. The plan holds
  node ids per block and bucket; every word is spelled from tokens when
  written (views), so nothing is a copy of the deck.
- **Refusals are `Vec<Diag>`** (code and token), written as `<code>
  <title>` lines in `<target>.refused` under `--writer syntax`. Phase 5
  brings messages.
- **Numbers**: `Spelling::number`/`decimal` reproduce `num`/`dec`
  (`unit::ngspice_spelling` for a deck read with ngspice arithmetic into an
  ngspice reader); `Spelling::format` is `format_spice_number` built on the
  target lexicon's suffix table.
