# Ponytail audit: SpiceRack (excluding the importer)

Scope: src/ir, src/codegen, src/python.rs, src/backend, lint, normalize, measure_parse,
veriloga, rawfile, psf, result, unit, simulation, lib, tools, tests, python/, Cargo.toml,
flake.nix, nix/, CI. Skipped: src/import*.rs, src/peg.rs, grammar/.
Caller counts come from `rg` over src, tools, tests (*.rs, *.py), python/, plus docs and
examples where the item is Python-facing. `.claude/worktrees` excluded.
Python-exposed API is treated as used unless both tests and docs are empty, and those
cases are marked "public API".

Ranked by lines saved within each group. Where the cut gives up something real, the item
says what.

## 1. Delete outright

1. `src/codegen/xschem.rs:1-989`: delete the module plus `Dialect::Cdl` and
   `Dialect::Verilog` (`codegen/mod.rs:37-39,109`). Nothing replaces it, because nothing
   reaches it. About 1000 lines. Evidence: `xschem::` has 1 caller (`codegen/mod.rs:109`).
   That call is only reached for `Cdl` and `Verilog`, and `Dialect::Cdl` / `Dialect::Verilog`
   have 0 uses outside xschem.rs. Not in tools (`BACKENDS` in `tools/spice2rack.rs:308`), not
   in Python (`emit_ir_netlist` cannot map those names), not in docs. The prod call passes
   `Deck::new(ir)`, so `title/globals/setup/bodies/lines/nets`, `SetupLine`, the `Lines`
   packed buffer, `Seen`/`sort_spans`, and every ngspice/Spectre/LTspice/HSPICE/Xyce branch
   in `Writer` only run under its own 5 unit tests. Caveat: `pub mod`. Keep it only if an
   out-of-repo schematic tool links it; otherwise `git rm`.
2. `src/result.rs:107-138` `compute_fft_metrics`: delete, nothing replaces it. 32 lines.
   Evidence: 0 callers in src, tools, tests, python, docs.
3. `src/psf.rs:1-92` plus `tests/test_backends.rs:1-22`: delete the module.
   `backend/mod.rs:299` becomes a 2-line trailer check that returns the same "run with
   -format nutbin" error. About 110 lines. Evidence: `parse_psf` returns `Err` on every
   path. It has 1 prod caller (`read_results`), and the only producer (the Spectre backend)
   always passes `-format nutbin` (`backend/spectre.rs:21`).
4. `src/python.rs:846,1845` `PySamplingAnalysis`: delete the class. 2 lines.
   Evidence: no method returns it and it has no `#[new]`, so Python cannot obtain an
   instance. The only reference is a `docs/reference.html:200` table row.
5. `python/spicerack/testbenches/analysis.py:74-76` `ac_gain_db`: delete. 4 lines.
   Evidence: 0 callers and not exported from `testbenches/__init__.py`.
6. `python/spicerack/testbenches/analysis.py:263` `YieldSummary.failed`: delete. 3 lines.
   Evidence: 0 callers (tests use `.failures` and `.passed`).
7. `src/python.rs:1726-1729` `add_spectre_montecarlo`: delete this alias of
   `add_spectre_monte_carlo`. 4 lines. Evidence: 0 tests, 0 docs. It is public API, so the
   cut is technically a break, but nobody uses it.
8. `src/ir/mod.rs:305` `Testbench::dut`: delete the field. About 15 lines, counting the
   test fixtures that set it. Evidence: written in 1 prod place (`python.rs:1445`), read in
   0. No codegen or backend touches it. Caveat: it is in `schema/circuit-ir.schema.json`,
   so `#[serde(default)]` it for one release first.
9. `src/backend/mod.rs:31,235` `UnsupportedAnalysis.install_cmd`: drop the field.
   3 lines. Evidence: the value is always `"See docs/backends/{required}.md"`, and
   `docs/backends/` does not exist, so it is a broken link computed from another field.
