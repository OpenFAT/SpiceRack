# Testing

The map of SpiceRack's tests: which tier runs what, and the oracle each test
trusts. The rules behind it are in `AGENTS.md` (Testing).

## Tiers

| Tier | Runs | Command | Where |
|---|---|---|---|
| pre-commit | format, lints, house style | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && python3 .github/scripts/check_style.py src tests` | local; the same three run as the `lint` job in `.github/workflows/test.yaml` on every PR |
| merge | both suites in the testing shell | `nix develop .#testing -c cargo test`, then `maturin develop && python3 -m pytest` in a `--system-site-packages` venv | `.github/workflows/test.yaml`, every PR and every push to `main` |
| docs | the site builds strictly; every Python example in it runs | `nix develop .#docs -c mkdocs build --strict`; `tests/test_docs.py` | `.github/workflows/docs.yaml`; pytest |
| release | tag matches `Cargo.toml` and `pyproject.toml`, wheels for every target, sdist | `.github/scripts/bump_version.py`, then the release workflow | `.github/workflows/tags.yaml`, `publish.yaml` |
| nightly | the spice-bench and corpus import suite (`--features corpus`) | `nix develop .#testing -c cargo test --no-default-features --features corpus --release --test test_import_suite` | `.github/workflows/nightly.yaml`, 03:17 UTC daily and on demand |
| golden | the transpiler rewrite's byte-identical gate against `9d1d6bb` ("Golden gate" below) | `golden record`, then `golden diff <baseline> <new> --xfail migration.xfail`; `golden check-determinism`; `cargo test --features corpus --test test_syntax` | local, before each rewrite phase lands |

