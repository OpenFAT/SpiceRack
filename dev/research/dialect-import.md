# Dialect import: grammar-first importers and lossless round trips

Survey date: 2026-10-24. SpiceRack at `b022664` (branch `dialect-import`); ESPice at `287fb0af` (2026-10-23, the
commit `grammar/UPSTREAM` names). Local paths are relative to the SpiceRack root unless they start with
`ESPice/` (= `~/Documents/Projects/Zig/ESPice/`). "Suite" = spice-bench decks fetched to the session scratchpad
(`suites/{circuitsim90,cmcqa,ngspice,postlayout,powergrid}`), runnable decks = `*.sp` outside `src*` dirs.
The measurements below come from throwaway scripts in the session scratchpad (`count/count.py`,
`count/blockers.py`, `pegbench/`). They are not checked in, and the method is described next to each number.

Suite size, counting files pulled in by `.include`/`.lib`:

| suite | decks | files incl. includes | logical lines |
|---|---|---|---|
| circuitsim90 | 43 | 43 | 122 283 |
| cmcqa | 5 660 | 5 662 | 405 566 |
| ngspice | 353 | 422 | 451 616 |
| postlayout | 3 | 329 | 302 629 |
| powergrid | 6 | 6 | 2 316 138 |

---

## 1. ESPice's PEG grammars

### Notation
ESPice uses Ford's PEG (POPL 2004) in peg/leg spelling (`ESPice/tests/frontend/peg.zig:6-18`):

- `#` starts a comment.
- `Rule <- a b / c` gives sequence and ordered choice.
- `e* e+ e?` are greedy and do not backtrack into the repetition.
- `&e` and `!e` are lookaheads.
- `'lit'` and `"lit"` are literals. A trailing `i` (`'lit'i`) matches ASCII case-blind.
- `[a-z_\]]` and `[^\n]` are byte classes, with escapes `\n \r \t \\ \] \' \" \xHH`.
- `.` matches any byte, and `( e )` groups.

Captures need no markup. A rule whose name starts with an uppercase letter leaves a tree row; a lowercase rule is
lexical and leaves nothing (`peg.zig:19-21`). There are no actions and no named or labelled captures.

The interpreter is a plain backtracking matcher over a node table (`peg.zig:30-52`, `Tag` has 12 kinds). It does
no memoization (`peg.zig:410-470`: `rule` → `match`; a failed sequence truncates the row list). It caps recursion
at depth 512 (`peg.zig:44`) and reports the furthest offset on failure (`peg.zig:395-404`). Every node counts its
hits so that untested alternatives can be listed (`peg.zig:22-24`, `gaps`/`reportGaps` at `:503-536`). The file
is 633 lines.

SpiceRack already has the port: `src/peg.rs`, 471 lines. It keeps the same notation and semantics
(`src/peg.rs:1-15`, `DEPTH_MAX` at `:18`, `compile` at `:61`, `parse` at `:100`). It drops the hit counters and the
gap report (no `hits` in `src/peg.rs`). Its test checks that every grammar compiles alone and composed the way
ESPice composes them (`src/peg.rs:461-470`). The grammars are copied unchanged into `grammar/`
(`grammar/README.md`, `grammar/UPSTREAM`).

### Composition
Files are concatenated into one rule set. A duplicate rule is an error (`peg.zig:179`, `src/peg.rs:69`), so a
"dialect" file supplies the rules the shared file leaves undefined. Nothing is overridden. ESPice's harness does
this with `grammarOf` (`ESPice/tests/frontend/root.zig:24-29`).

| layer | file(s) | input → captures | used for |
|---|---|---|---|
| include | `include.peg` | physical lines → `Title Include Lib Endl Load Vec Data EndData Option Other` (SPICE); `Region SpiceOn SInclude IncFile IncSection Section Library ...` (Spectre) (`include.peg:9-15`) | `.include`/`.lib`/`.load` expansion before line joining |
| lines | `ngspice_lines.peg`, `hspice_lines.peg`, `spectre_lines.peg` | file bytes → `Title`, `Line` made of `Piece` (joined with a blank), plus `Glue` (joined with nothing after `\\`, HSPICE), `EndRun/NextRun` (HSPICE `.END` runs), `SpiceLine/HPiece/HGlue` (Spectre `simulator lang=spice` regions) (`ngspice_lines.peg:7-8`, `hspice_lines.peg:7-10`, `spectre_lines.peg:7-9`) | comments, continuation, quoting |
| cards (SPICE) | `spice_cards.peg` + `ngspice.peg`, or `spice_cards.peg` + `hspice_ops.peg` + `hspice.peg` (`root.zig:617,621`) | lowercased logical lines, `\n`-ended → statements `Element Model Subckt Param Func Options Temp Ic Nodeset Global Include Save Analysis Cond Control Block Other` and fields `Name Word Number Pair Key Group Expr Str` (`spice_cards.peg:10-15`) | statements |
| dialect leaves | `ngspice.peg` / `hspice.peg` + `hspice_ops.peg` | `Number Mantissa Exponent Scale`, the expression ladder `Ternary Or And Cmp Add Mul Pow Unary Paren Probe Call Num Ref`, and the operator rules; `field gfield value namechar wordchar` (`ngspice.peg:62-73`, `hspice.peg:54-60`, `hspice_ops.peg:1-24`) | numbers, quoting, expressions |
| cards (Spectre) | `spectre.peg` (alone, `root.zig:825`); its `SpiceCard` lines are re-read by `spice_cards.peg` + `spectre_ops.peg` + `hspice.peg` (`root.zig:915`, `spectre.peg:1-6`) | case-kept logical lines → `Simulator SpiceRegion SpiceCard Include AhdlInclude Library Section Global Parameters ParamDecl Model Group Subckt Ends Function FArg Save Ic Nodeset Signal If Else Statistics Analysis Control Instance` and fields `Name Node Master Kind Param Key Vector Expr...` (`spectre.peg:8-13`) | Spectre statements |
| data files | `vec.peg`, `extdata.peg`, `ic0.peg`, `rlgc.peg`, `touchstone.peg` | HSPICE `.vec`, external `.data`, `.ic0`, W-element RLGC, Touchstone (headers of each file) | side files only, not needed for netlist IR |