10. Dead aliases in `normalize.rs:37` (`"vacask-shared"`), `measure_parse.rs:26`
    (`"ngspice-subprocess" | "ngspice-shared"`) and `normalize.rs:185-188`
    (`test_ngspice_variant_names`): delete. About 8 lines. Evidence: the sole writer of
    `RawData::backend_hint` is `simulation.rs:18` (`sim.name()`), which only yields the 5
    canonical names.
11. `src/backend/mod.rs:116-117` `ROUTES` rows `sampling/pce/embedded_sampling` and
    `sens_tran/sens_adjoint`, plus the dead keys `s_param`, `stability`, `pdisto`, `psp`,
    `hbac`, `hbnoise`, `hbsp`: delete. 2 rows. Evidence: `select()` has 1 prod caller
    (`simulation.rs:15`), whose key comes from `Analysis::kind_str`, and kind_str never
    returns those strings (`ir/mod.rs:370-394`).
12. Trivial one-caller wrappers, about 10 lines total:
    - `RawData::empty()` (`result.rs:42`, 1 caller `rawfile.rs:63`) becomes `default()`.
    - `compile_veriloga_impl` (`python.rs:894`, 1 caller) gets inlined.
    - The serde derives on `FeatureFlags`, `Issue` and `IssueSeverity` (`ir/mod.rs:439,453,459`)
      go: they are never serialized (0 `to_string`/`from_str` over them).

## 2. Merge duplicates

13. `src/ir/mod.rs:625-1453`: serde-derive tests. Delete these tests:
    - `test_ir_construction` (asserts struct literals hold their fields)
    - `test_serialization_roundtrip`
    - `test_full_ir_json_roundtrip` (180 lines)
    - `test_component_json_tags` (205 lines)
    - `test_analysis_json_tags`, `test_irvalue_json_tags`, `test_waveform_json_tags`
    - `test_minimal_ir_roundtrip`, `test_tuple_serialization_format`
    - `test_invalid_json_fails_gracefully`

    Replace them with one loop over the existing `test_all_component_variants_roundtrip`
    and `test_all_analysis_variants_roundtrip` lists that asserts `"type":"<Variant>"`.
    About 650 lines saved. Evidence: every variant is already round-tripped by those two
    tests. The deleted ones re-test `#[derive(Serialize)]` one variant at a time.
14. `tests/test_codegen.rs:1-486`: delete the file. About 480 lines. Evidence: all 9 tests
    have same-fixture twins in the crate:
    - `spice3.rs`: `test_ngspice_resistor_divider`, `test_mosfet_circuit`,
      `test_subcircuit_instances`, `test_waveforms_sin_pulse_pwl`, `test_all_analysis_types`
    - `spectre.rs`: `test_spectre_resistor_divider`, `test_spectre_mosfet`,
      `test_spectre_waveforms`, `test_spectre_options`

    The fixture builders are copies too (`resistor_divider_ir` and
    `sample_resistor_divider` are the same circuit).
15. Test fixtures that spell out empty fields: switch them to `..Default::default()` plus
    one shared `ir(top, tb)` helper. About 350 lines. Evidence: 40 `Subcircuit` literals
    write `osdi_loads: vec![]` and friends. 71 `CircuitIR` literals write
    `raw_dialect: None, globals: vec![], functions: vec![]`:
    - 21 in `tests/test_vacask_runs.rs`
    - 12 each in `ir/mod.rs` and `spice3.rs`
    - 9 in `vacask.rs`

    Both types derive `Default`. `python.rs:291,1446` already mixes the explicit fields
    with `..Default::default()`, which is redundant.
16. `src/python.rs:1186-1311` and `1468-1590`: 24 waveform-source methods. That is 6
    waveforms, times voltage/current, times `Subcircuit`/`Testbench`, copy-pasted with the
    same 12-to-15-argument signatures. Generate them with one `macro_rules!` that emits each
    class's `#[pymethods]` block (needs pyo3's `multiple-pymethods` feature). Or put the 12
    names in `python/spicerack/__init__.py` as thin wrappers over one native
    `_wave(kind, current, ...)`. About 180 lines. Evidence: the bodies differ only in
    `true`/`false` and in `self.push` vs `slf.as_super().stim`. The comment at
    `python.rs:1183` already claims "one table drives both".
