# SpiceRack as a transpiler: tokenize → parse → AST → render

Status: proposal, 2026-11-03. Nothing here is built yet.

Inputs:
- [`architecture-today.md`](architecture-today.md): the audit of `main` at `9efaefc` plus the
  WIP `d1dc27e`.
- [`compiler-architecture-reference.md`](compiler-architecture-reference.md): Zig, ESPice,
  VerA, LLVM, tree-sitter and Pandoc, all read from source.

Section numbers like "today §5.8" and "ref §7.3" point into those two notes. Their citations
are not repeated here.

## 0. The proposal in one paragraph

SpiceRack becomes the front half of Zig plus `zig fmt`'s renderer, with one change: the
renderer can write a dialect other than the one it read. There is no Sema, no AIR and no
lowering. The pipeline:

1. a **tokenizer** per dialect, which is one engine driven by a per-dialect lexicon table;
2. **one parser**;
3. **one AST**, holding netlist, analyses and expressions as trees, with the original spelling
   recoverable from token spans;
4. a thin, read-only **resolve** pass that builds a kit-wide symbol table;
5. **one renderer per statement shape** (SPICE-family and Spectre-family), driven by
   per-dialect *spelling* and *capability* tables.

Every refusal and every drop is a coded diagnostic.

Today's 99 dialect branches, 6 duplicated grammars, 8 hand-written expression scanners and
125 free-text refusals all collapse into these tables and layers. A bug like "Spectre `2*1M`
reaches SPICE as milli" stops being possible, not just fixed.

## 1. Why the current code produces hyper-specific fixes

Every fix in the last month built a small, construct-specific slice of a layer that doesn't
exist (today §5.9):

| Missing layer | What it cost | Fixes that were slices of it |
|---|---|---|
| One token/number layer | 6 number grammars (97 rules), 4 more number parsers in Rust | sky130 value forms; HSPICE spelling; **`2*1M` → milli (verified 2026-11-03 on `9efaefc`: `parameters b=2*1M` is written `.param b=2*1M`)** |
| Expressions as trees | expressions kept as source strings, rewritten by about 8 byte scanners | Monte Carlo lifting (36 files, +3,271 lines); `^` meaning; if-conditions; bsource `v()`/`i()` probes; controlled-source gains stored as `f64` |
| A kit-wide symbol table | `parse` reads one file; `Known` bolted on afterwards | the X-card late subckt name; a proprietary kit's 293 raw instances; `Instance::unresolved`; binned base names |
| Card shapes written once | 6 line stages, 4 copies of `dotcard`, 3 each of `If` and `LibDef` (PEG overlays can replace a rule but not extend it) | library sections (38 files); Spectre if/else (25 files); `.params`; `C1 a b cmod 2p` |
| Spelling and capability as tables | 99 explicit `Dialect` branches, plus about 12 via flags; two capability checks that disagree | charge capacitors (5 spellings in 15 files); `off`; VACASK library rules; LTspice XSPICE |
| A scope model | scope handled ad hoc in the lowering | `.param` inside subckts; MC process-vs-mismatch placement |
| Diagnostics with codes | 125 free-text `CodeGenError` sites; `Report.unparsed` without line numbers | every refusal test matches a substring; a restructure can change refusals silently |

The fixes were correct, but each one had to be written again for the next construct.

## 2. The pipeline

```
bytes ──decode──▶ text
text ──tokenize(Lexicon[d])──▶ Tokens            {tag:u8, start:u32} SoA; trivia stays in the gaps
Tokens ──parse──▶ Ast                            nodes SoA + extra_data + errors; never fails
[Ast per file] ──resolve(read order)──▶ Kit      symbol table + per-node facts (side tables, read-only)
(Kit, Ast) ──render(Spelling[t], Capability[t])──▶ text + [Diag]
```