`spectre_ops.peg` exists so that a Spectre statement translated to a SPICE card keeps Spectre's `%` and `!`. The
same rules are spelled differently in `hspice_ops.peg` (`spectre_ops.peg:1-14`).

Two facts matter for SpiceRack:

- **The cards layer expects lowercased input** (`spice_cards.peg:4-5`; literals such as `'.subckt'` carry no `i`).
  The lowering must therefore slice names out of the original-case line. ASCII folding keeps byte offsets, so a
  row's `start..end` can index both strings.
- **`.lib` and `.include` of a netlist are not card rules.** ESPice expands them before the card stage
  (`spice_cards.peg:83-86`). The `Lib`/`Endl` captures live in `include.peg:31-34`.

### Coverage and known gaps
- **Fixture gaps.** `FIXTURE_GAPS.tsv` lists grammar alternatives that no conformance deck exercises: 187 rows, of
  which `hspice.peg` composition 90, `ngspice.peg` composition 62, `hspice_lines.peg` 21, `ngspice_lines.peg` 14
  (`ESPice/tests/frontend/FIXTURE_GAPS.tsv`; counted with `cut -f1 | uniq -c`). Examples: HSPICE `.if`, `.control`,
  `.data` bodies, `.func` with `=`, comparison operators.
- **Generator disagreements.** `GENERATED.tsv` lists generated or mutated decks that the grammar and the Zig
  frontend read differently:

  | dialect | differ | oracle_refused |
  |---|---|---|
  | ngspice | 25 | 11 |
  | hspice | 14 | 9 |
  | spectre | 0 | 3 |

  Leading cards include `.model`, `.op`, `.option`, `.data`, `.mc` and elements (`GENERATED.tsv` rows).