17. `tests/test_espice_analyses.rs:1-210`: this file mirrors `tests/test_ngspice_analyses.rs`.
    Fold both into one suite instantiated by a macro over `BackendKind`. About 150 lines.
    Evidence: 8 identically named tests, and identical `run/measures/index_of/scalar/column`
    helpers (`espice:15-57` vs `ngspice:14-60`). `index_of` also re-implements
    `Results::column` name normalization.
18. `src/lint.rs:197-372`: delete the Rust unit tests. `tests/test_python_lint.py` (32 tests,
    a superset of all 23) covers the same cases through the binding. About 170 lines.
    Evidence: a one-to-one name match, e.g. missing_end, ground, duplicate, floating,
    zero R/C, model, param, and the backend cards.
19. Dialect-to-name mapping exists in 9 places:
    - `xschem::word` (`xschem.rs:127`)
    - `spice3::key` (`spice3.rs:9`)
    - `format!("{d:?}").to_ascii_lowercase()` (`codegen/mod.rs:75`, `spice3.rs:355`,
      `backend/mod.rs:236`)
    - `caret_means_power(&str)` (`codegen/mod.rs:45`)
    - `tools/spice2rack.rs:389-400`
    - `python.rs:229-234`
    - `import.rs:1105`

    Replace them with one `Dialect::name()` plus `FromStr`, and keep `family()` for the
    ESPice-reads-ngspice rule. About 40 lines, and the allocations go.
20. `BackendKind` (`backend/mod.rs:50-57,145-153`) is a strict subset of `Dialect`, and
    `BackendKind::dialect()` is an identity match. Use `Dialect` and let `run()` reject
    the non-runnable ones. About 30 lines. Evidence: 5 variants, mapped 1:1.
21. The SPICE number-with-suffix parser exists 5 times:
    - `codegen/vacask.rs:82-130` (`SCALES` + `parse_spice_num`)
    - `codegen/laplace.rs:50-54,399-430` (`SCALES` + `lex_number`)
    - `import.rs:216`, `import_spectre.rs:17`, `import_vacask.rs:15`
    - `unit::PREFIXES` is the same table read the other way

    Keep one `unit::parse_spice_number(&str) -> Option<(f64, usize)>`. About 30 lines in
    scope, plus about 60 more once the importer rewrite adopts it.
22. The rename passes `spectre::escaped` (`spectre.rs:457-508`) and `vacask::prepared`
    (`vacask.rs:605-642`) clone the IR and walk the same names. The walk covers subckt
    names, ports, models, instances, `names_mut()`, stimulus and globals. Add one
    `CircuitIR::names_mut(f)` in `ir`, next to `Component::names_mut`. About 25 lines.
23. Three per-component expression walkers repeat the same `match` arms:
    `codegen/mod.rs:51-70` (`component_exprs`), `vacask.rs:645-663` (`lower_values`) and
    `codegen/mod.rs:178-185` (`refuse_flags`). Replace them with `Component::params()` and
    `Component::exprs_mut()` in `ir`. About 25 lines.
24. The ` k=v` parameter formatters are duplicated:
    - `codegen/mod.rs:169` (`params`) and `:197` (`param_defs`)
    - `spectre.rs:53,62` (the same two, with `spectre_value`)
    - `vacask.rs:234` (`inst_params`) and `:559` (`model_params`)
    - inline in `spice3.rs:153`

    Replace them with one `params(ps, respell: fn(&str) -> Cow<str>)`. About 20 lines.
25. The `V(x)`/`I(x)` strippers `spectre::bare_node` (`spectre.rs:110`) and
    `vacask::strip_wrapper` (`vacask.rs:573`) do the same job: keep one. About 12 lines.
