# SpiceRack's architecture today

A read-only audit of `main` at `9efaefc` (2026-11-03), plus the uncommitted
WIP commit `d1dc27e` on the `pdk-zero` worktree, which holds the latest fix
(the cross-file Spectre master). Every claim cites `file:line` on `main`
unless it names another commit. Line counts are code only: a file's lines
before its `#[cfg(test)]`. Nothing was built or run for this note. Where a
hazard is inferred by reading the code and was not run, the text says
"by reading".

The short version: the pipeline has the stages you would expect, but three
layers that should be shared are missing. These are a token and number
layer, an expression tree, and a cross-file symbol table. Each grammar
re-derives the first two, the IR stores the second as source text, and the
third does not exist. So every construct that touches values, expressions or
names in another file gets its own patch, in each grammar that spells it and
in each codegen that writes it.

§7 maps every stage and module onto the target transpiler shape (tokenizer,
parser, AST, renderer, no lowering), and gives the layer that would own each
of the fixes in flight on `pdk-zero`.

---

## 1. The stages as they exist

```
bytes
  │ import::decode                      src/import/mod.rs:86-98   (UTF-16LE BOM / UTF-8 / Latin-1)
  ▼
String (whole file)
  │ Grammar::parse(lang.file)           src/import/mod.rs:103-106 (the dialect's `file` rule)
  ▼
peg::Tree: Title, Line{Piece|Glue}*     src/peg.rs:43-54
  │ import::logical                     src/import/mod.rs:216-236 (parts trimmed, joined by ' ' or '')
  ▼
String: logical lines, '\n'-ended       (title dropped; physical line numbers lost)
  │ to_ascii_lowercase if lang.fold     src/import/mod.rs:107-112 (SPICE family only)
  ▼
String: folded copy (same offsets)
  │ Grammar::parse(lang.deck)           src/import/mod.rs:113-116 (spice.peg + overlay, or spectre/vacask.peg)
  ▼
peg::Tree: rows tagged by import::Tag   src/import/mod.rs:59-76 (~130 tags, one schema for all dialects)
  │ lower::deck(tree, original text, d) src/import/lower/mod.rs:75-146
  ▼
(CircuitIR, Report)                     src/ir/circuit.rs:25-47, src/import/mod.rs:41-46
  │ codegen::netlist(ir, d)             src/codegen/mod.rs:49-80
  │   statistics lift/lower (IR -> IR)  src/codegen/mod.rs:53-67
  │   check_caret_meaning, check_functions  src/codegen/mod.rs:68-69
  │   dispatch per dialect family       src/codegen/mod.rs:70-79
  ▼
String: deck text
```

| Stage | Input | Output type | Where | Code lines | Reaches into |
|---|---|---|---|---|---|
| decode | `&[u8]` | `String` | `src/import/mod.rs:86-98` | 13 | nothing |
| grammar compile | `.peg` texts, layered | `peg::Grammar` (rule table) | `src/peg.rs:96-153`; `src/import/mod.rs:169-212` | 556 (peg.rs) | `Tag::of` for capture names (`src/import/mod.rs:159-162`) |
| line grammar | file text | `Tree` of `Title`/`Line`/`Piece`/`Glue` | the `file` rule of each `grammar/*.peg` | 6 line stages, 129 rules (§2.3) | nothing |
| logical lines | `Tree` | `String` | `src/import/mod.rs:216-236` | 21 | nothing |
| case fold | `String` | `String` | `src/import/mod.rs:107-112` | 5 | `Lang.fold` (`src/import/mod.rs:36`) |
| card grammar | folded text | `Tree` (`Row{rule, tag, start, end, next}`) | the `deck` rule; `spice.peg` (246 lines) under one of 4 overlays, or `spectre.peg` (271) / `vacask.peg` (188) alone | 1,246 grammar lines in total | nothing |
| lower | `Tree` and original-case text (same offsets) | `(CircuitIR, Report)` | `src/import/lower/*.rs` (10 files) | 2,850 | the grammar again: `.control` commands are re-parsed with the `one` rule (`src/import/lower/analyses.rs:21-30`); **codegen**: `codegen::format_spice_number` (`src/import/lower/values.rs:166`) and `codegen::statistics::unlift` (`src/import/lower/mod.rs:142-144`) |
| IR | | `CircuitIR` | `src/ir/*.rs` | 2,030 (plus 213 schema check) | `ir::features` calls `codegen::laplace::is_laplace` (`src/ir/features.rs:214`) |
| codegen pre-passes | `CircuitIR` | `Option<CircuitIR>` (a rewritten copy) | `src/codegen/statistics/*` | 987 | the IR's expression strings, by byte scanning (§3.4) |
| codegen spice3 | `CircuitIR`, `d` ∈ {ngspice, ESPice, LTspice, HSPICE, Xyce} | `String` | `src/codegen/spice3/*.rs` | 1,960 | `unit::ngspice_*` |
| codegen spectre | `CircuitIR` | `String` | `src/codegen/spectre/*.rs` | 2,107 | `codegen::vacask::parse_spice_num` (`src/codegen/spectre/values.rs:25`) |
| codegen vacask | `CircuitIR` | `String` | `src/codegen/vacask/*.rs` | 1,770 | |
| codegen xschem (CDL, Verilog, layouts) | `CircuitIR` | `String` | `src/codegen/xschem/*.rs` | 923 | |
| codegen shared | | | `src/codegen/mod.rs`, `append.rs`, `laplace.rs` | 377 + 567 | |

There are other readers of netlist text beside the PEG path. The backend
re-reads its own emitted deck with `spice3::control::{is_four, is_meas,
runs_in_control}` (`src/backend/ngspice.rs:13`, `src/codegen/spice3/control.rs:21-45`).
`lint::lint_netlist` re-tokenizes netlist text on whitespace
(`src/lint.rs:97-110`) and calls `codegen::statistics::has_statistical`
(`src/lint.rs:312`). `codegen::laplace` has its own lexer and parser
(`src/codegen/laplace.rs:327`, `:395`).

What the design note planned and the code did not build
(`dev/research/dialect-import.md:446-452`, `:487-490`):

- An include pass. It does not exist: `parse` reads one file
  (`src/import/mod.rs:101`), and `tools/spice2rack/main.rs:67` calls it per file.
- "Two lowerings" (spice, spectre). There is one lowering with
  `named = matches!(d, Spectre | Vacask)` branches through it
  (`src/import/lower/mod.rs:203`, `:209-234`).
- "Expressions as a parsed tree, so that `{}` vs `'…'` and `^`/`**` are
  re-spelled per dialect instead of leaking." This was step 10, and it is
  still deferred.
- ESPice re-reads the SPICE text of a Spectre `simulator lang=spice` region
  with the SPICE card grammar. SpiceRack keeps those lines raw
  (`grammar/spectre.peg:80-82`, `src/import/lower/named.rs:107-118`).

---

## 2. Where dialect knowledge lives

### 2.1 Counts

- **411** `Dialect::X` references in `src/` (`grep -ro 'Dialect::[A-Z][a-z]*' src | wc -l`).
- **99** explicit branch sites in `src/import`, `src/codegen` and `src/ir`
  outside tests (`match d`, `matches!(d, …)`, `d == Dialect::…`). Another
  **~12** branch on flags derived from the dialect: `let (lt, ng, hs)` in
  `src/codegen/spice3/analyses.rs:14` is used at `:56, :112, :135, :159, :169, :190, :210, :234, :262`,
  `ngspice(d)` in `src/codegen/spice3/values.rs:46`, and `named` in
  `src/import/lower/mod.rs:203`. A further **13** decisions branch on
  `ir.raw_dialect`, the origin dialect (§3.3).