| Stage | Reads | Returns | Per dialect | Diagnostic class |
|---|---|---|---|---|
| decode | `&[u8]` | `String` | nothing (UTF-16 BOM, UTF-8, Latin-1, as today) | none |
| tokenize | `&str`, `&Lexicon` | `Tokens` | the lexicon table plus at most two hooks | `L` |
| parse | `&Tokens`, `&Lexicon` (keywords, head forms) | `Ast` (`(T, errors)`, never `Err`) | nothing but table lookups | `P` |
| resolve | `&[Ast]` in read order | `Kit` (side tables keyed by node) | nothing | `R` |
| render | `&Kit`, `&Ast`, target | `String` + `Vec<Diag>` | the `Spelling` + `Capability` tables; one of two structural writers | `W` |

Each stage takes the previous stage's output by `&` and can't reach its internals (ref §1,
Zig's narrow stage boundaries). A stage that seems to need something from two stages back has
found a missing field in the stage between.

### 2.1 Tokenizer: one engine, a lexicon per dialect

ESPice's lesson (ref §3.2, §7.1) is that lexical differences are *facts*. A `Lexicon` is a
`const` table per dialect:

- **comments:** comment starters, with column rules (`*` in column 1, `;`, `//`, `$` after a
  separator);
- **lines:** continuation forms (`+` leading, `\` trailing) and the title-line rule;
- **case:** case folding;
- **expressions:** expression delimiters (`'…'`, `{…}`, none), whether `"` is a string, the
  operator set (`^` versus `**`, ternary);
- **numbers:** the number-suffix table (`M` = milli or mega, `meg`, `mil`, unit tails such as
  `1kOhm`);
- **keywords:** directive and statement keywords, as `StaticStringMap`-style lookups after an
  identifier is lexed;
- **letters:** the SPICE element-letter table (`R` → resistor, `X` → instance, …); empty for
  Spectre and VACASK.

Hooks exist only for context the table can't express:

- HSPICE's `$` comment rule (ESPice `lines_hspice.zig:88-106`);
- Spectre's `simulator lang=spice|spectre` switch, which swaps the active lexicon mid-file.
  The second one is how a `lang=spice` region becomes ordinary SPICE tokens rather than raw
  text.

Token rows are `{tag: u8, start: u32}`, and the end is re-lexed on demand (Zig `Ast.zig:28-31`).
**A number is a `Number` token whose value is decoded once, here, by the dialect's suffix
table, into a canonical `f64` plus its token.** No later stage parses a number again. This
single choke point is what removes the `M`-versus-`meg` class of bugs.

The tokenizer never fails: bad bytes become an `Invalid` token, and the parser reports it
(Zig `tokenizer.zig:389`). Comments and blank lines are not tokens. The renderer recovers them
from the gaps between tokens, as `zig fmt` does (`Render.zig:3074-3099`), so they survive a
round trip without the AST knowing about them.

### 2.2 Parser: one, dispatching on the statement head

SPICE cards are letter-led and positional (`R1 a b 1k`). Spectre and VACASK statements are
`name (nodes) master params`. The parser dispatches on the head of each logical statement.
Both heads build **the same node tags**, and everything after the head is shared code:

- parameter lists;
- expressions;
- `subckt`/`section`/`library`/`if` blocks;
- model bodies.

The parser recovers from an error at the next statement start, the analogue of Zig's
`findNextContainerMember` (ref §7.2). In SPICE that's simply the next logical line.

**Expressions are trees.** One Pratt parser builds them, with per-dialect operator and
delimiter facts from the lexicon. Function calls are nodes holding an interned name, so
`agauss`, `v(a,b)`, `limit` and user `.func`s are found by walking the tree. Every
expression consumer works on this tree: Monte Carlo, `^`, probes, conditions and gains.

### 2.3 The AST

The layout follows Zig's `Ast` (ref §1.2):
- nodes are SoA rows `{tag: u8, main_token: u32, lhs: u32, rhs: u32}`;
- payloads too wide for two words go in `extra_data: Vec<u32>`;
- names are interned once (`u32` handles, insertion order, so output is deterministic);
- a number's canonical value sits in a side table, and its original spelling stays in its
  token.

Each tag's doc comment says how to read `lhs`, `rhs` and its extra words, and a test pins the
row width.

Node tags are dialect-neutral netlist constructs, about 40 of them:

| Group | Tags |
|---|---|
| Structure | `Root`, `Title`, `Include{file}`, `LibCall{file, section}`, `Library{name, sections}`, `Section{name, body}`, `Subckt{name, ports, params, body}`, `If{cond, then, else}` (chains are `else If`), `Verbatim{dialect, span}` |
| Definitions | `Param{name, expr}`, `ParamList`, `Func{name, args, body}`, `Model{name, kind, params}` (bins are `Model` rows named `nch.1`, …), `Statistics{process, mismatch, correlate, truncate}`, `Vary{param, dist, spread, percent}` |
| Elements | `Instance{name, nodes, master, args}`, where `master` is `Primitive(p)` or `Name(id)` and `args` are positional values plus `key=expr` pairs |
| Analyses and output | `Analysis{kind, args}` (op, dc, ac, tran, noise, pz, sens, montecarlo, …, as one tag with a kind enum), `Options`, `Save`, `Print`, `Measure`, `Ic`, `Nodeset`, `Temp`, `Global` |
| Expressions | `Num`, `Ident`, `Str`, `Unary`, `Binary`, `Ternary`, `Call{name, args}`, `Probe{kind, nodes}`, `Index` |

Three decisions in that table carry most of the weight:

- **`Instance` covers every element.** SPICE `R1 a b 1k` becomes `Instance{master:
  Primitive(Resistor), args:[1k]}`, via the lexicon's letter table. Spectre `r1 (a b)
  resistor r=1k` becomes the same node via the keyword table. The renderer decides
  whether a target wants the letter, the master word, or both. Today's `Component` versus
  `Instance` split is a guess about this that the parser can't make (today §5.8).
- **`Verbatim{dialect, span}` replaces `raw_spice` and the deck-wide `raw_dialect`.**
  Anything the parser can't read is kept as a source span tagged with its own dialect,
  together with a `P` diagnostic. A same-dialect renderer copies it byte for byte; any
  other renderer refuses it with a `W` code (Pandoc's `RawBlock` rule, ref §7.3). It can
  never pass through silently.
- **The ngspice `.control` block stays `Verbatim`, as an explicit ngspice construct.** It
  is a script language, not a netlist, so it isn't parsed into netlist nodes. Its
  `pre_osdi` lines are the one exception: they're read as `OsdiLoad`, because they change
  the netlist.

### 2.4 Resolve: a kit-wide symbol table, read-only

The user's target is "no lowering", and resolve is not a lowering. It rewrites nothing; it
only answers what a name refers to. It's needed because cross-dialect rendering isn't purely
syntactic, which `zig fmt` never runs into because it writes the language it read:

- **The element letter.** Spectre `x1 (a b) leaf` becomes `Xx1` in SPICE if `leaf` is a
  subckt, and `Mx1` if it's a MOS model. `leaf` may be defined in another file. That's
  a proprietary kit's 293 raw lines.
- **Monte Carlo scope.** `agauss(…)` goes to Spectre `process` when evaluated at the top
  level, and to `mismatch` inside a subckt.
- **Name collisions.** Spectre instance names lose or gain letters, and must not collide.
- **Binned models.** The base name `nch` stands for `nch.1`, `nch.2`, ….

`resolve(&[Ast]) -> Kit` takes the files in their read order (a kit's include graph,
followed by the tokenizer stage's include list). It returns side tables keyed by
`(file, node)`:

- what each name binds to (subckt, model and its kind, param, func, binned base);
- each node's scope (top, subckt, section, if-branch);
- for each `LibCall`, the section it selects.

That's rust-analyzer's split: a syntax tree that knows nothing about meaning, plus a separate
semantic layer. A master that is still unknown keeps its name and gets an `R` diagnostic. The
renderer then picks the dialect's default spelling (X for SPICE), and the diagnostic is
there for anyone who cares.

### 2.5 Render: two structural writers, driven by tables

Render follows `zig fmt` (`Render.zig`, ref §1.5, §7.5):
- one `write_token(tok, space)` choke point;
- indentation owned by the output stream;
- refusal to render a tree with errors, unless the caller asks for `Verbatim` passthrough.

Two structural writers, because there are only two statement shapes:

- **SPICE family:** ngspice, ESPice, HSPICE, Xyce, LTspice, CDL. These differ only in table
  rows.
- **Named family:** Spectre and VACASK.

Each target has two tables:

- **`Spelling`** covers:
  - expression quote, comment leader, continuation;
  - `.lib`/include form, case, parameter-function form, ground name;
  - the number format (suffix table, used in reverse);
  - keyword spellings;
  - primitive ↔ letter or master word.
- **`Capability`**: `const fn action(target, Construct) -> Action`, one exhaustive `match`
  with no `_` arm. A new construct then fails to compile until every dialect answers (ref
  §7.4: LLVM's legalize table and VerA's total tables).
  - `Action::Native`: write it as is.
  - `Action::Spell(alt)`: a named alternative form, e.g. `Q='e'` vs `bsource q=`, `off`
    vs `region=off`, `limit` → `aunif`.
  - `Action::Lift(rule)`: a rewrite that emits extra statements, e.g. agauss → `rack_agauss`
    plus a `statistics` block. A lift rule runs before the writer and only produces
    **fixups**, a side table of edits keyed by node, the way Zig's `zig reduce` and
    `zig fetch --save` edit source (ref §1.5). The one renderer applies them; the AST is never
    mutated.

  `Spell` and `Lift` are allowed only when the result means the same thing in the target.
  Pandoc approximates freely and sometimes loses content without a word (ref §5.3, Q2), and
  SpiceRack must not: anything that can't keep its meaning is `Refuse(Code)`.
  - `Action::Refuse(Code)`: refuse with a code citing the manual clause.

There are three more tables and rules (from today §7):

- **A device table per dialect:** primitive ↔ master word or letter, each device's parameter
  keys and their types, and model kinds per level. The lexicon reads it on input, and
  `Spelling` reads it on output. One test checks that the two directions invert, which
  retires today's per-direction maps. Flags such as `off` are typed device parameters with a
  spelling per dialect.
- **A file role** (deck or library), recorded by resolve. A library file gets no title,
  ground, `.end` or control block in any target. That one rule replaces the VACASK
  library-file patches and HSPICE's `.end`.
- **Names are renamed as a whole.** When a target needs a different spelling of a name (a
  SPICE type letter, escaping), the renderer renames the declaration and every reference to
  it together, through one name map built from `Kit`. That ends the `v1` → `vv1` and
  `save main:d` class of bug.

**Monte Carlo is a `Lift` rule, not an IR pass.** The rule walks the expression tree, asks
`Kit` for the call's scope, and records fixups. Each call is written through the helper
function, and each `vary` row goes into a statistics block emitted before the body. The 987-line `statistics/`
pre-pass and its byte scanners become one rule of a few hundred lines over trees.

**Same-dialect round trip.** When the target dialect is the source dialect and a node is
unchanged, the renderer copies its original token span. Comments come back from the gaps.
`write(read(x, d), d) == x` then holds for every clean deck: a new guarantee, which today's
pipeline can't give (ref §7.7).

### 2.6 Diagnostics

A `Code` enum is the single catalogue (VerA `lib/diag/code.zig`, ref §4.1, §7.6):
- each code has a row `{title, cite, explain}`, where `cite` is the dialect-manual section;
- the first letter is the stage: `L`, `P`, `R`, `W`;
- codes are never renumbered or reused;
- diagnostics anchor to a token or node, and line and column are computed late;
- each stage appends and carries on;
- rendering into messages happens once, sorted by source position.

**Every refusal, drop, `Verbatim` passthrough and fallback emits a code**, so nothing is lost
silently (Pandoc's `BlockNotRendered`). Tests pin codes, never message text.

## 3. Today's bugs, owned by a layer

Each row becomes a general rule in one layer, plus a fixture for it.

| Bug or fix today | Layer that owns it in the new shape |
|---|---|
| `2*1M` written for SPICE, which reads it as milli | **tokenize**: the number is decoded once by Spectre's suffix table; **render**: written with the target's suffix table |
| sky130 `{a} + tail`, `{"…"}`, a lone `+` between params | **tokenize** (delimiters) + **parse** (expression grammar) |
| `.params`, `C1 a b cmod 2p`, `.model k v` without `=` | **lexicon** keyword alias; **parse**, as one positional-args rule |
| X card with the subckt name after the params | **parse**: the positional head rule, once |
| library sections, Spectre `library`/`section`, VACASK `section` | **parse**: one `Section` rule; **lexicon**: the keywords |
| `.if` / Spectre `if`/`else` / VACASK `@if` | **parse**: one `If` rule; **render**: `Spelling` |
| `.param` inside subckts | **parse** (`Param` in any body) + **resolve** (scope) |
| a Spectre `simulator lang=spice` region kept raw | **tokenize**: the lexicon-switch hook |
| an instance whose master is in another file (a proprietary kit's 293 lines) | **resolve** |
| `f1` → `1`, `v1` → `vv1`, `x{name}` | **render**: names go out as written, and only `Spelling` adds a letter, at one choke point |
| controlled-source gain as `f64`; cccs fields silently dropped | **AST**: args are expressions; **parse** keeps every `key=value` |
| bsource `v()`/`i()` probes | **AST**: `Probe` nodes; **render**: `Spelling` per target |
| charge capacitors (5 spellings) | **render**: `Capability::Spell` |
| the `off` flag | **render**: `Capability::Spell` / `Refuse` |
| Monte Carlo into Spectre and VACASK | **render**: a `Lift` rule over expression trees + **resolve** (scope) |
| Spectre `statistics` blocks read back | **parse**: `Statistics` nodes |
| VACASK library-file rules, `load` placement | **render**: a structural-writer rule |
| level-3 diode, r/c `.model` kinds with no Spectre form | **render**: a `Capability` cell per model kind |
| the 2 capability checks that disagree (LTspice XSPICE) | **render**: one table |
| two-direction maps that may disagree (Spectre/VACASK masters, Xyce 8 → 49) | one table, read in both directions by the lexicon and `Spelling`, plus a test that they invert |
| a Spectre `probe=` reference not renamed when its source gets a SPICE letter (found by reading, not run; ref §7) | **render**: names are renamed as a whole, through the one name map |
| the `.dc` writer guessing device versus parameter from a name's first letter (found by reading, not run) | **resolve**: the name's kind comes from `Kit` |

## 4. What goes away, and what stays

Goes away:
- `grammar/*.peg` and `src/peg.rs`: 1,246 + 601 lines;
- `src/import/lower/`: 2,850 lines;
- the `CircuitIR` expression strings and `raw_spice` / `raw_dialect`;
- `codegen/statistics` as an IR → IR pre-pass;
- the 8 byte scanners;
- `lint`'s private tokenizer, since lint walks the AST.

The 99 dialect branches become table rows. Rough size of the new core:
- tokenizer and lexicons, about 1.5k lines;
- parser, about 2.5k;
- AST and expressions, about 1k;
- resolve, about 0.5k;
- two writers plus tables, about 3.5k.

That's roughly 9k lines against about 14k today for the same job. This is an estimate, not a
measurement.

Stays, on top of the core:
- the simulator backends (`src/backend`);
- `rawfile`, `measure_parse`, `simulation`, `result`;
- the Python API.

These are consumers of the transpiler, not part of it.

## 5. Migration, gated byte-identical

The rule throughout is ref §7.7 and the compiler-construction skill's rule 9: freeze the
current pipeline as the oracle, and make each phase match it byte for byte. Every difference
is a listed, reviewed line in `migration.xfail`, and xfail is strict: an unexpected pass also
fails. Behaviour changes, such as the `2*1M` fix, land as their own commits after the phase
that enables them, never inside it.

| Phase | Lands | Gate |
|---|---|---|
| 0. Goldens | Record `write(read(deck, d_in), d_out)` and the refusal, for every corpus deck, PDK file (sky130, gf180, IHP, VACASK's own) and fixture, across every `(d_in, d_out)` pair. Key it by content hash. Add Spectre inputs (PDK `.scs`, `tests/fixtures/**/*.scs`, the made-up kits from `pdk-zero`). Record kit read orders. Measure the simulator-free tier's time. | The manifest regenerates identically twice (determinism) |
| 1. Tokenizer + lexicons | `src/syntax/token.rs`, `lexicon/*.rs` | Its logical statements equal today's `logical()` output on the whole manifest |
| 2. Parser → AST | `src/syntax/{parse,ast,expr}.rs`, plus a temporary `ast_to_ir` adapter that builds today's `CircuitIR` | `codegen(ast_to_ir(parse(x)))` is byte-identical to today on the manifest, so the front end is swapped while the old writers still run. Then delete PEG + lower. |
| 3. Resolve | `src/syntax/resolve.rs`; the adapter reads `Kit` | byte-identical, except the listed xfails: decks that were refusing or raw before, now resolving |
| 4. Renderers | the SPICE-family writer, then the named writer, each with `Spelling` + `Capability`; one target at a time, starting with ngspice | byte-identical per target against the old codegen; then delete that target's old module |
| 5. Diagnostics catalogue | `Code` rows for every `L`/`P`/`R`/`W` path; tests move from substrings to codes | the refusal set is unchanged (same decks, same constructs) |
| 6. Remove `CircuitIR` | The Python API builds an AST through a builder (decision D1); delete the adapter and every comment citing the old path, in one commit | the Python tests and the corpus gate pass |
| 7. New guarantees | the same-dialect identity `write(read(x,d),d) == x`; the `2*1M` fix; the proprietary-kit fixes | new tests, which are expected to fail before this phase |

Interop calls `import::{parse, decode}`, `codegen::netlist` and `veriloga::is_source`, and it
reads `CircuitIR` fields (subckt ports and params, sections, models, includes, `osdi_loads`,
`raw_spice`) in `pdk_scan.rs`. PDK-Manager stores `CircuitIR` JSON (`rack`). Phases 1–5 keep
that surface. Phase 6 moves Interop to the AST and `Kit`, and changes the store format (see
D3).

## 6. Decisions for the user

- **D1. The Python API.** Today Python builds a `CircuitIR` directly.
  - *Recommended:* keep the Python surface and have it build the AST through a typed builder,
    the way the parser does.
  - *Alternative:* keep `CircuitIR` as a thin builder-only facade that renders through the
    AST.
- **D2. Rewrite in place versus a new crate.**
  - *Recommended:* a new module `src/syntax/` inside SpiceRack, swapped in phase by phase
    behind the gate. A fresh crate written beside the old one loses the oracle and the
    corpus harness.
- **D3. The store format** (PDK-Manager's `rack`), which today is `CircuitIR` JSON.
  - *Recommended:* store the source files with their dialect plus their read order, and
    re-parse on demand. The AST is cheap to rebuild, and the source is the only lossless form.
  - *Alternative:* serialise the AST.
- **D4. The paused `pdk-zero` work** (`d1dc27e`).
  - *Recommended:* keep its fixtures and the agent's list of bugs as phase-7 tests. Don't
    merge its code; it's more slices of the missing layers.
- **D5. Language.**
  - *Recommended:* stay in Rust. The Zig patterns translate directly: SoA rows in `Vec`s,
    `u32` handles, exhaustive `match` in place of `EnumArray`. Interop and PDK-Manager link
    SpiceRack as a crate, and pyo3 builds the Python bindings.

## 7. Risks

- **Phase 2's adapter may not reach byte-identical on every deck**, because today's lowering
  has quirks. Each quirk is either reproduced deliberately in the adapter, with a
  `// quirk:` comment, or listed in xfail with its reason. That list is the honest inventory
  of what the old reader did.
- **The `.control` block and LTspice's oddities stay `Verbatim`** until someone decides they
  need parsing. That boundary is stated, not hidden.
- **Spectre ground truth is thin.** There is no public Spectre corpus. A licensed Spectre 21.1 host
  can check made-up decks, as T-S1…T-S3 did, but proprietary kits can only be checked
  by structural counts, never by content.