`.github/scripts/check_style.py` is vendored from the `code-style` skill
(https://github.com/OmarSiwy/NixOSConfig/tree/main/skills, sha256 `55f417592f6e7ec3…`).
`python3 .github/scripts/check_style.py --self-test` checks the checker.

## Oracles

Every test names the independent thing that says the output is wrong. A netlist
is checked by running it on the real simulator or against a golden with
provenance, never by re-deriving it the way the writer does.

### Rust (`tests/`, and `#[cfg(test)]` in `src/`)

| File | Oracle |
|---|---|
| `test_ngspice_analyses.rs` | real ngspice; each analysis asserts a closed-form number (`.sp`: a matched T pad). `.pss`: ngspice-45 has none, so its card against the ngspice manual example (§11.3.12) |
| `test_espice_analyses.rs` | real ESPice; the same closed forms as the ngspice suite |
| `test_laplace_ngspice.rs` | real ngspice 44.2; closed-form Laplace responses |
| `test_sources/` | real ngspice (and ESPice) against the waveform closed forms: PWL `r=`/`td=`; TRNOISE/TRRANDOM against their distribution, five standard errors at N = 4000; each dialect without a form refuses. Imported ngspice PWL `r=`/`td=`, `portnum`/`z0` and `.sp`: the original deck and the deck written from it on real ngspice, every vector of every plot equal to 1e-12. Imported Spectre PWL `delay`/`pwlperiod`: the Spectre Reference 5.0 semantics (`vsource` #4, #17, #18, p.686) as the closed form, on real ngspice running the deck written from it, to 1e-9 V |
| `test_vacask_runs/` | real VACASK: the emitted deck parses and writes a raw file with the expected numbers |
| `test_dialects.rs` | one circuit in every dialect, read back by ESPice (`--tokenizer=<d>`), same numbers; the dialects it cannot do yet are named in `ESPICE_REFUSES` |
| `test_import_suite/` | spice-bench decks (`SPICE_BENCH`, a pinned nix fetch): parse, re-written as ngspice, every plot and `.meas` reproduced by real ngspice; decks ngspice itself cannot run are named in `tests/fixtures/spice_bench_known_failures.txt`, decks past the CPU budget in `spice_bench_slow.txt`. Read/write text fixpoint over the ngspice, HSPICE, Xyce, VACASK and LTspice corpora (`*_CORPUS`, pinned in `flake.nix`) |
| `test_conditionals.rs` | conditional blocks and functions: the hand-written fixtures (`tests/fixtures/conditionals/`: a Spectre kit section with `real` functions and a structural `if`/`else if`/`else` chain, multi-line conditions, nesting, an unbraced branch; ngspice's `.if`/`.elseif`/`.else` forms) counted by hand; fixpoints in ngspice, Spectre, VACASK; real ngspice running the ngspice deck written back (and its HSPICE text in `hsa` mode) bit-identical to the original, and the Spectre kit written as ngspice to each branch's closed-form resistance |
| `test_monte_carlo.rs` | SPICE statistical calls and Spectre `statistics`: T-G1 to T-G10 against hand-worked Spectre text (each call `rack_f(args, v)`, `v` varied `dist=gauss std=1` or `dist=unif N=1`, `process` at the top and `mismatch` in a subcircuit; Spectre Reference 19.1 pp.175-179); T-R1 SPICE -> Spectre -> SPICE gives every call back (sky130, gf180mcu slices); T-R2 the Spectre table read back equal; T-R3, T-R4 the table's SPICE lowering; T-R5 `vary` lines equal the call sites counted in the source; the one-layer lint. `test_monte_carlo_ngspice.rs` T-N0, T-N1: real ngspice, N=2000, a deck written for Spectre and back and the original, and a Spectre `statistics` table written for ngspice, against the closed forms (mean within 4σ/√N, std within 4σ/√(2N), inter-instance ρ). T-S1 to T-S3: `tests/spectre/run.py` on a real Spectre (not installed here), expectations in `tests/spectre/expected.json` |
| `test_sections.rs` | library sections: the hand-written PDK-like fixtures (`tests/fixtures/pdk_sections/`: SPICE `.lib` corners, a Spectre `library`, bare Spectre sections with a `simulator lang=spice` region) counted by hand; read -> write -> read fixpoints in ngspice, HSPICE, Xyce, ESPice, Spectre and VACASK; SPICE <-> Spectre trips compared by meaning, the dialect differences listed in the test; real ngspice and ESPice running each corner from the written sections, bit-identical to the fixture's run |
| `test_sections_pdk.rs` | real PDK files: section names against a line scan, fixpoints; real ngspice on gf180mcu's `sm141064.ngspice` (nothing raw: subcircuit `.param`s are locals) and real VACASK on its `demo/cmos180n.lib`, written by SpiceRack, bit-identical to the original's run; gf180mcu's sections SPICE -> Spectre -> SPICE by meaning or refused by named reason; sky130's combined library and the ~190 files it reaches, nothing unparsed or raw, written back as ngspice and as HSPICE and run on `tt` by ngspice bit-identical to the PDK's own files; ESPice `--tokenizer=hspice` on the HSPICE tree, a named known gap (+5.1%) |
| `test_spectre_golden/` | goldens; Spectre is licence-gated, so the text is checked against the Spectre reference manual |
| `test_ltspice_dialect.rs` | emitted text against the LTspice help; no LTspice binary, so unverified by execution |
| `test_syntax.rs` | the hand-read `simulator lang=spice` region of `tests/fixtures/pdk_sections/bare.scs`, and one region marker per such line in every Spectre fixture; `kits_resolve`: every PDK kit and fixture as a `resolve::Kit`, its read order the golden walk's, every instance master bound or carrying its `R0001`, the counts printed; `throughput` (`--ignored`) times the stages (tokenize, parse, render) over every golden-gate input |
| `test_render.rs` | the pins `tests/fixtures/transpiler/pinned/`, the old writer's bytes (the pipeline the golden baseline recorded) for decks the corpus lacks: each Spectre `statistics` table (process, mismatch, correlations, sections) and each Laplace source on the five SPICE targets (the tables on Spectre and VACASK too), or the refusal's `W` codes |
| `src/syntax` (`#[cfg(test)]`) | one test per node tag and per header shape, each against its manual clause with the tree derived by hand (`src/syntax/tests/`); the device tables read and written back (`device.rs`); resolve, one test per answer against its clause, from an in-memory loader (`src/syntax/resolve/tests.rs`); 10⁴-deep `.subckt` and `.if` decks through the parser and the renderer in Debug (`src/syntax/tests/render.rs`; the `.if` one on a 256 MiB thread: the writers recurse over branches); structural Verilog (`render/spice/verilog.rs`) against IEEE 1364-2005 §12; the named writer's Monte Carlo rules (`render/named/lift.rs`) against the same hand-worked text (T-G1 to T-G4, T-G9, and VACASK's calls per its `docs/cmd-analysis-mc.md`), and the `tests/spectre/` decks real Spectre 21.1 ran, written byte for byte from their SPICE sources |
| `test_backends.rs`, `test_unit.rs` | spec examples |

### Python (`tests/*.py`)

| File | Oracle |
|---|---|
| `test_bench_*.py`, `test_characterize.py`, `test_run_corners.py` | ngspice on behavioural DUTs with closed-form answers |
| `test_remeasure.py` | closed forms, never the code's own method |
| `test_espice_backend.py`, `test_vacask_backend.py` | the real simulator's numbers for the emitted deck |
| `test_docs.py`, `test_python_examples.py` | the docs and examples run, and print the numbers they claim |
| `test_python_examples.py::test_20_pdk_hotswap`, `test_21_combined_hotswap` | real ngspice on the pinned sky130 and gf180mcu PDKs (`pdks` in `flake.nix`, `PDK_ROOT`): V(vout) against goldens recorded with ngspice-45 and ESPice 0.3.0, at 1e-3 relative (ngspice's default RELTOL); ESPice's sky130 inverter against ngspice's at the same tolerance; 0 < V(vout) < VDD |
| `test_python_simulator.py::TestNgbehavior` | ngspice's own "Compatibility modes selected" note (Rust: `test_ngspice_analyses.rs`); a `.spiceinit` whose `shell touch` marks it read, in the working and home directories |
| `test_python_*.py`, `test_design_testbenches.py` | API contract: spec examples per method |
| `test_bump_version.py` | a copy of the manifests, each bump kind and each refusal |

## Golden gate

The transpiler rewrite is gated byte for byte against the old pipeline. It
landed in phases: 0 the goldens; 1 the tokenizer and lexicons; 2 the parser
and AST, with a temporary adapter to the old IR feeding the old writers
(2b: the PEG reader deleted); 3 resolve (the `Kit`); 4 the renderers, one
target at a time; 5 the diagnostic catalogue; 6 the Python API builds its
decks as ngspice text the parser reads, and the IR, the adapter and the old
writers go; 7 the new guarantees (`write(read(x, d), d) == x`, the `2*1M`
fix, the proprietary-kit fixes), the behaviour changes the gate holds back
until then. `src/bin/golden` records, for every input and every target
dialect, the text `syntax::render` writes from the file's AST and kit, or
the exact refusal; a later phase records again and diffs. Each difference is
a line in `migration.xfail`, which is strict: an unlisted difference fails,
and so does a listed one that is now identical (XPASS). The oracle is
`9d1d6bb`.

```sh
export CARGO_BUILD_JOBS=4
cargo build -j 4 --release --no-default-features --bin golden
G=${CARGO_TARGET_DIR:-target}/release/golden
nix develop .#testing -c env IHP_PDK=<IHP-Open-PDK checkout> $G record <out_dir>
$G diff <baseline> <out_dir> --xfail migration.xfail --refusal-status-only <report> --skip verilog
nix develop .#testing -c env IHP_PDK=<...> $G check-determinism
```

A refusal is its codes, sorted, on a first line (`codes: P0001 W0013`: the
file's parse and kit notes and the writer's codes), then their messages
(`<deck>:<line>:<column>: <code> <title>`); `golden diff` compares two such
files by the code line alone, so a reworded message is no difference. `golden
diff … --refusal-status-only <report>` compares a refusal by status only (the
baseline's texts are free text) and writes each old/new pair to `<report>` for
review; written versus refused still differs. `--skip verilog` leaves Verilog
out: the old layout wrote the old IR's `Debug` text, which the structural
writer (phase 6) does not keep. A baseline's `ir.json` and `report.json`
(the IR, gone in phase 6) are passed over. The old messages beside the codes
they became, once, before the switch:
`tests/fixtures/transpiler/refusal-map.txt`. The extended baseline (the
fixtures `baseline-daa0ad0` lacks, recorded through the old writer) is
`baseline-daa0ad0-fx10` (`PHASE4-STATUS.md`, `PHASE6-STATUS.md`).

Inputs (the `MANIFEST` of each record lists them, with content hashes):

| Set | Read as | Inputs | Source |
|---|---|---|---|
| `spice-bench` | ngspice | 6,050 | `$SPICE_BENCH`; also the ngspice corpus (`NGSPICE_CORPUS` is the same path, the walks keep the same decks) |
| `hspice` | HSPICE | 75 | `$HSPICE_CORPUS` (ESPice conformance) |
| `xyce` | Xyce | 4,068 | `$XYCE_CORPUS` (Xyce_Regression) |
| `vacask` | VACASK | 151 | `$VACASK_CORPUS` |
| `ltspice` | LTspice | 4,342 | `$LTSPICE_CORPUS` (spice-datasets) |
| `pdk-sky130A` | ngspice | 194 | `$PDK_ROOT/sky130A`, entry `libs.tech/combined/sky130.lib.spice` |
| `pdk-gf180mcuD` | ngspice | 3 | `$PDK_ROOT/gf180mcuD`, entry `libs.tech/ngspice/sm141064.ngspice` |
| `pdk-ihp-sg13g2` | ngspice | 36 | `$IHP_PDK/ihp-sg13g2` (not pinned; the baseline used `5e6d592`), entries the 7 `corner*.lib` |
| `pdk-vacask` | VACASK | 1 | `$VACASK_CORPUS/demo/cmos180n.lib` |
| `fixtures` | by extension | 18 | `tests/**/*.{sp,lib,spice,cir,scs,sim,net}`, with `tests/fixtures/transpiler/` |

A kit is walked like Interop's `pdk-models`: the entries, then breadth first
through includes, `.lib` calls and Verilog-A sources. The read order is in
the manifest (`render::references` lists each file's). Since phase 3, each
kit file is read in its kit (`syntax::resolve::Kit`, loaded from the same
entries; `import::kit`), still behind an empty title line as the oracle read it, and each fixture in
its own kit (itself and the files it includes under `tests/`); a corpus deck
is read alone. The kit also follows includes the walk skips (inside a
`.subckt` or an `.if` branch: IHP's three `*_parm.lib`), which are no inputs,
so the manifest stays the baseline's. Every target is written for every
input. The manifest's `skip` lines name the input dialects no set reads
(ESPice reads ngspice text; CDL and Verilog have no reader).

Not covered: the inline decks of `tests/test_spectre_golden` and the
Python API's decks (no input file; the merge tier and pytest run them);
Spectre PDK files (none of the local kits has one).

Measured on a 32-thread, 31 GB development machine, 2026-11-03, at `9d1d6bb`:
- `golden record`, all 14,938 inputs with 4 workers (`GOLDEN_JOBS`): 37.8 s
  wall, 69.7 s user, peak RSS 2.3 GB (`command time -v`, nixpkgs `time`).
  It writes 164,318 files, 3.7 GB on disk (3.1 GB apparent).
- `check-determinism` (two records in two processes, then a diff): 1 min 46 s.
- The simulator-free tier, the corpus fixpoints alone:
  `nix develop .#testing -c cargo test -j 4 --no-default-features --features corpus --release --test test_import_suite -- fixpoint --test-threads=4`
  runs in 64.0 s wall, peak RSS 2.1 GB, once built. `ngspice_corpus_fixpoint`
  sets the wall time.

The PEG reader and its lowering went in phase 2b, the IR, the adapter and
the old writers in phase 6, so the recorded `baseline-daa0ad0` is the only
oracle left of them, and `golden diff` against it is the gate of every
phase.

Measured on a 32-thread, 31 GB development machine, 2026-11-03, release
build, before phase 2b's switch: the phase-1 tokenizer gate read all 14,938
inputs, 3,664,966 logical lines and 8,357,766 number tokens with 0
differences in 25.9 s; `throughput` (`--ignored`): 201.3 MB, 30.1 M tokens,
26.1 M nodes, tokenize 2.36 s (85.3 MB/s), parse 3.44 s (58.5 MB/s). Phase
6's `golden record` (every target through the renderer, Verilog's
structural writer among them): 2 min 7 s wall, 268 s user, 2026-11-04.

```sh
nix develop .#testing -c env IHP_PDK=<...> cargo test -j 4 --release \
  --no-default-features --features corpus --test test_syntax -- --test-threads=4
```

## Known gaps

`nix develop .#testing` provides every simulator and corpus the tests read
(ngspice, VACASK, ESPice built from source, the spice-bench suite, the
corpora, and the sky130 `8afc834` and gf180mcu `1e35547` PDKs as `PDK_ROOT`,
pinned by content hash in `flake.nix`'s `pdks`); outside it those tests fail, they
never skip.

- Three tests load sky130's whole `tt` corner in ngspice: examples 20 and 21
  and the `docs/models.md` blocks (`test_docs.py`). Each takes 34 s on an
  idle 20-thread machine and 49 to 55 s under load (load average 10), about
  2.5 min of the merge tier together. With them, pytest took 198 s and
  `cargo test` 37 s, so the merge tier stays well under its 15 min budget.
- ESPice 0.3.0 refuses gf180mcu's FET subcircuits (the BSIM4 instance
  parameter `mulu0`), so example 21 runs gf180mcu on ngspice alone and its
  ngspice number is checked against the golden only.

- LTspice and Spectre are not installed anywhere SpiceRack is tested, so their
  decks are checked against the manuals only.
- The spice-bench round trip runs only in the nightly tier: 2,586 CPU seconds
  (5 min 25 s wall on a 20-thread machine, 20 min on a 4-vCPU CI runner), past
  the merge budget. A change to `src/import` should run it locally before merging.
- No public Spectre corpus is pinned (ESPice's `spectre_fixtures` are gone
  upstream), so `spectre_corpus_fixpoint` is a known failure
  (`#[should_panic]`).
- ESPice reads back only the HSPICE deck in `test_dialects.rs`: the ngspice
  and ESPice decks spell Laplace as an XSPICE `s_xfer`, which ESPice cannot
  read, and the renderer refuses the B source for Spectre.
