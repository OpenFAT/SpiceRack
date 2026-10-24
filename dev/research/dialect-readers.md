# Dialect readers: Spectre, VACASK, Xyce, LTspice

Survey date: 2026-10-24. SpiceRack at `fed5dee` (branch `dialect-import`), ESPice at `287fb0af`. Follows
`dev/research/dialect-import.md` (called **DI** below; its §3 has the per-dialect reference table and §6 the
plan). Local paths are relative to the SpiceRack root unless they start with `ESPice/`
(`~/Documents/Projects/Zig/ESPice/`) or `$S/` (this session's scratchpad,
`/tmp/claude-1000/-home-omare-Documents-Projects-Rust-SpiceRack/ff86074c-6f19-46eb-80b8-1bf4d0ce5e16/scratchpad/`,
not checked in). Measurements come from throwaway programs in `$S/` (`specrt/`, `vacask_agent/`, `xyce_agent/`,
`lt_agent/`) that link `spicerack::peg` (`src/lib.rs:13`); nothing was added to the repo.

## 1. Spectre

### 1.1 Primary reference
Sources. **SN** = ESPice's deleted Spectre notes, recovered with `git -C ESPice show
05852567^:dev/research/spectre-syntax.md` (1252 lines; copy at `$S/spectre-syntax.md`). SN quotes the Cadence
manuals with printed page numbers: **UG** = *Virtuoso Spectre Circuit Simulator User Guide* 5.1.41 (2004),
https://web.eecs.utk.edu/~dbouldin/protected/spectreuser.pdf; **R11/R19** = *Spectre Circuit Simulator
Reference* 11.1 / 19.1 (URLs in SN "Sources"). The UG/R page numbers below are as SN quotes them; I did not
re-open the PDFs (unverified beyond SN). The grammar that encodes SN is `grammar/spectre.peg` and
`grammar/spectre_lines.peg`, which ESPice tests against the NetlistParse.rs corpus.

| topic | rule | source |
|---|---|---|
| title | line 1 of the top file is the title; an included file has none | UG p.29, p.50, p.73 (SN §1.2); `spectre_lines.peg:176-177` |
| statements | one per line; `= ( ) :` significant, blanks free between tokens | UG p.59 (SN §1.1) |
| comments | `//` to end of line (after a blank); `*` in column 1; no `/* */` | UG p.59 (SN §1.3); `spectre_lines.peg:188-194,212` |
| continuation | trailing `\` (preferred) or leading `+` | UG p.59 (SN §1.4); `spectre_lines.peg:196-203` |
| case | fully case-sensitive, keywords lowercase; `simulator lang=spice` regions fold case | UG p.50, p.59 (SN §2.1) |
| names | letter or `_` first, then letters, digits, `_`, plus `!` (`vdd!`); any byte after `\` (`\2N2222`, `net\<3\>`); nodes may be integers, `007` ≠ `7`; `.` is the hierarchy separator | UG p.60, p.62 (SN §2.2, §2.6); `spectre.peg:151-158` |
| numbers | SI scale factors, case-sensitive: `T G M K k _ % c m u n p f a` (`M` = 1e6, `m` = 1e-3); letters after the factor are a unit; `1meg` = 1e-3; no factor after an exponent | UG p.83-84 (SN §2.4); `spectre.peg:144-149` |
| strings | `"..."`, `\"` escapes | UG p.80, p.174 (SN §2.5) |
| expressions | bare (no braces/quotes). Precedence, tightest first: unary `+ -`, `**` (right), `* /`, `+ -`, `<< >>`, relational, `== !=`, `&`, `~^`/`^~` (XNOR), `\|`, `&&`, `\|\|`, `?:`. **No `^` operator** (XOR/power both absent); no `%` (use `fmod`); `-2**2` = 4 | UG p.87-88, R11 p.336, R19 p.475 (SN §6.1) |
| functions | `log ln log10 exp sqrt min max abs pow int floor ceil fmod sgn sign` + trig/hyperbolic; constants `M_PI`…`P_CELSIUS0`; `temp`, `tnom` predefined | R19 p.476-477, UG p.91-93 (SN §6.2-6.3) |
| user functions | `real f(real a, real b) { return expr }`, top level only | UG p.92-93 (SN §4.7) |
| parameters | `parameters p1=1 p2=p1*2` top level or in a subckt; subckt ones may omit a default; declared before use | UG p.85, R19 p.506 (SN §4.6) |
| subckt | `[inline] subckt name [(]ports[)]` … `ends [name]`; nests; local models/subckts shadow | UG p.94-104 (SN §4.5) |
| model | `model name master k=v`; master is a primitive (`diode bjt vbic mos1 bsim3v3 bsim4 psp103 …`) with `type=n/p`; model groups `model n m { 1: lmin=… }` select `n.1` | UG p.71, p.108 (SN §4.4) |
| instance | `name [(]nodes[)] master k=v`; first letter means nothing; without parens the master is the last bare word before the first `k=`; zero nodes legal | UG p.62-63 (SN §4.2) |
| primitives | `resistor r`, `capacitor c`, `inductor l`, `vsource`/`isource` (`dc`, `type=dc\|pulse\|pwl\|sine\|exp`, `val0 val1 period rise fall width delay`, `sinedc ampl freq sinephase damp`, `td1 tau1 td2 tau2`, `wave=[t v …]`, `mag phase`), `vcvs gain`, `vccs gm`, `ccvs rm probe`, `cccs gain probe`, `mutual_inductor coupling ind1 ind2`, `bsource v=/i=` | R5 pp.283-686 (SN §4.3) |
| analyses | `name [(nodes)] type k=v [{ children }]`: `dc` (no params = op; `dev= param= start stop step\|lin\|dec\|values`), `ac`, `tran stop step start maxstep method skipdc`, `noise iprobe=`, `xf sp stb pz pss pac pnoise pxf hb …`, `sweep`/`montecarlo` with a braced body; run in netlist order | UG p.66-68, p.165-168, R19 p.406 (SN §4.9) |
| control | `name options k=v` (also `alter altergroup info set shell check assert paramtest paramset`) | UG p.68-69, p.212 (SN §4.10) |
| save / ic / nodeset | unnamed: `save out M1:currents L1:1`, `ic n=v`, `nodeset n=v`; `inst:term` selects a terminal current | R11 p.366-367, R19 p.517 (SN §4.11) |
| include / lib | `include "f" [section=s]`; `library`/`section` … `endsection`/`endlibrary` only in a library file; `ahdl_include "x.va"` | UG p.72-76, R19 p.493-498 (SN §3.3-3.5) |
| SPICE regions | `simulator lang=spice` … `simulator lang=spectre`; Spectre's SPICE reader is closest to HSPICE (`.lib` = HSPICE section include) | UG p.41-56 (SN §3.1-3.2); `spectre.peg:3-6` |
| conditionals | `if (c) stmt [else stmt]` or braced lists, evaluated at flattening | UG p.109-111, R19 p.491 (SN §4.12) |
| ground / global | `0`, or the first name of `global` | R5 p.871, R19 p.482 (SN §4.8) |

### 1.2 `spectre.peg` captures → SpiceRack IR
Capture list: `spectre.peg:8-13`. IR types: `src/ir/mod.rs`. "ESPice" = `ESPice/src/frontend/spectre.zig`, which
turns Spectre statements into SPICE cards before ESPice's card reader runs (`spectre.zig:1-6`).

| capture (rule, line) | fields read | IR target | notes |
|---|---|---|---|
| `Simulator` (`:28`) | `Param` `insensitive=` | none (mode switch) | ESPice ignores the params (`spectre.zig:544`) |
| `SpiceRegion` / `SpiceCard` (`:20-21`) | card text | the SPICE path: `spice_cards.peg`+`spectre_ops.peg`+`hspice.peg`+`spicerack.peg` `OneCard` (compiles and reads `.meas …`; checked in `$S/specrt/src/bin/comp.rs`), then today's `import.rs` card lowering | ESPice rewrites numbers to plain floats and `'e'` to `"e"` first (`spectre.zig:63`); the lines layer already yields `SpiceLine` rows (`spectre_lines.peg:219`) |
| `Include` (`:31`) | `String`, optional section `Name` | `Subcircuit.includes` / `.libs (path, section)` (`ir/mod.rs:25-26`) | ESPice refuses it (`spectre.zig:563`); SpiceRack keeps references, as it does for SPICE |
| `AhdlInclude` (`:32`) | `String` | `osdi_loads` (`ir/mod.rs:27`) | the codegen writes `osdi_loads` as `ahdl_include` (`src/codegen/spectre.rs:365`), so this is its inverse |
| `Library` / `Section` (`:36-37`) | `Name`, body | none: raw (IR has no section definition; DI §4a row `.lib`) | library files only (R19 p.498) |
| `Global` (`:40`) | `Node`s | none: raw. First node = ground | ESPice: `.connect 0 g` plus `.global` (`spectre.zig:663`) |
| `Parameters` / `ParamDecl` (`:43-44`) | `Key`, value | top: `top.parameters`; in a subckt: `Subcircuit.parameters` (`ParamDef{name, default}`) | in Spectre a subckt's `parameters` are its overridable parameters, which is what `ParamDef` means (the codegen writes them that way, `spectre.rs:311-318`); ESPice folds them into the `.subckt` header (`spectre.zig:570-598`) |
| `Model` (`:48`) | `Name`, `Master`, `Param`s | `ModelDef{name, kind, parameters}` (`ir/mod.rs:89-93`) | `kind` must be the SPICE type: map master + `type=` through ESPice's table (`diode`→`d`, `bjt`→`npn/pnp`, `bsim3v3`→`nmos/pmos level=8`, `bsim4`→54, `psp103`→1040 …, `spectre.zig:433-451`; same table `ESPice/tests/frontend/lower_spectre.zig:40-58`) |
| `Group` (`:49`) | label, `Param`s | one `ModelDef` per bin, named `<model>.<label>` | as ESPice (`spectre.zig:627-636`); DI §4a: binned names are plain names in the IR |
| `Subckt` / `Ends` (`:52-54`) | `Name`, `Node` ports, body | `Subcircuit` (+ hoisting of nested ones as `import.rs::close_subckt` does) | `inline` has no IR field (lost) |
| `Function` / `FArg` (`:58-60`) | name, args, `Expr` | none: raw (same gap as SPICE `.func`, DI §4a) | ESPice → `.func` (`spectre.zig:762`) |
| `Save` / `Signal` (`:63,66`) | `Node` [`:` term] | `tb.saves`: a node → `v(n)`, `inst:term` → `i(inst)` | see §1.4 for the codegen's ambiguous `save V1` |
| `Ic` / `Nodeset` (`:64-65`) | `Signal`, value | `initial_conditions` / `node_sets` `(String, f64)` | numeric values only (IR is `f64`); `inst:term` signals refused, as ESPice does (`spectre.zig:679-688`) |
| `If` / `Else` (`:72-73`) | `Expr`, branches | none: raw (no IR for `.if`, DI §4a) | ESPice → `.if/.elseif/.else/.endif` (`spectre.zig:690-760`) |
| `Statistics` (`:78`) | — | none: set aside (raw) | ESPice warns and skips (`spectre.zig:559-562`) |
| `Analysis` (`:89`) | `Name`, `Node`s, `Kind`, `Param`s, body | `dc` with no params → `Op`; `dc dev=X param=dc start stop step` → `Dc{sweeps:[{source:X…}]}`; `ac start stop dec\|lin\|log=n` → `Ac`; `tran stop step start maxstep` (+ `skipdc=yes` → `uic`) → `Transient`; `(p n) noise iprobe=V start stop dec=` → `Noise`; `sweep param= start stop step\|dec … {` → `step_params` + the inner analyses; `pss pac pnoise pxf pstb sp stb` → the existing `Spectre*`/`Pss`/`SPar`/`Stability` variants | the analysis name is lost (the codegen invents `op1 dc1 ac1 tran1 noise1`); ESPice's subset is `tran dc ac noise` (`spectre.zig:948-1024`) with `values=` → `poi`, `lin=` → a step expression, `method=` → `.options` (`:409-413`) |
| `Control` (`:96`) `Kind2`=`options` | `Param`s | `SimOptions.portable`, with `temp=` → `temperature`, `tnom=` → `nominal_temperature`, and the inverse of `map_option_name` (`iabstol`→`abstol`, `vabstol`→`vntol`, `dcmaxiters`→`max_iterations`, `spectre.rs:344-351`) | the codegen writes `temp` as `mytemp options temp=` (`spectre.rs:431`); ESPice maps the same names (`spectre.zig:1026-1045`); other control kinds raw |
| `Instance` (`:102-103`) | `Name`, `Node`s, `Master`, `Param`s | by master: primitive → `Component` (lead keys `r c l gain gm probe rm ind1 ind2 coupling`, `spectre.zig:864-895`; source keys `dc mag phase type val0 …`, `:897-946`) ; a `model` name → its master's letter (`Diode/Bjt/Mosfet/Jfet`); a `subckt` name → `Instance`; anything else (Verilog-A module) → raw | needs a declare pass first, as instances may precede their model/subckt (UG p.87; `spectre.zig:509-529`) |
| `Expr` and its levels (`:121-142`), `Num` (`:146`), `Vector` (`:114`) | text | `IrValue::Numeric` via a **Spectre** number reader (case-sensitive `M`/`m`, `%`, `_`), else `IrValue::Expression` | `import.rs::num` lowercases first (`import.rs:195`), so it would read `1M` as 1e-3; Spectre needs its own |

