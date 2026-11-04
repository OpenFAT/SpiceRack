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
- No side channel around the tree (user decision 1): each statement is
  spelled in a dialect that has it, ngspice by default; nothing is added to
  a rendered deck: `git grep -n 'render::render\|insert_str' src/python`
  finds one line, `Deck::write`'s call, which returns `w.text` as is.
  - What only another dialect says is that dialect's text in a `simulator
    lang=<dialect>` region of the source: `NGSPICE_GIVEN`'s
    `Lexicon::regions` (ngspice, spectre, vacask, hspice, ltspice, xyce)
    switch the tokenizer to that lexicon until `simulator lang=spice`; the
    parser reads the region's statements as that dialect's nodes (a
    `Region` node; test `a_given_ngspice_deck_holds_regions_of_other_dialects`),
    and the reader keeps each as that dialect's text span (`Raw::Lines`,
    `Rd::text_dialect`): the `Verbatim{dialect, span}` contract. Only that
    dialect's family writes it (`W0027`, copied as written); every other
    refuses it (`W0001`, the capability table's `ForeignText`).
  - The analyses ngspice has no card for (sweep, montecarlo, pac, pnoise,
    pxf, pstb, hb, stb) are Spectre statements in the `spectre` region
    (`python/analysis.rs`, `Analysis::spectre`). `pss` is ngspice's `.pss`
    on every target; a Spectre target refuses it (`W0001`), where phase 6
    appended a Spectre `pss`. Transient noise has no statement in any
    dialect and is refused when the deck is built.
  - `extra_line(line, dialect="ngspice")` is a line in that dialect
    family's region (`deck::region_dialect`); `CornerCase` passes its
    backend, `network_params` LTspice (`.net`). One ngspice rule still
    reads such text: a `.four` there becomes the `fourier` command when
    the deck runs from `.control` (`spice::bench`, as for any `.four` the
    reader keeps), or ngspice ignores it under `-b -r`.
  - `ngbehavior` is an `.options ngbehavior=<mode>` card: the reader's
    `Opt::NgBehavior`, the capability table's `Construct::NgBehavior`:
    ngspice spells it as the `render::NGBEHAVIOR` line after the title,
    which its backend lifts onto the command line; every other target
    notes `W0030` (new) and writes nothing.
- Beside the text, in `render::Source`: `loads` (the top's Verilog-A/OSDI
  loads; ngspice loads them from a `.control` command, which the reader
  keeps no load of) and `numbers: Numbers::Given` (SI spelling, as the old
  Python path wrote).
- The store is source text (user decision 2, D3; `python/store.rs`): the
  deck's ngspice source behind a first line `* spicerack-source ngspice`,
  the loads as a `.control` block of `pre_osdi` lines after the title.
  `to_source()`, `from_source(text, dialect=None)` (`None`: the header's
  dialect, else ngspice; another dialect's text is written as ngspice
  first), `load(path)` on `Subcircuit` and `Testbench`, `Subcircuit.save(path)`
  and `Testbench.save_source(path)` (`Testbench.save(*outputs)` is the
  `.save` card every simulator call takes). A `Subcircuit` reads back into
  the cards its calls wrote; a `Testbench` as its top's statements, byte
  for byte: the same netlist on every target (`TestTestbenchStore`). Its
  analyses are text, so `run()` runs only those queued after loading. The source is the same for every target but one
  part: a `ModelLibrary`'s per-backend path (`backend_paths`), which the
  store keeps as its ngspice path. `to_json`, `from_json`, `save_json`,
  `load_json` raise `NotImplementedError` naming their replacement, on
  both classes (`TestRemovedJson`, one test each).
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
- A Spectre-only analysis is written among the top's elements, before the
  analyses the renderer writes (not in queue order), its block's body
  unindented; a VACASK `extra_line` is written at the deck's top level, not
  inside `control`.
- An `extra_line` is one dialect's text (ngspice's by default), written for
  that family only: an ngspice line on HSPICE, Xyce or LTspice is refused
  (`W0001`), where the old writer copied it to every SPICE target. A
  Python `pss` on Spectre is refused (`W0001`). `ngbehavior` is written
  after the title on ngspice (was the second line too).

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
- pytest: 513 (510 at `949be79`, one failing): the three JSON
  serialization tests replaced by six store tests, the failing one fixed.

### Gates for decisions 1 and 2 (`6061c42`..)

- golden: record against `baseline-daa0ad0-fx11` (`--refusal-status-only`,
  `--skip verilog`): IDENTICAL except the 15 listed xfails;
  `check-determinism`: IDENTICAL.
- fmt, clippy `-D warnings` (all targets, all features), `check_style.py`:
  clean.
- merge tier: 370 (366 + the region reader's test and the store's three).
- pytest: 521 (507 at `b11d249` + 14: the store, the removed JSON methods
  one each, a deck's dialect text).

## Left

- The writers recurse over a branch's body (`spice::W::body` ↔
  `conditional`, `named::W::body` ↔ `conditional`), and a plan's `Cond`
  drops recursively, so a 10⁴-deep `.if` deck needs a large stack
  (`src/syntax/tests/render.rs` runs it on 256 MiB, as `golden` does). A
  worklist over the branches (and a manual `Drop`) is the fix; the adapter
  it replaced walked them on a heap stack.
- Python decks: the Spectre-only analyses sit outside `.step` sweeps; a
  VACASK `montecarlo` is refused (the old writer wrapped the inner analysis
  in `mc`); a `log` `.step` is not read (Xyce's form has none). The
  reader keeps a region's statements as text; reading a Spectre region's
  analyses as `Bench::Analysis` (as a Spectre file's are) would let
  `pac`, `sweep` and the rest be checked and spelled per target, and
  reading ngspice's `.pss` as an analysis would give Spectre its `pss`
  back.
- The store keeps a `ModelLibrary`'s ngspice path: a library with
  `backend_paths` stores the deck its ngspice netlist is written from.
- `.options itl1=` reaches Spectre as `itl1=` (only the portable name
  `max_iterations` maps to `dcmaxiters`): a quirk older than this phase.
- A VACASK charge-defined capacitor is refused `W0105`; `W0103` (a charge
  expression the target cannot spell) may be the better code.
- Verilog: a simple name that is a Verilog keyword is not escaped; an
  instance's parameters are noted, not written.
