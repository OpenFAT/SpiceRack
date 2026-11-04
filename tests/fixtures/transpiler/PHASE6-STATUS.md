# Phase 6 (remove CircuitIR): status

Phase 6 of the rewrite (TESTING.md, "Golden gate"): the IR (`CircuitIR`,
`src/ir`), the adapter (`syntax::adapter`, `ast_to_ir`), the old writers
(`src/codegen`), `import::parse`'s IR output and the JSON form are gone. The
Python API builds its decks as ngspice text that the parser reads, and
`syntax::render` writes them; the backends take a `Kind` and rendered text.
Decisions D1–D3 (the spec, `transpiler-architecture.md` §6) as adopted.

## The Python API on the AST (D1)

- Each call spells its ngspice card at once (`python/model.rs`: `Card`,
  `Body`, `Bench`); a deck is that text, in deck order (`python/deck.rs`).
  The parser reads it with `lexicon::NGSPICE_GIVEN`: ngspice's lexicon with
  correctly rounded numbers (a double's SI spelling reads back as that
  double; ngspice's arithmetic puts `100u` an ulp away) and `.step` (Xyce's
  form). The tree's rows are the parser's, by construction: a token's text
  is re-lexed from the source, so any builder writes source bytes; this one
  writes them and lets the one parser make the rows.
- What no ngspice card holds goes beside the text:
  `render::Source::loads` (the top's Verilog-A/OSDI loads; ngspice loads
  them from a `.control` command, which the reader keeps no load of) and
  `render::Numbers::Given` (SI spelling, as the old Python path wrote).
  Added to the rendered text, never parsed: ngspice's `ngbehavior` line,
  the analyses ngspice has no card for and `.pss` on Spectre
  (`python/analysis.rs`, `Analysis::native`/`spectre`), and on Spectre and
  VACASK the `extra_line`s (in their own dialect).
- `to_json`/`from_json`/`save_json`/`load_json` are removed (D3: the store
  is source files; save the deck). The three serialization tests went
  with them; the 52 tests that inspected the JSON now check the card each
  call writes (`tests/test_python_subcircuit.py`'s `cards()`).
- The pre-existing failure `test_x_params_reach_the_card_on_every_backend`
  expects Spectre's bare `w=wn*2` (Spectre Reference 19.1 p.506).

### Python decks against the old path

Every deck pytest builds was logged on every target through both paths
(`P6_LOG`, a temporary hook, removed) and compared. What differs, by class:

- `raw_spice` lines are read as the deck's own statements, so each target
  writes them in its form: Spectre and VACASK write decks they refused
  ("raw SPICE line"); HSPICE quotes `{e}` as `'e'`; a raw `.ic v(a)=0
  v(b)=1` is two `.ic` cards; a raw `.model x sw(…)` moves to the models,
  `sw (…)`; `R(raw_spice="9kOhm")` is `9k`, `1Meg` is `1meg`. Another
  dialect's text there (an XSPICE `A` or OSDI `N` card on LTspice or
  HSPICE, a TRNOISE on ESPice) is refused (`W0001`, `W0009`) where the old
  writer copied it.
- A `.tran` step cap with no start: SPICE's placeholder start `0` is read
  as a start, so Spectre writes `start=0` and HSPICE `START=0` (the same
  run).
- `nominal_temperature` is a `.options tnom=`: merged into the options
  card on SPICE targets, `myopts options … tnom=` on Spectre (was `mytnom
  options tnom=`), after `temp` on VACASK.
- `.measure` cards reach Spectre spelled `.meas`.
- A refusal is its codes and messages (`<deck>:1:1: W0004 a Verilog-A load
  the target cannot spell`), not the old free text.
- Not carried: `add_fourier`'s `num_harmonics` (ngspice's `.four` has
  none; LTspice's has); a Python `A()` element on LTspice is refused
  (`W0001`), where the old writer wrote a `* XSPICE` comment.

## The backends

`backend::select(kind, backend, needs)`, `backend::check(needs, elements,
backend)` (was `CircuitIR::check_backend`), `backend::feature` (moved from
`ir::feature`), `backend::ngspice::NGBEHAVIOR`; `simulation::run(sim,
deck)`, `run_all(sim, deck)`, `run_each(sim, deck, kinds)`. measure and
rawfile never read the IR.

## Verilog

