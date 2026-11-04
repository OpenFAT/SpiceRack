# Phase 4 (renderers): status

Phase 4 of the rewrite (TESTING.md, "Golden gate"): `src/syntax/render/`
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
| CDL | `render::spice` (a row: ngspice cards in the xschem layout, `spice/cdl.rs`) | IDENTICAL | `codegen::xschem`: kept for the Python path and Verilog |
| Verilog | not ported: `golden record --writer syntax` writes it with `codegen::netlist` (see "Verilog" below) | n/a (old path; IDENTICAL) | `codegen::xschem` |
| Spectre | `render::named` | IDENTICAL (3,538 written, 11,400 refused) | `codegen::spectre`, `codegen::statistics::lift`: kept, as ngspice's |
| VACASK | `render::named` (its own spine, `named/vacask*.rs`) | IDENTICAL (the one flip against the raw baseline is the listed phase-3 `top.scs vacask`) | `codegen::vacask`, `codegen::statistics::{lower, vacask_calls}`: kept, as ngspice's |

No written-versus-refused flip on any SPICE or named target. The refusal text report
(`--refusal-status-only <file>`) pairs each old free-text refusal with its
`W` codes: 26,368 pairs, all the same status.

Every input dialect reaches every SPICE and named target through the new reader
(`render::read`), SPICE and named alike (Spectre, VACASK: `read/named*.rs`).

The last `--writer syntax` record is `/tmp/claude-1000/spicerack-golden/p4s-6`
(the statistics lift, the Laplace rule, the role frame, the name map and
CDL; `check-determinism --writer syntax` IDENTICAL; the merge tier 460
tests), the baseline `/tmp/claude-1000/spicerack-golden/baseline-daa0ad0`.
The named writer's: `/tmp/claude-1000/spicerack-golden/rb3` (branch
`render-named`, rebased on `cd4e498`), against `baseline-daa0ad0-fx`
(14,945 inputs with the seven fixtures): every target IDENTICAL (the 26
listed xfails), `check-determinism --writer syntax` IDENTICAL. The fixtures
reach Spectre and VACASK too: `test_render.rs` writes them there against
`codegen::netlist` and pins both.

The six T-S1..T-S3 decks real Spectre 21.1 ran (`tests/spectre/`) are
written byte for byte by `render::named` from their SPICE sources, plus
their `mc1 montecarlo` statement, which no SPICE source holds
(`named::lift`'s `the_decks_spectre_ran_are_written_byte_for_byte`); the
statistics tables reach Spectre (kept) and VACASK (one-layer draws) as
`codegen::netlist` writes them (`test_render.rs`).

The corpus holds no deck the statistics lift or the Laplace rule writes
(each Spectre deck with a `statistics` block is refused first for its
`montecarlo` line; no deck has a Laplace source). Seven made-up fixtures
cover them (`tests/fixtures/transpiler/stats_*.scs`, `laplace.{sp,cir,net}`,
README.md): `tests/test_render.rs` writes each with the renderer and the old
writer on all five SPICE targets and CDL, asserts them equal and pins them
(`pinned/`); it also checks inline decks (the T-R3/T-R4 tables, a 3x3
correlation, mismatch in several subcircuits, 21 Laplace expressions).

The fixtures are golden inputs too, which `baseline-daa0ad0` lacks. The gate
for them is an extended baseline: `baseline-daa0ad0` hard-linked, plus the
old writer's record (`golden record`, no `--writer`) of the inputs it lacks,
their `input`/`output` lines and the `set fixtures` count taken from that
record's MANIFEST (`/tmp/claude-1000/spicerack-golden/baseline-daa0ad0-fx`).
`golden diff baseline-daa0ad0-fx <record --writer syntax>` then compares the
new writer with the old one on them; against the plain baseline the same
record differs only in those inputs (MANIFEST `+` lines, absent files).

`func_stats.{sp,cir}` (the inlined `.func`s) extend it the same way:
`baseline-daa0ad0-fx9` is `baseline-daa0ad0-fx` hard-linked (`cp -al`), its
MANIFEST with `set fixtures 27` and the two inputs' `input`/`output` lines,
and their directories, all from an old-writer record (`golden record`, no
`--writer`) at `b05f4a0` plus the fixtures; that record's other MANIFEST
lines equal the baseline's. Against it the `--writer syntax` record is
IDENTICAL (the 26 listed xfails) and `check-determinism --writer syntax`
IDENTICAL; the merge tier 466 tests.

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
- `render/spelling.rs`: the `Spelling` row per target (8 rows: the SPICE
  family, CDL, Spectre, VACASK): comment leader, path quote, expression
  delimiters, function form (`Real` for Spectre, `None` for VACASK),
  `PARAMS:`, ngspice arithmetic; letters from the lexicon's device table
  (primitive → letter; test: inverts `device::find`); numbers from the
  lexicon's suffix table read in reverse (test: reads back to the same
  double, and equals `codegen::format_spice_number`).