Names. The IR keeps SPICE names without their letter (`import.rs:291`), and the codegen adds one back
(`r{name}`, `x{name}`, `spectre.rs:490,299`). For a fixpoint, the lowering drops the first letter of an instance
name only when it equals its primitive's SPICE letter (`r1 resistor` → `1`, `load resistor` → `load`). Then
`load` → `rload` → `load`, which is stable from the second pass on.

### 1.3 What ESPice's `spectre.zig` does that SpiceRack needs
1. A declare pass over model, subckt and `ahdl_include` names before reading instances (`spectre.zig:509-529`).
2. The primitive → letter table (`:423-430`) and the model master → SPICE type/LEVEL table (`:433-451`).
3. The source mapping from `type=` and its keys to `pulse/sin/exp/pwl(...)` (`:904-946`). A missing key reads
   as 0, and `wave=[…]` gives the PWL points.
4. Subckt `parameters` as subckt defaults (`:570-598`); `global` → ground (`:663-677`).
5. `if`/`else` → `.if/.elseif/.else/.endif`, braced or not (`:690-760`); a model group → binned models
   (`:627-636`).
6. The analysis translation (`:948-1024`) and the option-name map (`:1026-1045`).
7. Things SpiceRack does better already: `include`/`section` (ESPice refuses them, `:563`).

ESPice's approach (Spectre text → SPICE text → SPICE reader) would let SpiceRack reuse `import.rs` unchanged. But
the IR is richer than SPICE text for Spectre-only analyses (`SpectrePac`, `sweep` children), so a direct capture
→ IR lowering is the better fit. Its tables can be copied.

### 1.4 Round trip: SpiceRack's Spectre output through `spectre.peg`
Method: `$S/specrt` reads a deck with `spectre_lines.peg` `File`, joins `Piece`s with a blank, and parses the
result with `spectre.peg` `SDeck` (whole deck) and, per logical line, with the `OneStmt` entry below (`one`
binary). The decks are `spice2rack -o` output (`tools/spice2rack.rs:3`) for all ngspice, circuitsim90 and
powergrid decks (`ibmpg*` excluded), plus every 20th cmcqa deck. That gives 397 `.spectre.scs` files; the other
decks were refused by the codegen.

- A hand deck covering R/C/L, V (dc/ac/sin/pulse/pwl), I, D/M/Q, E/G/F/H, X, `.param`, `.model`, `.subckt`
  with `params:`, `.ic/.nodeset/.temp/.options/.save`, `.op/.dc/.ac/.tran/.noise`, `.include`, `.lib`, `.meas`
  (`$S/sp/rt.sp`, `$S/sp/rt2.sp`) emits a deck that parses whole. Each statement lands on the capture §1.2
  expects (`Instance`, `Model`, `Subckt`/`Parameters`/`Ends`, `Control options`, `Ic`, `Nodeset`, `Save`,
  `Analysis`, `Include` with section, and `SpiceRegion`/`SpiceCard` for the `.meas`).
- Suite: **351 of 397 parse whole; 46 fail.** Per-statement, the failures fall into these groups:

  | cause | example (emitted line) | decks |
  |---|---|---|
  | SPICE-legal name bytes not escaped (`#`, `/`, `:`, `-`, `<>`, leading `+`, leading digit), which UG p.62 requires escaping with `\` (SN §2.2) | `r354_20 (xpower_gate16/MM6_6:DRN VVDD:47488) resistor`, `cvdd#1 (11 2)`, `r1 (Vin Net-_R1-Pad2_)`, `model 1n4002 d`, `v2 (15+ GND)` | 19 |
  | ngspice vectors in `save` | `save vcc#branch`, `save @q1[gm]` | 5 |
  | model text copied verbatim (`emit_model`, `spectre.rs:287-293`): SPICE masters (`nmos`, `npn`, `d`), uppercase keys and SPICE scale factors (`0.26U`, `159N`), `version=3.3.0` | `model nm NMOS LEVEL=3 TOX=.50E-7 LD=0.26U`, instance `PCH W=4.5U` | 17 |
  | SPICE expression spelling leaks (`'…'`, `{…}`, `^`) | `model dplain d is='temper+1000'`, `vsource dc=2 ^ 3`, `x1 (n1 n2) sub1__sub foo='3*foo'` | 5 |

- **Silent changes that parse.** These are worse than a parse failure, because the deck still reads:
  - Model scale factors keep SPICE meaning in Spectre syntax: `IKF=30M`, `IKR=45M` (`555-timer-2`) and
    `IKR=93.200M` (`bjt_ac_par`) mean 30e-3 in SPICE and 30e6 in Spectre (UG p.84). `229P` reads as 229e15.
  - Model masters are SPICE types: over the 397 decks, `model … nmos` 111, `pmos` 91, `npn` 33, `d` 13,
    `VDMOS` 9 (`grep -h '^model' | awk '{print $3}'`). Spectre's masters are `bsim3v3`, `bjt`, `diode` …
    (SN §4.3-4.4), so these models are unknown to Spectre (unverified without Spectre). The lowering in §1.2 has
    to map master → kind, and the codegen the inverse.
  - `.dc V1 0 1 0.1 R1 1k 2k 500` emits only the first sweep. `Analysis::Dc` writes `sweeps.first()`
    (`spectre.rs:654-667`), and the IR holds both (`$S/sp/out/rt2.json`).
  - `.tran … uic` loses `uic`: `Transient { .. }` ignores it (`spectre.rs:684-697`). ESPice reads `skipdc=yes`
    as `uic` (`spectre.zig:974-977`).
  - `save i(V1)` emits `save V1` (`bare_node`, `spectre.rs:72-83,450`), while the instance is written `v1`
    (`spectre.rs:515`). That is case-sensitive, so it names nothing, and a bare name saves a node, not a current
    (R11 p.366). It should be `save v1:p` (SN §4.11).
  - Names are lowercased (`spectre.rs:64-66,490`). A Spectre deck with `R1` and `r1` would merge them on a
    Spectre → IR → Spectre trip.
- **ESPice fixtures** (`ESPice/tests/frontend/spectre_fixtures/`, 142 `.scs` from NetlistParse.rs @ `a71091f`, MIT,
  `README.md:1-9`; a title line prepended as the harness does): `SDeck` accepts 110 of 142. Of the 23 `err_*`
  decks, 22 are refused. The accepted one, `conditional/err_braceless.scs`, is legal per R19 p.491. Of the 119
  valid decks, 10 are refused, all by documented gaps: bitwise/shift/reduction operators (`spectre.peg:118-120`;
  `params_expr/{op_chains,relational_logical,shift_reduction,reduction_nor_nand,base_specifiers}`,
  `julia_parse_tests/var_reference`), NetlistParse.rs leniencies (`model/bare_param`, `instance/trailing_bare_param`,
  `include_global/lenient_section_empty`), and `protect/protect_block`.

### 1.5 Entry rule for SpiceRack (line at a time)
`import.rs` reads one logical line at a time so that a refused card stays raw (`spicerack.peg:1-4`).
`spectre.peg`'s `Subckt`, `If`, `Model` group, `Function` and braced `Analysis` span lines, so SpiceRack needs a
Spectre twin of `OneCard`. Draft, checked over the same 397 decks and the 119 valid fixtures (`$S/specrt/spectre_one.peg`,
composed after `spectre.peg`):

