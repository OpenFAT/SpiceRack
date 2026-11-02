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
| `test_dialects.rs` | one circuit in every dialect, read back by ESPice (`--tokenizer=<d>`), same numbers |
| `test_import_suite.rs` | spice-bench decks (`SPICE_BENCH`): parse, JSON round trip, re-emitted as ngspice, every plot and `.meas` reproduced |
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

- LTspice and Spectre are not installed anywhere SpiceRack is tested, so their
  decks are checked against the manuals only.
- There is no nightly tier.