- `render/capability.rs`: `const fn action(target, Construct) -> Action`, 29
  constructs × 9 targets, one `match`, no `_` arm. `Action::{Native,
  Spell(Alt), Lift(Rule), Refuse(Code)}`; 13 `Alt`s, 6 `Rule`s.
- `syntax/diag.rs`: W0001–W0020 added (W0019 a `vary` of an undeclared
  parameter, W0020 a correlation matrix that is not positive definite);
  W0101–W0109 the named writer's (numbered apart while the two branches
  landed separately).
- `render/read/`: the reader. A port of `syntax::adapter`'s rules onto a
  plan of node ids (`Deck`, `Block`: which statement lands in which block and
  bucket, as the old reader placed it) and per-statement views (`Card`,
  `Val`, `Wave`, `BenchCard`, `Model`, `Head`), computed when written. The
  name map (`Rd::masters`: an `X` card's master after the hoisted
  `parent__name` renames) is the only made-up name table. `unlift.rs` is the
  Monte Carlo read-back as fixups over `Call` nodes (`Rd::etext` spells a
  value with them); no byte scanner.
- `render/spice/`: the SPICE-family writer: spine (`mod.rs`), cards
  (`cards.rs`), testbench (`bench.rs`), deck-wide refusals (`checks.rs`),
  the Laplace rule (`laplace.rs`), CDL's xschem layout (`cdl.rs`).
- `render/draws.rs`: the statistics lift, shared: SPICE's two-layer draws,
  VACASK's one-layer ones (`Layers`).
- `render/names.rs`: the name map, shared: each element's declaration
  (its type letter, then its name); the named writer spells the letter
  lowercase and escapes the name after it, as the old writers did.
