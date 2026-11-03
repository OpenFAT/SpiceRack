# Testing

The map of SpiceRack's tests: which tier runs what, and the oracle each test
trusts. The rules behind it are in `AGENTS.md` (Testing).

## Tiers

| Tier | Runs | Command | Where |
|---|---|---|---|
| pre-commit | format, lints, house style | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && python3 tools/check_style.py src tests tools` | local; the same three run as the `lint` job in `.github/workflows/test.yaml` on every PR |
| merge | both suites in the testing shell | `nix develop .#testing -c cargo test`, then `maturin develop && python3 -m pytest` in a `--system-site-packages` venv | `.github/workflows/test.yaml`, every PR and every push to `main` |
| docs | the site builds strictly; every Python example in it runs | `nix develop .#docs -c mkdocs build --strict`; `tests/test_docs.py` | `.github/workflows/docs.yaml`; pytest |
| release | tag matches `Cargo.toml` and `pyproject.toml`, wheels for every target, sdist | `tools/bump_version.py`, then the release workflow | `.github/workflows/tags.yaml`, `publish.yaml` |
| nightly | the spice-bench and corpus import suite (`--features corpus`) | `nix develop .#testing -c cargo test --no-default-features --features corpus --release --test test_import_suite` | `.github/workflows/nightly.yaml`, 03:17 UTC daily and on demand |

