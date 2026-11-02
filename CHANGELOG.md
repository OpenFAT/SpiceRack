# Changelog

One line per change, written by hand, citing its pull request. Every number is measured.

## Unreleased

- `TESTING.md` (tiers and oracles), this changelog, and `tools/check_style.py` vendored.
- ir: Testbench.dut removed; old JSON carrying it still loads (#13).
- python: YieldSummary.failed and the add_spectre_montecarlo alias removed; use add_spectre_monte_carlo (#13).
- backend: Dialect replaces BackendKind (backend::parse, backend::RUNNABLE, Dialect::run); backend aliases in one table, lint treats vacask-shared as VACASK (#13).
- codegen: Laplace H(s) constants scaled exactly by a decimal shift (#13).
- internal: one walker each for identifiers, expressions and parameters; one table for the 24 waveform-source methods; one analysis suite for ngspice and ESPice (#13).
- nix: `.#testing` builds ESPice from source (OpenFAT/ESPice `5f0b8fd`) and pins the spice-bench suite (6,050 decks) and the Xyce, LTspice, HSPICE and VACASK corpora by content hash (#14).
- tests: a missing simulator, corpus or unbuilt extension fails instead of skipping; 12 ESPice Python tests and 15 Rust tests that skipped in CI now run (#14).
- ci: a nightly tier (`nightly.yaml`) runs the import suite over the corpora, behind the `corpus` feature; the merge tier leaves it out (20 min of a 4-vCPU runner) (#14).
- tests: spice-bench decks ngspice-45 cannot run are named known failures (469 listed, 28 quarantined past the CPU budget); a CPU-limit kill is now reported as a timeout (#14).
- backend: `select` routes on the `Analysis` itself, not a kind string; one predicate (`runs`) for selection and `check_backend`, with a compile-time check that every feature bit has its issue row (audit 30, 41, 42).
- ir: a frequency sweep's spacing is `Sweep` (dec, oct, lin, log), held in `FreqSweep` and the SpectreRF steady state in `PssBase`, both flattened so IR JSON is byte-identical; `AcSweepParams` is now `FreqSweep`. A bad sweep name is a `ValueError` at the Python call and a load error in JSON, where it used to reach the deck (audit 40).
- result: `RawData::backend_hint` is an `Option<Dialect>` (was a string, empty for ngspice) and `VarInfo::var_type` a `VarType`; `parse_measures` takes the `Option<Dialect>`. One `normalize::canonical` pass returns a name's canonical form and whether it is a current (was `normalize_var_name` plus `is_current_name`, two lowercasings each). `Results::column` allocates only for a name with capitals, `RawData::re`/`Results::get`/`axis` borrow a real column, and a Python result column is built into the list straight from it; Python sees the same lists (audit 43, 44).
- codegen: an ngspice deck with a `.meas` or `.four` (from the testbench, or a raw or extra line, `+` continuations included) is written with its first ac/tran/dc in a `.control` block, each `.four` as a `fourier` command (the old rewrite kept only the first); the backend no longer re-parses the deck to move it there and only appends the `write`. A deck handed to the ngspice backend with a `.meas` or `.four` and no `.control` block that runs it is refused with an error naming the card, where it used to be rewritten or, with no ac, tran or dc to move, run with those results silently dropped (audit 38). The ngspice run path compares first words as `&str`, with no `String` per line (audit 45).
- codegen: ngspice, LTspice, HSPICE, Xyce, ESPice, Spectre and VACASK decks are written into one `String` (`write!`), cards and blocks appending to it instead of each building a `String` or a `Vec<String>` to join; Spectre escapes and VACASK quotes each name as it is written, so `netlist()` no longer clones the IR. Every deck the two test suites emit, in every dialect, and the same IRs with every name renamed to need escaping, are byte-identical before and after (9378 decks). 40000-element deck, release, best of 15: ngspice 28.2 to 17.6 ms, Spectre 63.7 to 44.7 ms, VACASK 49.5 to 26.1 ms (audit 46, 47).

## 0.1.2 (2026-10-31)

- codegen: Verilog-A model parameters reach ngspice.
- ci: one-click release: bump, test, merge, release and PyPI in one run (#11).

## 0.1.1 (2026-10-31)

- First PyPI release, as `fast-spicerack` (the import name stays `spicerack`).
- testbenches: converter, regulator, reference, op-amp, loop, noise, RF, oscillator, VCO, PLL, DLL and clock benches.
- remeasure: deciding numbers recomputed from raw waveforms (#2).
- docs: the user guide in Markdown, built by MkDocs.
- nix: VACASK's device models installed with the binary and compiled for a generic CPU.