- `render/named/`: the named-family writer: the Spectre spine (`mod.rs`),
  its instances, waveforms and models (`cards.rs`), its testbench
  (`bench.rs`: the fold both targets share), the deck-wide refusals both
  share (`checks.rs`), the Monte Carlo rules (`lift.rs`), the VACASK spine
  (`vacask.rs`), instances (`vacask_cards.rs`) and `control` block
  (`vacask_bench.rs`). The two decks share the plan, the views, the checks,
  the fold, the lift's walk, the draws and the frame; their spines differ
  (VACASK loads a module and declares a model per device kind and writes
  one `control` block), so VACASK is a spine of its own, not a row. Its
  declarations are the name map's, spelled as the old writers did (a
  lowercase letter, then the name escaped: Spectre `\`, VACASK `'…'`);
  its references (`probe=`, `ind1=`, `iprobe=`) are as written, as the
  old writers spelled them (P7-4..P7-6).
- `render/role.rs`: the file role rule (`frame(Role) -> Frame`: title,
  `.end`, control blocks, ground); `Rd::role` is the kit's answer, or
  `resolve::role` for a file read alone. A writer asks it once and keeps
  the old rules beside it as quirks (Phase 7 list).
- Fixups a writer's `Lift` records in the reader: one table keyed by token
  (`read::Fix`), applied by `Rd::espan`/`Rd::etext` beside the read-back's
  calls: `Fix::Respell` (`Rd::respell_token`: a value token spelled as
  other text, the draws' mismatch references) and `Fix::Lift`
  (`Rd::lift_call`: a statistical call rewritten: Spectre's
  `rack_f(args, v)`, VACASK's `limit` ternary and sigma, Xyce's clamp; a
  read-back call lifted again is one fixup). The statistics lift's
  `.param` insertions, replaced values and subcircuit locals are its own
  side table (`draws::Draws`), keyed by node; the Monte Carlo lift's
  variables and `rack_*` functions `named::lift::Lifted`'s.

## Open (in the order the next agent meets them)

1. **Spectre and VACASK writers: done** (`render/named/`). What they
   leave, each a `ponytail:` in the code, none reached by the gate:
   - a `.func` whose body calls a statistical function: **done**, inlined
     on Spectre at each call as the old lift did (T-G8), a script fixup
     per inlined call
     (`Fix::Inline`, `Step`: a body's tokens are spelled once per call, an
     argument's once per use, so one fixup per token cannot say it);
     VACASK refuses any function (`W0106`), as the old writer did;
   - a source's lifted expressions are walked in item order (the old pass:
     the waveform's, then the value; a source with no DC value repeats its
     waveform's first value, which one fixup per token cannot name twice);
     a Spectre model group's own parameters are shared by its bins, so a
     call there would be named once;
   - a view holds a value as SPICE text, so its number is read again
     (`named::spice_num`, the old `parse_spice_num`); `named::checks::calls`
     scans a function body's text for `v()`/`i()` (as item 3).
2. **Verilog** (target 5b) stays on `codegen::xschem` (see "Verilog").
3. **`check_functions` and `^`** (`spice/checks.rs`) scan the kept value
   text, as the old writer did (`calls`, `contains('^')`, a `ponytail:`);
   over `Call` nodes and the operator tokens they would be a walk of the
   value trees, with the same verdict on the corpus to be checked.
4. **Seven capability cells reproduce a silent drop** (`// quirk:` in
   `capability.rs`): LTspice drops a Verilog-A/OSDI load (`Spell(Omit)`,
   two cells); CDL copies another family's text (`ForeignText`: `Native`);
   Spectre and VACASK write a sectionless `.lib` as `include "f"
   section=` and drop a `.print` (two cells each). Phase 7 decides.
5. The reader duplicates `syntax::adapter`'s rules (copied, then typed to
   views). Phase 6 deletes the adapter; the rules live here after it.
6. `Spelling` holds what the six rows differ in; the dot-card keywords the
   five share are literals in `spice/`. The named rows add them
   (`model`/`.model`, `subckt`/`.subckt`, `parameters`/`.param`).

## Verilog

The old layout writes every element of a Verilog module that is not an
instance as `// error: Unsupported component for backend verilog: {c:?}`:
the `Debug` text of the old IR's `Component` (3.4 M such lines across the
corpus's 14,936 Verilog outputs: `Resistor { name: "2", n1: "2", …, value:
Numeric { value: 1000.0 }, … }`). A writer reading the plan would have to
print the IR's field names and Rust's float `Debug` to keep those bytes,
i.e. rebuild the IR it replaces. Verilog therefore stays on `codegen::xschem`
until that line becomes a coded comment (`// error: W00xx <title>`, a
Verilog-wide xfail class, Phase 6 or 7); its instances and module shape
then port as CDL did (a layout row of the SPICE writer: `push_instance`'s
named ports, `wire`s, escaped identifiers).

## Phase 7 list (behaviour changes the gate holds back)

Each is a `// quirk:` in the code, the old bytes kept in this phase.

