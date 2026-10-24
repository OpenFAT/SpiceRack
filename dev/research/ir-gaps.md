# IR gaps: every raw card in the suite, by construct

Survey date: 2026-10-24. SpiceRack at `ed184dc` (branch `dialect-import`). This follows
`dev/research/dialect-import.md` §4, whose counts predate `5e6463d` (BJT terminals), `cba3e03` (option flags) and
`b181dd3` (nested subckt, `.if`, scoped `.lib`). Local paths are relative to the SpiceRack root unless absolute.
`$S` = the session scratchpad `/tmp/claude-1000/-home-omare-Documents-Projects-Rust-SpiceRack/ff86074c-…/scratchpad`.

## Sources

| tag | document | page numbers |
|---|---|---|
| NG | Ngspice User's Manual, version 47+ (dev), 2026-09-28 (`$S/ng.pdf`; https://ngspice.sourceforge.io/docs.html) | PDF page = printed page (checked on pp.58, 66, 80, 89) |
| NGSRC | ngspice-45 source, `/tmp/ngspice-45-research/src/...` | file:line |
| HCR | HSPICE Reference Manual: Commands and Control Options, E-2010.12 (`$S/hcmd.pdf`) | printed page, from its TOC |
| HSA | HSPICE Simulation and Analysis User Guide, B-2008.09 (`$S/hsa.pdf`) | **PDF** page ("pdf p.") |
| LT | LTspice XVII help mirror, `https://ltwiki.org/LTspiceHelpXVII/LTspiceHelp/html/<page>.htm` | page name |
| R5 | Spectre Circuit Simulator Reference 5.0, Sept 2003, http://eece.cu.edu.eg/~fhussien/Spectre_tutorial.pdf | printed page (= PDF page − 2) |
| UG | Virtuoso Spectre User Guide 5.1.41 (`$S/sug.pdf`; https://web.eecs.utk.edu/~dbouldin/protected/spectreuser.pdf) | PDF page |
| V | VACASK `docs/` at `9ca4982` (2026-10-22), clone at `$S/VACASK` | file:line |
| XRG | Xyce Reference Guide 7.9 (`$S/xyce_rg_7.9.pdf`; https://xyce.sandia.gov/files/xyce/Xyce_Reference_Guide_7.9.pdf) | PDF page |

"Not found" means a case-insensitive search of that text extraction found nothing. It is not proof that the
feature is absent, so those entries are marked (unverified).

## 1. Census

### Method

1. Build and run `target/release/spice2rack -o <dir> --python <deck>` over every runnable deck: `*.sp` outside
   `src*` directories, 6 065 decks in total. Each ngspice deck gets its own output directory, because 73 ngspice
   basenames collide.
2. `$S/irgaps/dump.py` reads each `<stem>.json` (the `CircuitIR`). It collects `raw_spice` from `top` and every
   `subcircuit_defs` entry, `Component::RawSpice`, and `testbench.extra_lines`. The last two were never non-empty.
3. `$S/irgaps/cls.py` classifies each line. Lines inside an `.if`…`.endif` or `.lib name`…`.endl` block are
   counted under that block (the importer stores a block's lines one by one, `src/import.rs:670-690`).

Result: **929 raw cards in 244 decks.**

| suite | cards | decks |
|---|---|---|
| ngspice | 908 | 234 |
| cmcqa | 12 | 6 |
| circuitsim90 | 7 | 3 |
| postlayout | 2 | 1 |
| powergrid | 0 | 0 |

The `# gap: card with no IR form` markers in the generated `.py` files add up to 917. The other 12 cards are in
cmcqa's `.tmp*.sp` decks, whose `.py` files are dotfiles.

Caveat: includes are kept as references (`src/import.rs:692-700`), so constructs inside included files (PDK
`.lib` sections, local `.param`) are not in this census. The earlier research counts those
(`dialect-import.md` §4b).

### Constructs

"Sole" counts the decks where this construct is the only raw construct, so fixing it alone leaves the deck with no
raw cards. Related cards are grouped. For example, VDMOS covers 3-terminal `M` cards (48), `.model … VDMOS pchan`
(31) and `M … tj tc … thermal` (12).

| construct | cards | decks | sole | why it is raw today |
|---|---|---|---|---|
| waveform argument is an expression (`pulse(0 'vdrive' …)`) | 207 | **1** (`ngspice/iscas85/c7552/c7552_ann.sp`) | 1 | `args()` takes numbers only (`src/import.rs:265-268`), and `IrWaveform` fields are `f64` (`src/ir/mod.rs:157-164`) |
| LTRA: `O` element + `.model … ltra` with bare flags (`steplimit`, `nocontrol`, `quadinterp`) | 115 | 18 | 16 | no `O` arm in `element()` (`src/import.rs:457`); `pairs()` rejects a bare word in a model (`src/import.rs:594`) |
| `.print` / `.plot` | 105 | **90** | **58** | no IR field (kept raw since `1195444`, `src/import.rs:574-578`) |
| VDMOS: 3-terminal `M`, `pchan`/`nchan` model flag, `tj tc … thermal` | 91 | 35 | 29 | `Mosfet` has exactly 4 nodes (`src/ir/mod.rs:117`; `src/import.rs:439`); model flag as above |
| `TRNOISE(…)` / `TRRANDOM(…)` sources | 58 | 14 | 11 | no `IrWaveform` variant (`src/import.rs:274-281`) |
| `.if/.elseif/.else/.endif` blocks | 53 | 3 | 2 | kept as a raw block (`src/import.rs:604,685-690`) |
| E/F/G/H `POLY(n)` | 50 | 5 | 3 | no IR form (`src/import.rs:400,409`) |
| waveform without parentheses (`pulse 0 5 1n …`, `pwl 0 'vp' …`), `Sine(…)` | 32 | 26 | 8 | `waveform()` needs a `(` group (`src/import.rs:361`); `sine` is not a recognised kind (`src/import.rs:274-281`) |
| TXL `Y` element | 31 | 9 | 5 | no `Y` arm |
| ngspice `.probe` | 22 | 9 | 6 | kept raw on purpose (`755df9c`, `src/import.rs:576-578`) |
| CPL: `P` element + `.model … cpl` with matrix values | 20 | 9 | 5 | no `P` arm; `R=0.03 0 0 …` is not a `k=v` list |
| `.sp` analysis + `portnum`/`z0` on V | 20 | 7 | 6 | the importer never builds `Analysis::SPar` (`src/import.rs:510-540`); V has no port fields |
| `M` with 5 to 7 nodes (B3SOI/B4SOI) | 15 | 5 | 0 | 4-node `Mosfet` |
| `.func` | 14 | 4 | 3 | no IR field |
| R/C/L naming a `.model` (`r1 1 0 rmod`, `r1 1 2 1k rmodel l=1u`) | 14 | 9 | 6 | deliberate (`src/import.rs:287-300`) |
| instance flag `off`/`on`/`thermal` on M/S/W/D | 13 | 7 | 5 | `split_params` needs `k=v` after positionals (`src/import.rs:259-262`); S/W need an exact token count (`src/import.rs:445-446`) |
| V/I other: `{dc=-6}` (6 cmcqa decks), `DC 0 Sine(…)` | 13 | 7 | 6 | malformed card; `sine` alias |
| `.global` | 11 | 9 | 5 | no IR field |
| K with an expression coupling (`{L12/sqrt(…)}`) | 10 | 3 | 3 | `coupling: f64` (`src/ir/mod.rs:104`; `src/import.rs:317-322`) |
| `.pz` | 8 | 8 | 1 | IR has `PoleZero` (`src/ir/mod.rs:207`); the importer does not build it |
| `.pss` | 6 | 6 | 1 | IR `Pss` (`src/ir/mod.rs:209`) lacks `sciter`, `steadycoeff`, `uic` |
| `.sens` | 6 | 6 | 5 | IR has `Sensitivity` (`src/ir/mod.rs:206`); the importer does not build it |
| `.tran 0.05ms 'ttime'` / `.temp 0 25 50…` or `{envtemp}` / `.ic v(x)={envtemp}` | 3/3/1 | 3/3/1 | 0/0/0 | `f64` fields (`src/ir/mod.rs:177-179,203`) |
| `.width` | 3 | 3 | 0 | printer width for `.print` (NG §11.6.7 p.360) |
| `.ac LIN 5k …`, `.csparam`, `DISTOF1`, `U` (URC), `.model` with a stray `.00` | 1 each | 1 each | — | importer parse / malformed |

The task listed some constructs that do not occur in deck text at all: `.param` inside `.subckt`, `.option` with an
expression value, `.step`, `.control` leftovers, `.lib name`/`.endl`, XSPICE `A`, and `W` other than one flag card.
All count 0 raw cards. They live in included files, or the importer already absorbs them (`.control`:
`src/import.rs:659-684`).

**Greedy "decks with zero raw cards"** (the marginal gain of each fix, in this order): `.print/.plot` +58, VDMOS +29,
LTRA +18, paren-less waveforms +15, TRNOISE +11, `.global` +9, R/C/L model +9, `.pz` +8, V/I other +7, CPL +7,
TXL +9, `.probe` +7, flags +7, `.sp` +6, `.pss` +6, `.sens` +6, SOI +5, `.func` +4, POLY +3, `.tran` expr +3, …,
waveform expressions +1. This counts decks that become raw-free in the IR. It does not count decks that every
dialect can emit.

## 2. Dialect spellings and faithful translations (constructs in ≥ ~5 decks)

✔ = a native form with the same semantics. ✘ = no form. ~ = a form exists but the semantics differ or are
unverified.

| construct | ngspice | HSPICE | LTspice | Spectre | VACASK | Xyce |
|---|---|---|---|---|---|---|
| `.print`/`.plot` | `.print prtype ov…` NG §11.6.2 p.354, `.plot` p.355. **Ignored when a rawfile is written (`-r`)**, NG §11.6 p.353 | ✔ `.PRINT`, HCR p.252 | ✘ not in the dot-command list (LT `DotCommands`) (unverified) | ✘ no tabular print found; `print` appears only as a netlist keyword (R5 p.875 keyword list) (unverified) | ✘ `print` evaluates expressions to stdout, not waveforms (V `cmd-print.md:3-8`) | ✔ `.PRINT <analysis> …`, which also selects the outputs (XRG p.128) |
| VDMOS | `M nd ng ns mname`, `.model m VDMOS <Pchan>` NG §7.7 p.173; `M nd ng ns tj tc mname thermal` p.178; 3 to 5 nodes NGSRC `parser/inp2m.c:32-35,98-101` | ✘ not found (unverified) | ✔ `Mxxx Nd Ng Ns <model>`, `pchan` flag (LT `M-device`); ✘ no thermal nodes there | ✘ R5 names VDMOS only as a use of Philips MOS models (R5 p.520) | ✔ `spice/vdmos.osdi` / `sp_vdmos` (V `dev-spice.md:20`); already mapped in `src/codegen/vacask.rs:176` | ~ MOSFET level 18 is an RPI VDMOS, a different model (XRG §2.3.20.8 p.380) |
| LTRA `O` | `OXXX n1 n2 n3 n4 mname`, `.model … ltra r l g c len` + flags, NG §6.2 pp.126-127 | ~ W-element (RLGC), HSA pdf p.194 (different algorithm) | ✔ `Oxxx L+ L- R+ R- <model>`, same flags (LT `O-device`) | ~ `tline` lossy `r=`/`g=` per length (R5 pp.645-648) or 1-conductor `mtline` (R5 pp.570-575) | ✘ no transmission line (V `dev-builtin.md:10-18`; `src/codegen/vacask.rs:909-911`) | ✔ LTRA, XRG §2.3.21 p.763 |
| TRNOISE/TRRANDOM | `TRNOISE(NA NT NALPHA NAMP RTSAM RTSCAPT RTSEMT)` NG p.100, `TRRANDOM(TYPE TS …)` p.101; NGSRC `vsrc/vsrc.c:24-25` | ✘ only the `.TRANNOISE` analysis, HCR p.320 | ✘ not in LT `V-device` | ✘ `noisefile`/`noisevec` are small-signal spot noise only (R5 pp.685-686) | ✘ trannoise injects device noise, not a source (V `cmd-analysis-trannoise.md:3,13`) | ✘ not found (unverified) |
| paren-less waveform, `sine` | "punctuation (parentheses …) is optional", NG ch.3 p.73; `sine` alias NGSRC `vsrc.c:19` | — | — | — | — | — |
| TXL `Y` / CPL `P` | `YXXX N1 0 N2 0 mname <LEN=>`, `PXXX NI1…GND1 NO1…GND2 mname`, NG §6.4 p.130 | ~ W-element (HSA pdf p.194) | ✘ (only O and U lines in LT) (unverified) | ~ `mtline` with RLGC matrices (R5 pp.570-575) | ✘ | ✘ only lumped `YTRANSLINE` (XRG p.25) |
| ngspice `.probe` | inserts 0 V sources, vectors `r1#branch` (NG §11.6.5 pp.356-357) | ✘ HSPICE `.PROBE` only selects outputs (HCR p.255) | ✘ | ✘ | ✘ | ✘ |
| `.sp` + port | `V … portnum n z0 x`, NG p.103; `.sp dec/lin …` p.336 | ~ (unverified) | ~ `.NET` (LT `DotCommands`) | ~ `port` + `sp` (unverified here) | ~ `acsp` (V `cmd-analysis-acsp.md`) | ~ (unverified) |
| R/C/L with model | `RXXX n+ n- <value> <mname> <l= w=>`, NG §3.3.2 p.80 | ✔ `Rxxx n1 n2 <mname> Rval`, HSA pdf p.168 | ✘ `Rxxx n1 n2 <value> [tc=]` only (LT `R-device`) | ✔ `Name 1 2 [0] ModelName …`, `model resmod resistor rsh=…`, R5 p.629 (parameter names differ) | ~ `sp_resistor` renames the model's `r`/`tc1`/`tc2` (`src/codegen/vacask.rs:160-166`) | ✔ `R<name> n+ n- [model] [value]`, XRG p.201 |
| instance flag `off`/`on`/`thermal` | M/D/Q `<off>` NG pp.133-159; S/W `<ON><OFF>` p.92; D `thermal` p.142; NGSRC `vdmos.c:16,23`, `sw.c:14`, `dio.c:13,27` | ~ (unverified) | ✔ M `[off]` (LT `M-device`), S `[on,off]` (LT `S-device`) | ✘ | ✘ | ~ (unverified) |
| `.global` | `.GLOBAL n…`, NG §2.7 p.58 | ✔ HCR p.114 | ✔ LT `DotGlobal` | ✔ `global`, **first node is ground, at most one statement** (R5 pp.871-872) | ✔ `global n…`, top level only (V `input-cir-nodes.md:22-31`) | ✔ XRG §2.1.11 p.44 |
| `.pz`/`.sens`/`.pss` | NG §11.3.6-7 p.335; `.pss gfreq tstab oscnob psspoints harms sciter steadycoeff <uic>` §11.3.12 p.341 | `.sens`/`.pz` exist (codegen `src/codegen/spice3.rs:502-509`) | — | `pss`/`sens` exist (codegen) | `pss` (V `cmd-analysis-pss.md`) | — |
| POLY | `E N+ N- POLY(ND) NC1+ NC1- P0 P1…`, NG p.105, p.122 | ✔ `POLY(NDIM)`, HSA pdf pp.256-257 | ✔ `Exxx n+ n- POLY(<N>) …` (LT `E-device`) | ✔ `pvcvs`/`pvccs`/`pcccs`/`pccvs coeffs=[…]`, same coefficient order (R5 pp.590,592,614,616) | ~ no polynomial builtin; a behavioural source would do (V `dev-builtin-behavioral.md:1-25`), but SpiceRack refuses B for VACASK (`src/codegen/vacask.rs:897-902`) | ✔ XRG §2.2.5 pp.174-175 |
| `.func` | `.func f(x,y) {x*y}`, NG §2.12 pp.66-67 | ✔ `.PARAM f(a,b)='expr'`, HCR p.237 | ✔ `.func name(args) {expr}`, may be local to a subckt (LT `DotFunc`) | ~ `real f(real a){return …;}`, **top level only, real arguments only** (R5 pp.870-871); whether a body may read `v()` is unverified | ✘ not found in `docs/` (unverified) | ✔ `.FUNC`, XRG §2.1.10 p.42 |
| `.if` | NG §2.15 pp.68-69 | ✔ `.IF`/`.ELSEIF`, HCR pp.142, 93 | ✘ not in LT `DotCommands` | ✔ `if (c) {…}` (UG p.109) | ✔ `@if/@elseif/@else/@end`, instances and models only (V `cir-conditional.md:7-30`) | ✘ not found (unverified) |
| expression in an `f64` slot (waveform args, K, `.tran`, `.temp`, `.ic`) | `{}`/`'…'` anywhere a number goes | ✔ `'…'` | ✔ `{}` | ✔ bare expressions in instance parameters (UG pp.85-86) | ✔ expressions (V `expr-overview.md`) | ✔ `{}` |

## 3. Proposed IR changes, ranked by decks unblocked per unit of code

A struct-literal change has to touch every construction site. Current counts, from
`grep -rn "<Variant> {" src tools tests`:

| variant | sites | files |
|---|---|---|
| `Resistor` | 39 | 13 |
| `Mosfet` | 24 | 9 |
| `VoltageSource` | 56 | 11 |
| `Transient` | 33 | 11 |
| `IrWaveform::Pulse` | 20 | 10 |

`#[serde(default)]` keeps old JSON readable. It does not reduce the Rust edit count.

| # | change (exact) | codegen changes | decks (greedy / touched) | cost |
|---|---|---|---|---|
| 1 | **Importer only.** Join a bare `pulse/sin/sine/pwl/exp/sffm/am` word with the tokens after it into one group; add the `sine` alias; parse `.ac` points with `num()`; build `PoleZero` (`.pz n1 n2 n3 n4 cur\|vol pol\|zer\|pz`) and `Sensitivity` (`.sens out [dc\|ac var n f1 f2]`) | none; the variants are already emitted (`src/codegen/spice3.rs:502-511`) | 15+8+6+1 ≈ 30 / 41 | ~40 lines in `src/import.rs` |
| 2 | `Testbench.prints: Vec<PrintCard>`, `#[serde(default)]`; `struct PrintCard { analysis: String, outputs: Vec<String> }` | spice3: emit `.print {analysis} {outputs}` for ngspice/ESPice/HSPICE. LTspice, Spectre, VACASK: omit. That is faithful under the rawfile run model, because ngspice ignores `.print` with `-r` (NG p.353) and SpiceRack compares rawfiles. A future Xyce codegen must emit them (XRG p.128). `.plot` maps to the same card; `.width` can then be dropped | **58 / 90** | ~40 lines + 1 `Testbench` literal site per constructor (`src/import.rs:479-494`) |
| 3 | **Bare flags with no type change:** an empty value in `ModelDef.parameters`, and in element `params` for D/Q/M/J/Z/S/W, means a flag (`("pchan","")`, `("off","")`). This is the rule `cba3e03` already uses for options. The importer puts bare words in `pairs()`/`split_params` and gives S/W an optional trailing `on`/`off` | spice3 `model()` (`src/codegen/spice3.rs:119-126`) and `params()` (`src/codegen/mod.rs:86-88`) emit a bare `k`. Spectre: refuse a flag (no equivalent). VACASK: map `pchan` to the polarity sign in `model_type` (`src/codegen/vacask.rs:167-178`), refuse the rest | 7 alone; it is the prerequisite for VDMOS (29) and LTRA (18) | ~30 lines |
| 4 | `Mosfet { nb: Option<String>, #[serde(default)] extra_nodes: Vec<String>, … }` for 3 to 7 nodes (VDMOS 3/5, B4SOI 7, NGSRC `inp2m.c:17-35`). Old JSON (`"nb":"x"`) still deserializes | spice3: print the nodes. Spectre: refuse `nb=None` or extra nodes. VACASK: `sp_vdmos` (terminal list unverified) | **29+5 / 40** (with #3) | 24 sites / 9 files |
| 5 | `CircuitIR.globals: Vec<String>`, `#[serde(default)]` (top level; ngspice, VACASK and Spectre all scope it globally) | spice3 `.global …`; Spectre: one `global 0 <nodes>` (first node = ground, R5 p.871); VACASK `global …` | 9 / 9; faithful in every dialect | ~20 lines |
| 6 | R/C/L: `#[serde(default)] model: Option<String>`, with `value: Option<IrValue>` (old JSON still reads) | spice3: `R n1 n2 [value] [model]`. LTspice: refuse (no model form). Spectre: `resistor`/`capacitor`/`inductor` model kind, parameter renames unverified, refuse until mapped. VACASK: refuse (rename hazard `src/codegen/vacask.rs:160-166`) | 9 / 9 | 39+ sites; the `Option` value touches every reader |
| 7 | Positional device `Component::Line { letter: char /* O Y P U */, name, nodes: Vec<String>, model: String, params }` + #3 + model values with blanks (`R="0.03 0 0 …"`) | spice3: emit for ngspice/ESPice; LTspice O/U only; HSPICE refuse. Spectre/VACASK refuse for now (`mtline` mapping later) | LTRA 18 + TXL 9 + CPL 7 + URC 1 / 37 raw-free, **0 new cross-dialect decks** | ~50 lines |
| 8 | `IrWaveform::TrNoise { na, nt, nalpha, namp, rtsam, rtscapt, rtsemt: f64 }`, `TrRandom { kind: u8, ts, td, p1, p2: f64 }` (NG pp.100-101) | ngspice/ESPice emit; every other dialect refuses (no faithful form, table §2) | 11 / 14 (2 of them need #10 for `'ttime10'`) | ~30 lines, match arms in 3 codegens |
| 9 | `.pss`: `Pss { … #[serde(default)] sc_iter: Option<u32>, steady_coeff: Option<f64>, uic: bool }` (NG p.341 lists both as positional); `.sp`: `VoltageSource.#[serde(default)] port: Option<RfPort { num: u32, z0: f64 }>` | spice3 `.pss` (`src/codegen/spice3.rs:521-522`) prints all 7 fields; `.sp` adds `portnum n z0 x` | 6 + 6 / 13 | `Pss`: few sites; `port`: 56 `VoltageSource` sites |
| 10 | **Expression-capable numbers:** `#[serde(untagged)] enum IrNum { Num(f64), Expr(String) }`. Untagged keeps old JSON byte-compatible: a number stays a number. Use it in the `IrWaveform` fields, `MutualInductor.coupling`, `Vcvs/Vccs/Cccs/Ccvs` gains, `Transient.{step,stop,start,max_step}`, `temperature`, `initial_conditions`/`node_sets` values | spice3 `waveform()` (`src/codegen/spice3.rs:38-59`): `{expr}`, or `'expr'` for HSPICE. Spectre: expression text (`emit_waveform_params`, `src/codegen/spectre.rs:131-208`). Its `delay != 0.0` tests become `!= Num(0)`. VACASK: the arithmetic (`period <= rise+fall+width`, `td2 = fall_delay - rise_delay`, `phase != 0`, `src/codegen/vacask.rs:251-296`) must emit `(a)-(b)` text or refuse when not numeric | 1 (c7552, 207 cards) + 4 (paren-less PWL with `'vp'`) + 3 K + 3 `.tran` + 2 TRRANDOM + 1 `.temp/.ic` ≈ 14 | largest: ~150 sites (Pulse 20, Transient 33, sources 56) |
| 11 | `Component::Poly { name, kind: char /* E F G H */, np, nm, controls: Vec<String> /* node pairs (E/G) or V names (F/H) */, coeffs: Vec<f64> }` | spice3 `POLY(n)`; Spectre `pvcvs/pvccs/pcccs/pccvs (… ) coeffs=[…]` (R5 pp.590-616, same term order); VACASK refuse until B is supported | 3 / 5 | ~60 lines |
| 12 | `CircuitIR.functions: Vec<FuncDef { name: String, args: Vec<String>, body: String }>`, `#[serde(default)]` (top level only; a `.func` inside `.subckt` stays raw) | spice3 `.func n(a) {body}`; HSPICE `.param n(a)='body'` (HCR p.237); Spectre `real n(real a) { return body; }` top level (R5 pp.870-871), refuse if the body reads `v()`/`i()` (unverified); VACASK refuse | 4 / 4 | ~40 lines |

Also do this, independent of the table: a **raw-text provenance tag**, `CircuitIR.#[serde(default)] raw_dialect:
Option<String>`, set by `import::parse`. Today `body()` copies ngspice raw lines into the HSPICE and LTspice decks
as they are (`src/codegen/spice3.rs:142`), and only Spectre/VACASK refuse (`src/codegen/mod.rs:56-62`). TRNOISE,
`.probe`, TXL and CPL go out verbatim to HSPICE, which has no such cards. The rule should be: a non-empty
`raw_spice` emits only to the dialect family named in `raw_dialect` (ngspice ≡ ESPice), and every other dialect
refuses. That is about 10 lines and closes the "never a silently different deck" contract (`src/codegen/mod.rs:54`).

### Stay raw (no faithful cross-dialect form, or not worth an IR shape)

| construct | why it stays raw |
|---|---|
| ngspice `.probe` | Its semantics are inserted 0 V sources with ngspice-named vectors (`mq4:s#branch`, NG p.357). HSPICE's `.PROBE` means something else (HCR p.255). |
| `.if` blocks | A conditional netlist tree is needed. LTspice and Xyce have no form, and VACASK allows only instances and models inside (`cir-conditional.md:22-28`). 2 sole decks. |
| `.csparam` | Experimental, and only feeds `.control` (NG §2.13 p.67). |
| `.width` | Printer layout. Drop it once #2 exists. |
| `DISTOF1/2` | 1 deck, and only used by `.disto`. |
| `.temp` with a list | 2 decks; the meaning differs between simulators (unverified). |
| malformed cards: `V… {dc=-6}` (6 cmcqa generator decks), the stray `.00` in `.MODEL MPSA92 PNP(…)` | Keep them verbatim rather than guess the intent. |
| TRNOISE/TRRANDOM, TXL/CPL | Even with #7/#8, no dialect other than ngspice can emit them. The IR form only adds structure and correct refusals. |

Cheapest high yield, in order:

1. importer fixes (#1)
2. `prints` (#2)
3. bare flags (#3)
4. `Mosfet` nodes (#4)
5. `globals` (#5)
6. the raw-dialect tag

Together these take about 150 lines plus the 24 `Mosfet` sites. They leave about 150 of the 244 raw-holding decks
raw-free (greedy, §1). `IrNum` (#10) costs the most code for about 14 decks. It is still the right long-term shape,
because the same gap blocks the Python API (`py_value`, `tools/spice2rack.rs:39-45`).
