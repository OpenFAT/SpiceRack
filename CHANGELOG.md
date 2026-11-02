# Changelog

One line per change, written by hand, citing its pull request. Every number is measured.

## Unreleased

- `TESTING.md` (tiers and oracles), this changelog, and `tools/check_style.py` vendored.
- nix: `.#testing` builds ESPice from source (OpenFAT/ESPice `5f0b8fd`) and pins the spice-bench suite (6,050 decks) and the Xyce, LTspice, HSPICE and VACASK corpora by content hash (#14).
- tests: a missing simulator, corpus or unbuilt extension fails instead of skipping; 12 ESPice Python tests and 15 Rust tests that skipped in CI now run (#14).
- tests: spice-bench decks ngspice-45 cannot run are named known failures (469 listed, 28 quarantined past the CPU budget); a CPU-limit kill is now reported as a timeout (#14).

## 0.1.2 (2026-10-31)

- codegen: Verilog-A model parameters reach ngspice.
- ci: one-click release: bump, test, merge, release and PyPI in one run (#11).

## 0.1.1 (2026-10-31)

- First PyPI release, as `fast-spicerack` (the import name stays `spicerack`).
- testbenches: converter, regulator, reference, op-amp, loop, noise, RF, oscillator, VCO, PLL, DLL and clock benches.
- remeasure: deciding numbers recomputed from raw waveforms (#2).
- docs: the user guide in Markdown, built by MkDocs.
- nix: VACASK's device models installed with the binary and compiled for a generic CPU.