A structural writer, `render/spice/verilog.rs` (≈170 lines with its test):
one `module` per block, ports `inout`, the nets instances use `wire`,
instances by port name where the deck defines the master and by position
otherwise, escaped identifiers (IEEE 1364-2005 §3.7.1, §12); every other
element is a `// W0029 …` comment, and what a module cannot hold is a
`W0029` note. The old layout wrote the IR's `Debug` text, so the golden
diff skips Verilog (`--skip verilog`).

## Public API (for Interop's `pdk-models`)

- `import::decode(bytes) -> String`
- `import::parse(src, d) -> Result<(Tokens, Ast), String>`
- `import::kit(entries, form, d, root) -> Kit` (from disk, under `root`)
- `syntax::resolve::Kit::load(entries, form, d, loader) -> Kit`
- `syntax::render::render_kit(kit, file, name, target)`,
  `render_text(text, from, name, target)` → `Result<Written, Diags>`;
  `render(&Source, target)` for a tree in hand
- `syntax::render::references(&Source) -> Vec<Reference>` (the files a file
  names at its top and in each section: Interop's walk)
- `syntax::Dialect`; `syntax::diag::{Code, info, Kind}`

The note at the top of `src/syntax/mod.rs` shows the three steps.

## Gates

- golden: `golden record` (`/tmp/claude-1000/spicerack-golden/p6-2`)
  against `baseline-daa0ad0-fx11` (`--refusal-status-only`, `--skip
  verilog`): IDENTICAL except 15 listed xfails. fx11 is fx10 (step 0: the
  six fixtures' headers rewritten, re-recorded through the old writer) with
  `func_stats.{sp,cir}`'s headers rewritten again (no citation of the old
  writer) and re-recorded through the old writer at `65b63c9`: their
  outputs unchanged, only the input hashes. The `ir`/`report` xfail lines
  went with the IR; `diff` passes over a baseline's `ir.json` and
  `report.json`.
- `check-determinism`: IDENTICAL (3 min 54 s).
- fmt, clippy `-D warnings` (all targets, all features), `check_style.py`:
  clean.
- merge tier: 366 (467 at `949be79`): −112 unit tests of deleted files
  (`codegen/spice3/mod.rs` 35, `ir/mod.rs` 19, `codegen/vacask/mod.rs` 16,
  `codegen/laplace.rs` 16, `codegen/spectre/mod.rs` 13, `codegen/xschem/mod.rs`
  7, `import/tests.rs` 4, `codegen/statistics/tests.rs` 2 — T-N0/T-N1, ported
  to `tests/test_monte_carlo_ngspice.rs`); −1 `test_sources`
  (`ngspice_pwl_repeat_delay_and_rf_port_cards_lower_into_the_ir`, IR fields
  only); −3 `test_spectre_golden` (the pac/pnoise/pxf/pstb, sweep/montecarlo
  and transient-noise forms, moved to `python/analysis.rs`, which spells
  them now); −1 `test_monte_carlo` (`spectre_run_decks_are_current`, a
  duplicate of `named::lift`'s); −1 `test_render` (two statistics tests
  merged); +15 new unit tests (dialect, import, lint, python model and
  analysis, Verilog, `backend::check`, the ngspice deck predicates), +2
  T-N0/T-N1. `test_import_suite` (corpus tier): its JSON round trip and the
  `GOLDEN_DIR` dump deleted; the 7 fixpoints (now text fixpoints) pass.
- pytest: 507 (510 at `949be79`, one failing): the three JSON
  serialization tests deleted, the failing one fixed.

## Left

- The writers recurse over a branch's body (`spice::W::body` ↔
  `conditional`, `named::W::body` ↔ `conditional`), and a plan's `Cond`
  drops recursively, so a 10⁴-deep `.if` deck needs a large stack
  (`src/syntax/tests/render.rs` runs it on 256 MiB, as `golden` does). A
  worklist over the branches (and a manual `Drop`) is the fix; the adapter
  it replaced walked them on a heap stack.
- Python decks: the Spectre-only analyses are appended after the rendered
  analyses (not interleaved in queue order) and outside `.step` sweeps; a
  VACASK `montecarlo` is refused (the old writer wrapped the inner analysis
  in `mc`); a `log` `.step` is not read (Xyce's form has none).
- `.options itl1=` reaches Spectre as `itl1=` (only the portable name
  `max_iterations` maps to `dcmaxiters`): a quirk older than this phase.
- A VACASK charge-defined capacitor is refused `W0105`; `W0103` (a charge
  expression the target cannot spell) may be the better code.
- Verilog: a simple name that is a Verilog keyword is not escaped; an
  instance's parameters are noted, not written.