`tools/check_style.py` is vendored from the `code-style` skill
(https://github.com/OmarSiwy/NixOSConfig/tree/main/skills, sha256 `55f417592f6e7ec3…`).
`python3 tools/check_style.py --self-test` checks the checker.

## Oracles

Every test names the independent thing that says the output is wrong. A netlist
is checked by running it on the real simulator or against a golden with
provenance, never by re-deriving it the way codegen does.

### Rust (`tests/`, and `#[cfg(test)]` in `src/`)

| File | Oracle |
|---|---|
| `test_ngspice_analyses.rs` | real ngspice; each analysis asserts a closed-form number (`.sp`: a matched T pad). `.pss`: ngspice-45 has none, so its card against the ngspice manual example (§11.3.12) |
| `test_espice_analyses.rs` | real ESPice; the same closed forms as the ngspice suite |
| `test_laplace_ngspice.rs` | real ngspice 44.2; closed-form Laplace responses |
| `test_sources/` | real ngspice (and ESPice) against the waveform closed forms: PWL `r=`/`td=`; TRNOISE/TRRANDOM against their distribution, five standard errors at N = 4000; each dialect without a form refuses. Imported ngspice PWL `r=`/`td=`, `portnum`/`z0` and `.sp`: the original deck and the deck written from its IR on real ngspice, every vector of every plot equal to 1e-12. Imported Spectre PWL `delay`/`pwlperiod`: the Spectre Reference 5.0 semantics (`vsource` #4, #17, #18, p.686) as the closed form, on real ngspice running the deck written from the IR, to 1e-9 V |
| `test_vacask_runs/` | real VACASK: the emitted deck parses and writes a raw file with the expected numbers |
| `test_dialects.rs` | one circuit in every dialect, read back by ESPice (`--tokenizer=<d>`), same numbers; the dialects it cannot do yet are named in `ESPICE_REFUSES` |
| `test_import_suite/` | spice-bench decks (`SPICE_BENCH`, a pinned nix fetch): parse, JSON round trip, re-emitted as ngspice, every plot and `.meas` reproduced by real ngspice; decks ngspice itself cannot run are named in `tests/fixtures/spice_bench_known_failures.txt`, decks past the CPU budget in `spice_bench_slow.txt`. Parse/emit fixpoint over the ngspice, HSPICE, Xyce, VACASK and LTspice corpora (`*_CORPUS`, pinned in `flake.nix`) |
| `test_conditionals.rs` | conditional blocks and functions: the hand-written fixtures (`tests/fixtures/conditionals/`: a Spectre kit section with `real` functions and a structural `if`/`else if`/`else` chain, multi-line conditions, nesting, an unbraced branch; ngspice's `.if`/`.elseif`/`.else` forms) counted by hand; fixpoints in ngspice, Spectre, VACASK; real ngspice running the ngspice deck written back (and its HSPICE text in `hsa` mode) bit-identical to the original, and the Spectre kit written as ngspice to each branch's closed-form resistance |
| `test_monte_carlo.rs` | SPICE statistical calls and Spectre `statistics` (`docs/research/monte-carlo-spectre.md` §10.6): T-G1 to T-G10 against the note's hand-worked Spectre text; T-R1 SPICE -> Spectre -> SPICE gives every call back (sky130, gf180mcu slices); T-R2 the Spectre table read back equal; T-R3, T-R4 the table's SPICE lowering; T-R5 `vary` lines equal the call sites counted in the source; the one-layer lint; the `tests/spectre/` decks current. T-N0, T-N1 (`src/codegen/statistics/tests.rs`): real ngspice, N=2000, the lifted-then-lowered deck and the original against the closed forms (mean within 4σ/√N, std within 4σ/√(2N), inter-instance ρ). T-S1 to T-S3: `tests/spectre/run.py` on a real Spectre (not installed here), expectations in `tests/spectre/expected.json` |
| `test_sections.rs` | library sections: the hand-written PDK-like fixtures (`tests/fixtures/pdk_sections/`: SPICE `.lib` corners, a Spectre `library`, bare Spectre sections with a `simulator lang=spice` region) counted by hand; read -> write -> read fixpoints in ngspice, HSPICE, Xyce, ESPice, Spectre and VACASK; SPICE <-> Spectre trips compared by meaning, the dialect differences listed in the test; real ngspice and ESPice running each corner from the written sections, bit-identical to the fixture's run |
| `test_sections_pdk.rs` | real PDK files: section names against a line scan, fixpoints; real ngspice on gf180mcu's `sm141064.ngspice` (nothing raw: subcircuit `.param`s are locals) and real VACASK on its `demo/cmos180n.lib`, written by SpiceRack, bit-identical to the original's run; gf180mcu's sections SPICE -> Spectre -> IR by meaning or refused by named reason; sky130's combined library and the ~190 files it reaches, nothing unparsed or raw, written back as ngspice and as HSPICE and run on `tt` by ngspice bit-identical to the PDK's own files; ESPice `--tokenizer=hspice` on the HSPICE tree, a named known gap (+5.1%) |
| `test_spectre_golden/` | goldens; Spectre is licence-gated, so the text is checked against the Spectre reference manual |
| `test_ltspice_dialect.rs` | emitted text against the LTspice help; no LTspice binary, so unverified by execution |
| `test_backends.rs`, `test_unit.rs` | spec examples |
| `src/ir` (`schema_check.rs`) | the JSON Schema 2020-12 spec: an IR with every field set, every component and analysis variant, the shipped example and decks imported from ngspice, Spectre and VACASK validate against `schema/circuit-ir.schema.json`; the checker panics on any keyword it does not implement, and rejects one counterexample per keyword |

### Python (`tests/*.py`)

| File | Oracle |
|---|---|
| `test_bench_*.py`, `test_characterize.py`, `test_run_corners.py` | ngspice on behavioural DUTs with closed-form answers |
| `test_remeasure.py` | closed forms, never the code's own method |
| `test_espice_backend.py`, `test_vacask_backend.py` | the real simulator's numbers for the emitted deck |
| `test_docs.py`, `test_python_examples.py` | the docs and examples run, and print the numbers they claim |
| `test_python_examples.py::test_20_pdk_hotswap`, `test_21_combined_hotswap` | real ngspice on the pinned sky130 and gf180mcu PDKs (`nix/pdks.nix`, `PDK_ROOT`): V(vout) against goldens recorded with ngspice-45 and ESPice 0.3.0, at 1e-3 relative (ngspice's default RELTOL); ESPice's sky130 inverter against ngspice's at the same tolerance; 0 < V(vout) < VDD |
| `test_python_simulator.py::TestNgbehavior` | ngspice's own "Compatibility modes selected" note (Rust: `test_ngspice_analyses.rs`); a `.spiceinit` whose `shell touch` marks it read, in the working and home directories |
| `test_python_*.py`, `test_design_testbenches.py` | API contract: spec examples per method |
| `test_bump_version.py` | a copy of the manifests, each bump kind and each refusal |

## Known gaps

`nix develop .#testing` provides every simulator and corpus the tests read
(ngspice, VACASK, ESPice built from source, the spice-bench suite, the
corpora, and the sky130 `8afc834` and gf180mcu `1e35547` PDKs as `PDK_ROOT`,
pinned by content hash in `nix/pdks.nix`); outside it those tests fail, they
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
  read, and codegen refuses the B source for Spectre.