26. `src/python.rs` analysis builders: 11 `PySimulator` / `PyTestbench` pairs build the
    same `Analysis` from the same signature. Examples: `ac`/`add_ac`,
    `transient`/`add_transient`, `noise`/`add_noise`, `tf`, the sens pair, `polezero`,
    `distortion`, `spectre_sweep`, `monte_carlo`. Give each a shared `fn x(...) -> Analysis`
    and make the pymethods one-liners. About 60 lines.
27. `ir/mod.rs:259-284`: `IrWaveform::nums_mut` and `exprs_ref` repeat all 6 field lists.
    Add `nums(&self)` and build `exprs_ref` as a filter over it. About 7 lines.
28. `.options plotwinsize=0 numdgt=15` is injected twice: `backend/ltspice.rs:19` and
    `xschem.rs:324`. The second is gone with item 1.
29. `lint.rs:179-183` re-lists the ngspice backend aliases, misses `vacask-shared`, and
    duplicates `BackendKind::from_str`. `python.rs:119-124` hardcodes the alias list in its
    error text. Derive both from one table. About 6 lines.

## 3. Simplify

30. `src/ir/mod.rs:529-608` `check_backend` is 7 copy-pasted `if features.x && !caps.x`
    blocks. Make it one table of `(flag, severity, message)` over the feature bitset (see
    item 41). About 65 lines. Bonus: the copies have drifted. `supports_features` checks
    `verilog_cosim` and `check_backend` never does.
31. `tools/spice2rack.rs:31-290`: the `--python` emitter plus the gap tally. Its only
    consumer is `dev/research/ir-gaps.md`, and no CI or test runs it. If that research is
    done, cut it. About 260 lines. Optional: it is a dev tool, not dead code.
32. `src/normalize.rs:45-217`: trim 23 tests to the distinct cases.
    `test_backward_compat_ngspice_defaults` repeats `test_ngspice_voltage` and
    `test_ngspice_current`, and the variant-names test goes with item 10. About 40 lines.
33. `src/codegen/spice3.rs:480-494`: the R/C/L arms are 3 identical arms. Make them one arm
    with a letter, as the V/I arm already does. About 10 lines.
34. `src/veriloga.rs`: 17 `#[cfg(feature = "backend")]` attributes on consecutive items.
    Wrap them in one `#[cfg] mod run { ... }` with a re-export. About 15 lines.
35. `src/codegen/spectre.rs:89-104,512` `KEEP_CASE` `thread_local!` is hidden global state
    for one bool that `netlist()` already knows. Lowercase the `inst_ref` targets in
    `escaped` (which already folds when `!keep_case`), or pass the bool. About 8 lines.
36. `flake.nix:91-97`: the shellHook symlinks `openvaf` to `openvaf-r` in `~/.cache`. Do it
    once with `ln -s openvaf-r $out/bin/openvaf` in `nix/openvaf.nix` `postInstall`
    (line 62). About 6 lines, and the shell entry stops writing to `$HOME`.
37. `flake.nix:100-125`: the `testing` shellHook pip-installs `ciel` and fetches two PDKs
    over the network on every shell entry. No Rust or Python test reads `PDK_ROOT`; only
    examples 20 and 21 do. Move it to an explicit `nix run .#fetch-pdks`. About 25 lines.
38. `backend/ngspice.rs:15-160` re-parses the deck that `spice3::netlist` just wrote, to
    move `.meas` and `.four` into a `.control` block. The codegen already knows when the
    testbench has measures or Fourier, so it could emit the block itself. About 80 lines,
    with the parsing and its 10 tests. Medium risk: hand-written decks passed straight to
    `BackendKind::run` still need `append_write_block`.

## 4. DOD layout fixes

39. `ir/mod.rs:17` `raw_dialect: Option<String>` should be `Option<Dialect>`. It is
    string-compared in 6 places: `codegen/mod.rs:74,136`, `spice3.rs:353`,
    `spectre.rs:465,512`, `vacask.rs:607`. `import.rs:1105` builds the string from a
    `Dialect` only for codegen to turn it back.