- **7** grammar files: one shared SPICE file and 6 dialect files.
- **4** codegen families: `spice3` serves 5 dialects through `d`,
  `spectre` and `vacask` serve one each, and `xschem` serves CDL, Verilog
  and the layouts. On top of these sit `statistics`, which picks a
  different transform per dialect (`src/codegen/mod.rs:57-66`), and
  `laplace`.
- `src/backend/mod.rs` has 44 more references. Those are simulator
  selection and are out of scope here.

### 2.2 The branch sites, classified

The classes are (a) lexical, (b) syntactic, (c) semantic, (d) output
spelling, (e) capability.

| Site | What it decides | Class |
|---|---|---|
| `src/import/mod.rs:178`, `:195` | which grammar texts, and `fold` (case-blind or not) | a, b |
| `src/import/lower/values.rs:31` | ngspice numbers through `unit::ngspice_value`, other dialects through the grammar's `Mag`/scale rows | a |
| `src/import/lower/values.rs:200-229` | Spectre `\x` escapes, VACASK `'…'` name quoting | a |
| `src/import/lower/values.rs:282` | LTspice `µ` → `u` | a |
| `src/import/lower/values.rs:158` | ngspice `{"e"}` and `{a} + b` normalized (`one_group`) | b, c |
| `src/import/lower/values.rs:163-167` | a Spectre or VACASK number respelled into SPICE suffixes in the IR | a, d |
| `src/import/lower/mod.rs:102` | prescan of model kinds and subckt names (Spectre, VACASK only) | c |
| `src/import/lower/mod.rs:121` | LTspice implicit `Rser=1m` made explicit | c |
| `src/import/lower/mod.rs:127` | VACASK loads sorted (for the fixpoint) | d |
| `src/import/lower/mod.rs:142` | Spectre: un-lift SpiceRack's own statistics pattern | c |
| `src/import/lower/mod.rs:193` | a raw SPICE element stays among components; a raw Spectre line goes to `raw_spice` | b |
| `src/import/lower/mod.rs:203-234`, `:329-332` | Spectre and VACASK statements take a different path; `parameters` in a subckt are the definition's, not locals | b, c |
| `src/import/lower/mod.rs:303-315` | Xyce MOS `level=8/9` → 49 | c |
| `src/import/lower/named.rs:52`, `:87`, `:190`, `:202` | VACASK module → kind, drop of `spice/*.osdi`, `type=` dropping | c |
| `src/import/lower/named_elements.rs:242`, `:250`, `:318`, `:326` | which built-in masters each dialect has | c |
| `src/import/lower/named_elements.rs:347`, `:358` | key table per dialect; VACASK names lowercased | c, a |
| `src/import/lower/named_elements.rs:385`, `:424`, `:459` | waveform meaning (Spectre `exp`, VACASK `exp`, Spectre `pwl`) | c |
| `src/import/lower/analyses.rs:64`, `:98` | Xyce `.options <pkg>` | b, c |
| `src/codegen/mod.rs:57-66` | which statistics transform runs | c, e |
| `src/codegen/mod.rs:70`, `:85` | dispatch | |
| `src/codegen/mod.rs:198-204` | which dialect has statistical functions | e |
| `src/codegen/mod.rs:257-259` | `^` means power or XOR | c |
| `src/codegen/spice3/mod.rs:53`, `:66` | ngspice `.control` run block, `ngbehavior` | c, d |
| `src/codegen/spice3/mod.rs:77-80` and `src/codegen/spice3/sections.rs:63-66` | HSPICE function as `.param f(x)='…'` (the same block twice) | d |
| `src/codegen/spice3/mod.rs:115`, `:152-170` | Xyce options; HSPICE `.probe`/`.option post`; ngspice noise `.save all` | d, c |
| `src/codegen/spice3/mod.rs:173`, `:231` | LTspice no `.print`; HSPICE no `.end` without analyses | d, e |
| `src/codegen/spice3/mod.rs:273-322` | `pre_osdi`, `.hdl`, Xyce refuses loads | d, e |
| `src/codegen/spice3/mod.rs:350-352` | HSPICE quotes paths | d |
| `src/codegen/spice3/mod.rs:362` | Xyce `level 8` → 49 (the same fact as `src/import/lower/mod.rs:303`) | c |
| `src/codegen/spice3/mod.rs:391` | HSPICE omits `PARAMS:` | d |
| `src/codegen/spice3/values.rs:56`, `:68` | HSPICE `'…'` expression delimiters, `hspice_value` | d |
| `src/codegen/spice3/components.rs:51` | LTspice refuses R/C/L model reference | e |
| `src/codegen/spice3/components.rs:71-75` | charge capacitor `Q={}` / `Q=` / `Q=''` | d |
| `src/codegen/spice3/components.rs:154-175` | B source as HSPICE `E vol=`/`G cur=`, Xyce `B V={}` | d |
| `src/codegen/spice3/components.rs:359`, `:371` | XSPICE: HSPICE and Xyce refuse; **LTspice writes it as a comment** | e |
| `src/codegen/spice3/components.rs:446`, `:452`, `:462` | Laplace per dialect | e, d |
| `src/codegen/spice3/analyses.rs:135`, `:141`, `:159-262` | per-analysis spelling and refusals | d, e |
| `src/codegen/spice3/analyses.rs:287-320` | `.step` per dialect | e, d |
| `src/codegen/spice3/conditionals.rs:31` | Xyce and LTspice have no `.if` | e |
| `src/codegen/spice3/sections.rs:50` | LTspice has no `.lib` section | e |
| `src/codegen/spice3/waveform.rs:31-32` | TRNOISE/TRRANDOM ngspice only | e |
| `src/codegen/xschem/*.rs` (about 20 sites) | comment char, ident spelling, port lists | d |
| `src/ir/features.rs:58-94`, `:169` | a 7-row capability table for `check_backend` | e |

Rough totals: (a) ~10, (b) ~6, (c) ~25, (d) ~40, (e) ~25. Two things stand
out. Capability (e) lives in two places: the `MISSING` table
(`src/ir/features.rs:58`) and about 25 refusals scattered through codegen.
They disagree. LTspice XSPICE is a `check_backend` error
(`src/ir/features.rs:59-62`), but `netlist()` writes it as a comment
(`src/codegen/spice3/components.rs:370-374`). That breaks the "never a
silently different deck" contract of `src/codegen/mod.rs:27-28` and
`:47-48`. Semantic facts are also written twice, once in each direction:
the Xyce level map, and the Spectre and VACASK master maps (§2.5).

### 2.3 Duplicated logic across the grammars

`spice.peg` is shared by four SPICE overlays; `spectre.peg` and `vacask.peg`
stand alone (`grammar/README.md`). Counting rules by name:

| Concern | spice.peg | ngspice | hspice | ltspice | xyce | spectre | vacask | Copies |
|---|---|---|---|---|---|---|---|---|
| line stage (`file`, `l_*`, `Line`, `Piece`, `Glue`) | | 21 rules | 24 | 15 | 13 | 34 (incl. a copy of HSPICE's, `spectre.peg:50-63`) | 22 | 6 (+1 copy) |
| numbers (`Num`, `Mag`, `mantissa`, `scale`, 10-13 scale tags) | | 15 (`ngspice.peg:101-115`) | 16 (`hspice.peg:87-102`) | 16 (`ltspice.peg:63-78`) | 15 (`xyce.peg:59-73`) | 17 (`spectre.peg:239-255`) | 18 (`vacask.peg:154-171`) | 6 |
| expression ladder | | 14 `x_*` (`ngspice.peg:145-163`) | 14 (`hspice.peg:115-130`) | 13 (`ltspice.peg:89-105`) | 16 (`xyce.peg:84-102`) | 14 `s_*` (`spectre.peg:221-235`) | 20 `v_*` (`vacask.peg:128-149`) | 6 (91 rules) |
| `field`/`gfield`/`value`/`namechar`/`wordchar` | | `ngspice.peg:167-175` | `hspice.peg:132-140` | `ltspice.peg:107-111` | `xyce.peg:104-111` | own `Name`/`Val` | own | 4 + 2 |
| `dotcard` alternation | `spice.peg:62-65` | | `hspice.peg:77-80` | `ltspice.peg:53-56` | `xyce.peg:50-53` | `stmt` `spectre.peg:85-86` | `stmt` `vacask.peg:46-47` | 4 copies differing by one alternative |
| parameters (`Params`, `Assign`) | `spice.peg:91-92` | `ngspice.peg:124`, `:130` | | `Params_lt` `ltspice.peg:45` | | `spectre.peg:155-157` | `vacask.peg:64-65` | 5 |
| models | `spice.peg:76-87` | | | | `MVal` `xyce.peg:114` | `Model`, `Model_group`, a `Block` form (`spectre.peg:111`, `:140`, `:160`) | `vacask.peg:68` | 3 (+2) |
| instances | 21 per-letter rules plus `X` (`spice.peg:164-224`) | | | `Raw_lt` | | `Inst` (`spectre.peg:205-206`) | `Inst` (`vacask.peg:115`) | 3 families |
| subckt | `spice.peg:28-32` | | | | | `spectre.peg:163-167` | `vacask.peg:71-74` | 3 |
| library section | `spice.peg:37-39` | | | `never` | | `spectre.peg:102-106` | `vacask.peg:79-80` | 3 |
| conditional | `If` `spice.peg:44-49` **and** `ifblock` `:55` | | | | | `If` `:130-135` **and** a `Block` if `:141` | `If` `:84-88` **and** `Block` `:91-92` | 3, each with a raw fallback |
| include, save, global | `spice.peg:68-71`, `:118-123`, `:115` | | `Out` `hspice.peg:82` | `Lib` `ltspice.peg:49` | `Include` `xyce.peg:38` | `spectre.peg:92-95`, `:178-183`, `:151` | `vacask.peg:50-61`, `:105-108` | 3-4 |

Three things follow from this table.

1. **A rule replaced in an overlay is a full copy.** The PEG layering
   replaces a rule by name (`src/peg.rs:125-126`) and cannot extend one. So
   adding `Probe` to HSPICE meant copying the whole 4-line `dotcard`
   alternation (`grammar/hspice.peg:77-80`). LTspice (`Step`) and Xyce
   (`Step`) did the same. Adding a dot card to all SPICE dialects now means
   editing 4 copies. The ngspice `Params` override (`grammar/ngspice.peg:124`)
   is `spice.peg:91` plus one alternative.
2. **Card shapes and value syntax are tangled together.** The card rules
   call `value`, `expr`, `Num` and `namechar`, and each overlay redefines
   those. The card grammar therefore has no shape that is independent of
   the dialect's lexical rules, and a new value form (sky130's
   `{a} + b`, `grammar/ngspice.peg:130-131`) changes card rules.
3. **The expression ladders are parsed and then discarded.** All 91
   `x_*`/`s_*`/`v_*` rules are lowercase and so not captured
   (`src/peg.rs:13-15`). The tree keeps one `Expr` row whose text is the
   source (`grammar/ngspice.peg:132`). The precedence work the grammars do
   (LTspice's `-2**1.5`, `ltspice.peg:95-97`; Xyce's measured levels,
   `xyce.peg:80-83`) decides only where an expression ends. Its structure
   never reaches the IR.

### 2.4 Duplicated logic across the codegens

Each family writes its own deck spine over the same IR walk: header,
functions, globals, loads, includes, libs, model libraries, parameters,
statistics, models, subckts, sections, body, stimulus, options, ICs, saves
and analyses. The three spines are `src/codegen/spice3/mod.rs:46-237`,
`src/codegen/spectre/mod.rs:77-~235` and `src/codegen/vacask/mod.rs:87-~370`.
Each family then has its own:

| Concern | spice3 | spectre | vacask |
|---|---|---|---|
| subcircuit | `spice3/mod.rs:379` | `spectre/mod.rs:315`, `:331` | `vacask/mod.rs:431`, `:449` |
| instance | `spice3/mod.rs:431` | `spectre/mod.rs:420` | `vacask/mod.rs:555` |
| conditionals | `spice3/conditionals.rs:21` | `spectre/mod.rs:371` | `vacask/mod.rs:491` |
| options | `codegen/mod.rs:348` + `spice3/mod.rs:448` (Xyce) | `spectre/mod.rs:435` | `vacask/mod.rs:522` |
| model | `spice3/mod.rs:358` | `spectre/mod.rs:240`, `:274` | `vacask/models.rs:58` |
| number spelling | `spice3/values.rs:20`, `:30` | `spectre/values.rs:39` | `vacask/values.rs:36` |
| value/expression respelling | `spice3/values.rs:67-116` | `spectre/values.rs:21-29` | `vacask/values.rs:69-111` |
| component | `spice3/components.rs:14` (one `match` over 24 variants) | `spectre/components.rs:17` | `vacask/components.rs:13` |
| waveform | `spice3/waveform.rs:42` | `spectre/values.rs:65` | `vacask/components.rs:353` |

`Component::Resistor` is matched in 12 files and `Analysis::Transient` in
18. A new `Component` variant is therefore an arm in each of three
component writers, three waveform or value helpers, the statistics walk
(`src/codegen/statistics/mod.rs:144`), two lowerings, the schema and Python.
`9cba644` (charge capacitors) touched 15 files.

### 2.5 Tables written twice, once per direction

