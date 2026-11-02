# Testing

The map of SpiceRack's tests: which tier runs what, and the oracle each test
trusts. The rules behind it are in `AGENTS.md` (Testing).

## Tiers

| Tier | Runs | Command | Where |
|---|---|---|---|
| pre-commit | format, house style | `cargo fmt --check && python3 tools/check_style.py src` | local |
| merge | both suites in the testing shell | `nix develop .#testing -c cargo test`, then `maturin develop && python3 -m pytest` in a `--system-site-packages` venv | `.github/workflows/test.yaml`, every PR and every push to `main` |
| docs | the site builds strictly; every Python example in it runs | `nix develop .#docs -c mkdocs build --strict`; `tests/test_docs.py` | `.github/workflows/docs.yaml`; pytest |
| release | tag matches `Cargo.toml` and `pyproject.toml`, wheels for every target, sdist | `tools/bump_version.py`, then the release workflow | `.github/workflows/tags.yaml`, `publish.yaml` |
| nightly | none yet | | |

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
| `test_ngspice_analyses.rs` | real ngspice; each analysis asserts a closed-form number |
| `test_espice_analyses.rs` | real ESPice; the same closed forms as the ngspice suite |
| `test_laplace_ngspice.rs` | real ngspice 44.2; closed-form Laplace responses |
| `test_vacask_runs.rs` | real VACASK: the emitted deck parses and writes a raw file with the expected numbers |
| `test_dialects.rs` | one circuit in every dialect, read back by ESPice (`--tokenizer=<d>`), same numbers; the dialects it cannot do yet are named in `ESPICE_REFUSES` |
| `test_import_suite.rs` | spice-bench decks (`SPICE_BENCH`, a pinned nix fetch): parse, JSON round trip, re-emitted as ngspice, every plot and `.meas` reproduced by real ngspice; decks ngspice itself cannot run are named in `tests/fixtures/spice_bench_known_failures.txt`, decks past the CPU budget in `spice_bench_slow.txt`. Parse/emit fixpoint over the ngspice, HSPICE, Xyce, VACASK and LTspice corpora (`*_CORPUS`, pinned in `flake.nix`) |
| `test_spectre_golden.rs` | goldens; Spectre is licence-gated, so the text is checked against the Spectre reference manual |
| `test_ltspice_dialect.rs` | emitted text against the LTspice help; no LTspice binary, so unverified by execution |
| `test_backends.rs`, `test_unit.rs` | spec examples |

### Python (`tests/*.py`)

| File | Oracle |
|---|---|
| `test_bench_*.py`, `test_characterize.py`, `test_run_corners.py` | ngspice on behavioural DUTs with closed-form answers |
| `test_remeasure.py` | closed forms, never the code's own method |
| `test_espice_backend.py`, `test_vacask_backend.py` | the real simulator's numbers for the emitted deck |
| `test_docs.py`, `test_python_examples.py` | the docs and examples run, and print the numbers they claim |
| `test_python_*.py`, `test_design_testbenches.py` | API contract: spec examples per method |
| `test_bump_version.py` | a copy of the manifests, each bump kind and each refusal |

## Known gaps

`nix develop .#testing` provides every simulator and corpus the tests read
(ngspice, VACASK, ESPice built from source, the spice-bench suite and the
corpora); outside it those tests fail, they never skip.

- LTspice and Spectre are not installed anywhere SpiceRack is tested, so their
  decks are checked against the manuals only.
- There is no nightly tier. The spice-bench round trip runs in the merge
  tier: 2,586 CPU seconds, 5 min 25 s wall on a 20-thread machine.
- No public Spectre corpus is pinned (ESPice's `spectre_fixtures` are gone
  upstream), so `spectre_corpus_fixpoint` is a known failure
  (`#[should_panic]`).
- ESPice reads back only the HSPICE deck in `test_dialects.rs`: the ngspice
  and ESPice decks spell Laplace as an XSPICE `s_xfer`, which ESPice cannot
  read, and codegen refuses the B source for Spectre.