40. `Analysis.variation: String` appears in 9 places: Ac, Noise, Distortion, SPar,
    Stability, AcSweepParams and the 4 Spectre P* variants. Make it an
    `enum Sweep { Dec, Oct, Lin, Log }` parsed once at the Python boundary. Today
    `spectre::sweep_points` (`spectre.rs:153`) and `vacask::freq_sweep` (`vacask.rs:410`)
    each lowercase and re-validate it per emit. Also, `{variation, points, start, stop}`
    repeats in 9 variants, and `AcSweepParams` already is that struct. The 4
    `SpectrePac/Pnoise/Pxf/Pstb` variants share
    `{pss_fundamental, pss_stabilization, pss_harmonics}`. Use `FreqSweep` + `PssBase`
    with `#[serde(flatten)]`, which keeps the JSON shape. `StepParam.sweep_type`,
    `PoleZero.tf_type/pz_type` and `Spectre*.inner_type` are the same pattern.
41. `FeatureFlags` (`ir/mod.rs:440-448`) and `BackendCapabilities` (`backend/mod.rs:62-70`)
    are 7 parallel bools each, plus the 7-argument `caps()` constructor and the `&&` chain
    in `supports_features`. Use one `u8` bitset: `supports = f & !caps == 0`, and the caps
    table becomes 5 `const` masks.
42. `Analysis::kind_str` returns a `&str` key that `backend::route` then looks up in a
    `&[&[&str]]` table (`backend/mod.rs:103-127`). That string lookup is the dead-key
    source in item 11. Match on `Analysis` directly.
43. `RawData::backend_hint: String`, with empty meaning ngspice (`result.rs:22,72`), should
    be a `BackendKind`. `normalize_var_name`, `is_current_name` and `parse_measures` take
    `&str` backends and string-match them per variable. `VarInfo.var_type: String`
    (`result.rs:30`) is compared to `"current"`: make it an enum.
44. `result.rs:70-83` `Results::from_raw` calls both `is_current_name` and
    `normalize_var_name` per variable. Each lowercases (2 allocations) and runs
    `as_current` again. Use one call that returns `(canonical, is_current)`.
    `Results::column` (`result.rs:87`) allocates a lowercase copy per lookup, and
    `Results::get` clones the whole column per Python `__getitem__`/`__getattr__`.
45. `backend/ngspice.rs:50,108-110`: `first_word` allocates a `String` per line, called in
    5 full passes over the deck, and `format!(".{}", c)` allocates per line per card inside
    `position`. Compare `&str` with `eq_ignore_ascii_case` instead.
46. Codegen builds `Vec<String>` per card and then `join`s, with 44
    `push_str(&format!(...))` sites (spice3 19, spectre 14, vacask 11). Write straight into
    one `String` with `write!`.
47. `spectre::escaped` and `vacask::prepared` clone the entire `CircuitIR` on every
    `netlist()` call to rename strings (`spectre.rs:513`, `vacask.rs:606`). After item 22,
    escape at write time instead of on a copy.

## Total

The 47 items add up to roughly 3,900 lines, some optional:

| Group | Lines |
|---|---|
| Delete outright | about 1,190 (about 1,000 of it is xschem.rs alone) |
| Merge duplicates | about 2,250, mostly tests: ir serde about 650, test_codegen about 480, fixtures about 350, lint about 170, espice mirror about 150, waveform/analysis bindings about 240, the rest about 210 |
| Simplify | about 510, of which spice2rack `--python` (about 260) and the ngspice deck rewrite (about 80) are optional |
| DOD layout fixes | small line counts, but they remove 30-plus per-emit or per-variable string compares and allocations |

These items touch no Python-exposed method that has a test or docs. The exceptions are
item 7 (an untested, undocumented alias) and item 8 (a schema field, deprecate first).