- The Spectre master map: forward in `src/codegen/spectre/mod.rs:274`
  (`spectre_master`), inverse in `src/import/lower/named.rs:223-254`
  (`spectre_kind`, whose comment calls it "the inverse of
  `codegen::spectre`'s map").
- The VACASK module map: forward in `src/codegen/vacask/models.rs:136`, `:158`,
  inverse in `src/import/lower/named.rs:258-276`, and a third list of the
  built-ins at `src/import/lower/named.rs:20-31`.
- Spectre and VACASK source keys: `src/import/lower/named_elements.rs:35-81`
  ("the inverse of `codegen::spectre` and `codegen::vacask`").
- Xyce level 8 → 49: `src/import/lower/mod.rs:303-315` and
  `src/codegen/spice3/mod.rs:362`.
- Brace-depth scanning of value strings: `src/import/lower/values.rs:254-275`
  (`one_group`) and `src/codegen/spice3/values.rs:78-116` (`hspice_value`)
  re-derive the same grouping.

Nothing checks that a forward table and its inverse agree, except the
round-trip tests that happen to cover them.

---

## 3. The IR

### 3.1 Main types

- `CircuitIR` (`src/ir/circuit.rs:25-47`): `top: Subcircuit`,
  `testbench: Option<Testbench>`, `subcircuit_defs`, `model_libraries`,
  `raw_dialect: Option<Dialect>`, `globals`, `functions: Vec<FuncDef>` and
  `sections: Vec<LibSection>`.
- `Subcircuit` (`src/ir/circuit.rs:87-114`): `ports`, `parameters`
  (`ParamDef{name, default: Option<String>}`, `:182-185`), `components`,
  `instances`, `models`, **`raw_spice: Vec<String>`**, `includes`, `libs`,
  `osdi_loads`, `verilog_blocks`, `locals`, `statistics` and
  `conditionals`. The 738da71 and 9efaefc fixes added `locals`,
  `statistics` and `conditionals`.
- `Instance` (`src/ir/circuit.rs:189-199`): `name`, `subcircuit`,
  `port_mapping`, `parameters: Vec<(String, String)>` and `at`. WIP
  `d1dc27e` adds `unresolved: bool`.
- `ModelDef` (`src/ir/circuit.rs:210-214`): `name`, `kind: String`
  (SPICE's `nmos`, or a Spectre master kept as-is,
  `src/import/lower/named.rs:252`) and `parameters: Vec<(String, String)>`.
- `Component` (`src/ir/component.rs:21-226`): 24 variants, one per device
  **and per form**. Examples are `Capacitor` and `ChargeCapacitor`
  (`:97-104`), `Vcvs` and `Poly`, `BehavioralVoltage`, and `RawSpice` (`:223-225`).
- `IrValue` (`src/ir/waveform.rs:24-28`): `Numeric{f64}`,
  `Expression{String}` or `Raw{String}`. `IrNum` (`:34-37`) is
  `Num(f64)` or `Expr(String)`.
- `Testbench` (`src/ir/testbench.rs:26-49`): `analyses`, `options`,
  `saves: Vec<String>`, **`measures: Vec<String>`**,
  **`extra_lines: Vec<String>`**, `step_params` and `prints`.
- `Conditional`/`Branch` (`src/ir/circuit.rs:123-139`):
  `condition: Option<String>`.
- `Statistics` (`src/ir/statistics.rs:16-61`).

### 3.2 Escape hatches

| Hatch | Where | Filled by | Consumed by |
|---|---|---|---|
| `Subcircuit.raw_spice: Vec<String>` | `src/ir/circuit.rs:94` | `Cx::raw` (`src/import/lower/mod.rs:186-200`), `Unparsed` (`:236-240`), Spectre SPICE regions (`src/import/lower/named.rs:111-114`) | written verbatim; refused by a dialect of another family (`src/codegen/spice3/mod.rs:244-268`, `src/codegen/mod.rs:264-277`) |
| `Component::RawSpice{line}` | `src/ir/component.rs:223` | a SPICE element card with no IR form, kept in place (`src/import/lower/mod.rs:193-197`) | the same checks |
| `IrValue::Raw{text}` | `src/ir/waveform.rs:27` | a value that is neither number nor identifier-led (`src/import/lower/values.rs:124-126`) | written verbatim by every codegen (`src/codegen/spice3/values.rs:58`, `src/codegen/spectre/values.rs:58`) |
| `Testbench.extra_lines` | `src/ir/testbench.rs:37` | VACASK control statements (`src/import/lower/named.rs:119-128`) | written verbatim |
| `Testbench.measures: Vec<String>` | `src/ir/testbench.rs:31` | `.meas` text | written verbatim with a `.meas` prefix (`src/codegen/spice3/mod.rs:180-186`) |
| every `(String, String)` parameter list | instances, models, R/C/L `params` | `Cx::params`/`vtext` (`src/import/lower/values.rs:139-186`) | each codegen's respeller |
| `ModelDef.kind: String` | `src/ir/circuit.rs:212` | SPICE type, or a Spectre master verbatim | `spectre_master`; other dialects |
| `SimOptions.backend_specific` | `src/ir/testbench.rs:237+` | Xyce `pkg.key` (`src/import/lower/analyses.rs:64-103`) | `src/codegen/spice3/mod.rs:485-497` |

The numeric fields are also narrow. `Testbench` and its analyses have 29
`f64` fields and no `IrNum` (`src/ir/testbench.rs`). `Component` has 13
`f64` fields: `Vcvs.gain`, `Cccs.gain`, `MutualInductor.coupling`,
`TLine.z0`/`td`, `Poly.coeffs` and others. Any of these written as an
expression (`.tran 0 {tstop}`, `E1 a b c d {g}`) has no IR form. The card
then fails the grammar (`spice.peg:194` wants `Num`) or the lowering
(`num("gain")?`, `src/import/lower/named_elements.rs:208`), and goes raw.

### 3.3 Dialect-neutral versus dialect-tinted

Neutral: the shape. That covers subcircuits, ports, instance/component
order (`at`), sections, conditionals' structure and analyses as enums.

Tinted:

- **Expressions are source text.** `IrValue::Expression{expr}`,
  `ParamDef.default`, `FuncDef.body`, `Branch.condition`,
  `BehavioralVoltage.expression` and every `(String, String)` value are
  slices of the source with only the outer delimiters removed
  (`src/import/lower/values.rs:113-122`, `src/import/lower/conditionals.rs`
  in `9efaefc`). They keep the source's operators (`^`, `**`, `%`, `~`),
  its function names, its number suffixes inside the expression and its
  case. Lowering reads the original-case text at the folded offsets
  (`src/import/lower/mod.rs:45-47`).
- **Numbers in value strings get a third spelling.** `vtext` respells a
  whole-number Spectre or VACASK value into SPICE suffixes
  (`src/import/lower/values.rs:163-167`, "the IR's SPICE spelling"). It
  leaves an expression as written. So `r=2*1M` from Spectre reaches the IR
  as `2*1M`. That reads as mega in Spectre and as milli in every SPICE
  reader. `spectre_value` also respells only whole numbers
  (`src/codegen/spectre/values.rs:21-29`), and so does VACASK
  (`src/codegen/vacask/values.rs:69-81`). By reading, then, a suffix inside
  an expression is never translated, and nothing refuses it. Only `^` is
  checked (`src/codegen/mod.rs:149-169`).
- **`raw_dialect` is one field with six jobs.** The comment
  (`src/ir/circuit.rs:30-34`) says it is the dialect of the raw lines. It
  is set to the source dialect for every import
  (`src/import/lower/mod.rs:130`), and then read as:
  1. the raw lines' owner (`src/codegen/spice3/mod.rs:244-268`, `src/codegen/mod.rs:266`);
  2. ngspice arithmetic for numbers (`src/codegen/spice3/mod.rs:51`);
  3. the meaning of `^` (`src/codegen/mod.rs:150`);
  4. Spectre case-keeping (`src/codegen/spectre/mod.rs:79`);
  5. VACASK lowercasing of names and expressions (`src/codegen/vacask/mod.rs:97`, `src/codegen/vacask/values.rs:98-111`);
  6. VACASK's user loads and analysis case (`src/codegen/vacask/mod.rs:162`, `:351`, `:422`).

  One `CircuitIR` therefore cannot hold content from two dialects. A
  Spectre deck with a `simulator lang=spice` region, or a kit merged from
  several files (`Known::add` in `d1dc27e`), has no correct value for it.
- **Names.** Spectre names are kept as written in WIP `d1dc27e`. Before
  that, the SPICE letter was stripped (`src/import/lower/named_elements.rs:357`).
  VACASK names are lowercased. SPICE names keep source case, though the
  language is case-blind.

### 3.4 Expressions: strings, operated on by hand

The expression has no tree. Every operation on it is a byte scanner over
the string:

- `codegen::calls` (`src/codegen/mod.rs:225-241`) and `check_caret_meaning`'s `contains('^')` (`:155`);
- `codegen::undelimited` (`src/codegen/mod.rs:246-252`);
- `statistics::rewrite_calls`, `rename_idents`, `matching`, `split_args`
  (`src/codegen/statistics/mod.rs:50-97`, `:117-140`, `:172`, `:190`).
  These drive Monte Carlo lifting, lowering and function inlining;
- `one_group` (`src/import/lower/values.rs:254-275`) and `hspice_value`
  (`src/codegen/spice3/values.rs:78-116`);
- `unwrapped` for conditions (`src/import/lower/conditionals.rs`, `9efaefc`);
- `lint::idents`, `param_assigns` (`src/lint.rs:370`, `:394`);
- `laplace::lex`/`parse`, a real parser but only for `Laplace(...)` (`src/codegen/laplace.rs:327-460`).

None of these knows strings, quoting, `v(a,b)` probes or the dialect's
identifier set (`rename_idents` treats `.` as a separator, `src/codegen/statistics/mod.rs:119`).
Each is correct for the cases its commit tested.

---

## 4. Refusals and diagnostics

- **Import never fails on content.** `parse` returns `Err(String)` only
  when a grammar cannot run at all: "line grammar stops at byte N" or
  "card grammar stops at byte N" (`src/import/mod.rs:106`, `:116`). A card
  the grammar refuses becomes `Report.unparsed: Vec<String>`, the logical
  line's text. A card with no IR form increments `Report.raw: usize`
  (`src/import/mod.rs:41-46`). Neither records a reason, a rule, a source
  file or a physical line number. The logical lines have already lost them
  (`src/import/mod.rs:216-236`). The `Report` of a deck is not stored in
  the IR. One more diagnostic goes to stderr:
  `eprintln!("warning: .subckt {} has no .ends")` (`src/import/lower/mod.rs:430`).
- **Codegen refuses with `CodeGenError`** (`src/codegen/mod.rs:29-42`).
  Its variants are `UnsupportedComponent{backend, component: String}`,
  `UnsupportedAnalysis{backend, analysis: String}` and `Other(String)`.
  About 125 construction sites are spread over 22 files: 17 in
  `vacask/components.rs`, 9 each in `vacask/mod.rs`, `vacask/analyses.rs`
  and `codegen/mod.rs`, 8 in `spice3/mod.rs`, and so on. Several of them go
  through per-family helpers (`spectre/values.rs:223-236`,
  `vacask/values.rs:195-208`). The message is free text written at the
  site. Its first refusal stops the deck, so a deck with ten problems
  reports one.
- **Capability is also checked separately.** `CircuitIR::check_backend`
  returns `Issue{severity, message}` from the `MISSING` table
  (`src/ir/features.rs:41-94`). `lint::Finding{line, message, …}`
  (`src/lint.rs:24-30`) covers the emitted text.
- **There is no catalogue.** No refusal carries a code, no enum names the
  reasons, and there is no list a test or user can enumerate. A test can
  find a refusal only by substring: `unwrap_err().to_string().contains("…")`.
  The "unsupported fails and never skips" rule (`AGENTS.md`) is enforced by
  convention at each site. The LTspice XSPICE comment (§2.2) shows that a
  site can break it unnoticed.

---

## 5. Root causes, example by example

### 5.1 Library sections (`738da71`, 38 files, 6 grammar, 9 codegen)

Before this commit, `.lib name` … `.endl` was `libblock`, a raw `Block`
(the removed lines in `grammar/spice.peg`, `git show 738da71`). The fix
added the following:

- `LibDef` in `spice.peg:37-39`, `Library`/`LibDef` in `spectre.peg:102-106`, `LibDef` in `vacask.peg:79-80`, and `LibDef <- never` in `ltspice.peg:51`;
- a third `Scope::Section` with its own leak rules (`src/import/lower/mod.rs:66-73`, `:205-208`, `src/import/lower/sections.rs`);
- the `LibSection` type (`src/ir/circuit.rs:56-76`);
- three writers: `spice3/sections.rs`, `spectre/library.rs` and `vacask/sections.rs`.

**Why no general rule:** "a named block of cards that reads as the top of a
deck" is spelled three ways, in three grammars that share no block
combinator. The meaning of a card depends on its container (top, subckt,
section). That dependency is coded as `scope` guards inside
`Cx::card`'s 170-line `match` (`src/import/lower/mod.rs:202-374`), not as
a table of card kind × container.

### 5.2 `.param` inside a subckt (`738da71`)

The grammar already read it. The lowering sent it raw because the IR had
no field for it. The fix added `Subcircuit.locals` and the guards
`Tag::Params if inside && !named`
(`src/import/lower/mod.rs:328-340`). The same tag means a local in SPICE
and the definition's parameter in Spectre and VACASK, so the branch is on
`named`. **Why:** the IR held only what an earlier writer needed. With no
scope model, each newly found nesting (`.param` in subckt, `.func` in a
subckt (still raw, `:328`), a function in a section,
`LibSection.functions` in `9efaefc`) becomes a new field and a new guard.

### 5.3 sky130 value forms (`738da71`)

ngspice's `{"e"}`, `{a} + b` and the lone `+` between assignments needed
an `expr` alternative (`grammar/ngspice.peg:120`), a copied `Params`
(`:124`) and a new `Val_p` (`:130-131`). They then needed
`one_group`, a string rewrite in the lowering
(`src/import/lower/values.rs:254-275`). For HSPICE output, the
`hspice_value` string rewrite came later (`src/codegen/spice3/values.rs:78-116`,
`9cba644`). **Why:** `Val_p` parses `{1+x} + x*AGAUSS(…)` into a full
expression (`x_ternary`), and the parse is then thrown away
(§2.3, point 3). The fact that two groups plus a tail make one
expression has to be re-derived by scanning braces. It is re-derived
twice: once to normalize (`one_group`) and once to spell for HSPICE
(`hspice_value`). With a tree, `{a} + b` is `Add(a, b)`. Each
dialect's printer chooses the delimiters, and no special case exists.

### 5.4 The X card's late subckt name (`9cba644`)

sky130's `special_pfet_latch` alias puts parameters before the subckt
name. The fix is a second `X` shape in the grammar
(`grammar/spice.peg:218-223`), "only that shape". **Why:** the grammar has
to decide by shape which bare word is the master, because nothing at
parse time knows which words name subcircuits. `nodes`
(`spice.peg:208`) and Spectre's `Inst` (`spectre.peg:205-206`) make the
same guess, and so does the lowering's model/value test for `R1 a b rmod`
(`src/import/lower/elements.rs:52-60`, using `Cx.models` built from this
file only, `src/import/lower/mod.rs:82-86`). With a symbol table, the rule
"the master is the word that names a subckt" would cover every ordering.

### 5.5 Charge capacitors (`9cba644`, 15 files)

A `C` card with `q=` and no `c=` became a new `Component::ChargeCapacitor`
(`src/ir/component.rs:97-104`). That meant a special case in the lowering
(`src/import/lower/elements.rs`, the `None if tag == Tag::C && … "q"` arm),
three spellings in spice3 (`src/codegen/spice3/components.rs:71-75`), a
Spectre `bsource q=` and a VACASK refusal. **Why:** `Component` is a
closed enum of device × parameter form, with fields typed per form. A
parameter that changes what the device computes therefore has to be a new
variant, and every codegen has to add an arm. The general form, a device
kind plus terminals plus named parameters whose values are expression
trees, with a table of which `(kind, param)` each dialect can spell,
would cover `q=`, `Q=` and `bsource q=` with table rows.

### 5.6 Monte Carlo lifting (`cfabf54`, 36 files, +3,271 lines)

This commit added 987 lines of `codegen/statistics`. Lifting rewrites each
`agauss(…)` call into `rack_f(args, z)` with declared variables, and
lowering turns Spectre tables back into calls
(`src/codegen/mod.rs:53-67`). The import side then had to undo the lift:
`lower::deck` calls `codegen::statistics::unlift`
(`src/import/lower/mod.rs:142-144`), so the reader depends on the writer.
**Why:** every step is a find-calls-and-rewrite over strings
(`rewrite_calls`, `rename_idents`, `split_args`;
`src/codegen/statistics/mod.rs:50-190`). Each dialect's statistical
function set is a list checked by name (`STATISTICAL`, `:46`;
`check_functions`, `src/codegen/mod.rs:197-222`). An expression tree with
a call table per dialect (name, arity, deterministic or random, the
target's spelling) would make lift and lower ordinary tree rewrites, and
"LTspice has no agauss" a table lookup.

### 5.7 Spectre if/else (`9efaefc`, 25 files, 4 grammar)

This commit added `If`/`Branch`/`Cond` three times (`grammar/spice.peg:44-49`,
`grammar/spectre.peg:130-135`, `grammar/vacask.peg:84-88`). It kept the
old raw `Block` forms as fallbacks in all three (`spice.peg:55`,
`spectre.peg:141`, `vacask.peg:91-92`). It also added
`Conditional{branches, at}` with `condition: Option<String>`, and three
writers (`spice3/conditionals.rs`, `spectre/mod.rs:371`,
`vacask/mod.rs:491`). Reading `.if` bodies exposed an unrelated loss: SFFM
and AM fields 6-8 that the IR lacks were dropped, and now go raw
(`9efaefc` message; `src/import/lower/elements.rs`). **Why:** the same
three-grammar, three-writer fan-out as 5.1. The condition is a string, so
the Spectre writer passes it through `spectre_value`
(`src/codegen/spectre/mod.rs:379-380`), and only the outer delimiters
change. `.if (a == 1)` reaches Spectre with its SPICE operators and
suffixes unchanged (by reading).

### 5.8 A Spectre instance kept raw when its master is in another file (WIP `d1dc27e`)

On `main`, `Cx::inst` resolves the master in three places. It checks
`subckts` (`src/import/lower/named_elements.rs:277`) and `kinds`
(`:287`), both built by `declarations()` from this file's rows only
(`src/import/lower/named.rs:37-72`), and otherwise returns `None` (`:336`).
The card then goes raw (`src/import/lower/mod.rs:230-232`). One raw line
then makes the whole deck unwritable for any other dialect family
(`src/codegen/spice3/mod.rs:257`, `src/codegen/mod.rs:269-276`). The
WIP fix adds:

- a `Known{subckts, models}` passed in by the caller (`parse_with`, `src/import/mod.rs` in `d1dc27e`);
- a merge "this file wins" (`src/import/lower/mod.rs` in `d1dc27e`);
- `Instance.unresolved: bool` for a master that is still unknown;
- a fallback `Component::Osdi` for a known model of an unknown kind.

**Why:** per-file parsing with no symbol table. The meaning of
`name (nodes) master` depends on what `master` is, and the answer lives
in another file of the kit. The planned include pass
(`dev/research/dialect-import.md:446`) would have provided it. `Known` is
that table, built by the caller instead of the reader. It still resolves
at parse time, so the IR records a guess (`unresolved`) instead of a name
to resolve later. The same root cause sits behind 5.4 (SPICE `X`) and
`R1 a b rmod` (`Cx.models`).

### 5.9 What the eight have in common

| Missing layer | Examples it would have absorbed |
|---|---|
| one token/number layer shared by all grammars (§2.3: 6 number grammars, plus `unit::ngspice_value`, `unit::parse_spice_number`, `vacask::parse_spice_num`, `Cx::num`) | the `M`/`meg` mega-milli traps, sky130 value forms, HSPICE spelling |
| an expression tree in the IR, with per-dialect operator, function and delimiter tables | 5.3, 5.6, 5.7's conditions, `^` (`src/codegen/mod.rs:149`), the suffix-in-expression hazard (§3.3) |
| a symbol table across files, with resolution after parsing | 5.4, 5.8, `R1 a b rmod` |
| card shapes written once, with the dialect supplying lexemes and keywords | 5.1, 5.7 (three `If`s, three `LibDef`s), the 4 `dotcard` copies |
| capability and spelling as tables (dialect × construct → spelling or refusal code) | 5.5's five spellings, the ~25 scattered capability refusals, `MISSING` disagreeing with `netlist` |
| a scope model (top, subckt, section, branch) applied to card kinds | 5.1, 5.2 |

Each fix landed a slice of one of these layers, specialized to the
construct at hand.

---

## 6. Tests, goldens, and what a byte-identical gate needs

### 6.1 Tiers (from `TESTING.md` and `Cargo.toml:40-47`)

| Tier | What | Time (documented) |
|---|---|---|
| pre-commit | fmt, clippy, `check_style.py` | target under 10 s (`AGENTS.md`) |
| merge | `cargo test` (343 tests after `9efaefc`) and pytest | cargo 37 s, pytest 198 s (`TESTING.md`, Known gaps) |
| nightly | `--features corpus --test test_import_suite`: the spice-bench round trip on real ngspice, plus corpus fixpoints | spice-bench 2,586 CPU s, which is 5 min 25 s wall on 20 threads or 20 min on a 4-vCPU runner (`TESTING.md`; `Cargo.toml:41`). Fixpoint-only time is not documented |

### 6.2 What exists that can gate a restructure

- **Fixpoint** (`tests/test_import_suite/fixpoint.rs:15-24`). This checks
  `netlist(ir, d)` → `parse` → equal IR, and equal text on a second emit,
  for one dialect at a time. Corpora: spice-bench (6,050 decks, `CHANGELOG.md:18`;
  `ngspice_corpus_fixpoint`), HSPICE (ESPice conformance `tests/netlist`),
  Xyce Regression, VACASK sources and LTspice spice-datasets (4,342 decks,
  `dev/research/dialect-readers.md:1004`), pinned in `flake.nix:122-126`.
  **Spectre has no corpus.** `spectre_corpus_fixpoint` is a known
  `#[should_panic]` (`TESTING.md`, Known gaps).
- **`$GOLDEN_DIR`** (`tests/test_import_suite/fixpoint.rs:41-57`). When
  this is set, every corpus deck's IR (pretty JSON) and `Report` is
  written to `$GOLDEN_DIR/<Dialect>/<path>.json`, "for diffing two
  importers byte for byte". This is the saved-IR golden. It is generated
  on demand and not checked in.
- **A precedent for a byte-identical refactor gate.** `CHANGELOG.md:26`
  (#15) records "every deck the two test suites emit, in every dialect, and
  the same IRs with every name renamed to need escaping, are
  byte-identical before and after (9378 decks)". That harness is not in
  the tree.
- **Hand-counted fixtures and real-PDK tests** in the merge tier:
  `tests/test_sections.rs`, `test_sections_pdk.rs` (sky130's ~190 files,
  gf180mcu), `test_conditionals.rs` and `test_monte_carlo.rs`. These check
  meaning and simulator runs, not bytes.
- **Inline goldens**, with the expected text in the test source:
  `tests/test_spectre_golden/*.rs` (79 asserts) and the codegen unit tests
  in `src/codegen/*/mod.rs`. There are no golden files on disk except
  `tests/spectre/expected.json`.

### 6.3 What is missing for a byte-identical gate

1. **Emitted text per deck per target, checked in or cached.** The gate
   needs a manifest keyed by deck content hash. For each corpus deck and
   each PDK file, it holds: the IR JSON and `Report` (`$GOLDEN_DIR`
   already writes these), and for every target dialect, the emitted text
   or the refusal message. Today, the cross-dialect refusals are "counted,
   not failed" (`tests/test_import_suite/main.rs:16-17`), so a restructure
   can change which decks refuse without any test failing.
2. **Spectre input.** Spectre has no corpus, so the PDK Spectre files
   (gf180mcu, and sky130 if present under `PDK_ROOT`) and
   `tests/fixtures/**/*.scs` are the only Spectre bytes. They should be in
   the manifest.
3. **A cheap tier.** Fixpoint plus `$GOLDEN_DIR` needs no simulator, so it
   could run without the 2,586 CPU s ngspice part. Its time is not
   measured anywhere. Measure it before choosing the tier.
4. **Refusal identity.** Messages are free text (§4), so the gate would
   compare strings. A refusal code would let a restructure reword a
   message without failing the gate.
5. **Multi-file input.** `Known` (WIP) means one file's IR depends on
   others. The manifest must record the read order of a kit, or the gate
   compares against a different input.

---

## 7. Mapped onto the transpiler shape

The new target has four parts per dialect: a tokenizer, a parser, an AST,
and a renderer that writes that AST in any dialect, for netlists and
analyses alike, with no lowering. "No lowering" still leaves two passes
between parse and render, and I name them below rather than hide them:

- **resolve:** binds every name reference to its declaration across the
  files of a kit. It annotates the AST and converts nothing.
- **AST rewrites:** tree-to-tree transforms that a target needs, such as
  Monte Carlo lifting or Laplace. Today these exist as string rewrites.

Meaning that differs between dialects is fixed by each dialect's parser
when it builds the node. For example, `^` becomes `Pow` or `Xor` there,
and a Spectre `cccs` becomes the same device node as a SPICE `F` card.
The renderer then only spells.

### 7.1 What survives, what changes owner, what disappears

| Today | Fate in the new shape | Note |
|---|---|---|
| `import::decode` (`src/import/mod.rs:86-98`) | **survives**, before the tokenizer | |
| `peg.rs` engine (`src/peg.rs`) | **survives** as the parser engine if PEG stays | It must capture expression structure: today lowercase rules leave no rows (`src/peg.rs:13-15`) |
| line stage of each `.peg` (§2.3, 129 rules) | **becomes the tokenizer** | It should emit tokens with file/line spans and decoded number values, not a re-joined string (`src/import/mod.rs:216-236`). Comments, continuation, quoting and suffixes are all lexical (a). |
| `logical()`, case fold (`src/import/mod.rs:107-112`, `:216-236`) | **disappears** into the tokenizer | Case-blindness becomes token comparison, not a lowercased copy |
| card grammars `spice.peg` + 4 overlays, `spectre.peg`, `vacask.peg` | **become the parsers** | Card shapes are written once per family (SPICE, Spectre/VACASK). The dialect supplies keywords and lexemes, ending the 4 `dotcard` copies and the 3 `If`/`LibDef` copies. The 6 expression ladders become 6 thin precedence tables over one expression parser that **keeps the tree**. |
| `import::Tag` (`src/import/mod.rs:59-76`) | **becomes the AST node kinds**, typed | |
| `src/import/lower/*` (2,850 lines) | **disappears as a stage**. Its knowledge moves: | |
| …`values.rs` number decode, name unescape, `µ` (`:28-66`, `:198-230`, `:281-287`) | tokenizer | |
| …`one_group`, `vtext` respelling (`:155-186`, `:254-275`) | parser: `{a} + b` parses to `Add(a, b)` | |
| …`declarations()`, `Cx.models`, `Known` | **resolve** | §7.2 |
| …master and key maps (`named.rs:223-276`, `named_elements.rs:20-81`) | one **device table** per dialect (master ↔ device kind ↔ parameter keys), read by that dialect's parser and renderer | Ends the forward and inverse copies of §2.5 |
| …LTspice implicit `Rser` (`lower/mod.rs:118-125`), Xyce level map (`:303-315`) | the device table's per-dialect defaults, applied by the renderer | |
| …`unlift` (`lower/mod.rs:142-144`) | **disappears**: the AST keeps the calls as written | |
| …`Scope` guards (`lower/mod.rs:66-73`, `:328-340`) | parser: a `.param` node inside a subckt *is* a local by position | |
| `CircuitIR` (`src/ir/`) | **replaced by the AST** | `schema/circuit-ir.schema.json` and the Python API build `CircuitIR` today. They need a decision (keep the IR as one renderer target, or rebase them on the AST) that this note does not make. |
| `raw_spice`, `RawSpice`, `IrValue::Raw`, `extra_lines`, `measures: Vec<String>`, `Report.raw` | **disappear** | A statement is an AST node or a parse error with a span and a code. "Kept verbatim for its own dialect" survives only as an explicit `Verbatim{dialect, span}` node, if at all. |
| `raw_dialect` (six jobs, §3.3) | **disappears** | Origin lives in each node's span. Spelling belongs to the renderer and meaning to the node. |
| f64-typed fields (`src/ir/component.rs:111-133`, 29 in `testbench.rs`) | **disappear**: every value slot is an expression node | |
| codegen families `spice3`, `spectre`, `vacask`, `xschem` | **survive as renderers** | One AST walk, not three spines (§2.4). Each renderer is a spelling table plus the shapes that really differ. |
| `check_caret_meaning`, `check_functions`, `refuse_raw_spice`, `refuse_flags`, `check_raw_dialect`, `model_refuse` (`src/codegen/mod.rs:117-307`, `src/codegen/spice3/mod.rs:244`) and ~25 scattered capability refusals | **disappear into one capability table** (dialect × node kind or parameter → spelling, or a refusal code) that the renderer consults | It merges with `ir/features.rs:58` `MISSING`, which ends the disagreement in §2.2 |
| `codegen/statistics` (987 lines) | **survives as AST rewrites** | It operates on call nodes, so `rewrite_calls`, `rename_idents` and `split_args` go |
| `codegen/laplace.rs` | **survives as an AST rewrite** on a `Laplace` call node | Its private lexer goes |
| `lint.rs` text tokenizer | **re-based on the AST** | |
| backend reading its deck back (`src/backend/ngspice.rs:13`) | reads the render's own record of what it wrote | |
| `unit.rs` | **survives**, split between tokenizer (decode) and renderer (spelling) | |

### 7.2 The fixes in flight on `pdk-zero`, and the layer that owns each

These are the fixes in WIP commits `26b27a9` and `d1dc27e`, read with
`git show <sha>:<path>`. Each row names the general rule that would
replace the patch.

| In-flight fix | Today's patch | Owner in the new shape | The general rule |
|---|---|---|---|
| **Cross-file `Known` and `Instance::unresolved`** | The caller builds `Known{subckts, models}` (`d1dc27e:src/import/mod.rs:107-132`). The reader merges it, "this file wins". A master still unknown becomes `unresolved: true` (`d1dc27e:src/ir/circuit.rs:205`, `named_elements.rs:109`). | **resolve** | Parse never decides what `master` is. The parser emits `Instance{name, terminals, master: Ref, params}` for every Spectre `name (nodes) master` and SPICE `X`. Resolve binds `Ref` across the kit's files in a recorded order, and an unbound ref is a coded diagnostic. That one pass also covers the SPICE `X` late name (§5.4) and `R1 a b rmod`. |
| **Spectre names stripped of letters (`f1` → `1`), `v1` written as `vv1`** | On main the import strips the SPICE letter (`src/import/lower/named_elements.rs:357`) and each writer prepends it. The WIP keeps names (`d1dc27e:…/named_elements.rs:441`) and so had to touch every format string: about 20 in `spectre/components.rs`, plus `n.dev(letter, …)` (`d1dc27e:src/codegen/spectre/names.rs:57`), plus a whole-IR re-strip pass for SPICE targets (`d1dc27e:src/codegen/mod.rs:302-390`). | AST (name as written) + **renderer naming policy**, with resolve | The AST holds the name as written and the device kind separately. A SPICE renderer has one rule: if a name does not begin with its kind's letter, the letter is prefixed. That rename goes into a map. Because resolve turned every reference (`probe=`, `ind1=`, `save`, `.ic`, `sweep dev=`) into a link, every reference is renamed with it, and no `vv1` can arise. |
| **Controlled-source gains as f64, expressions lost** | `Vcvs.gain: f64` and the others (`src/ir/component.rs:111-133`). The grammar wants `Num` (`grammar/spice.peg:194-198`). The lowering uses `num("gain")?` (`src/import/lower/named_elements.rs:208`). An expression gain goes raw, and on Spectre import the WIP keeps it as an unresolved primitive and refuses it (`d1dc27e:src/codegen/mod.rs:327-345`). | **AST** | Every value slot is an expression node. A number is a literal node, so `gain={g*2}` is no special case. |
| **Unknown fields silently dropped on cccs** | On main the `cccs`/`ccvs` arm (`src/import/lower/named_elements.rs:221-241`) reads `probe` and `gain` and drops the rest, `m=2` included. The WIP adds `only(&[…])` guards per arm (`d1dc27e:…/named_elements.rs:150`, `:235`, `:261`). | **parser + device table** | A device node keeps all its parameters, known keys typed by the device table and the rest as named parameters. The renderer asks the capability table about each key, and an unspellable key is a coded refusal. Nothing is dropped, by construction rather than by a guard per arm. |
| **bsource `v()`/`i()` probes** | The WIP types a `bsource` (`d1dc27e:…/named_elements.rs:286-316`). The expression stays Spectre text, so every other target refuses it whole (`d1dc27e:src/codegen/mod.rs:350-356`). Main finds `v()` by call name (`9efaefc`). | **parser (expression tree) + renderer** | `v(a,b)`, `i(v1)` and Spectre `v1:p` parse to `Probe{kind, refs}` nodes, with the refs resolved. Each renderer spells probes from its table, and a probe with no spelling in the target is a coded refusal for that node only. |
| **VACASK library-file rules** | A deck with no analyses or stimulus is guessed to be a library (`26b27a9:src/codegen/vacask/mod.rs:109-111`). Then: title as a comment, no `ground`, no control block, device models declared per definition (`:132-150`, `:220-230`), and `temper` handled (`:325-335`). | **AST (file role) + renderer** | The AST root records the file's role (deck or library) and its includes, from the parse and the kit. The VACASK renderer has one rule per role. The role is read from the input, not guessed from "no analyses". |
| **`.params`** | A keyword added to the ngspice `Params` copy (`26b27a9:grammar/ngspice.peg:129`) | **tokenizer (keyword table)** | Keyword aliases are one table per dialect, read by the tokenizer. Adding one is a row, not a copied rule. |
| **`C1 a b cmod 2p`** | The lowering re-reads the flags to decide whether a bare word is a model, then takes the next as the value (`26b27a9:src/import/lower/elements.rs:41-70`). It uses `Cx.models`, which holds this file only. | **parser + resolve** | The parser keeps the positional words as they appear (`Positional[cmod, 2p]`). Resolve decides which word names a model (`Ref`) and which is the value, using the kit's model table. The positional order is the dialect's card shape. |
| **`.model key value` with no `=`** | Pairs are re-merged after parsing (`26b27a9:src/import/lower/values.rs:156-172`, ngspice only) | **parser** (ngspice model-parameter list) | ngspice's model-parameter list accepts `key value` as a pair. That is syntax, so it is a rule of the ngspice parser and never a fix-up of flags. |
| **`off` flags** | `refuse_flags` refuses every valueless parameter for Spectre and VACASK (`src/codegen/mod.rs:281-307`). The WIP exempts `off` on semiconductors (`26b27a9:src/codegen/mod.rs:296-310`). | **device table + capability table** | `off` is a typed device parameter (an initial-state hint), not a valueless string. The table says Spectre spells it `region=off` and VACASK omits it, as a rendering note. Other flags get their own rows instead of one blanket refusal. |
| **Binned model base names** | A Spectre model group is split into `nch.1`, `nch.2`… (`src/import/lower/named.rs:160-180`). The Spectre writer re-groups by the `.digits` naming convention (`src/codegen/spectre/library.rs:21-60`, `:162`), and `Known` infers the base the same way (`d1dc27e:src/import/mod.rs:125-128`). | **AST + resolve** | The AST has a `ModelGroup{base, bins[{selector params, params}]}` node. SPICE `.model nch.1` cards parse into it. An instance's `master` resolves to the group, and the renderer writes a group or numbered cards. Nothing depends on a naming convention. |

Across the eleven fixes, the owners are: resolve 5 times, the AST or
expression tree 5, device and capability tables 4, the parser 4, the
tokenizer 1 and renderer policy 3. Some fixes have more than one owner.
None of the eleven belongs to a lowering stage, which supports dropping
it. The new architecture still needs the resolve pass, though, and §5
and these rows show that most of the current patches exist because that
pass is missing.

---

## Appendix: how the counts were taken

- Code lines per file: `wc -l` up to the first `#[cfg(test)]`.
- Dialect branch sites: `grep -rnE 'match (self\.)?d\b|matches!\((self\.)?d,|(self\.)?d [!=]= Dialect|== Dialect::|!= Dialect::' src/import src/codegen src/ir`, minus test modules (99 lines).
- Grammar rules per concern: `grep -cE '^(x|s|v)_[a-z0-9]+ +<-'` (expressions),
  number-rule names, and `l_*`/`Line`/`Piece`/`Glue`/`Title`/`file` (lines),
  per `.peg`. Overrides of `spice.peg` come from comparing rule-name sets with `comm -12`.
- Refusal sites: `grep -rn 'unsupported_component(\|unsupported_analysis(\|unsupported_comp(\|CodeGenError::Other\|UnsupportedComponent {\|UnsupportedAnalysis {' src`.
- Files per fix: `git show --name-only --format= <sha>`.
