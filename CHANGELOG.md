# Changelog

One line per change, written by hand, citing its pull request. Every number is measured.

## Unreleased

- `TESTING.md` (tiers and oracles), this changelog, and `tools/check_style.py` vendored.

## 0.1.2 (2026-10-31)

- codegen: Verilog-A model parameters reach ngspice.
- ci: one-click release: bump, test, merge, release and PyPI in one run (#11).

## 0.1.1 (2026-10-31)

- First PyPI release, as `fast-spicerack` (the import name stays `spicerack`).
- testbenches: converter, regulator, reference, op-amp, loop, noise, RF, oscillator, VCO, PLL, DLL and clock benches.
- remeasure: deciding numbers recomputed from raw waveforms (#2).
- docs: the user guide in Markdown, built by MkDocs.
- nix: VACASK's device models installed with the binary and compiled for a generic CPU.
