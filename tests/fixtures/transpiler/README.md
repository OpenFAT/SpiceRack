# Transpiler fixtures

Made-up Spectre inputs for the transpiler rewrite
(`docs/research/transpiler-architecture.md`). They are known-bug inputs:
today's reader keeps several of their lines raw or refuses them, and the
golden baseline (`tools/golden`, TESTING.md "Golden gate") records exactly
that. Phase 3 (resolve) and phase 7 (new guarantees) are expected to change
their goldens, each change a reviewed line in `migration.xfail`.

No simulator has run these files. Nothing here is a hand-derived expected
value yet; each phase-7 test that uses one adds its own. The statistics and
Laplace fixtures are not in the `daa0ad0` baseline: `tests/test_render.rs`
writes each with the renderer and with the old writer
(`codegen::netlist(import::parse(..))`, whose statistics text T-R3/T-R4 of
`tests/test_monte_carlo.rs` check against monte-carlo-spectre.md §10.6),
asserts the two equal, and pins them in `pinned/<file>_<ext>/<target>.txt`
(or `.refused`: the old message and the new codes).

| File | Probes |
|---|---|
| `f1.scs` | One subcircuit holding the Spectre statement shapes the audit found mishandled: an instance with and without parenthesised nodes (`r1`/`r2`, `x1`/`x2`, `m1`/`m3`); an instance of a master defined nowhere (`othercell`, `extnch`); a 5-terminal MOS; `cccs` with `probe=` and a gain that is a number, an expression (`2*m`) or comes with an extra `m=`; `bsource` with `v=`/`i=` over `v(a,b)` and `v(a)` probes; a `vcvs` gain expression; a capacitor whose value is a ternary or a vector (`[1p 2p]`); a resistor expression; `parameters` inside a subcircuit. |
| `top.scs`, `lib.scs` | A two-file kit: `top.scs` includes `lib.scs`, and its instances name a subcircuit (`leaf`) and a model (`nch_mod`) defined only in `lib.scs`, with and without parentheses. Read alone, as today, `top.scs` cannot tell which SPICE letter each instance needs: the cross-file case resolve exists for (a proprietary kit's 293 raw instances). `f1` names a `probe=` source that is not in the file. |
| `gf180_inverter.scs` | From the paused `pdk-zero` work (`d1dc27e`, `tests/spectre/kits/`): a gf180mcu inverter that includes the PDK's ngspice files, one with `section=typical`, and instantiates `pfet_03v3`/`nfet_03v3` defined there. |
| `stats_process.scs`, `stats_correlate.scs`, `stats_correlate_cc.scs`, `stats_refused.scs` | Spectre `statistics` tables lowered for SPICE (`docs/research/monte-carlo-spectre.md` §6b, §10.5), none of which the corpus holds: process and mismatch `vary`, gauss, unif and lnorm, `percent`, `truncate` (4 sigma and over), a 2x2 `correlate` with a numeric `cc` and one with an expression `cc` in a library section; and the refused forms (`correlate dev=`, `truncate tr=3`). |
| `laplace.sp`, `laplace.cir`, `laplace.net` | Laplace behavioural sources (a lead-lag `V=` and a two-pole `I=`, each a numerator over a denominator) read as ngspice, Xyce and LTspice: the ngspice `s_xfer`, HSPICE/ESPice `LAPLACE` and LTspice `Laplace=` forms, Xyce's refusal. |
| `sky130_nmos.scs` | From the same place: a sky130 NMOS that includes the combined library with `section=tt` and instantiates `sky130_fd_pr__nfet_01v8` defined there. |

`f1.scs`, `top.scs` and `lib.scs` were written for the audit
(`docs/research/architecture-today.md`); the two kit decks are copied as
they are, including their header comments, which name `pdk-zero`'s
`tests/test_kits.rs` and `run.py` (not on this branch).