```peg
# SpiceRack: one Spectre logical line at a time, as spicerack.peg's OneCard
# does for SPICE; a block's opener and closer are separate lines, which the
# lowering pairs.
OneStmt    <- (SubcktHead / EndBlock / Ends / BlockHead / Close / SimSpice / stmt) nl !.
SubcktHead <- ('inline' s)? 'subckt' s Name ports?
# A line that opens a brace block: model group, if branch, function body,
# statistics, sweep / montecarlo / altergroup.
BlockHead  <- 'model' s Name s Master (s param)* s? '{'
            / 'if' s? '(' s? Expr s? ')' s? '{'?
            / 'statistics' s? '{'
            / 'real' s Name s? '(' s? (FArg (s? ',' s? FArg)*)? s? ')' s? '{'
            / Name s? (nodelist s?)? (Kind / Kind2) !wchar (s param)* s? '{'
# `}`, `} else {`, `} else if (c) {`, and a `{` on its own line.
Close      <- '}' (s? 'else' (s 'if' s? '(' s? Expr s? ')')? s? '{')? / '{'
SimSpice   <- 'simulator' s 'lang' s? '=' s? 'spice' (s Param)*
# A library file's openers and closers (§3.4).
EndBlock   <- ('endsection' / 'endlibrary') (s Name)? / ('section' / 'library') s Name
```

`EndBlock` must come before `Ends`. Otherwise `Ends` matches the `ends` of `endsection`, and PEG does not
backtrack into the choice. Results: the per-statement refusals match the whole-deck failures (the same 46 decks);
the fixtures' valid decks refuse 25 of 436 lines. Those 25 are the 10 gaps above plus function-body and
`statistics` inner lines (`return …`, `process {`), which the lowering reads as block content, not statements.


## 2. VACASK

### 2.1 Sources
- **V** = the VACASK clone at `9ca4982` (2026-10-22), `$S/VACASK`. Within it: **L** = `lib/dfllexer.l`, **P** =
  `lib/dflparser.y` (line numbers), plus the `docs/*.md` files. Upstream is https://codeberg.org/arpadbuermen/VACASK.
- **probe** = a run of the binary from SpiceRack's `nix develop .#testing`
  (`/nix/store/g5ial84gcp2da9h7y63r4c6dc1iqip57-vacask-unstable-2026/bin/vacask`).
  - That binary is built from the fork `robtaylor/VACASK@bcd48e2` (`nix/vacask.nix:10`), which is **older than
    the docs**, so where the two differ both are noted.
  - Harness: `$S/vacask_agent/pegtest`, a copy of `src/peg.rs` plus the line joining of `import.rs:54-78`.

### 2.2 Syntax reference
| topic | VACASK | source |
|---|---|---|
| title | line 1 is always the title; included files have none | L159-167, P315, L319-359 |
| statement end | newline, except inside `()`, `[]`, `{}` | L913-919, `docs/input-overview.md:24` |
| continuation | `\` immediately before the newline; `\` then blanks is an error; **no `+`** | L921, L926 |
| comments | `//` and `/* … */` anywhere outside strings; `*` in column 1 is **not** a comment (probe: syntax error) | L176-187, `input-overview.md:28` |
| case | sensitive everywhere (probe: master `V` ≠ `v`) | `input-overview.md:20` |
| identifiers | `[a-zA-Z_$][0-9a-zA-Z_$]*`, plus `'…'` quoted parts (`''` = a quote); no `!`, `.` or `\`; hierarchy separator `:` (`v('x1:out')`); nested definitions `a::b`; `$mfactor` is the multiplier. bcd48e2 quotes only `[0-9a-zA-Z_$:.()]` | L869-911, `docs/input-identifiers.md:6-19`, `cir-hier.md:7`, `cir-subckt-nested.md:48`, `cir-mfactor.md` |
| keywords | only at statement start; longest match: `include section endsection load model global ground subckt ends parameters control endc embed @if @elseif @else @end`; inside `control`: `analysis sweep save` | L783-868, `docs/input-reserved.md:5,30` |
| numbers | `0\|[1-9][0-9]*` (`007` is 3 tokens); hex `0x..` (bcd48e2: digits only); floats; SI prefix **with no exponent**, then a unit: `T G M x X meg`(1e6) `k K m u n p f a mil`; `inf` `nan`; `1V`, `1e3k` are errors (probe); 32-bit integers, `7/2` = 3 | L594-693, `docs/input-numbers.md`, `expr-scalars.md:26` |
| strings | `"…"` with C escapes, no newline; `<<<MARK … >>>MARK` raw | L188-271, L695-704, `docs/input-strings.md` |
| vectors / lists | `[a, b]` (commas required; probe: `[0 0 1 1]` fails), `{a, b}` lists (bcd48e2: `[a; b]`), indexing `x[i]` | P740-781, `docs/expr-vectors.md:47-64` |
| constants | `M_PI`, `P_K` …; `$temp $tnom $scale` | `docs/expr-constants.md`, `expr-special.md` |
| parameters | `parameters a=1 b=a*2` or `parameters ( … )`; a value is required; dependent parameters cannot be overridden | P848-855, P1085, `docs/cir-subckt.md:20-58` |
| model | `model NAME MASTER [params \| (params)]`; no `{}` groups (binning: `docs/cir-binning.md`) | P1067-1083 |
| subckt | `subckt NAME (ports)` … `ends [NAME]`; **parens required** (probe), nestable; no `global/ground/load/control` inside (bcd48e2: no `ends NAME`) | P324-460 |
| instance | `name (nodes) master params`; **parens required** (probe `v1 1 0 v` fails); blank-separated params only | P856-894 |
| behavioural | `name (p n) v=\|i=\|potential=\|flow=… expr`, with no master | P896-1065, `docs/dev-builtin-behavioral.md:9-35` |
| devices | builtin `vsource isource vcvs vccs ccvs cccs mutual`, all through a `model x vsource` (probe: a bare `vsource` is "not found"); R/C/L are OSDI (`resistor.osdi`); SPICE devices are `spice/*.osdi` → `sp_resistor sp_bjt sp_mos1 sp_bsim4v8 …` | `docs/dev-builtin.md`, `dev-vacask.md`, `dev-spice.md` |
| sources | `type="dc\|sine\|pulse\|exp\|pwl\|am\|fm"`; sine `sinedc ampl freq tdphase theta delay`; pulse `val0 val1 delay rise fall width period`; exp `val0 val1 delay td2 tau1 tau2` (`td2` is a duration); pwl `wave=[t,v,…] period`; `dc mag phase`. bcd48e2 uses `sinephase`/`pwlperiod` | `docs/dev-builtin-src.md:24-222`, `lib/devvisrc.cpp:26-59` |
| load | `load "x.osdi\|x.va"`, top level only | P1161-1172, `docs/cir-loading.md` |
| include | `include "f" [section=s]` (`section s` … `endsection`, no name after it); `lang=ngspice\|hspice\|pspice\|xyce\|spectre` only with `CADNIP_PARSERS` (the nix build refuses it); handled in the lexer | L272-557, `docs/input-include.md:85-120` |
| conditionals | `@if e` / `@elseif e` / `@else` / `@end`, nestable; instances and models only | P462-509, `docs/cir-conditional.md` |
| ground / global | `0` is ground; `ground a b`; `global n…`; top level only | P1145-1158, `docs/cir-nodes.md:13` |
| control | one `control … endc` block. `analysis NAME TYPE params`, one or more `sweep NAME params` that bind to the **next** analysis, `save`, `options`, `var`, `print`, `postprocess`, `alter`, `mc … endmc` | P1174-1296, `docs/cmd-overview.md:15-41`, `cmd-sweep.md:53-72` |
| analyses | `op dcinc dcxf ac acxf acstb acsp noise tran pss pac hb hbac hbnoise`, **no `dc`** (a DC sweep is `sweep` + `analysis … op`); sweep `instance= parameter= \| model= \| option= \| variable=`, `from to step \| mode= points= \| values=[…]`; ac/noise `from to mode points`; tran `stop step start maxstep icmode ic nodeset`, where ic/nodeset are `{"n", v, …}` | `docs/cmd-analysis-overview.md:28-44`, `cmd-sweep.md:16-50`, `cmd-analysis-ac.md:41-46`, `cmd-analysis-tran.md:32-39` |
| save | `save default \| full \| v(n) \| i(inst) \| p(inst, var)` | P1095-1143, `docs/cmd-save.md` |

**Expression precedence**, loosest first (P216-230, `docs/expr-operators.md:66-83`; checked by probe):

| level | operators |
|---|---|
| 1 | `?:` (right) |
| 2 | `\|\|` (short-circuit) |
| 3 | `&&` (short-circuit) |
| 4 | `\|` |
| 5 | **`^` (bitwise XOR)**: `2^3` = 1 |
| 6 | `&` |
| 7 | `== !=` |
| 8 | `< > <= >=` |
| 9 | `<< >>` |
| 10 | `+ -` |
| 11 | `* /` (no `%`) |
| 12 | `**` (right): `2**3**2` = 512 |
| 13 | unary `- + ! ~`, tighter than `**`: `-2**2` = 4 |
| 14 | call, `x[i]` |

`docs/cmd-print.md:17` writes `gain^2`; that is XOR, a doc slip.

### 2.3 Draft grammar (Spectre-family names)
Files: `$S/vacask_agent/grammar/vacask_lines.peg` (37 lines) and `vacask.peg` (141 lines, with per-rule source
comments). Both are listed below in full.

| corpus | accepted |
|---|---|
| `V/test/*.sim` | 114/114 |
| `V/demo/**/*.sim` | 91/91 |
| `V/benchmark/*/vacask*/*.sim` | 13/13 |
| 29 `.inc` (from `VLib`) | 29/29 |
| a synthetic deck with every construct (the binary accepts it too) | 1/1 |
| 5 hand decks the binary rejects | 0/5 (correctly refused) |
| SpiceRack VACASK output (`$S/out/*.vacask.sim`, `$S/py/*.vacask.sim`) | 63/94 |

Of the 31 SpiceRack failures, **28 are also rejected by the binary**: `version=3.3.0` in 17 model cards, `net-_r1-pad2_`,
`mnd#/mp1`, `model 1n4002`, `is='1e-12 * temper'`, and `*` comments in included model files. The other **3** use
`ic=["2"; 5e-1]` (`src/codegen/vacask.rs:332-337`), the bcd48e2 list syntax that `9ca4982` removed.

```peg
# VACASK: physical lines -> the title and logical statements.
# Source: lib/dfllexer.l (L = line) and docs/input-overview.md.
# Captures: Title; Line, made of Piece (joined to the previous part with a
# blank) and Glue (a bracket, joined with nothing).
# A statement ends at a newline outside (), [], {} (L913-L919); `\` right
# before the newline joins the next line (L921); `//` and `/* */` are
# comments anywhere outside strings (L176-L187). Case is kept.

File     <- Title (eol body)? !.
# L159: line 1, whatever it holds, is the title.
Title    <- (!eol .)*
body     <- (skip / Line)*
skip     <- sep* eol / sep+ !.
Line     <- sep* unit (sep* unit)* sep* eolx
unit     <- Piece / group
sep      <- blank / lcomment / bcomment / bs
# L559-L585: newlines inside brackets are not statement ends. The lexer
# counts each bracket kind; pairing is checked here.
group    <- &'(' Glue gin* &')' Glue / &'[' Glue gin* &']' Glue / &'{' Glue gin* &'}' Glue
gin      <- sep / eol / unit
Glue     <- [()\[\]{}]
Piece    <- atom (atom / blank)*
atom     <- longstr / dstring / sqname / !sepstart !eol ![()\[\]{}] .
sepstart <- '//' / '/*' / bs
bs       <- '\\' '\r'? '\n'
lcomment <- '//' (!eol .)*
bcomment <- '/*' (!'*/' .)* '*/'
# L705, L214-L271: "..." with backslash escapes, never across a newline.
dstring  <- '"' ('\\' !eol . / !'"' !eol .)* '"'
# L869: a quoted identifier part, any non-space bytes, '' is a quote.
sqname   <- "'" ("''" / ![' \t\r\n] .)* "'"
# L695, L188: <<<MARK newline ... >>>MARK. ponytail: ends at the first
# `>>>word`, not only the matching marker (PEG has no back-reference).
longstr  <- '<<<' [a-zA-Z0-9_]+ [ \t\r]* '\n' (!('>>>' [a-zA-Z0-9_]) .)* '>>>' [a-zA-Z0-9_]+
blank    <- [ \t\r]
eol      <- '\r'? '\n'
eolx     <- eol / !.
```

```peg
# VACASK: logical lines -> statements. Sources: lib/dflparser.y (P = line),
# lib/dfllexer.l (L = line), docs/*.md. Input: vacask_lines.peg's logical
# lines, case kept, each ended by `\n`; the title is not part of it.
#
# Captures the lowering reads:
#   statements  Include Lang Section Load Embed Global Ground Parameters
#               ParamDecl Model Subckt Ends AtIf AtElseIf AtElse
#               ControlBlock Analysis Sweep Save Signal Control Instance
#   fields      Name Node Master Kind Param Key Expr (and its levels)
#               Vector List Select String Num Ref Call

VDeck      <- line*
# An included library file: sections, or plain statements.
VLib       <- line*
line       <- stmt? nl
# Keywords are keywords only at the start of a statement (L783-L868).
stmt       <- Include / Section / Load / Embed / Global / Ground / Parameters
            / Model / Subckt / AtIf / ControlBlock / Instance

# L272-L557: `include "f" [lang=x] [section=s]`, either order.
Include    <- 'include' !nc s? String (s? ('lang' s? '=' s? Lang / 'section' s? '=' s? Name))*
Lang       <- [a-zA-Z_$] nc*
# L536-L557: a library file's `section name` ... `endsection`.
Section    <- 'section' !nc s? Name nl line* 'endsection' !nc

# P1161: `load "file.osdi" [map=[...]]`; P1090: `embed "name" <<<M ... >>>M`.
Load       <- 'load' !nc s? String (s? plist)?
Embed      <- 'embed' !nc s? String s? String
# P1145-P1158: `global a b`, `ground (0 gnd)`.
Global     <- 'global' !nc s? nodes
Ground     <- 'ground' !nc s? nodes
nodes      <- '(' s? Node (s? Node)* s? ')' / Node (s? Node)*

# P1085: `parameters a=1 b=a*2`, every one with a value.
Parameters <- 'parameters' !nc s? ('(' s? ParamDecl (s? ParamDecl)* s? ')' / ParamDecl (s? ParamDecl)*)
ParamDecl  <- Key s? '=' s? Expr

# P1067: `model name master [params]`.
Model      <- 'model' !nc s? Name s? Master (s? plist)?

# P324-P457: `subckt name (ports)` ... `ends [name]`; the parentheses are
# required; definitions nest.
Subckt     <- 'subckt' !nc s? Name s? '(' s? (Node (s? Node)*)? s? ')' nl sline* Ends
sline      <- (Include / Parameters / Model / Subckt / AtIf / Instance)? nl
Ends       <- 'ends' !nc (s? Name)?

# P462-P509: `@if c` ... [`@elseif c` ...]* [`@else` ...] `@end`; branches
# hold instances, models and nested blocks.
AtIf       <- '@if' !nc s? Expr nl cline* AtElseIf* AtElse? '@end' !nc
AtElseIf   <- '@elseif' !nc s? Expr nl cline*
AtElse     <- '@else' !nc s? nl cline*
cline      <- (Include / Model / AtIf / Instance)? nl

# P1267-P1296: `control` ... `endc`; `analysis`, `sweep`, `save` are
# keywords only here (L811-L868).
ControlBlock <- 'control' !nc s? nl ((Include / Analysis / Save / Control)? nl)* 'endc' !nc
# P1174-P1217: sweeps on the lines before an analysis belong to it.
Analysis   <- (Sweep s? nl)* 'analysis' !nc s? Name s Kind (s? plist)?
Sweep      <- 'sweep' !nc s? Name (s? plist)?
Kind       <- [a-zA-Z_$] nc*
# P1095-P1143: `save v(out) i(v1) p('x1:m1', id) default`.
Save       <- 'save' !nc s? ('(' s? sigs s? ')' / sigs)
sigs       <- Signal (s? Signal)*
Signal     <- Name (s? '(' s? sarg (s? ',' s? sarg)? s? ')')?
sarg       <- String / Node
# P1229-P1265: any other command: `name [words] [(args)] [params]`
# (options, var, print, postprocess, alter, elaborate, clear, abort, mc...).
Control    <- !ckw Kind (s Name !(s? '='))* (s? '(' s? Expr (s? ',' s? Expr)* s? ')')? (s? plist)?
ckw        <- ('analysis' / 'sweep' / 'save' / 'endc') !nc

# P856-P1065: `name (nodes) master [params]`; with no master it is a
# behavioral source `name (p n) v=expr [, params]`.
Instance   <- !kw Name s? '(' s? (Node (s? Node)*)? s? ')' (s? Master !(s? '='))? (s? plist)? (s? ',' s? plist)?
kw         <- ('include' / 'section' / 'endsection' / 'load' / 'embed' / 'global' / 'ground'
            / 'parameters' / 'model' / 'subckt' / 'ends' / 'control' / 'endc') !nc

# P848: a parameter list, or one in parentheses; no commas.
plist      <- '(' s? Param (s? Param)* s? ')' / Param (s? Param)*
Param      <- Key s? '=' s? Expr

# P216-P230, docs/expr-operators.md: `?:` (right) < `||` < `&&` < `|` < `^`
# < `&` < `== !=` < `< > <= >=` < `<< >>` < `+ -` < `* /` < `**` (right)
# < unary `- + ! ~` < `[]` selection.
Expr       <- Ternary
Ternary    <- Or (s? '?' s? Ternary s? ':' s? Ternary)?
Or         <- And (s? OrOp s? And)*
And        <- BitOr (s? AndOp s? BitOr)*
BitOr      <- BitXor (s? BitOrOp s? BitXor)*
BitXor     <- BitAnd (s? XorOp s? BitAnd)*
BitAnd     <- Eq (s? BitAndOp s? Eq)*
Eq         <- Rel (s? EqOp s? Rel)*
Rel        <- Shift (s? RelOp s? Shift)*
Shift      <- Add (s? ShiftOp s? Add)*
Add        <- Mul (s? AddOp s? Mul)*
Mul        <- Pow (s? MulOp s? Pow)*
Pow        <- Unary (s? PowOp s? Pow)?
Unary      <- SignOp s? Unary / postfix
postfix    <- Select / primary
Select     <- primary (s? '[' s? Expr s? ']')+
primary    <- Paren / String / Vector / List / Call / Num / Ref
Paren      <- '(' s? Expr s? ')'
Call       <- Ref s? '(' s? (Expr (s? ',' s? Expr)*)? s? ')'
Ref        <- Name
# P740-P781: `[a, b]` vector, `{a, b}` list; `[]` `[,]` `{}` `{,}` empty.
Vector     <- '[' s? (',' / Expr (s? ',' s? Expr)*)? s? ']'
List       <- '{' s? (',' / Expr (s? ',' s? Expr)*)? s? '}'
OrOp       <- '||'
AndOp      <- '&&'
BitOrOp    <- '|' !'|'
XorOp      <- '^'
BitAndOp   <- '&' !'&'
EqOp       <- '==' / '!='
RelOp      <- '<=' / '>=' / '<' !'<' / '>' !'>'
ShiftOp    <- '<<' / '>>'
AddOp      <- [+\-]
MulOp      <- '*' !'*' / '/'
PowOp      <- '**'
SignOp     <- [+\-!~] !'='

# L594-L693: integer, 0x hex, float, SI prefix then letters (`1kOhm`),
# case-blind inf/nan. `M` `x` `X` `meg` are mega, `m` milli, `mil` 25.4u.
# A letter right after an exponent or plain number is an error (`1e3k`, `1V`).
Num        <- ('0' [xX] [0-9a-fA-F]+ / Mantissa (Exponent / Scale [a-zA-Z_]*)? / 'inf'i / 'nan'i) !nc
Mantissa   <- [0-9]+ ('.' [0-9]*)? / '.' [0-9]+
Exponent   <- [eE] [+\-]? [0-9]+
Scale      <- 'meg' / 'mil' / [munpfakKMGTxX]

# L869: letters, digits, `_`, `$`, not starting with a digit; `'...'`
# parts take any non-space bytes (`'x1:out'`, `'v1:flow(br)'`).
Name       <- (namestart / sq) (nc+ / sq)*
namestart  <- [a-zA-Z_$]
sq         <- "'" ("''" / ![' \t\r\n] .)* "'"
nc         <- [0-9a-zA-Z_$]
# P511: a node is an identifier or a non-negative integer.
Node       <- Name / '0' !nc / [1-9] [0-9]* !nc
Master     <- Name
Key        <- Name
String     <- '"' ('\\' !nl . / !'"' !nl .)* '"'
            / '<<<' [a-zA-Z0-9_]+ [ \t\r]* nl (!('>>>' [a-zA-Z0-9_]) .)* '>>>' [a-zA-Z0-9_]+
s          <- [ \t\r]+
nl         <- '\n'
```

**Rule reuse against `spectre.peg`.**
- **Same text:** `Expr Ternary Or Eq Add Pow Paren OrOp AndOp EqOp RelOp AddOp PowOp Mantissa Exponent Master Key
  nl`. `Call` and `Mul` have the same text but read through a different `Ref` and `MulOp` (no `.` in names, no `%`).
- **Same name, other body:** `And`/`Rel` (bitwise and shift levels in between), `Unary`/`SignOp` (`~`, postfix
  `Select`), `primary` (adds `Vector`, `List`), `Vector` (commas), `Num`/`Scale`, `Name`/`Node`, `Include`
  (`lang=`), `Section`, `Parameters`/`ParamDecl`, `Model`, `Subckt`, `Instance`, `Analysis` (keyword-led), `Control`
  (generic command), `Save`/`Signal`.
- **New captures:** `VDeck VLib Lang Load Embed Ground AtIf AtElseIf AtElse ControlBlock Sweep BitOr BitXor BitAnd
  Shift Select List`.

So one lowering serves both: Spectre's `Instance`/`Model`/`Subckt`/`Parameters`/`Analysis`/`Save`/`Include` rows mean
the same in VACASK. It differs only in the master tables and the `ControlBlock`/`Sweep` grouping.

**Line at a time.** Like `spectre.peg`, `vacask.peg`'s `Subckt`, `AtIf` and `ControlBlock` span lines. SpiceRack
needs a VACASK `OneStmt` in the shape of §1.5: `SubcktHead`, `Ends`, `ControlHead` (`control`), `Endc`, `AtIf`/
`AtElseIf`/`AtElse`/`AtEnd` as single-line rows, and `Sweep` read on its own and attached to the next `Analysis`
by the lowering. This is not yet written (unverified).

### 2.4 Differences the lowering must handle
1. **No `dc` analysis.** `sweep … instance=X parameter="dc"` lines (or no parameter, which sweeps the principal
   one, `cmd-sweep.md:25`) followed by `analysis … op` become `Analysis::Dc{sweeps}`. Other sweep targets
   (`variable=`, `option=`, `model=`), and sweeps over other analyses, go to `step_params`/`SpectreSweep`
   (`ir/mod.rs:186,216`). A sweep binds only to the next analysis (`cmd-sweep.md:55`). Lin `points` counts
   intervals, not points.
2. **Analyses only inside `control`.** Options, saves and `var` accumulate in order (`cmd-overview.md:15-23`).
   `var` is a circuit variable, not a `.param`.
3. **Analysis spellings.** ac/noise use `from to mode points`; noise uses `out=` and `in=`; `dcxf` maps to `Tf`;
   tran ic/nodeset are analysis parameters, which map to `initial_conditions`/`node_sets`. `dcinc acxf acstb pss
   pac hbac hbnoise` have no IR form, or only Spectre-named ones.
4. **Masters are indirect.** Instance master → `model` → device module. The `Component` variant follows the module:
   - builtins `vsource isource vcvs vccs ccvs cccs mutual`, plus `resistor capacitor inductor` and the `sp_*` set;
   - `type=1/-1` gives npn/pnp, n/p (`vacask.rs:167-176,532`);
   - model lookup is scoped, subckt first (`cir-masters.md:47`);
   - an unknown OSDI master stays raw.
5. **Sources.** `type="…"` with named keys; `exp td2` is a duration where SPICE's TD2 is absolute (the codegen
   converts it, `vacask.rs:282`); `wave=[t,v,…]` → `Pwl`; `am`/`fm` → `Am`/`Sffm`; `ccvs`/`cccs` `ctlinst="v1"` →
   `vsense`; `mutual ind1= ind2= k=` → `MutualInductor`. Key names differ between revisions (`tdphase` vs
   `sinephase`).
6. **Numbers and names.**
   - `M`, `x` and `X` are 1e6, so a Spectre/VACASK number reader is needed, not `import.rs::num`.
   - Names are case-sensitive.
   - `'x1:m1'` is a quoted hierarchical name.
   - `$mfactor` ↔ SPICE `m` (inverse of `vacask.rs:231`).
   - Expressions: `$temp` ↔ `temper`; `^` is XOR; `-a**b` = `(-a)**b`; integer division.
7. **Behavioural** `name (p n) v=expr` / `i=expr` → `Behavioral{Voltage,Current}`.
8. **`@if`** has no IR form: evaluate it or keep it raw. `ground a b` ties nodes to 0.
9. **Raw or ignored:** `embed`, `postprocess`, `print`, `alter`, `elaborate`, `mc … endmc`.
   `include … lang=` maps to a `ModelLibrary`, or to a nested import in that dialect.
10. **Codegen issues this research found:**
    - `ic=["n"; v]` (`vacask.rs:332-337`) is bcd48e2-only; `9ca4982` wants `{"n", v}` (`V/test/test_tranic.sim:21`).
    - Model text such as `version=3.3.0`, SPICE expressions and illegal names reach the output.
    - `vacask.rs:444` refuses a nonzero tran `start`, while upstream documents `start=` (`cmd-analysis-tran.md:34`;
      bcd48e2 not checked, unverified).

### 2.5 Corpora
- **Native decks:** `V/test/*.sim` 114 (run by ctest from `test/CMakeLists.txt`; expected failures in
  `test/check_*.cmake`), `V/demo/**/*.sim` 91 (26 dirs), `V/benchmark/*/vacask*/runme.sim` 13, `.inc` 29.
- **Foreign decks:** `test/*.cir` 28 and `*.scs` 6 are `lang=` includes. `benchmark/*/{ngspice*,xyce,gnucap}` hold
  24 decks of the same circuits, for cross-simulator checks.
- **The nix binary (bcd48e2) on the 9ca4982 decks** (`vacask -qp -sp --no-output`, 30 s; `$S/vacask_agent/runall/res2.txt`):
  test 31/114, demo 58/91, benchmark 9/13. 61 of the failures are syntax newer than the binary (`lang=` 37, `{}`
  lists and quoted identifiers 16, behavioural syntax 8); the rest are newer devices or analyses. **Numeric
  checks against current upstream syntax need the flake bumped past `9ca4982`.** The nix store's own bcd48e2
  copies, `share/doc/vacask/{test,demo}` (40 tests), match the binary.


## 3. Xyce

### 3.1 Sources and method
- **RG** = *Xyce Reference Guide 7.9*, `$S/xyce_rg_7.9.pdf`, cited by printed page.
- The 7.10 RG (https://xyce.sandia.gov/download/2068/, 888 pp.) was diffed against 7.9. Chapters 2.1-2.3 differ
  only in the `.SENS objfunc=` notation. From chapter 3 on, 7.10's pages are 2 lower (for example §3.1 is p.835
  in 7.10 and p.837 in 7.9).
- The 7.10 Release Notes (https://xyce.sandia.gov/download/1652/, p.2) add one syntax change: `TABLE` data is
  sorted automatically.
- **run** = checked on Xyce 7.10.0, built from nixpkgs (`nix eval nixpkgs#xyce.version` → `"7.10.0"`; binary
  `/nix/store/7glhfffprbvfz11bi9pd1fr5np8nw5df-xyce-7.10.0/bin/Xyce`), with test decks in `$S/xyce_agent/t/`.

### 3.2 Syntax reference
**Lines** (RG §2.1.39 p.163, §2.1.6 p.33; every row also checked by run)

| item | Xyce |
|---|---|
| title | line 1 is always the title; a card there is not read (run) |
| `.END` | ends input: nothing after it is read (run), unlike ngspice (`ngspice_lines.peg:261-263`); optional |
| `*` | comment line (§2.1.39.1) |
| leading whitespace | **a comment unless followed by `+`** (RG p.163; run: an indented `R2 1 0 1k` is ignored, `   + …` continues). 13 regression decks rely on it |
| `;` | inline comment anywhere, even inside `'…'`/`{…}` (§2.1.39.2; run: `'2;3'` fails) |
| `$ // # \` | **not** comments and no `\` continuation (run: `R1 1 0 1k $ x` → "Unrecognized parameter $") |
| `+` | continues, across blank and `*` lines (run) |
| case | insensitive; output names are uppercased (run); CRLF is accepted, form feed is an error (run) |

**Numbers** (the RG has no table; run):
- Scale factors: `T G MEG X(=1e6) K MIL M U N P F`. **No atto**: `1a` = 1 (RG §3.1 p.837, Table 6-2 p.852).
- Unit tails are ignored, and exponent plus scale is allowed (`1e3k` = 1e6).
- A `D` exponent is refused.

**Expressions** (RG §2.2 p.164):
- Written as `{…}` or `'…'`. `.param`/`.global_param` also take unbraced expressions.
- `:` is a name byte (`X1:R`), so `a?b:c` needs blanks (p.165).
- Table 2-32 lists the operators but **not their precedence**. Measured on 7.10 (run), loosest first:

| level | operators | evidence |
|---|---|---|
| 1 | `?:` right | `0?1:0?2:3` = 3 |
| 2 | `\| & \|\| &&`, one level, left | `1\|0&0` = 0 |
| 3 | `^` = **XOR** (Table 2-32 "boolean XOR") | `2^3` = 0, `1+1^1` = 0 |
| 4 | prefix `~` (NOT) over a whole relation | `~2==1` = 1 |
| 5 | `== != <> < <= > >=`, one level | `3>2>1` = 0 |
| 6 | `+ -` | |
| 7 | `%`, looser than `*` (fmod) | `7%4*3` = 7 |
| 8 | `* /` | |
| 9 | unary `+ -` | `-2**2` = -4 |
| 10 | `**` right | `2**3**2` = 512 |

- `!` is refused; `!=` works.
- **`LOG` is base 10** (p.169, Table 6-1 p.849; run `log(100)` = 2), `LN` is natural. ngspice's `log` is
  natural, so it needs rewriting.
- Other functions: Table 2-33 pp.166-170 (`IF LIMIT TABLE … DDT SDT STP URAMP …`). Special variables `TIME FREQ
  TEMP VT` (p.172).

**Cards**

| card | form (RG) |
|---|---|
| `.PARAM` | `n[(args)]=v [[,] n=v]*`, commas allowed (p.120; run); last definition wins (§3.2 p.837) |
| `.GLOBAL_PARAM` | as `.PARAM`, may depend on TIME, not inside a subckt (p.45, pp.171-172) |
| `.FUNC` | `name(args) [=] {body}` (p.42) |
| `.SUBCKT` | `name [nodes] [PARAMS:] n=v` with `PARAMS:` optional; X lines likewise (p.159, Table 2-35 p.178; run); nests and scopes models/params/funcs (p.160) |
| `.MODEL` | `name type [(]n=v[)]`, all params in parens or none (p.89, §6.1.10 p.847); `LEVEL` (p.90); binning `name.1` … (§2.1.21.2 p.90) |
| devices | Table 2-35 pp.177-178: B C D E F G H I J K L M O P Q R S T U V W X Z, plus **`Y<type> <name> …`** (YPDE, YACC, YLIN, YMEMRISTOR); `$G…` and `.GLOBAL` nodes are global (§2.3.1.1 p.182); `[ ]` are name bytes (§2.3.2 p.184) |
| B/E/G | `B … V={}`/`I={}` (p.238); E: gain, `VALUE=` (`VOL=` alias), `TABLE {e} = (x,y)…`, `POLY(n)` (p.232-233); no LAPLACE/FREQ (§6.1.5 p.845) |
| V/I | `[[DC] v] [AC m [ph]] [tran]`, with PULSE SIN (6th arg PHASE) EXP PAT PWL SFFM (p.226); `PWL t v …`, `PWL FILE "f" [TD=] [R=]` (pp.229-230) |
| `.DC` | `[LIN\|DEC\|OCT] var a b s …`, `var LIST …`, `DATA=t`; var = source, param, TEMP or `dev:param`; **no `PARAM` keyword** (pp.27-28, §6.1.15 p.851; run) |
| `.TRAN` | `tstep tstop [tstart [dtmax]] [NOOP\|UIC] [{schedule(…)}]`; tstep is the *initial* step (pp.161-162). UIC behaves as NOOP (run), but Table 6-1 p.850 calls it "ignored" — the RG contradicts itself |
| `.AC` `.NOISE` | p.24; `V(o[,r]) src sweep` (p.93) |
| `.STEP` | `[LIN] p a b c`, `DEC/OCT`, `LIST`, `DATA=`; **no `PARAM`** (run); model params as `RMOD:R` (pp.155-158) |
| other | `.DATA` (p.26), `.HB` (p.47), `.SAMPLING` (p.150), `.EMBEDDEDSAMPLING` (p.30), `.PCE` (p.122), `.SENS` (p.153), `.FOUR` (p.40), `.FFT` (p.35), `.LIN` (p.52) |
| `.OPTIONS` | **`.OPTIONS <pkg> n=v`** (p.96): DEVICE TIMEINT NONLIN NONLIN-TRAN LINSOL OUTPUT PARSER MEASURE …; top level only (p.97). A bare `.options reltol=` is warned about and **ignored** (run) |
| includes | `.INC`/`.INCLUDE`/`.INCL` (p.49); `.LIB file entry` and `.LIB entry … .ENDL` (pp.50-51) |
| `.PRINT` | `.PRINT <TRAN\|DC\|AC\|NOISE\|…> [FORMAT=] [FILE=] vars` (pp.128-129). Output is only what is printed, or `-r` (§3.5) |
| `.MEASURE` | TRAN/DC/AC/NOISE (+ `_CONT`), written to `.mt0/.ms0/.ma0` (pp.54, 65, 78; run) |
| `.IC` `.DCVOLT` `.NODESET` | `V(n)=v` or `n v` (pp.48, 92) |
| `.PREPROCESS` | `REPLACEGROUND TRUE` makes GND/GND!/GROUND equal 0; **off by default** (p.125; run); `REMOVEUNUSED`, `ADDRESISTORS` (pp.126-127) |
| `.SAVE` | **saves the operating point** (`TYPE=IC\|NODESET FILE=`, p.152); an ngspice `.save v(1)` is an error (run) |
| `.TEMP` | ignored (Table 6-1 p.850; run); use `.OPTIONS DEVICE TEMP=` (Table 2-3 p.98) |
| unsupported | `.PROBE .PLOT .PZ .DISTO .TF .MC .TEMP .control` (Table 6-1 pp.849-850) |

### 3.3 Draft grammar
Files: `$S/xyce_agent/h/grammar/xyce_lines.peg` (29 lines) and `xyce.peg` (62 lines). `xyce.peg` fills the same
holes as `ngspice.peg`, with a rebuilt expression ladder: it drops `And`, `AndOp` and `numtok` and adds `Xor`,
`XorOp`, `Mod`, `ModOp` and `Not`.

```peg
# xyce_lines.peg
File       <- Title (eol body)? !.
Title      <- (!eol .)*
body       <- (end / Line / skip)*
end        <- '.end'i ![^ \t\r\n] .*                       # RG: input ends here
skip       <- blank* eol / blank+ !. / comment_ln / blank* '+' (!eol .)* eolx
comment_ln <- ('*' / blank+ !'+') (!eol .)* eolx            # RG p.163
Line       <- !blank !'*' !'+' !end Piece cmt? more* eolx
more       <- eol (blank* eol / comment_ln)* blank* '+' Piece cmt?
Piece      <- (!';' !eol .)*
cmt        <- ';' (!eol .)*
blank      <- [ \t]
eol        <- '\r'? '\n'
eolx       <- eol / !.
```

```peg
# xyce.peg: the holes spice_cards.peg leaves
Number   <- [+\-]? Mantissa Exponent? Scale? [a-z0-9_]*
Mantissa <- [0-9]+ ('.' [0-9]*)? / '.' [0-9]+
Exponent <- [e] [+\-]? [0-9]+                      # no D exponent
Scale    <- 'meg' / 'mil' / [tgxkmunpf]            # X = 1e6, no atto
Expr     <- '{' x Ternary x '}' / "'" x Ternary x "'"
pairexpr <- &([a-zA-Z_.0-9(+\-~$]) Ternary !wordchar
Ternary  <- Or (x '?' x Ternary x ':' x Ternary)?
Or       <- Xor (x OrOp x Xor)*
Xor      <- Cmp (x XorOp x Cmp)*
Cmp      <- Add (x CmpOp x Add)*
Add      <- Mod (x AddOp x Mod)*
Mod      <- Mul (x ModOp x Mul)*
Mul      <- Unary (x MulOp x Unary)*
Unary    <- SignOp x Unary / NotOp x Not / Pow
Not      <- NotOp x Not / Cmp
Pow      <- atom (x PowOp x Unary)?                # right-assoc, tighter than unary -
atom     <- Paren / Probe / Call / Num / Ref / Str # Str: table("file")
Paren    <- '(' x Ternary x ')'
Probe    <- ([vi] [rimpdbcgse]* / [npw]) x '(' x Pname (x ',' x Pname)? x ')'
Pname    <- (![ \t,()\n] .)+
Call     <- Ref x '(' x (Ternary (x ',' x Ternary)*)? x ')'
Num      <- Mantissa ([e] [+\-]? [0-9]+)? [a-z]*
Ref      <- [a-zA-Z_$] [a-zA-Z0-9_.$:]*           # ':' is hierarchy (p.165)
x        <- [ \t]*
OrOp     <- '||' / '|' / '&&' / '&'
XorOp    <- '^'
ModOp    <- '%'
PowOp    <- '**'
CmpOp    <- '==' / '!=' / '<>' / '<=' / '>=' / '<' / '>'
AddOp    <- [+\-]
MulOp    <- '*' !'*' / '/'
SignOp   <- [+\-]
NotOp    <- '~'
namechar <- ![ \t\n,=()'"{}] .
wordchar <- ![ \t\n,=()'"{}] .                     # [ ] are name bytes (p.184)
field    <- Pair / Group / Expr / Str / Word / [,()=]
gfield   <- Group / Expr / Str / Word / [,=]
value    <- Expr / Str / pairexpr / Word / blob
blob     <- '{' (blob / !'}' .)* '}'               # composite {name=..}, {POLY(2)..}
```

Ten rules are identical to `ngspice.peg`: `Mantissa Paren Pname Call x AddOp SignOp namechar field gfield`. They
could move to a shared leaves file. Today each dialect file has to be complete, because a duplicate rule is an
error (`src/peg.rs:69`).

**Acceptance.** Harness: `$S/xyce_agent/h/src/main.rs`, which mimics `logical_lines` and `card_tokens`
(`import.rs:54-115`). It runs over all 4,070 `Netlists/**/*.cir` (252,976 logical lines, 0 line-grammar failures):

| composition | accepted | refused |
|---|---|---|
| xyce_lines + xyce.peg | 99.7% | 806 |
| ngspice_lines + ngspice.peg (baseline) | 99.4% | 1,460 (indented comment lines read as cards, `&` in `.func`) |
| xyce + 3 edits to a scratch copy of `spice_cards.peg` | **99.98%** | 57 (mostly deliberate error decks: `v2"`, `.tr`) |
| circuitsim90's 43 Xyce decks | 100% | 0 |

Refusals without the edits: `.global_param` 445, `.sampling` 107, `.embeddedsampling` 67, `.result` 31, `.mpde`
27, `.pce` 22, `.preprocess` 20, `.initcond` 10, `.mor` 9, `.param a=1, b=2` 4. The three edits are in shared
upstream rules:

1. `Param` (`spice_cards.peg:64`) gains `'.global_param'` and comma separators.
2. `OtherKind` (`:109-112`) gains `sampling embeddedsampling pce mpde result preprocess initcond mor rol dcop
   objective`; the analyses among them are better placed in `Kind`.
3. `.incl` joins `include.peg` and `import.rs::include_line` (`import.rs:123-137`).

Edits 1 and 2 belong in ESPice, or in a dialect hole for `OtherKind` (DI §6 already notes this).

### 3.4 Differences the lowering must handle
- Line layer: indented lines are comments, `.END` stops input, `;` is the only inline comment.
- `^` is XOR and `**` is power, so `^` must not be read as ngspice power. `~` is NOT, `|`/`&` are logical, and `%`
  sits between `*` and `+`. **`log` is log10.**
- `.OPTIONS <pkg> k=v`: `control_card` flattens every word into `portable` (`import.rs:560-565`), which turns the
  package (`timeint`) into a flag. Keep `(pkg, key)`, e.g. `backend_specific["xyce"]` with key `timeint.reltol`.
- `.PRINT` is output selection: read it into `saves`. `.SAVE` means something else (p.152), so it must never be
  read into `saves`.
- `.STEP`/`.DC` have no `PARAM` keyword and take `dev:param`, `LIST`, `DATA=` and `TEMP`.
- `.GLOBAL_PARAM`; funcs as `.PARAM f(x)={…}`; optional `PARAMS:` (already handled by `ports`,
  `spice_cards.peg:27`).
- **Y devices**: the element name is the first two tokens (`YPDE d1 …`); `element()` must not take `ypde` as the
  name.
- B `V=/I=`; E/G `VALUE=/VOL=/TABLE/POLY`.
- Ground: only `0`, unless `.PREPROCESS REPLACEGROUND TRUE`. `$G*` nodes are global.
- Numbers: `X` = 1e6, `a` is not atto, no `D` exponent.
- `.TEMP` is ignored. Read `.OPTIONS DEVICE TEMP=/TNOM=` into `temperature`/`nominal_temperature`.

### 3.5 A minimal Xyce codegen (beyond `spice3.rs`'s ngspice output)
`codegen::Dialect` has no Xyce variant (`src/codegen/mod.rs:22-33`). A `Dialect::Xyce` branch in `spice3.rs`
needs:

1. **Output.** Run `Xyce -r out.raw [-a] deck.cir`. It writes a SPICE3 rawfile with every variable and every
   plot, with no `.PRINT` needed (RG p.134; ch. 3 p.835 lists `-r/-a`; ch. 8 pp.859-860). On 7.10 a deck without
   `.PRINT` only warns. `spicerack::rawfile::parse_raw_all` (`src/rawfile.rs:28`) reads Xyce's real and complex
   rawfiles unchanged (run, `$S/xyce_agent/h/src/bin/raw.rs`). The names are uppercase (`V(1)`, `V1#branch`,
   `FREQUENCY`), so compare case-blind. The alternative is `.PRINT <type> FORMAT=RAW FILE=` per analysis
   (pp.128-129); `.DC` output omits the sweep variable unless it is printed (Table 6-2 p.851).
2. **One analysis per deck.** With `.dc` + `.tran` or `.dc` + `.ac` in one deck, only the last analysis ran,
   silently (run; no RG statement found). The codegen, or the test harness, must split testbenches.
3. No `.control`/`pre_osdi` (Table 6-1 p.850); Verilog-A goes through `-plugin` (ch. 3).
4. No `.save`: drop it and rely on item 1.
5. **Options** through Tables 2-3/2-5/2-6 (pp.97-104):
   - `reltol` and `abstol` → `.OPTIONS TIMEINT RELTOL= ABSTOL=`; Xyce's `ABSTOL` means something else (p.844).
   - `vntol` → `NONLIN ABSTOL=` and `NONLIN-TRAN ABSTOL=`.
   - `gmin`, `tnom` and `temp` → `.OPTIONS DEVICE …`.
   - `itl1` → `NONLIN MAXSTEP=`, `itl4` → `NONLIN-TRAN MAXSTEP=` (p.844).
   - `.temp T` → `.OPTIONS DEVICE TEMP=T`.
   - To get ngspice-comparable time points: `.OPTIONS OUTPUT INITIAL_INTERVAL=<tstep>` (p.110; run).
6. **`.step`**: drop `param` (p.155). The ngspice codegen refuses `.step`; the LTspice one writes `param`
   (`spice3.rs:191-206`).
7. **Expressions**: `{…}`, `**` for power, `ln` stays `ln`, and an ngspice `log` becomes `ln`. Never write `!`;
   use `~`. This needs a parsed expression, not today's verbatim strings (DI §4a last row).
8. **Model LEVELs** (Table 2-36 p.180, Table 6-2 p.852; run `Xyce -param`):

   | model | ngspice level | Xyce level |
   |---|---|---|
   | BSIM3 | 8, 49 | **9**. 49 is accepted on 7.10 (not in the RG); **8 is rejected** ("no valid model card") |
   | BSIM4 | 14, 54 | 14, 54 |
   | BSIM-SOI | 10, 58 | 10 (57 as an alias) |
   | others | | VBIC Q 11/12, MEXTRAM Q 504/505, HICUM Q 230/234, PSP 103, BSIM-CMG 107/110 |

   The switch model differs (p.852).
9. **Sources**: PULSE, SIN, EXP and SFFM are as in ngspice. SIN takes a 6th argument, PHASE (p.227). PWL is
   written flat, `PWL t v …` (pp.229-230).
10. **Unchanged:** `.meas` (results in `.mt0`, run), `.ic V(n)=v`, `.nodeset` and `.end`.

Numeric check on an RC pulse: `v(2)` at 1 µs is 6.319164e-01 in Xyce and 6.319360e-01 in ngspice, a relative
difference of 3e-5 (run). Use rtol ≈ 1e-4, or match TIMEINT RELTOL/ABSTOL.

For a **fixpoint test** (Xyce text → IR → Xyce text → IR), the codegen must at least: write `.OPTIONS <pkg>` from
the `(pkg, key)` entries, write `.PRINT` from `saves` per analysis type, write `.step` without `param`, and spell
`^`/`**`/`log` consistently. Steps 1-2 are needed for the numeric check, not for the fixpoint.

### 3.6 Corpora
| path | decks | gold outputs |
|---|---|---|
| `/home/omare/Documents/Projects/VAMS-Conformance-Suite/external/Xyce_Regression` (HEAD `59a5f3f`, 7.9 release) | 4,070 `.cir` under `Netlists/` (268 dirs) | **yes**: `OutputData/<DIR>/<deck>.cir.prn` (1,367 `.prn`, 49 `.raw`, 66 `.mt0`, 67 `.ma0`, 45 `.ms0`); 2,121 `*.cir.sh` per-test scripts; comparators in `TestScripts/` (`xyce_verify`, `check_prn.sh`) |
| `$S/suites/circuitsim90/src/Xyce_Regression` | 43 (`Netlists/CircuitSim90`) | none |

Use in the corpus: 3,783 decks have `.print` (tran 2,072, dc 1,285, ac 259, sens 242, hb 115, noise 98);
`.options` packages timeint 682, device 634, nonlin 263, output 198, nonlin-tran 185.


## 4. LTspice

### 4.1 Sources
- **ADI** = https://github.com/analogdevicesinc/ltspice-reference @ `b4358f8` (2026-09-16, Apache-2.0, "version
  24+"), published at https://analogdevicesinc.github.io/ltspice-reference/. Three of its files are cited by line:
  **SSR** = `ai_ref/SPICE-SYNTAX-REFERENCE.md`, **SCR** = `ai_ref/SIMULATION-COMMANDS-REFERENCE.md`, **CER** =
  `ai_ref/CIRCUIT-ELEMENTS-REFERENCE.md`. Clone: `$S/lt_agent/ltspice-reference`.
- **XVII** = https://ltwiki.org/LTspiceHelpXVII/LTspiceHelp/html/<Page>.htm.
- **IV-GS** = https://ltwiki.org/LTspiceHelp/LTspiceHelp/A_General_Structure_and_Conventions.htm.
- There is no local LTspice binary. `~/.config/ltspice/LTspice.ini` shows that 26.0.2 was run here once, but no
  `.exe` exists, so nothing below was checked against the simulator.

### 4.2 Syntax reference
| topic | LTspice | source |
|---|---|---|
| title | line 1 is a comment | SSR:19, IV-GS |
| `.end` | optional, and **everything after it is ignored** (ngspice reads on, `ngspice_lines.peg:261-263`) | SSR:20, SCR:704-711 |
| comments | `*` first character; `;` anywhere to end of line. `$`, `//` and `#` are not documented as comments (unverified), and the corpus has 0 lines with `$` or `//` | SSR:55-56, 69-100 |
| continuation | leading `+` | SSR:57 |
| `@`, `&` first | FRA analyser device and FRA probe. `ngspice_lines.peg` reads `@` as end of file (`:246`) and `&` as a comment (`:254`) | SSR:97-98, CER:874,983 |
| case | ignored; Unicode case folding | IV-GS; SSR:47 |
| nodes | `0` and `GND` are ground; `0` ≠ `00`; `$G_*` is global | SSR:140-146, SCR:690-702 |
| encoding | UTF-8 (the 24+ netlister), UTF-16LE with or without BOM, Latin-1 when not valid UTF-8 | SSR:37-45 |
| numbers | `T G Meg K mil m u n p f` (`u` may be `µ`), **no `a`**; `M` = `m` = 1e-3; unit tails ignored (`4Farads` = 4e-15); **`6K34` = 6.34K** for every multiplier (the "Accept 3K4 as 3.4K" setting; local ini `Accept_3k4_Notation=true`) | SSR:104-136, IV-GS |
| braces | `{expr}` substitutes; `.param` values and B `V=`/`I=` take bare expressions; `.step list` and `file=` values must be plain numbers; `'expr'` is undocumented (unverified) | XVII DotParam.htm, CER:713-724, SCR:487-489 |
| operators | loosest first: `& \| ^` (Boolean AND/OR/**XOR**, one level), relational, `+ -`, `* / %`, `**`, unary `! ~` (tighter than `**`: `-2**1.5` = (-2)**1.5, real part). XVII has no `== != % ~`. `^` is `**` only inside a Laplace expression | CER:737-746, SCR:410, XVII B-device.htm |
| truth | true when x > 0.5 (`if`, `&`, `\|`, `!`, `buf`, `inv`); `u(x)` is 1 for x > 0 | XVII B-device.htm, DotParam.htm |
| `?:` `&&` `\|\|` | undocumented (unverified); 0 corpus uses | — |
| functions | `.param`: abs … if int inv limit ln log log10 … table u uramp (XVII DotParam.htm), ADI adds `mod select xor` (SCR:407); B sources add `absdelay ddt idt idtmod dnlim uplim white`; constants `pi BOLTZ ECHARGE PLANCK KELVIN GMIN` | XVII B-device.htm, SCR:402-407 |
| `.param` / `.params` | local inside a `.subckt`; passed symbolically | XVII DotParam.htm |
| `.func` | `name(args) {expr}`, dynamic scope | SCR:436-454, XVII DotFunc.htm |
| `.subckt` | `name ports [params: p=v]` … `.ends`; X lines have no `params:` keyword | SCR:612-636, XVII X-device.htm |
| `.model` | types D NPN PNP NJF PJF NMOS PMOS NMF PMF SW CSW VDMOS NIGBT PIGBT URC LTRA; `ako:` derivation | SCR:638-651 |
| R/C/L | R `tc=`; C `ic= Rser= Lser= Rpar= Cpar= m= RLshunt=`, `Q=` nonlinear; L `ic= Rser= Rpar= Cpar=`, **implicit 1 mΩ Rser** unless in a K statement, `Flux=`, hysteretic core | CER:54-157 |
| V/I | `V n+ n- val [AC=] [Rser=] [Cpar=]`; a V with Rser cannot sense for F/H/W. Waveforms: `PULSE(V1 V2 Td Tr Tf Ton Tper Ncycles)`, **`SINE(`**`Voff Vamp F Td Theta Phi Ncycles)`, `EXP SFFM PWL`, `PWL REPEAT …`, `PWL FILE=`, `wavefile=`, `Trigger=`; I also takes `load tbl= step( )` | CER:383-514, XVII V-device.htm |
| E/G/F/H/B | gain, `table=`, `Laplace=`, `value=`, `POLY`; B `V= I= R= P=` | CER:516-588, 709-735 |
| A, S/W, K, T/O/U | A: 8-terminal special functions (INV … MODULATE); `S … [on\|off]`; `K L1 L2 … k` couples all pairs | CER:590-818 |
| `.tran` | `Tstop` alone, or `Tstep Tstop [Tstart [dTmax]]`, with modifiers `uic steady nodiscard startup step …`; default dTmax = Tstop/1024 | SCR:58-95, 553 |
| other analyses | `.ac oct\|dec\|lin\|list\|file=`; `.dc` nests up to 3 deep; `.noise` (no points-per-summary); `.tf`, `.op`, `.four`, `.net`, `.fra` | SCR:97-281, 794-810 |
| `.step` | `[oct\|dec\|lin] item a b inc`, `list`, `file=`; item = **`param X`** (the keyword is required), a source, `temp`, or `NPN 2N2222(VAF)` | SCR:456-499 |
| `.meas` | `[TRAN\|AC\|DC\|NOISE] name …` | SCR:285-355 |
| `.options` | SPICE3 names plus `plotwinsize`(300) `numdgt` `measdgt` `gfloat` `topologycheck` `sstol` `ptrantau` `solver` `reject_number_tails` … | SCR:518-605 |
| includes | `.include/.inc` (URLs fetched); `.lib file`, `.lib file section`, `.lib section … .endl`. The netlister writes unquoted paths with blanks (2,658 corpus lines); how LTspice tells them from `file section` is unverified | SCR:653-688, XVII DotInclude.htm |
| `.save` | wildcards; everything is saved by default | SCR:357-369 |
| others | `.ic .nodeset .global`; `.temp` = `.step temp list`; `.backanno` (GUI only); `.savebias/.loadbias/.savestate/.loadstate`; `.wave`; `.machine … .endmachine` (a block); `.ferret`; `.softstart` (undocumented) | SCR:501-818, XVII Dot*.htm |

### 4.3 Draft grammar
Files: `$S/lt_agent/harness/grammar/ltspice_lines.peg` (39 lines) and `ltspice.peg` (71 lines), listed below.
`ltspice.peg` defines `ngspice.peg`'s holes minus `numtok`. Against `ngspice.peg` the bodies of `Number Scale
Exponent Expr pairexpr Unary atom Num OrOp AndOp CmpOp MulOp PowOp NotOp value` differ.

`spice_cards.peg` refuses `.backanno .savebias .loadbias .savestate .loadstate .wave .machine .endmachine .state
.rule .output .fra .ferret .softstart .fourier .endl` and `.params` (`spice_cards.peg:64,94-112`), and `@`/`&`
device lines (`Element` needs `&[a-zA-Z]`, `:117`). So the dialect file adds an entry rule `LtCard` that tries
those first and falls back to `OneCard`. The importer would look it up with
`cards.rule("LtCard").or(cards.rule("OneCard"))` (`src/import.rs:40`). This works around the shared-`OtherKind`
limit DI §6 notes without touching an upstream file.

```peg
# LTspice: physical lines -> the title and logical statements.
# Sources: ADI ltspice-reference ai_ref/SPICE-SYNTAX-REFERENCE.md (SSR),
# SIMULATION-COMMANDS-REFERENCE.md (SCR). Captures as ngspice_lines.peg:
# Title; Line made of Piece (joined with a blank).

File       <- Title (eol body)? !.

# SSR "Netlist Structure": line 1 is a comment (the title).
Title      <- (!eol .)*

body       <- (end / Line / skip)*

# SSR, SCR ".END": `.end` ends the netlist; everything after it is ignored.
end        <- blank* '.end'i ![a-zA-Z0-9_] .*

# SSR "Comments": `*` first non-blank starts a comment line; `;` anywhere
# ends the line. Blank lines go. A `+` with nothing to continue is dropped.
# `$`, `#`, `//` are not comments (undocumented); `@` and `&` start FRA
# devices (SSR "Line Types"), unlike ngspice_lines.peg.
skip       <- blank* [*;] (!eol .)* eolx
            / blank* eol
            / blank+ !.
            / blank* '+' (!eol .)* eolx

Line       <- blank* ![+*;] !end !eolx Piece comment? more* eolx
# SSR: `+` first continues the previous line; comment and blank lines
# between are skipped (unverified, as ngspice and HSPICE do).
more       <- eol skip_cont* blank* '+' Piece comment?
skip_cont  <- blank* [*;] (!eol .)* eol / blank* eol

Piece      <- chunk*
chunk      <- quoted / ![;\r\n] .
# A `;` inside double quotes (a path) is no comment (unverified).
quoted     <- '"' [^"\r\n]* '"'?
comment    <- ';' (!eol .)*

blank      <- [ \t\r]
eol        <- '\r'? '\n'
eolx       <- eol / !.
```

```peg
# LTspice: the dialect rules spice_cards.peg leaves open. Same rule set as
# ngspice.peg minus numtok; differs in Number/Scale, Expr, the operator
# ladder (Unary, AndOp, OrOp, CmpOp, MulOp, PowOp, NotOp) and value.
# Sources: ADI ltspice-reference ai_ref/ (SSR = SPICE-SYNTAX-REFERENCE.md,
# CER = CIRCUIT-ELEMENTS-REFERENCE.md, SCR = SIMULATION-COMMANDS-REFERENCE.md);
# LTspice XVII help B-device.htm, DotParam.htm.

# SSR "Engineering Multipliers": T G Meg K mil m u n p f; `u` or `µ`
# (XVII help); no `a`. Trailing letters are ignored (`10V`); `6K34` is
# 6.34K, so digits may follow the scale (the tail takes them).
Number     <- [+\-]? Mantissa Exponent? Scale? ([a-z0-9_.] / 'µ')*
Mantissa   <- [0-9]+ ('.' [0-9]*)? / '.' [0-9]+
Exponent   <- [e] [+\-]? [0-9]+
Scale      <- 'meg' / 'mil' / 'µ' / [tgkmunpf]

# DotParam.htm: `{expr}` invokes evaluation. Quotes are not expressions
# (undocumented); `"..."` is a string (SCR .PARAM).
Expr       <- '{' x Ternary x '}'
# B-device.htm `V=<expression>`: unbraced after `key=`.
pairexpr   <- &([a-zA-Z_.0-9(+\-!~]) Ternary !wordchar

# CER "Supported Operators (ascending precedence)": `& | ^` (one level,
# Boolean, true is > .5), comparisons, `+ -`, `* / %`, `**`, `! ~`.
# `?:` is undocumented (kept: spice_cards.peg's `cond` names Ternary).
Ternary    <- Or (x '?' x Ternary x ':' x Ternary)?
Or         <- And (x OrOp x And)*
And        <- Cmp (x AndOp x Cmp)*
Cmp        <- Add (x CmpOp x Add)*
Add        <- Mul (x AddOp x Mul)*
Mul        <- Pow (x MulOp x Pow)*
Pow        <- Unary (x PowOp x Unary)*
# B-device.htm: `-2**1.5` is 0, the real part of (-2)**1.5: the sign binds
# tighter than `**`, unlike ngspice (`-2^2` = -4).
Unary      <- (SignOp / NotOp) x Unary / atom
# SCR .PARAM: `select(n, "1N4148", "1N4007")` takes strings.
atom       <- Paren / Probe / Call / Num / Str / Ref
Paren      <- '(' x Ternary x ')'
Probe      <- [vi] x '(' x Pname (x ',' x Pname)? x ')'
Pname      <- (![ \t,()\n] .)+
Call       <- Ref x '(' x (Ternary (x ',' x Ternary)*)? x ')'
Num        <- Mantissa ([e] [+\-]? [0-9]+)? ([a-z0-9] / 'µ')*
Ref        <- [a-zA-Z_$] [a-zA-Z0-9_.$]*
x          <- [ \t]*

OrOp       <- [&|^]
# `&` shares the `|` level: this level never matches (`x` always does).
AndOp      <- !x
CmpOp      <- '==' / '!=' / '<=' / '>=' / '<' / '>'
AddOp      <- [+\-]
MulOp      <- '*' !'*' / [/%]
PowOp      <- '**'
SignOp     <- [+\-]
NotOp      <- [!~] !'='

namechar   <- ![ \t\n,=()'"{}] .
wordchar   <- ![ \t\n,=()'"{}\[\]] .

field      <- Pair / Group / Expr / Str / Word / [,()=]
gfield     <- Group / Expr / Str / Word / [,=]
value      <- Expr / Str / pairexpr / Word

# LTspice dot cards spice_cards.peg's Kind/OtherKind refuse, kept as text.
# Entry: the importer starts here when the rule exists.
LtCard     <- (LtOther / FraDevice) nl !. / OneCard
# SSR "Line Types": `@` is the FRA device, `&` its probe (CER).
FraDevice  <- [@&] Name fields
LtOther    <- '.params' s (Assign / FuncDef) (s (Assign / FuncDef))*
            / '.' LtKind !namechar rest
LtKind     <- 'backanno' / 'savebias' / 'loadbias' / 'savestate' / 'loadstate' / 'wave'
            / 'machine' / 'endmachine' / 'state' / 'rule' / 'output' / 'fra' / 'ferret'
            / 'softstart' / 'fourier' / 'endl'
```

**Acceptance.** Harness: `$S/lt_agent/harness`. It decodes UTF-16LE, then UTF-8, then Latin-1, mirrors
`logical_lines`, and sets aside include lines as `import.rs` does.

| corpus | files | line failures | cards accepted / refused |
|---|---|---|---|
| spice-datasets `ltspice_examples` | 3,257 | 0 | 49,859 / 0 |
| spice-datasets `ltspice_demos` | 1,085 | 0 | 24,456 / 0 |
| ADI doc `spice` blocks | 95 | 0 | 297 / 0 |
| SpiceRack LTspice output (2 hand decks) | 2 | 0 | 44 / 0 |
| baseline: ngspice grammars on examples / demos | — | 0 | 3,295 / 1,096 refused (`.backanno` 4,342, `.params` 19, `.softstart` 10, `µ` inside braces) |

**Caveat.** `field` falls back to `Word`, so a malformed expression (`v=a^^b`) still parses. Zero refusals is weak
evidence. Negative checks (`.bogus 1`, `R1 a b {1+}`) are refused.

### 4.4 Differences the lowering must handle
These are checked against SpiceRack's code.

1. **A one-argument `.lib file` swallows the rest of the deck.** `include.peg:35` wants two arguments, so
   `include_line` returns `None` (`import.rs:123-137`). The `None if word == ".lib"` arm then opens a section
   block that runs to `.endl` (`import.rs:701-706`). Checked: `t\n.lib LTC.lib\nR1 a 0 1k\n.tran 1u` gives 0
   components. This is the most common LTspice include: 4,605 one-argument `.lib` lines in the corpus. For
   LTspice, read a one-argument `.lib` as a library include, with the whole rest of the line as the path
   (unverified rule). Unquoted paths with blanks would otherwise split.
2. **`^` is XOR** (except inside Laplace, where it is `**`) and `**` is power. In the other direction, an ngspice
   `x^2` sent to LTspice must become `x**2`. Corpus: 0 uses of `^`, 23 of `**`.
3. **Unary minus binds tighter than `**`.**
4. **Truth means > 0.5.** Lower `if(c,a,b)` as `if(c>0.5,a,b)` when `c` is not a comparison.
5. **Numbers.** `num()` (`import.rs:194-231`) returns `None` for `6k34`, `1µ` and `10µF`, so they become `Raw`.
   `µ` is on 14,445 corpus lines. There is no `a` scale, so emit `1e-18` for atto.
6. **Waveforms.** `waveform()` knows `sin`, not `SINE` (`import.rs:270-275`); `SINE(` is on 655 corpus lines.
   PULSE and SINE take an extra `Ncycles` argument. `PWL REPEAT`, `FILE=`, `Trigger=` and `wavefile=` have no IR form.
7. **Parasitics.** V `Rser/Cpar` (180 corpus lines), C `Rser/Lser/Rpar/Cpar/RLshunt` (3,896), L `Rser/Rpar/Cpar`
   (2,099). Other simulators need these as explicit networks. The implicit 1 mΩ inductor Rser also needs one:
   strict equivalence means adding it on import and emitting `Rser=0` on export.
8. **B sources.** `V=`/`I=` map to ngspice B; `R=`/`P=` do not map. Laplace and table forms need `s_xfer`/`table`
   (the codegen writes `Laplace=`, `spice3.rs:95-100`, marked unverified there).
9. **`.tran`.** `control_card` needs `step stop` and knows only `uic` (`import.rs:519-535`), so `.tran Tstop` (1,322
   corpus lines) and modifiers (2,773) stay raw.
10. **`.step`** needs the `param` keyword. The codegen writes it (`spice3.rs:199-206`); the importer keeps it raw.
11. **Options** that exist only in LTspice should be dropped on export (SCR:518-605).
12. **Encoding.** 3,587 of 4,342 XVII `.net` files are Latin-1 (`µ` = 0xB5, `§` in `X§U1`). `import::parse` takes
    `&str`, so the caller must decode by BOM, then UTF-8, then Latin-1. Names hold `§` and `||`.
13. **Blocks and cards.** `.end` ends input. Drop `.backanno`. `.machine … .endmachine` goes into `BLOCKS`
    (`import.rs:604`). `.params` goes into the `.param` arm (`import.rs:743`).
14. **Codegen spot checks, unverified.** It emits `V1 in 0 0 AC 1 PULSE(…)`, while LTspice documents the
    waveform before `AC` (XVII V-device.htm). `.subckt … PARAMS:` matches SCR:615. `spice3.rs:302` drops `.save`
    for LTspice; that is right only for `.save all`.

### 4.5 Corpora
- **symbench/spice-datasets** (https://github.com/symbench/spice-datasets @ `a3a1c4f`, GPL-3.0; its README
  disclaims copyright, which stays with ADI). `ltspice_examples/` holds 3,257 `.net` files netlisted from the XVII
  examples folder; `ltspice_demos/` holds 1,085 ADI demo-circuit netlists. They use CRLF and are mostly Latin-1.
  Clone: `$S/lt_agent/spice-datasets`.
- **ADI ltspice-reference**: about 95 runnable `spice` blocks, including `.meas` decks in `ai_ref/MEAS-REFERENCE.md`
  (Apache-2.0).
- **nunobrum/spicelib and PyLTSpice** (GPL-3.0): `examples/testfiles/*.net`, plus golden edited copies in
  `unittests/golden/`.
- **The LTspice install** ships `.asc` examples under `%LOCALAPPDATA%\LTspice\examples\` (ADI `ai_ref/README.md:155-157`).
  Netlisting them needs LTspice, and redistribution was not checked.
- **No simulator** is available locally, so the fixpoint check (DI §5.2) is the only test. LTspice under wine is
  possible (unverified).


## 5. Ordered implementation plan, smallest first

1. **Close the silent changes in today's codegens** (§1.4, §2.4.10). Each is a few lines plus a unit test.
   - Spectre: refuse a `Dc` with more than one sweep (`spectre.rs:654-667`).
   - Spectre: write `uic` as `skipdc=yes` (`:684-697`).
   - Spectre: write `save i(X)` as `save x:p` (`:450`).
   - Spectre: refuse, or convert to plain floats, model parameter values carrying SPICE scale factors, and map
     model kinds to Spectre masters (ESPice table, `spectre.zig:433-451`) (`:287-293`).
   - Spectre and VACASK: escape or refuse names with bytes illegal in the target (UG p.62; VACASK L869).
   - VACASK: write `{"n", v}` lists, or pin the syntax to the flake's `bcd48e2` explicitly (`vacask.rs:332-337`).
2. **LTspice reader** (§4). It shares the SPICE lowering, so it needs `ltspice_lines.peg` + `ltspice.peg` in
   `grammar/` and a `Dialect::Ltspice` arm in `grammars()` (`import.rs:30-37`, entry `LtCard`), plus importer
   fixes the corpus shows:
   - a one-argument `.lib` (§4.4.1);
   - `SINE(`;
   - `6k34` and `µ` in `num`;
   - `.tran Tstop` and its modifiers;
   - `.params`;
   - `.machine` in `BLOCKS`;
   - byte decoding (UTF-16LE / UTF-8 / Latin-1).

   Test: fixpoint (DI §5.2) over `spice-datasets`, 4,342 decks. There is no simulator.
3. **Xyce reader** (§3.3-3.4). Add `xyce_lines.peg` + `xyce.peg`, and an `XyCard` entry rule in the
   `LtCard` style, so `.global_param`, `.sampling` etc. need no upstream edit. Then the importer:
   - `.options <pkg>` as `(pkg, key)` in `backend_specific["xyce"]`;
   - `.print` → `saves`, and `.save` kept raw;
   - Y-device names;
   - `.step`/`.dc` without `param`.

   Gate: ≥ 99.9% of the 252,976 regression lines read.
4. **Xyce codegen** (§3.5): a `Dialect::Xyce` arm in `spice3.rs` with
   - `.OPTIONS <pkg>`;
   - `.PRINT` per analysis type;
   - no `.save`/`.control`;
   - `.step` without `param`;
   - the BSIM3 level renumbering (8 → 9);
   - `.temp` → `.OPTIONS DEVICE TEMP=`.

   Tests:
   - fixpoint on Xyce_Regression;
   - numeric: run `Xyce -r` (nixpkgs 7.10, add it to `nix develop .#testing`) with one analysis per deck, read with
     `parse_raw_all`, case-blind names, rtol ≈ 1e-4 against ngspice;
   - gold `.prn` files in `OutputData/` for Xyce's own decks.
5. **Spectre reader** (§1.2, §1.5). Add `spectre_one.peg` to `grammar/` (outside the UPSTREAM set) and a
   `lower_spectre` over the capture table in §1.2, with ESPice's primitive/master/source tables and a Spectre
   number reader. `SpiceLine` regions go through the existing SPICE lowering with the
   `spice_cards`+`spectre_ops`+`hspice` composition.

   Tests: fixpoint on the 351 SpiceRack-emitted decks that parse today, and on the 119 valid ESPice fixtures.
   Numeric checks through ESPice, the only local Spectre reader (DI §5).
6. **VACASK reader** (§2.3-2.4). Reuse `lower_spectre`, with VACASK's master tables, `ControlBlock`/`Sweep`
   grouping (sweep + `op` → `Dc`), and a VACASK `OneStmt`. Test: fixpoint over the 218 native decks. Numeric:
   bump `nix/vacask.nix` past `9ca4982` first, since bcd48e2 runs only 98 of the 218 native decks.
7. **Parsed expressions in the IR** (DI §6 step 10). Only this fixes the cross-dialect spelling problems found
   in every section: `^` (power in ngspice/HSPICE, XOR in LTspice/Xyce/VACASK, absent in Spectre), `log` (log10
   in Xyce), `!` vs `~`, `{}` vs `'…'` vs bare, the truth threshold (> 0.5 in LTspice), and `-a**b`. Until then,
   each codegen must refuse an `Expression` it cannot prove dialect-neutral.

`ponytail:` skipped: a separate lowering per dialect (two families cover all six), and grammar edits in the
ESPice-owned files (entry-rule wrappers cover the extra keywords). Revisit both if ESPice adopts an `OtherKind`
hole.