- **Documented deliberate departures:**
  - A lone `=` is not equality (`ngspice.peg:53-54`).
  - HSPICE has no `%`, `\` or unary `!` (`hspice_ops.peg:1-9`).
  - HSPICE braces are name bytes (`hspice_ops.peg:19-24`).
  - Spectre `else` is accepted only on the line where its branch ends (`spectre.peg:69-71`).
  - Spectre bitwise and shift operators are not read (`spectre.peg:118-120`).
  - `statistics` blocks are read and set aside (`spectre.peg:76-79`).
- **The research notes the headers cite** (`dev/research/{ngspice,hspice,spectre,data-file}-syntax.md`) are not in
  the ESPice working tree. Commit `05852567` (2026-10-17, "docs: drop ... research notes") deleted them, and they are
  recoverable with `git show 05852567^:dev/research/ngspice-syntax.md` (1372, 1003 and 1252 lines for
  ngspice/hspice/spectre). Their sources:
  - **ngspice:** manual "Version 47+" (2026-09-28) and ngspice git `pre-master-48` @ `940bb50`, with behaviour
    checks on ngspice-44.2.
  - **HSPICE:** UG08 (*Simulation and Analysis* B-2008.09, cseweb.ucsd.edu mirror), UG06, SH01 (Star-Hspice 2001.2),
    CR10 (*Commands and Control Options* E-2010.12, nthuee.org mirror), and Xyce XDM as the open implementation.
  - **Spectre:** UG 5.1.41 (2004, web.eecs.utk.edu mirror), R5/R11/R19 (*Spectre Reference* 5.0/11.1/19.1), and
    Kundert App. B.

  (`git show 05852567^:dev/research/{hspice,spectre}-syntax.md`, "Sources" sections.)
- **Licence:** not reviewed (the user owns ESPice). The repo is Apache-2.0 (`ESPice/LICENSE`).

### Measured: the copied grammars over the suite, through `src/peg.rs`
Method: the lines grammar on raw deck bytes, then joined `Piece`s, then `to_ascii_lowercase`, then
`spice_cards+ngspice` `Deck`, with no include expansion (scratchpad `pegbench/src/bin_check.rs`).

| suite | accepted | failures (first failing card) |
|---|---|---|
| circuitsim90 | 43/43 | none |
| powergrid | 6/6 | none |
| ngspice | 347/353 | `.lib` 4, `.csparam` 1 (not in `OtherKind`), `.elseif (select2 = 2)` 1 (lone `=`) |
| cmcqa | 2 532/5 660 | `.lib "sm141064.ngspice" typical` in all 3 128 failures |
| postlayout | 0/3 | `.lib` |

Grammar acceptance is therefore not the problem. Running `include.peg`'s pass first (or keeping `.lib` out of the
card text) fixes 3 135 of 3 137 failures.

Speed and memory (`pegbench/src/main.rs`, release build, this machine):

| deck | size | lines layer | card `Deck` parse | rows | per-card loop | peak RSS |
|---|---|---|---|---|---|---|
| `ibmpg1.sp` | 2 MB | 0.37 s | 0.21 s | 537 k | — | — |
| `ibmpg3.sp` | 96 MB, 1.6 M cards | 9.45 s | 7.6 s | 16.0 M | 10.1 s | 457 MB |

That is about 5.6 MB/s end to end, which is fine for the suite. The ceiling is the backtracking interpreter. A
first-byte dispatch table on `choice` nodes is the cheapest upgrade (unverified gain).

---

## 2. Rust PEG options (short; the interpreter already exists)

| crate | latest | grammar at run time | `'x'i` | notes |
|---|---|---|---|---|
| pest / pest_vm / pest_meta | 2.9.4 (2026-10-16, crates.io API) | yes: `pest_meta::parse_and_optimize(&str)` → `pest_vm::Vm::new(rules).parse(rule, input)` (pest `meta/src/lib.rs`, `vm/src/lib.rs`) | `^"x"`, ASCII via `eq_ignore_ascii_case` (`pest/src/position.rs` `match_insensitive`) | `Pairs` keeps the whole token queue (`pest/src/iterators/pairs.rs`, `Rc<Vec<QueueableToken>>`); no memoization; syntax `~`, `\|`, `^""`, `'a'..'z'` (https://pest.rs/book/grammars/syntax.html) |
| peg (rust-peg) | 0.8.6 (2026-05-04) | no, proc macro | none since 0.6 (https://github.com/kevinmehall/rust-peg/issues/216) | fastest PEG in parse-rosetta-rs (JSON 29 ms vs pest 62 ms; https://github.com/rosetta-rs/parse-rosetta-rs README) |
| peginator 0.7.0 (2024-02-11), pom 3.4.0, nom 8.0.0, chumsky 1.0.0-alpha.8, tiexiu 0.2.0 (TatSu port, run-time but immature) | — | only tiexiu | — | (crates.io API) |

**Recommendation:** keep `src/peg.rs`.

- It reads ESPice's notation with no translation and produces offset rows, not `Pairs`.
- It already compiles every grammar (`src/peg.rs:461-470`).
- pest_vm would need a translation script: `<-` → `= { }`, juxtaposition → `~`, `/` → `|`, `'x'i` → `^"x"`,
  `.` → `ANY`, `[a-z]` → `'a'..'z'` alternations, `[^…]` → `!(…) ~ ANY`, and capture-by-case → silent `_{}` for
  lowercase rules. It would also add a dependency for no gain.
- Optional, later: port `peg.zig`'s hit counters, so that SpiceRack's suite run can report grammar alternatives the
  suite never exercises (`peg.zig:503-536`).

---

## 3. Primary syntax references per dialect

Short base names:

- **LT** = https://ltwiki.org/LTspiceHelpXVII/LTspiceHelp/html/ (mirror of the LTspice XVII help).
- **ADI** = https://analogdevicesinc.github.io/ltspice-reference/ai_ref/SIMULATION-COMMANDS-REFERENCE.html
- **V** = https://codeberg.org/arpadbuermen/VACASK/src/branch/main/docs/ (cloned at `9ca4982`, 2026-10-22).
- **XRG/XUG** = https://xyce.sandia.gov/files/xyce/Xyce_Reference_Guide_7.9.pdf and `Xyce_Users_Guide_7.9.pdf`
  (printed pages).
- **SUG** = Spectre User Guide 5.1.41 (https://web.eecs.utk.edu/~dbouldin/protected/spectreuser.pdf).
- **HCR** = HSPICE Commands and Control Options E-2010.12.
- **HSA** = HSPICE Simulation and Analysis B-2008.09 (URLs in §1).

The Spectre and HSPICE references are the same documents ESPice's notes cite (§1).

| | LTspice (no grammar yet) | Xyce (no grammar yet) | VACASK (no grammar yet) | Spectre (`spectre*.peg`) | HSPICE (`hspice*.peg`) |
|---|---|---|---|---|---|
| title / end | line 1 ignored, `.end` optional (LT A_General_Structure_and_Conventions.htm) | line 1 title; `.END` last (XUG §4.1.3.1 p.42) | line 1 title (V input-overview.md) | line 1 title (SUG p.29) | line 1 title; `.END` ends a run, more runs may follow (HSA ch.4 pp.40-42; `hspice_lines.peg:19-22`) |
| comments | `*` line, `;` inline (LT ibid.) | `*` at column 1, `;` inline; **a line starting with whitespace is a comment unless its first non-blank is `+`** (XRG §2.1.39 p.163; re-check against Xyce 7.10 before relying on it) | `//`, `/* */` (V input-overview.md) | `//`, `*` at column 1 (SUG p.59) | `*` line, `$` after a blank, comma or number (HSA p.40-45; `hspice_lines.peg:55-62`) |
| continuation | `+` | `+` | trailing `\`; newlines inside `() [] {}` ignored (V input-overview.md) | trailing `\` or leading `+` (SUG p.59) | `+`, trailing `\`/`\\` (`hspice_lines.peg:37-45`) |
| case | insensitive (LT ibid.) | insensitive except file names (XUG Table 7-1; netlist rule unverified) | sensitive (V input-overview.md) | sensitive; `lang=spice` regions folded (SUG p.59) | insensitive except file names (HSA p.55) |
| expressions | `{}`; `**` power, **`^` is XOR**, `&`/`\|` boolean; `if(x,y,z)` true when x>.5; `table()`, `limit`, `u`, `uramp`… (LT DotParam.htm) | `{}` or `'…'`; `**` power, **`^` XOR**, `%`, `?:` at very low precedence; logical/relational only inside IF or ternary (XRG §2.2 pp.163-165) | bare; `+ - * / **`, `== != < <= > >=`, `&& \|\| !`, bitwise `& \| ^ ~ << >>`, `?:`, `a[i]` (V expr-operators.md) | bare; `**` right-assoc, `?:` (SUG pp.86-88) | `'…'` (and `"…"`); no `%`, `\`, unary `!` (`hspice_ops.peg:1-9`; HSA Table 3 p.42) |
| numbers | `M` milli, `Meg` mega, `6K34` = 6.34k (LT ibid.) | no `a` (atto) (XRG §6.2 Table 6-2) | `M`, `x`, `X`, `meg` = 1e6; hex, `inf`, `nan` (V input-numbers.md) | case-sensitive `M` mega / `m` milli (`spectre.peg:144-149`) | `X` and `MEG` = 1e6; no exponent+scale (`hspice.peg:5-14`) |
| params / funcs | `.param` (local in subckt), `.func name(args) {expr}` (LT DotParam.htm, DotFunc.htm) | `.PARAM`, `.GLOBAL_PARAM` (may depend on time), `.FUNC` (XRG §2.1.26 p.120, §2.1.12 p.45, §2.1.10 p.42) | `parameters a=1 b=a*2`; dependent params not overridable (V cir-subckt.md) | `parameters`, `real f(real a) { return … }` (SUG p.61, p.92) | `.PARAM name='expr'`, `f(a,b)='expr'`, last wins (HCR pp.236-237) |
| subckt | `.subckt n ports` … `.ends`; params on X line (LT DotSubckt.htm) | `.SUBCKT n nodes [PARAMS:] k=v` (XRG §2.1.37 p.159) | `subckt n (t…)` … `ends`; nestable (V cir-subckt.md) | `[inline] subckt n [(]ports[)]` … `ends` (SUG p.94, p.99) | `.SUBCKT n ports k=v` (HCR p.301) |
| instance / model | SPICE letters; `Rser= Lser= Rpar= Cpar=` on C, `Rser= Cpar=` on V (LT C-device.htm, V-device.htm); B `V=`/`I=` (LT B-device.htm) | SPICE letters plus `Y<type> name …` devices, e.g. `YMEMRISTOR` (XRG §2.3) | `name (nodes) master k=v`; `model n type k=v`; `load "x.osdi"` (V cir-instance.md, cir-masters.md, input-cir-loading.md) | `name (nodes) master k=v`; `model n master k=v` (SUG p.66, p.71) | SPICE letters (HSA ch.8) |
| analyses | `.tran Tstep Tstop [Tstart [dTmax]] [uic\|startup\|steady\|nodiscard]` (LT DotTran.htm, DotTranModifiers.htm); `.ac/.dc/.noise` (ADI); `.step` up to 3 nested (LT DotStep.htm); `.meas` (LT DotMeasure.htm) | `.TRAN … [NOOP] [UIC] [{schedule()}]` p.161; `.AC` p.24; `.DC [LIN\|DEC\|OCT] … \| LIST \| DATA=` p.27; `.STEP` p.155; `.DATA` p.26; `.SAMPLING` p.150; **only `.PRINT`ed variables are output** (XRG Table 6-2 p.851) | `control` … `endc`: `analysis n tran stop= step= …`, `ac from= to= mode= points=`, no dc analysis (DC sweep = `sweep` + `op`), `options rawfile="ascii"` (V cmd-overview.md, cmd-analysis*.md) | `name [(nodes)] tran stop=…` etc. (SUG p.66, p.139, p.141, p.135) | `.TRAN tstep tstop [START=] [UIC] [SWEEP …]` (HCR p.314); `.ALTER` p.33; `.DATA` p.64; `.TEMP` list p.309 |
| include / lib | `.include f`, `.lib f` (LT DotInclude.htm, DotLib.htm); ADI also lists `.lib f section` | `.INC[L[UDE]]`, `.LIB f entry` / `.LIB entry` … `.ENDL` (XRG p.49-50) | `include "f" [section=n]`, `section`/`endsection`; optional `include "x.sp" lang=ngspice\|hspice\|xyce\|spectre` if built with `CADNIP_PARSERS` (V input-include.md) | `include "f" section=n`, `library`/`section` (SUG p.72-75) | `.LIB 'f' entry`, `.LIB entry` … `.ENDL` (HCR p.150, p.97) |
| conditionals | — | — | `@if/@elseif/@else/@end` (V cir-conditional.md) | `if (c) {…} else {…}` (SUG p.109) | `.IF` (HCR p.142) |

**Pitfalls for one shared IR:**

- `^` means XOR in LTspice and Xyce but power in ngspice and HSPICE.
- `M` means mega in Spectre and VACASK but milli in SPICE.
- Xyce outputs only what `.PRINT` names.
- VACASK has no `dc` analysis.
- Spectre and VACASK are case-sensitive.

VACASK's own parser is flex/bison: `lib/dfllexer.l`, `lib/dflparser.y`, `lib/parser.cpp` (V repo tree). SpiceRack's
flake builds the fork `github:robtaylor/VACASK@bcd48e2` (`nix/vacask.nix:7-12`).

---

## 4. IR gaps

### 4a. What the capture trees map to in `CircuitIR` today
IR types: `src/ir/mod.rs:8-300`. The importer is `src/import.rs`; it does not use `src/peg.rs` yet (no `peg` in
`src/import.rs`).

| capture (grammar) | IR target | status |
|---|---|---|
| `Element` R/C/L (`spice_cards.peg:117`) | `Component::{Resistor,Capacitor,Inductor}` (`ir/mod.rs:101-103`) | ok; extra pairs → `params` |
| `Element` V/I with `Group` pulse/sin/pwl/exp/sffm/am | `VoltageSource/CurrentSource` + `IrWaveform` (`ir/mod.rs:105-106,155-162`) | numeric args only (`IrWaveform` fields are `f64`) |
| `Element` E/G/F/H linear | `Vcvs/Vccs/Cccs/Ccvs` (`:109-112`) | gain is `f64` only |
| `Element` E/G `value=`/`vol=`/`cur=`, B `v=`/`i=` | `BehavioralVoltage/Current` (`:107-108`) | E/G renamed `_name`, letter becomes B (`import.rs:309-311`), so `i(e1)` probes change |
| `Element` E/F/G/H `poly(n)`, `table`, `laplace` | none | raw |
| `Element` D/Q/M/J/Z | `Diode/Bjt/Mosfet/Jfet/Mesfet` (`:113-117`) | BJT has 3 nodes, so a Q with a substrate or thermal node goes raw; positional area after the model goes raw (`import.rs:345`) |
| `Element` S/W/T/K | `VSwitch/ISwitch/TLine/MutualInductor` (`:104,118-120`) | T only with exactly `z0=`, `td=` |
| `Element` A (XSPICE), N (OSDI), O/U/Y/P | `Xspice` exists (`:121`); the importer never fills it | raw |
| `Model` | `ModelDef{name,kind,parameters}` (`:89-93`) | binned `name.N` kept as plain names; no bin grouping for Spectre/VACASK |
| `Subckt`/`Ends`, `ports`, header `Pair`s | `Subcircuit` + `ParamDef` (`:17-30,73-76`) | one level (`import.rs:497`, `ponytail:` note); nested goes raw |
| `Param`/`Assign` top | `top.parameters` | ok |
| `Param` inside a subckt | none | raw (`import.rs:537,555`); `ParamDef` means header default (`spice3.rs` `subcircuit` emits `PARAMS:`) |
| `FuncDef`, `Func` (`.func`) | none | raw |
| `Options` | `SimOptions.portable: Vec<(String,String)>` (`:271-274`) | flags without `=` (`noacct`) go to `extra_lines` |
| `Temp` | `temperature: Option<f64>` (`:175`) | a list goes raw |
| `Ic`/`Nodeset` | `initial_conditions`/`node_sets` `Vec<(String,f64)>` (`:177-178`) | expressions lost |
| `Global` | none | raw |
| `Include` (`.hdl`, `.osdi_include`) / include.peg `Include`, `Lib` | `includes`, `libs: Vec<(path,section)>`, `osdi_loads` (`:25-27`) | `.lib name` … `.endl` definitions go raw (`import.rs:549`) |
| `Save`, `Other` print/plot/probe | `saves` | ok |
| `Other` meas | `measures: Vec<String>` (`:174`) | verbatim text, so the dialect spelling leaks |
| `Other` step | `step_params` (`:179,184-190`) | linear/`sweep_type` only; no `list`/`param` forms |
| `Analysis` op/dc/ac/tran/noise/tf/four | `Analysis` variants (`:197-212`) | `.dc` only 4-tuples; `.ac` no `list`; `.tran` no segments; `.pz/.sens/.disto/.pss/.sp` exist in the IR but `import.rs::control` never builds them (`import.rs:419-477`) |
| `Cond` (`.if`), `Control` (`.control`), `Block` (`.data`, `.variation`) | none | raw / `extra_lines` |
| Spectre `If`, `Function`, `Statistics`, `Library`/`Section`, `SpiceRegion` | none | — |
| any expression (`Expr`) | `IrValue::Expression{expr}` (`:146-150`) or verbatim `String` in `params`/`ParamDef.default` | the value text keeps its dialect spelling (`{…}` vs `'…'`, `^`), so it leaks |

### 4b. Construct counts in the suite
Counts are logical lines, with the number of files containing the construct in parentheses. Includes are followed.
Method: `count/count.py`, case-insensitive line-start regexes.

| construct | circuitsim90 | cmcqa | ngspice | postlayout | powergrid |
|---|---|---|---|---|---|
| POLY E/F/G/H | 0 | 0 | 397 (14) | 0 | 0 |
| E/G `table`/`value` | 0 | 0 | 1381 (16) | 0 | 0 |
| B-source | 0 | 0 | 210 (33) | 0 | 0 |
| `.func` | 0 | 0 | 75 (6) | 0 | 0 |
| `.global` | 0 | 0 | 9 (8) | 2 (1) | 0 |
| `.param` inside `.subckt` | 0 | 111 | 2377 | 388 | 0 |
| nested `.subckt` | 0 | 0 | 7 | 0 | 0 |
| `params:` on `.subckt` | 0 | 0 | 143 (6) | 0 | 0 |
| `.lib` section definition (`.lib name`) / `.endl` | 0 | 50 (1) | 5 (4) | 62 (2) | 0 |
| `.lib file section` call | 0 | 3195 (3129) | 8 (6) | 3 (3) | 0 |
| `.if/.elseif` | 0 | 0 | 12 (3) | 24 (2) | 0 |
| `.control` | 0 | 0 | 2 (2) | 0 | 0 |
| `.step` / `.alter` / `.data` | 0 / 0 / 0 | 0 | 0 | 0 | 0 |
| `.meas` | 0 | 132 (132) | 147 (13) | 0 | 0 |
| binned model `name.N` | 0 | 198 (1) | 26 (7) | 4497 (68) | 0 |
| model with `lmin=`/`wmin=` | 0 | 194 (1) | 30 (9) | 12 (6) | 0 |
| Q with ≥5 positional words, 5th not a number (4/5-terminal BJT) | 0 | 10 523 | 71 | 6 | 0 |
| XSPICE A-device | 4 (4) | 0 | 11 (11) | 0 | 0 |
| T/O/U/W/Y lines | 2 (2) | 0 | 138 (64) | 0 | 0 |
| element/model with `{` | 0 | 12 (6) | 3616 (31) | 802 (120) | 0 |
| element with `'…'` | 0 | 169 (1) | 886 (23) | 118 (19) | 0 |
| waveform with expression arg | 0 | 0 | 0 | 0 | 0 |
| `.tran` with >4 args (segments) | 0 | 0 | 0 | 0 | 0 |
| `.tran … uic` | 0 | 0 | 12 | 2 | 0 |
| `.tran` / `.ac` / `.dc` / `.op` / `.noise` | 38/2/3/7/0 | 132/282/4288/762/193 | 175/33/138/43/13 | 3/0/0/0/0 | 2/0/0/4/0 |
| `.pz` / `.sens` / `.tf` | 0 | 0 | 8/6/4 | 0 | 0 |

The cmcqa constructs inside includes all come from one PDK file, gf180 `sm141064.ngspice`, which 3 128 decks reach
through `.lib "sm141064.ngspice" <corner>` (`grep -h '^\.lib' cmcqa/**/*.sp | uniq -c`). With `libs` kept as
references, that file never enters the IR.

### 4c. What actually reaches raw passthrough today, and what it breaks
Method: `import::parse` on all 6 065 decks, then `codegen::netlist` for each dialect (`pegbench/src/bin_emit.rs`).

- 977 decks hold raw lines (11 383 `raw_spice` lines, 339 `extra_lines`). Leading token, as decks/lines:

  | token | decks / lines |
  |---|---|
  | `q` | 603 / 10 573 |
  | `.options` | 201 / 203 |
  | `v` | 62 / 434 |
  | `.model` | 49 / 92 |
  | `m` | 37 / 80 |
  | `.option` | 34 / 34 |
  | `o` | 18 / 65 |
  | `.global` | 9 / 11 |
  | `p` | 9 / 10 |
  | `y` | 9 / 31 |
  | `.pz` | 8 / 8 |
  | `.sp` | 7 / 7 |
  | `i` | 7 / 18 |
  | `.pss` | 6 / 6 |
  | `.sens` | 6 / 6 |
  | `e` / `g` | 5 / 15, 5 / 25 |
  | `.func` | 4 / 14 |
  | `.subckt` (nested) | 4 / 7 |
  | `.if` | 3 / 7 |

- Refusals by codegen:

  | dialect | refused |
  |---|---|
  | ngspice, hspice, ltspice, espice | 0 |
  | spectre | 22 (behavioural sources) |
  | vacask | 2 784 |

  VACASK's refusals: 972 + 972 + 585 for "model … LEVEL= has no VACASK device", 143 `.measure`, PWL sources, B
  sources.
- **Bug: raw lines silently change the Spectre and VACASK decks.** The contract is "an error, never a silently
  different deck" (`src/codegen/mod.rs:35-36`). In practice:
  - `spectre.rs` never reads `raw_spice`; the only uses are in tests. So top and subcircuit raw lines are dropped,
    and the codegen copies `tb.extra_lines` (SPICE dot cards) into the Spectre deck verbatim (`spectre.rs:456-458`).
  - `vacask.rs` copies `top.raw_spice` (`:690-692`) and `extra_lines` (`:743-745`) verbatim, SPICE text in a VACASK
    deck, and drops subcircuit `raw_spice`.

  Count: 962 of the 977 raw-holding decks emit to Spectre with no error, and 181 emit to VACASK.
- **Gaps that force raw**, and therefore break cross-dialect emission: BJT with more than 3 terminals; option flags;
  XSPICE/OSDI-N/O/U/Y/P elements; POLY/TABLE sources; `.func`; `.global`; local `.param`; nested subckt; `.lib`
  section bodies; `.if`; `.temp` lists; `.dc` `list`/`dec`/`param`; `.pz/.sens/.disto/.pss/.sp` (importer only).
  Brace and quote leakage (`w={l*2}` in `params`) does not force raw, but it emits HSPICE decks whose braces HSPICE
  reads as name bytes (`hspice_ops.peg:19-21`).

### 4d. What the public Python API can express
API in `src/python.rs`:

- `Subcircuit` methods `:953-1244`; `Circuit` extends `Subcircuit` (`:222`).
- `CircuitSimulator` `:339-660`, `Testbench` `:1266-1445`.

| construct | IR | Python API | where | suite decks using it (deck text only) |
|---|---|---|---|---|
| R/C/L value as expression or param name | `IrValue::Expression` | no: `PyValueArg` is `Float \| Unit` only | `python.rs:147-160` | ngspice 18 |
| R/C/L extra params (`tc1=`, `m=`) | `params` | no kwargs on `R/C/L` | `:960-978` | ngspice 22, postlayout 1 |
| D/Q/J/Z params or area | `params` | no kwargs (only `M` has `**kwargs`) | `:1030-1070` vs `:1047` | cmcqa 432, ngspice 21, circuitsim90 3 |
| BJT 4/5 terminals | no | no (`Q(collector, base, emitter, model)`) | `:1035` | cmcqa 585, ngspice 18 |
| E/G/F/H gain as expression | no (`f64`) | no | `:1004-1028` | 0 |
| PULSE delay ≠ 0 | yes (`delay`) | **no `delay` argument; `pulse()` hard-codes `delay: 0.0`** | `:170-172`, `:1112-1119` | circuitsim90 13, ngspice 65, powergrid 2 |
| SIN delay/damping/phase | yes | no (offset, amplitude, frequency only) | `:166`, `:1104-1110` | circuitsim90 2, ngspice 10 |
| EXP / SFFM / AM | yes | no constructor | — | circuitsim90 2, ngspice 1 |
| waveform plus `dc`/`ac` on one source | yes | only `SinusoidalVoltageSource` takes `ac`; Pulse/PWL take neither | `:1104-1124` | ngspice 71, postlayout 2 |
| PWL current source in a Subcircuit/Circuit | yes | only on `Testbench` | `:1345` | ngspice 7 |
| subckt header defaults | yes | yes, `Subcircuit(name, ports, **params)` | `:938` | ngspice 6 |
| local `.param` in a subckt | no | `parameter()` writes a header default, a different semantics | `:1186-1188` | (via PDK libs) cmcqa 3 128 |
| `.option` flag | no | `options(**kwargs)` needs a value | `:238` | cmcqa 132, ngspice 104, postlayout 2 |
| `.global`, `.func`, `.if`, nested subckt, POLY, XSPICE beyond `A()` | no | `raw_spice()` only | `:1169` | 8 / 4 / 3 / 4 / 5 / 28 |
| `.meas`, `.save`, `.ic`, `.nodeset`, `.step` | strings / pairs | `measure(*args)`, `save`, `initial_condition`, `node_set`, `step` | `:388-438` | — |

Decks fully expressible without raw text (blocker sets per deck from `count/blockers_deckonly.py`, deck text only):

| suite | through the IR | through the Python API |
|---|---|---|
| circuitsim90 | 43/43 | 27/43 |
| cmcqa | 4 943/5 660 | 4 511/5 660 |
| ngspice | 172/353 | 115/353 |
| postlayout | 0/3 | 0/3 |
| powergrid | 6/6 | 4/6 |

### 4e. Ranked SpiceRack improvements (decks unblocked, greedy over deck-only blocker sets, IR + Python)

| # | change | kind | decks unblocked (greedy) |
|---|---|---|---|
| 1 | BJT with optional substrate and thermal nodes (`Bjt.ns`, `Bjt.nt: Option<String>`), plus `Q(..., substrate=, thermal=)` | IR + API | 594 (603 decks / 10 573 raw `q` lines) |
| 2 | `**kwargs` params on `D/Q/J/Z` (as `M` has) | API | 437 |
| 3 | option flags: `SimOptions` entries with an empty value, emitted bare; `options("noacct")` | IR + API + importer | 186 (218 counting IR only) |
| 4 | **bug:** Spectre/VACASK must refuse `raw_spice`/`extra_lines` instead of dropping or copying them (`spectre.rs:456`, `vacask.rs:690,743`) | bug | correctness for 962 + 181 decks |
| 5 | `ac=`/`dc=` on every waveform source | API | 29 |
| 6 | `delay=` on `Pulse*Source` | API bug | 47 |
| 7 | importer: A/N/O/U/Y/P elements into `Xspice` or a generic device; IR for OSDI `N` | importer + IR | 24 |
| 8 | importer: `.pz/.sens/.disto/.pss/.sp` into existing `Analysis` variants | importer | 13 |
| 9 | `**kwargs` on `R/C/L` | API | 13 |
| 10 | `str` (expression) values for R/C/L/V/I (`PyValueArg::Expr`) | API | 9 |
| 11 | SIN delay/damping/phase | API | 10 |
| 12 | `.global`, `.dc` list/dec/param, binned-model grouping, PWL current on Subcircuit, `.func`, nested subckt, POLY, `.if`, EXP/SFFM/AM constructors, `.temp` list | IR / API | 5, 5, 7, 5, 4, 4, 3, 3, 3, 2 |
| 13 | **test bug:** the suite compares only the first plot's last point (§5) | test | every deck |

---

## 5. Round-trip verification design

### What the current test checks
`tests/test_import_suite.rs` runs `ngspice -b -r out.raw` on the original and the re-emitted deck (`:42`), parses
the result with `rawfile::parse_raw` (`:57`), and compares the last point of each shared vector (`:61-80`). There
are two problems:

- **Only one plot is compared.** `parse_raw` returns a single `RawData` with one `plot_name`
  (`src/rawfile.rs:25`, `src/result.rs:6-20`). ngspice writes one plot per analysis to the file. For a deck holding
  `.op`, `.ac` and `.tran` it wrote 3 plots ordered *AC, Operating Point, Transient*, which is not card order
  (observed with ngspice-44.2, `grep Plotname`). So only the first plot is compared.
- **`.meas` never runs.** With `-b -r`, ngspice prints "No .measure possible in batch mode (-b) with -r rawfile
  set!" (observed, ngspice-44.2).

### ASCII raw
Both routes exist:

- **`SPICE_ASCIIRAWFILE=1`:** ngspice manual §12.7.1 "Ngspice specific variables": "0 for binary, and 1 for ascii".
  Observed: `SPICE_ASCIIRAWFILE=1 ngspice -b -r f.raw` writes `Values:`, all 3 plots.
- **`set filetype=ascii`:** manual §13.7 and §12.6 (.spiceinit); `write` is §13.5.108. Observed: inside `.control`,
  `run` then `write f.raw` writes only the current (last) plot. All plots need `-r`, `set appendwrite`, or one
  `write` per `setplot`.

ASCII is not needed, since `rawfile.rs` reads both formats.

### Proposed checks, cheapest first

1. **Accounting.** Every captured statement row maps to exactly one IR item or one counted raw line. Test
   `rows(Element|Model|…|Analysis|Other) == ir_items + raw + extra`, so nothing vanishes. This catches the Spectre
   drop bug in §4c.
2. **Fixpoint per dialect.** `ir1 = lower(parse_d(src))`; `txt = netlist(ir1, d)`; `ir2 = lower(parse_d(txt))`.
   Assert `ir1 == ir2` and `netlist(ir2, d) == txt`, so emission is idempotent. Run it for d ∈ {ngspice, hspice,
   ltspice, espice, spectre, vacask, xyce}. No simulator is needed, so it works for HSPICE, Spectre and LTspice.
3. **Analysis multiset.** The sorted `Debug` of `ir.testbench.analyses`, plus the analysis-kind `extra_lines`, must
   be equal before and after. This is cheap and catches dropped or merged analyses.
4. **Numeric, same simulator.** Steps:
   - Write a multi-plot raw reader: loop `parse_raw_utf8` until EOF, returning `Vec<RawData>`.
   - Compare plot count and the plot-name sequence.
   - Compare variable sets case-insensitively. Exclude internal `#branch` names that the re-emitted deck may rename;
     for E/G→B, map `_name`.
   - Grid: op, dc and ac must have identical sweep vectors (same length, values within 1e-12 relative). Transient
     time grids differ, so linearly interpolate the converted run onto the original's time points and compare every
     point with `|a-b| <= atol + rtol*max(|a|,|b|)` (`rtol` 1e-3, `atol` 1e-6 V / 1e-9 A per variable kind).
   - Compare AC complex values on real and imaginary parts, or magnitude in dB plus phase, with the same rule.
   - Compare noise and other plots like dc.
   - For `.meas`: make a second run without `-r`, parse stdout with `measure_parse::parse_measures(text, "ngspice")`
     (`src/measure_parse.rs:22`), and require equal name→value maps.
5. **Numeric, cross dialect.** Emit to X, run simulator X, and compare against the ngspice original with step 4's
   comparator, at a looser tolerance (different device models and timestep control).

### Simulators available (checked 2026-10-24)

| simulator | status |
|---|---|
| `ngspice` | ngspice-44.2 at `~/.nix-profile/bin/ngspice` |
| `xyce`/`Xyce` | not on PATH; nixpkgs has 7.10.0 (`nix eval nixpkgs#xyce.version`) and ESPice's `benchmarking` shell includes `pkgs.xyce` (`ESPice/flake.nix:114-124`) |
| `vacask`, `openvaf` | in SpiceRack's `nix develop .#testing` (`flake.nix:76-84`, `nix/vacask.nix`); not on PATH; the default shell has none ("flake: no simulators in the default shell", commit `8e029a7`) |
| `espice` | no binary (`ESPice/zig-out/` holds only `include/` and `share/`); `zig build` in ESPice produces `espice` (`ESPice/build.zig:78-79`). ESPice reads ngspice, HSPICE and Spectre, so it is the only local simulator that can execute the HSPICE and Spectre emissions. |
| `spectre`, `hspice` | not available (commercial). Fixpoint (step 2) plus ESPice runs. |
| `ltspice` | not available; `wine` is on PATH, so LTspice under wine is possible (unverified). Fixpoint only. |

---

## 6. Recommended architecture and implementation plan

### Architecture
The shape is one interpreter, one grammar set per dialect, and two lowerings.

```
deck text ──include pass (include.peg)──► .include/.lib refs → IR includes/libs; .lib bodies split off
          ──lines grammar────────────────► logical lines (Piece/Glue joined), lowercased copy + original
          ──cards grammar (src/peg.rs)───► capture rows
          ──lower_spice | lower_spectre──► CircuitIR (case from the original text, same offsets)
CircuitIR ──codegen::netlist(ir, d)──────► text
```

| dialect | lines | cards | lowering |
|---|---|---|---|
| ngspice, ESPice | `ngspice_lines.peg` | `spice_cards.peg`+`ngspice.peg` | spice |
| HSPICE | `hspice_lines.peg` | `spice_cards.peg`+`hspice_ops.peg`+`hspice.peg` | spice |
| LTspice | **new** `ltspice_lines.peg` (`;` comments, no `$`, `.end` optional) | `spice_cards.peg`+**new** `ltspice.peg` (Number with `6K34`, `^` = XOR, `**`, `if()`, `table()`) | spice |
| Xyce | **new** `xyce_lines.peg` (leading-blank comment rule, `;`) | `spice_cards.peg`+**new** `xyce.peg` (`^` XOR, `%`, Y-devices) | spice |
| Spectre | `spectre_lines.peg` | `spectre.peg`; `SpiceCard` via `spice_cards.peg`+`spectre_ops.peg`+`hspice.peg` | spectre |
| VACASK | **new** `vacask_lines.peg` (`//`, `/* */`, `\`, bracket newlines) | **new** `vacask.peg`, reusing `spectre.peg`'s capture names (`Instance Model Subckt Parameters Analysis Save`) plus `Load`, `Control`, `Sweep`, `AtIf` | spectre |

The lowering is a `match g.name(row.rule)` over the capture names in §4a, plus ESPice's node-count table for SPICE
elements (`ESPice/tests/frontend/lower.zig:940-965`, model lookup for D/Q/M). For scale, ESPice's own lowerings to
a canonical netlist are 1 318 lines (`lower.zig`) and 767 lines (`lower_spectre.zig`).

Where new grammars live: in SpiceRack's `grammar/`, outside the `UPSTREAM` set, because ESPice has no reader for
LTspice, Xyce or VACASK to check them against. `spice_cards.peg` stays unchanged. Extra keywords (`.csparam`,
`.backanno`, `.global_param`, `.print` forms) are handled by a dialect file, which needs `OtherKind` to be a hole
the dialect file fills; that has to be agreed upstream in ESPice first. Until then, a pre-filter in Rust can keep
such cards as raw.

`ponytail:` skipped on purpose:

- No grammar codegen and no pest.
- No per-dialect lowering beyond the two families. Dialect differences are small tables (element node counts,
  option-name maps) inside the two lowerings.

### Ordered plan, smallest first
1. **Fix the silent-drop bug.** Spectre and VACASK should return `Err` on any `raw_spice`/`extra_lines` (§4c). Add
   the accounting check (§5.1) to `tests/test_import_suite.rs`.
2. **Read every plot.** Add a multi-plot `parse_raw_all` plus the comparator of §5.4, and a second ngspice run
   without `-r` for `.meas`.
3. **Switch `src/import.rs` from tokens to `src/peg.rs` for ngspice.** Use the include pass, `ngspice_lines.peg`,
   `spice_cards+ngspice`, and `lower_spice`. Gate: the same IR as today on the 6 065 decks, or strictly fewer raw
   lines. Measured cost: about 17 s on the 96 MB `ibmpg3.sp`.
4. **HSPICE import.** Same lowering with the HSPICE composition; fixpoint test (§5.2).
5. **IR/API items 1–3 and 5–6 of §4e** (BJT terminals, D/Q/J/Z kwargs, option flags, source `ac`/`dc`, pulse
   delay). Together they clear about 1 290 deck-blocks.
6. **Spectre import** (`spectre.peg` → `lower_spectre`). Fixpoint; numeric check through ESPice.
7. **`vacask_lines.peg` + `vacask.peg` from V docs.** Fixpoint; numeric check through `nix develop .#testing`.
8. **`xyce*.peg` from XRG 7.9.** Add a Xyce codegen (none exists: `codegen::Dialect` has no Xyce variant,
   `src/codegen/mod.rs:21-33`); `.PRINT` every saved vector; numeric check with nixpkgs Xyce 7.10.
9. **`ltspice*.peg` from LT/ADI.** Fixpoint only (no local LTspice).
10. **Remaining IR gaps by deck count** (§4e row 12), then expressions as a parsed tree, so that `{}` vs `'…'` and
    `^`/`**` are re-spelled per dialect instead of leaking.