**The file role** (`role::frame` against the old rules; `spice/mod.rs`
`W::frame`, `spice/cdl.rs`):
- P7-1. A library (`Role::Library`) is written with a title line and a
  closing `.end` on every SPICE target, and its Verilog-A loads in an
  ngspice `.control` / `pre_osdi` block; the role writes it bare. Every
  PDK kit file the gate reads is one (sky130, gf180mcu, IHP, VACASK's
  `cmos180n.lib`), and every corpus deck with no analysis, control block or
  top-level source.
- P7-2. HSPICE: a deck (`Role::Deck`) with no analysis (sources alone, or
  a control block) ends without `.END`, the old writer's guess at a
  library; the role ends it. (A library with no analysis agrees.)
- P7-3. CDL: the layout titles and ends (`.END`) a library too.

**The name map** (`render/names.rs`: a declaration gains its letter, a
reference does not):
- P7-4. Spectre `cccs`/`ccvs` `probe=`: the controlling source as written
  (`in1`), its declaration `Vin1` (`read/named_elements.rs`).
- P7-5. Spectre `save dev:p` is `i(dev)` as written (`i(in1)`, not
  `i(Vin1)`); `save main:d` (any other terminal) is kept as written, which
  no SPICE output names (`read/named_bench.rs`).
- P7-6. Spectre `dc dev=` and `noise iprobe=` name the source as written
  (`read/named_bench.rs`).
- P7-7. HSPICE writes a `B` source as `E1`/`G1` (`vol=`/`cur=`), and the
  Laplace lift a `B1` as `EB1`/`GB1`/`AB1` (+ `.model B1_xfer`): renames of
  the declaration alone, which a reference (`i(B1)`) does not follow and
  which may meet a deck's own `E1` (`spice/cards.rs`).
- P7-8. CDL writes an instance with the old IR's letter rule (`Xx2` is
  `x2`), not the map's `Xx2` (`spice/cdl.rs`).
- SPICE input: a declaration's letter is uppercased (`vsense` is
  `Vsense`), a reference keeps its case: harmless (SPICE folds case), no
  fix needed. `v1` → `vv1` cannot arise in the SPICE writer: the map adds a
  letter only to a name that does not start with it; it is the SPICE →
  Spectre direction's (the named writer's) case.

- P7-11. Spectre and VACASK title a library too, and VACASK gives it
  `ground 0` and a `control` block (`named::W::frame`).

**CDL** (`spice/cdl.rs`):
- P7-9. Another family's text is copied as written (`ForeignText`:
  `Native`, a capability quirk).
- P7-10. Subcircuit models, sections, conditionals, includes and functions
  are not written (the LVS layout's old shape), with no diagnostic.

## Differences from the old pipeline the corpus does not reach

No golden input meets these; `tests/test_render.rs` pins the first three.
- A `statistics` block inside a subcircuit or a branch is refused (W0018),
  on the SPICE targets and VACASK; the old writers dropped it unwritten.
- A `vary` of an undeclared parameter is W0019, a correlation matrix that is
  not positive definite W0020 (the old free-text refusals; same status).
- A Laplace number is the token's value (decoded once by the lexicon), not
  `unit::parse_spice_number` of its text; `-s^2` is read as the AST's
  precedence gives it.
- A CDL element the ngspice card refuses is `* error: <W code> <title>`; the
  old layout wrote its free-text error.
- A later mismatch rename does not reach the draws an earlier one added to
  the same subcircuit (the old `rename_ident` scanned them too: a `std=`
  naming another mismatch parameter).

## Decisions taken

- **CDL is a row of the SPICE writer.** Its elements were already ngspice
  cards (`codegen::card(c, Ngspice)`); the xschem layout around them is
  `spice/cdl.rs` (one `.SUBCKT` per block, elements sorted by name).
- **The statistics lift and the Laplace rule produce fixups, never text
  scans**: the draws are a side table keyed by `.param` and subcircuit
  node, a mismatch reference a respelled token; `H(s)` is evaluated over
  its tree on a heap stack.

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
