# SPICE Monte Carlo functions in Spectre and VACASK

Question: how should SpiceRack write SPICE's statistical functions (`agauss`, `gauss`,
`aunif`, `unif`, `limit`) to Spectre and VACASK so that codegen never refuses them, and how
should it read Spectre `statistics` blocks back? Today `check_functions` refuses all five
for Spectre, VACASK and LTspice (`src/codegen/mod.rs:46-52`, `:180-201`).

**Answer:**

- **Spectre.** Turn every call into a deterministic user function of a new *normalized*
  random parameter. For example, `agauss(nom, avar, sigma)` becomes
  `rack_agauss(nom, avar, sigma, z)`. Declare `z` as `parameters z=0` and vary it in a
  `statistics` block with `dist=gauss std=1` (or `dist=unif N=1`).
  - All three SPICE arguments stay in the expression where they were written. That
    handles instance-dependent spreads (IHP, gf180), sigma switches (IHP `mm_ok`), nesting
    and constant folding with no special cases.
  - The block is `process` when the call is evaluated at top level. It is `mismatch` when
    the call is evaluated inside a subcircuit, because Spectre samples mismatch per
    subcircuit instance.
  - The nominal run sees `z=0`, so it sees exactly `nom`.
- **VACASK.** VACASK gained Monte Carlo and four of the five functions with the same
  argument semantics (`docs/cmd-analysis-mc.md`, HEAD `d7b4db4`, 2026-10-29). The comment at
  `src/codegen/mod.rs:46-51`, which says it has none, is out of date. VACASK writes the
  calls verbatim. Only `limit` needs lowering, through `aunif`.
- **Spectre import.** A `statistics` block is read into a structured IR table. SPICE
  writers lower that table to `agauss`/`aunif`, using the two-layer `.param` idiom where
  ngspice needs it for a once-per-scope draw. The generated `rack_*` calls are recognized
  and turned back into `agauss(...)` text, so SPICE → Spectre → SPICE round-trips exactly.

One semantic choice is forced and documented in §5.1. In ngspice, HSPICE and Xyce, a
one-layer random `.param` is redrawn *at every reference*. Spectre has no per-reference
scope. The mapping draws once per evaluation scope instead, which is what VACASK does. The
two agree except when a one-layer random `.param` is referenced more than once. A lint rule
flags that case.

## Sources

Read locally unless a URL is given.

| Tag | Source |
|---|---|
| `R19` | Cadence *Spectre Circuit Simulator Reference*, Product Version 19.1, Jan 2020 (public mirror `https://ee.kpi.ua/~yv/edu/ok/book/spectre_refManual.pdf`). Monte Carlo pp.165-179; operators pp.474-475; user functions p.481 |
| `UG51` | Cadence *Spectre Circuit Simulator User Guide* 5.1.41, Jul 2004 (`https://web.eecs.utk.edu/~dbouldin/protected/spectreuser.pdf`), pp.176-179 |
| `DG` | J. O'Riordan (Cadence), *Recommended Spectre Monte Carlo Modelling Methodology*, 2003, `https://designers-guide.org/modeling/montecarlo.pdf` |
| `AB-24668`, `AB-30525` | Andrew Beckett (Cadence) answers in the Cadence forums: `…/f/custom-ic-skill/24668/monte-carlo-simulation-with-verilog-a-model-using-spectre`, `…/f/custom-ic-design/30525/regarding-section-statistics-for-monte-carlo` |
| `CBLOG` | Cadence blog, A. Rawat, 2020-05-28, `https://community.cadence.com/cadence_blogs_8/b/di/posts/library-characterization-tidbits-overriding-the-one-sigma-rule-of-liberty-for-lvf-modeling` (read via the Wayback Machine) |
| `NG47` | ngspice manual, Version 47 (`https://ngspice.sourceforge.io/docs/ngspice-manual.pdf`) |
| `NG-SRC` | ngspice git master `a80f6e3e`: `src/frontend/numparam/xpressn.c`, `src/frontend/inpcom.c`, `src/frontend/inp.c`, `src/maths/misc/randnumb.c` |
| `NG-RUN` | ngspice-44.2 runs made for this note (decks reproduced in §1.2) |
| `HS08` | HSPICE User Guide: Simulation and Analysis B-2008.09, App. A pp.873-881, p.538 (`https://cseweb.ucsd.edu/classes/wi10/cse241a/assign/hspice_sa.pdf`). An old version; medium confidence for current HSPICE |
| `XY-RG`, `XY-UG` | Xyce 7.9 Reference Guide (Table 2-33 pp.168-170, §2.1.33) and Users' Guide (§7.5.3.7 p.93), `https://xyce.sandia.gov/files/xyce/` |
| `VAC` | VACASK, `https://codeberg.org/arpadbuermen/VACASK`, HEAD `d7b4db43` (2026-10-29): `docs/cmd-analysis-mc.md`, `docs/expr-operators.md`, `include/rpnbuiltin.h` |
| `SKY` | `~/.ciel/ciel/sky130/versions/8afc8346…/sky130A/libs.tech/combined/` |
| `GF` | `~/.ciel/ciel/gf180mcu/versions/1e355470…/gf180mcuD/libs.tech/ngspice/` |
| `IHP` | IHP-Open-PDK `5e6d592e` (2026-09-01), `ihp-sg13g2/libs.tech/ngspice/models/` |
| `IHP-1254` | `https://github.com/IHP-GmbH/IHP-Open-PDK/issues/1254` (ngspice-47 evaluation-scope table) |

**Not reachable:**
- Current Spectre manuals (login only).
- Cadence `spp` documentation.
- Any Cadence statement on whether Spectre accepts `agauss()` in `simulator lang=spice`.
  `R19` has no `agauss` anywhere, and the function table on pp.476-477 lacks it.
- Current Synopsys HSPICE documentation.

I found no open PDK that ships both SPICE and Spectre statistics for the same parameters.
IHP-Open-PDK has no Spectre models; its only `.scs` file is a measurement deck under
`libs.doc/meas/pnpMPA/`. sky130 and gf180 ship SPICE only. The closest first-party mapping
is the `CBLOG` table:

> `parl1=agauss (0,1,3)` ↔ `vary parl1 dist=gauss std=1/3` "Absolute variation at 3-sigma is 1"

That gives std = avar/sigma.

## 1. The SPICE functions

### 1.1 Argument semantics

This is ngspice's implementation, verbatim in effect (`NG-SRC xpressn.c:45-85`). Here
`g ~ N(0,1)` comes from `gauss1()`, polar Box-Muller with no truncation
(`randnumb.c:220`). `u ~ U[-1,1)` comes from `drand()`, which returns
`2*CombLCGTaus()-1` (`randnumb.c:94-95`).

| Call | ngspice value | Mean | Std | Guard |
|---|---|---|---|---|
| `agauss(nom, avar, sigma)` | `nom + (avar/sigma)·g` | nom | avar/sigma | returns `nom` if `avar<=0` or `sigma<=0` (`xpressn.c:48-49`) |
| `gauss(nom, rvar, sigma)` | `nom + (nom·rvar/sigma)·g` | nom | \|nom·rvar/sigma\| | returns `nom` if `rvar<=0` or `sigma<=0` (`xpressn.c:59-62`) |
| `aunif(nom, avar)` | `nom + avar·u` | nom | \|avar\|/√3 | none |
| `unif(nom, rvar)` | `nom + nom·rvar·u` | nom | \|nom·rvar\|/√3 | none |
| `limit(nom, avar)` | `nom + (u>0 ? avar : −avar)` | nom | \|avar\| | none; a two-point distribution (`xpressn.c:82-84`) |

What other simulators do:

- **`NG47`.** §2.11.5 (pp.64-66) and §18.2 (pp.549-550) say the same in prose. For
  example: "gauss(nom, rvar, sigma) … standard deviation rvar (relative to nominal),
  divided by sigma".
- **`gauss(0, …)` is identically 0**, because its spread is relative to `nom`.
- **HSPICE** has the same five functions (`HS08` App. A):
  - "rel_variation: UNIF and GAUSS vary the nominal_val by +/- (nominal_val ⋅ rel_variation)".
  - "if sigma=3, then the standard deviation is abs_variation divided by 3".
  - An optional 4th argument, `multiplier`: "HSPICE recalculates many times and saves the
    largest deviation … The resulting distribution is bimodal".
  - ngspice-44.2 rejects the 4-argument form (`NG-RUN`).
- **Xyce** (`XY-RG` Table 2-33):
  - It has the same functions. `AGAUSS(μ,α,n)` has "standard deviation α/n", and "n is
    optional, and defaults to 1". So the 2-argument `agauss` is legal in Xyce and refused
    by ngspice.
  - `LIMIT(x,y)` is random ("x+y or x-y").
  - **The 3-argument `LIMIT(x,y,z)` is a deterministic clamp** ("y if x < y …"). Only
    `limit` with 2 arguments is statistical.
- **LTspice** uses other names. `gauss(x)` takes one argument (`src/codegen/mod.rs:49-51`).
  This note does not cover LTspice.

### 1.2 When a draw happens (global vs local)

**ngspice, as the code behaves.** Some background first:

- `inpcom.c:1151-1155` runs `inp_fix_agauss_in_param` for each of the five names. Its
  comment (`inpcom.c:8226-8237`) reads: "Convert .param lines containing function 'agauss'
  … into .func lines: .param xxx1 = 'aunif()' ---> .func xxx1() 'aunif()' … find all xxx1
  and convert them to a function: xxx1 ---> xxx1()".
- A `.param` evaluates once per scope: once at top level, once per instance inside a
  subcircuit.

The consequences:

- **A `.param` whose own text contains a call is a macro.** Every reference to it is a
  fresh draw, even two references in one expression. Measured (`NG-RUN`, deck `b.cir`):
  - `.param g=agauss(0,1,1)` used by `v8 'g'` and `v9 'g'` gave 0.181 and 0.146.
  - Inside a subcircuit, `.param m=agauss(0,1,1)` used twice gave 0.261 and 0.267.
  - The manual's own `'aga-aga'` gave −0.946, not 0.
- **A `.param` that only references such a param is a value.** It is drawn once per scope
  and shared by every reference. This is the *two-layer idiom*. Measured (`NG-RUN`, deck
  `a.cir`):
  - Top level `.param g_r=agauss(0,1,1)` followed by `.param g=g_r`: four uses across two
    instances all read 1.409.
  - Inside a subcircuit, `.param m_r=agauss(0,1,1)` followed by `.param m=m_r`: two uses
    in `x1` both read −1.252, while `x2` read 0.524. That is one draw per instance.
- **A direct call in an element, a model or a subcircuit default is drawn once per
  evaluation of that card.** At top level that is once. Inside a subcircuit it is once per
  instance, since expansion copies the card. `IHP-1254` tabulates this for ngspice-47. A
  `.model` at top level draws "one per run"; a model, instance or default inside a
  subcircuit draws "one per instance".
- **B-sources.** `agauss` "does not exist in the B source parser … in each time step a
  different return from agauss would result". So it is replaced by a constant per B-line
  after subcircuit expansion (`inp.c:2554-2565`).
- **The manual says the opposite.** `NG47` §18.2 p.550 says: "The parameters aga, aga2,
  lim obtain their numerical values once … v14 14 0 'aga' v15 15 0 'aga' * same value".
  The code and the runs contradict it.
- **There is no nominal mode.** Every ngspice run draws. The seed is 1 at start-up unless
  `.option seed=val|random` (`NG47` §11.1 p.321), `set rndseed` or `setseed` changes it
  (`NG47` §18.1).

**HSPICE** follows the same layer rule: "each time you use a parameter, it gets assigned a
new random value". With `.param my_resval=resval` in between, "the assignment of a random
value is only done once". Model cards are drawn once unless `.OPTION MODMONTE` is set
(`HS08` p.874). Outside Monte Carlo, `nominal_val` is the "default value in all other
analyses".

**Xyce** also follows it: "If there is only a single layer, then the random operator is
treated as a local variation … If there is more than one layer, then it is instead treated
as global" (`XY-UG` §7.5.3.7). Outside `.SAMPLING … USEEXPR=TRUE` the functions return the
mean (`XY-RG` Table 2-33, footnote 1).

So the simulators have two scopes in practice:

- **Global (process):** one draw per run, through a two-layer param or a top-level card.
- **Local (mismatch):** one draw per subcircuit instance, or per top-level element.

In ngspice, HSPICE and Xyce there is also a degenerate *per-reference* scope for one-layer
params.

## 2. How the open PDKs use them

Counts are by card. The scan joined `+` continuations and skipped comments.

### sky130

All calls in `SKY` are `AGAUSS(N,N,N)` (203) or `GAUSS(N,N,N)` (69), always with literal
arguments and `nom=0`. There are no `aunif`, `unif` or `limit` calls.

**Process:** 195 top-level `.param`s in `continuous/parameters_*.spice` and
`parameters/critical.spice`. All are one-layer, gated by switches. For example:
- `parameters_fet_tt.spice:19`:
  `.param sw_polycd = {0.0+corner_factor*0.0} + process_mc_factor*MC_PR_SWITCH*AGAUSS(0,1.5e-09,1)`
- `parameters_res_nom.spice:30`:
  `… + process_mc_factor*MC_PR_SWITCH*EXP(AGAUSS(0,0.26,1))` (a nested call)

The switches are set per corner section:
- `.lib tt` sets `MC_MM_SWITCH=0`, `MC_PR_SWITCH=0` and `process_mc_factor=1`
  (`sky130.lib.spice:82-86`).
- `.lib mc` sets `MC_PR_SWITCH=1` (`:1378-1400`).
- `.lib tt_mm` sets `MC_MM_SWITCH=1` (`:730-731`).

**Mismatch:** inside subcircuits, always shaped
`sw_mm_* * mismatch_factor*MC_MM_SWITCH*AGAUSS(0,1.0,1)/sqrt(w*l*mult)`. There are:
- 15 subcircuit `.param` locals, e.g. `models_fet.spice:889-890`
  `+ swx_vth0_delta = {…AGAUSS(0,1.0,1)/sqrt(l*w*mult)…}`, read by `delvto` at `:896`;
- 13 `.model` cards inside subcircuits, e.g. `models_resistors.spice:233-234`
  `.model rbody_model r` `+ rsh = {…AGAUSS…}`;
- 15 instance values, e.g. `models_resistors.spice:266` `Rsky130_fd_pr__res_generic_po r0 r1 r = {r_tot*(1+…AGAUSS…)}`.

**Two sky130 quirks a faithful translation must keep:**
- **Every `GAUSS` call has `nom=0`**, so every one is identically 0 in ngspice, HSPICE and
  Xyce (§1.1). Example: `parameters_res_nom.spice:17`
  `+ process_mc_factor*MC_PR_SWITCH*GAUSS(0,0.035,1)`. The resistor and junction process
  terms are dead.
- **`EXP(AGAUSS(0,s,1))` is added to the nominal, not multiplied.** Example:
  `parameters_fet_tt.spice:21`
  `.param sw_nw_rs_mult = {1.0+corner_factor*0.0} + process_mc_factor*MC_PR_SWITCH*EXP(AGAUSS(0,0.12,1))`.
  With the `mc` section selected it is about 2.0, not about 1.0.

These are PDK bugs. Translate them, don't fix them.

**ngspice draws sky130's process params per reference.** Its one-layer process params are
referenced from many model cards inside subcircuits, so per-reference drawing makes
"process" Monte Carlo behave as per-device in ngspice (§1.2). The names
(`process_mc_factor`, `MC_PR_SWITCH`) say the intent is global.

### gf180mcu

`GF` has 266 `agauss` calls and no other statistical function. In `sm141064.ngspice` the
calls take three shapes: `agauss(0,1,3)` with literal arguments (91), `agauss(0,var_k,1)`
(21) and `agauss(0,var_vth,1)` (21).

**Process:** the two-layer idiom. In `.lib statistical`, `sm141064.ngspice:556-602`:
- `.param mc_sig_vth2='agauss(0,1,3)'`, then
- `.param mc_sig_vth='mc_sig_vth2'`, then
- `.param nfet_03v3_sig_vth1='(5e-3*mc_sig_vth+30e-3*mc_sig_vthn)*sw_stat_global*mc_skew'`.

That is std 1/3 per normalized variable, gated by `sw_stat_global` (`design.ngspice:62`).

**Mismatch:** a one-layer subcircuit `.param` with an **instance-dependent spread**
(`sm141064.ngspice:693-708`):
- `.param var_vth='0.7071*par_vth*1e-06/p_sqrtarea'` and `.param mis_vth='agauss(0,var_vth,1)'`;
- read once, by `delvto='mis_vth*sw_stat_mismatch'`.

### IHP sg13g2

`IHP` has 137 `agauss` and 129 `gauss` calls.

**Process:** the two-layer idiom with a relative `gauss` and nonzero nominals
(`sg13g2_moslv_stat.lib:21-42`):
- `.param num_sigmas=1`;
- `.param mc_sg13g2_lv_nmos_vfbo ='gauss(sg13g2_lv_nmos_vfbo_norm, 0.0050, num_sigmas)'`;
- `.param sg13g2_lv_nmos_vfbo = mc_sg13g2_lv_nmos_vfbo`.

**Mismatch:** direct calls in instance parameters inside the subcircuit, with a per-instance
sigma switch (`sg13g2_moslv_mod_mismatch.lib:66-88`):
- `.subckt sg13_lv_nmos … mm_ok=0 …`;
- `+ w='agauss(w, sg13g2_lv_nmos_dw_mm, (mm_ok != 1 ? 0 : 1))'`;
- `+ delvto='agauss(0, sg13g2_lv_nmos_delvto_mm/sqrt(m*l*w*1e12), (mm_ok != 1 ? 0 : 1))'`.

The switch relies on `sigma<=0` returning `nom` (`xpressn.c:48`). Translated literally into
a `vary … std=0`, Spectre rejects it: "SFE-3225: Variance … cannot be zero! Set
'ignorezerovar=yes'" (`AB-24668`).

### What this means for the mapping

- Calls sit in five places: top `.param`s, subcircuit `.param` locals, `.model` cards
  inside subcircuits, instance parameters inside subcircuits, and nested inside `exp()`.
- Arguments are literals, but also expressions over subcircuit parameters (`w`, `l`, `m`,
  `mm_ok`, `var_vth`).
- None of the three PDKs puts a call in a `.subckt` header default, a top-level `.model`
  card or a `.func`.

## 3. The Spectre statistics model

These rules come from `R19` pp.175-179 unless noted.

- **Blocks.** `statistics { process { vary … } mismatch { vary … } correlate … truncate … }`.
  Several statistics blocks "either accumulate or overlay".
  - In a library file the block sits inside a `section … endsection` (`AB-30525`).
- **Scopes.** "The distributions specified in the process block are sampled once per Monte
  Carlo iteration … the distributions specified in the mismatch block are sampled on a per
  subcircuit instance basis".
  - Also: "If you place model cards and/or device instances in subcircuits, and add a
    mismatch block … you can effectively model device-to-device mismatch" (p.178).
  - The varied parameter is a top-level `parameters` declaration that the subcircuit body
    references. `DG` §5: "mismatch only Monte Carlo analysis … results in a different value
    of XRSP for every subcircuit (resistor in this case) instance".
- **Both scopes on one parameter.** "The sampled process value becomes the mean for the
  mismatch random number generator".
- **Syntax.** `vary PAR_NAME dist=<type> {std=<value> | N=<value>} {percent=yes|no}`, with
  `type` one of `gauss`, `lnorm` or `unif`.
  - `gauss`: "the mean value is taken as the current value of the parameter being varied",
    giving `Normal(mean, std)`.
  - `lnorm`: `log(x) = Normal(log(mean), std)`, natural log.
  - `unif`: `x = unif(mean-N, mean+N)`. **`N` is the half-range**, so
    "std=N/sqrt(3)".
  - `percent=yes`: std or N is a percentage of the mean, e.g. "Normal (200, 12%*(200))".
    The manual recommends against it for `lnorm`. With a mean of 0 it gives zero variance
    and SFE-3224/3225 (`AB-24668`).
- **Expressions are allowed** in `cc`, `std` and `N`: "Correlation coefficients can be
  constants or expressions, as can std and N" (p.179).
- **Only numeric parameters can be varied, per the old User Guide.** "Derived parameters
  that have their default values specified as expressions of other parameters cannot have
  distributions specified for them" (`UG51` p.179, 2004). I could not check whether this
  still holds, so this design never varies an expression-valued parameter.
- **Correlation.**
  - `correlate param=[p1 p2] cc=` correlates process parameters.
  - `correlate dev=[i1 i2] {param=[…]} cc=` correlates the mismatch of instances; `*`
    wildcards are allowed.
- **Truncation.** "The default truncation factor for gaussian distributions … is 4.0 sigma.
  Randomly generated values that are outside the range … are automatically rejected and
  regenerated".
  - It is set with `truncate tr=<value>`. Process, mismatch and the whole block each have
    their own factor, and a missing one inherits from the statistics block.
  - A 4σ rejection lowers the std by a factor of 0.99946, which is −0.054% (computed).
  - ngspice and VACASK never truncate ("tails are not clipped", `VAC` cmd-analysis-mc.md:47).
- **The analysis.** `mc1 montecarlo numruns=200 seed=1234 variations=process|mismatch|all
  sampling=standard|lhs|orthogonal|lds donominal=yes … { <analyses> }` (pp.165-174).
  - `variations` defaults to `process`. `numruns` defaults to 100 and "does not include
    the nominal run".
  - "If you do not specify a seed, each time you run the analysis, you will get different
    results".
  - `dut=`, `ignore=`, `dutparams=` and `dist=default|unif|gauss` narrow or override the
    variations.
- **Nominal.** Outside `montecarlo`, and in its nominal run, every varied parameter keeps
  its declared value. "The nominal or mean value … is the current value of the parameter
  just before the Monte Carlo analysis starts" (`UG51` p.177).
- **Expression tools** the mapping needs:
  - The ternary `(cond) ? x : y` exists. "There is no short circuiting of logical
    expressions involving && and ||" (`R19` pp.474-475).
  - User functions: `real f(real a, …) { return <expr>; }`. They "must be declared only at
    the top level", "may be called from anywhere an expression can be currently used", and
    their body may reference only arguments and `temp`/`tnom` (`R19` p.481). SpiceRack
    already writes `.func` this way (`src/codegen/spectre/mod.rs:89-105`).

## 4. VACASK

VACASK now has Monte Carlo (`VAC` `docs/cmd-analysis-mc.md`). Line numbers below are in
that file unless noted.

- **Control loop.** `mc name samples=n [seed=s] [lh=b] [strict=b] [debug=d] … endmc`
  (lines 7-23).
  - The seed defaults to 0. Latin hypercube sampling is on by default (`lh=1`).
  - After the loop "the circuit is re-elaborated with all distribution functions evaluating
    to their nominal value" (line 25).
- **Functions.** `gauss(nom,rvar,sigma)` (std `nom*rvar/sigma`), `agauss(nom,avar,sigma)`
  (std `avar/sigma`), `unif(nom,rvar)` on `[nom*(1-rvar), nom*(1+rvar)]` and
  `aunif(nom,avar)` on `[nom-avar, nom+avar]` (lines 31-36). These are ngspice's
  semantics.
  - `rvar/avar/sigma<=0` returns `nom` (line 38).
  - There is **no `limit`**. The builtins table registers only the four, at
    `include/rpnbuiltin.h:590-594`. The generator is at `:518-612`.
- **Nominal.** "Outside a Monte Carlo loop these functions return `nom` unchanged"
  (line 49).
- **Where draws happen.** Only in "an instance parameter, a model parameter, a `parameters`
  statement (subcircuit or toplevel)". Elsewhere a call returns `nom`, e.g. in a
  conditional's condition (lines 53-59; code check at `rpnbuiltin.h:572-576`).
- **Draw identity is per call site.** "Each call site is identified by the enclosing
  instance/model/subcircuit-instance name, the parameter name, and the position of the call
  … the same call site always gets the same sample value within one Monte Carlo iteration"
  (line 61).
  - So a top-level `parameters p=agauss(…)` is one draw per sample, shared by every
    reference.
  - A subcircuit `parameters` call is one draw per subcircuit instance.
  - This is exactly the Spectre process/mismatch split, and not ngspice's per-reference
    rule.
- **No user functions.** VACASK has none (`src/codegen/vacask/mod.rs:89-92`). It does have
  `a ? b : c` (`docs/expr-operators.md:54`).

No VACASK binary is installed here. The per-call-site claims come from the doc and the code
and have not been run.

## 5. SPICE → Spectre

### 5.1 Scope rule (process vs mismatch)

A statistical call becomes **one independent random variable per call site per evaluation
scope instance**. Which block it lands in depends on where the call's expression is
evaluated:

| The call's expression | ngspice draws | Spectre block |
|---|---|---|
| top-level or section `.param` (one- or two-layer) | once per run (two-layer), or per reference (one-layer) | `process` |
| top-level element or instance parameter | once per element | `process`; each site is its own variable, so elements are independent |
| top-level `.model` card | once per run (`IHP-1254`) | `process` |
| subcircuit `.param` local, subcircuit default, instance or element inside a subcircuit, `.model` inside a subcircuit | once per subcircuit instance | `mismatch` |
| B-source / behavioural expression | a constant per card after expansion (`inp.c:2554`) | same as its card's scope |
| `.func` body | per call of the function | inline the body at each call site, then apply this table |

Why the rule is once per scope and not per reference:

1. Spectre has no per-reference scope.
2. It is what the PDKs mean: sky130's `process_mc_factor*MC_PR_SWITCH` and gf180/IHP's
   `_stat` sections.
3. It is what the ngspice manual documents (§1.2).
4. It is what VACASK implements (§4).
5. The two rules differ only when a one-layer random `.param` is referenced more than once.

Exact ngspice parity would mean macro-expanding every one-layer random `.param` at each
reference before lifting. That would turn sky130's process Monte Carlo into thousands of
per-device variables. It is not adopted. Lint warns instead (§10.5). Add the expansion
only if exact parity is ever needed.

Placing a call inside a subcircuit in `mismatch` is what gives it one value per instance,
as in ngspice. Spectre does this per *hierarchical* instance: `x1.xa` and `x2.xa` are
different instances. Confidence is medium: `R19` says "per subcircuit instance" without
discussing nesting.

### 5.2 Lifting

Two calls stay out of this:
- Xyce's 3-argument `limit(x,y,z)` is not statistical. Write it as
  `min(max(x, y), z)`, or refuse it as today.
- The HSPICE 4-argument form is handled in §5.3.

Every other call `f(args…)` in an expression is rewritten to `rack_f(args…, z)`, where `z`
is a fresh parameter. The five functions are emitted once, at the top of the deck:

```spectre
real rack_agauss(real nom, real avar, real sigma, real z) {
    return nom + ((avar > 0 && sigma > 0) ? avar : 0) / ((sigma > 0) ? sigma : 1) * z;
}
real rack_gauss(real nom, real rvar, real sigma, real z) {
    return nom + nom * ((rvar > 0 && sigma > 0) ? rvar : 0) / ((sigma > 0) ? sigma : 1) * z;
}
real rack_aunif(real nom, real avar, real u) { return nom + avar * u; }
real rack_unif(real nom, real rvar, real u) { return nom + nom * rvar * u; }
real rack_limit(real nom, real avar, real u) { return (u > 0) ? nom + avar : nom - avar; }
```

The guards reproduce `xpressn.c:48-49` and `:59-62`. The division is written so that no
branch ever divides by zero, because `R19` p.474 promises no short-circuiting. Each `z` is
declared and varied in the scope §5.1 picks:

```spectre
parameters rack_sw_polycd_1=0 rack_rbody_model_rsh_1=0
statistics {
    process  { vary rack_sw_polycd_1       dist=gauss std=1 }
    mismatch { vary rack_rbody_model_rsh_1 dist=gauss std=1 }
}
```

For `aunif`, `unif` and `limit` the variable is `dist=unif N=1`, which is uniform on
[−1, 1] (§3). That matches `drand()` on [−1, 1).

Why the normalized-variable form:

- **The arguments stay in their own scope.** `nom`, `avar` and `sigma` are evaluated where
  they were written, so `w`, `m`, `mm_ok` and `var_vth` resolve per instance. A
  `vary p std=<expr>` in a global block could not see subcircuit parameters.
- **It never varies an expression-valued parameter** (`UG51` p.179).
- **It never has zero variance** (SFE-3225), even when `sigma=0` or `mean=0`.
  Beckett recommends exactly this: "have a normalized statistical parameter - e.g. a mean of
  0, and standard deviation of 1, and then scale it in the model" (`AB-24668`).
- **Nesting needs no special case.** `EXP(AGAUSS(0,0.26,1))` becomes
  `exp(rack_agauss(0,0.26,1,rack_…_1))`, and `agauss(agauss(0,1,1),1,1)` gets two variables.
- **Duplicated `nom` is harmless.** `rack_gauss` evaluates `nom` once as an argument, so a
  `nom` that contains its own call is not drawn twice.
- **The nominal run is exactly `nom`**, since every `z` is 0. So it equals the SPICE deck
  with each call replaced by its first argument.
- **`CBLOG` agrees.** `agauss(0,1,3)` gives std 1/3 here too: `rack_agauss(0,1,3,z)`
  is `z/3` with `z ~ N(0,1)`.

**Names.** `rack_<owner>_<field>_<k>`:
- `owner` is the `.param` name, or `<model>`/`<instance>`.
- `field` is the parameter, or empty for a `.param`.
- `k` is the 1-based index of the call in that expression.
- A per-IR counter suffix is added only on collision.

The names come from the owner, not a global counter, because `pdk-models` converts files
separately and those files are then included together. For example,
`parameters_fet_tt.spice` and `models_fet.spice` both go into sky130's `.lib tt`. Owner
names are already unique wherever the PDK's own names are.

**Placement.** The `parameters rack_…=0` line and its `statistics` block are written:
- at the deck top for top-level calls, and for calls in subcircuits defined at the top;
- inside the same `section` for calls in a `LibSection` body or its subcircuits, matching
  `AB-30525`.

Several blocks accumulate (`R19` p.176). The `real rack_*` functions go once at the deck
top, or once per library file at its top, outside any section. Spectre requires them at
top level and before use (`R19` p.481).

### 5.3 Edge cases

- **Non-constant arguments.** They are kept verbatim as function arguments (§5.2), covering
  IHP `sqrt(m*l*w*1e12)`, gf180 `var_vth` and sigma switches.
- **Calls in subcircuit parameter defaults** (`.subckt s a r='agauss(1k,50,3)'`) are not
  used by any PDK checked. Write `r=rack_agauss(1k,50,3,rack_s_r_1)` in the header with a
  mismatch variable. This assumes Spectre evaluates a default once per instance, in the
  instance's scope, which `R19` does not say. Mark it unverified and test it on a real
  Spectre (§10.6 T-S4).
- **`.model` cards.** At top level the variable goes in `process`; inside a subcircuit, in
  `mismatch` (`R19` p.178). A binned model group keeps one variable per bin card, since
  each bin is its own card.
- **Two calls in one expression** get two variables, matching ngspice.
- **`limit`** becomes `rack_limit(nom, avar, u)` with `u ~ unif N=1`. This is a two-point
  distribution with the same mean and std as ngspice. The ngspice manual itself shows the
  same construction in a control-language `define`
  (`limit(nom, avar) (nom + ((sgauss(0) >=0) ? avar : -avar))`, `NG47` §13.5.13 p.404, from `examples/Monte_Carlo/MonteCarlo.sp`).
- **HSPICE's 4-argument `multiplier`** has no ngspice or Spectre equivalent. With a literal
  integer `m`, it can be emitted as `m` variables reduced by a generated
  `rack_maxabs(a,b) = abs(a) >= abs(b) ? a : b`, which matches "saves the largest
  deviation" (`HS08`). Any other `m` is refused. This is the one refusal left, and only for
  the HSPICE dialect. Skip it until an HSPICE deck needs it.
- **Xyce's 2-argument `agauss`/`gauss`** take `sigma=1` (`XY-RG`).
- **Seeds.**
  - ngspice `.option seed=N` maps to `seed=N` on a written `montecarlo`. It carries the
    seed only, never the same sample stream.
  - The IR's `Analysis::SpectreMonteCarlo` already holds `seed`
    (`src/ir/testbench.rs:165-170`).
- **Truncation.** No `truncate` statement is written, so Spectre's default 4σ rejection
  stays. That costs −0.054% on each std and is invisible at practical run counts. It is the
  behaviour Spectre PDK users expect. Spectre accepts an explicit `truncate tr=` if exact
  untruncated parity is ever wanted, but large `tr` values have not been tried.
- **`variations`.** Spectre defaults to `process` (`R19` p.166). A SPICE deck lifted this
  way has variables in both blocks. A SPICE-origin `montecarlo` should therefore be written
  with `variations=all`, which is what an ngspice loop does.
- **Calls in raw lines** (a card the IR has no field for, such as IHP's `.if` branches if
  they stay raw) are not lifted. They reach only their own dialect, as today
  (`src/import/mod.rs:9-12`).
- **Structural `if` conditions.** These are lifted like any expression. VACASK returns
  `nom` there (§4). Spectre's behaviour under Monte Carlo is untested.

## 6. Spectre → SPICE (and back)

Two inputs, two paths.

**(a) Generated calls, from SpiceRack's own output.** A call
`rack_f(a…, v)`, where `v` is declared `parameters v=0` and varied with exactly
`dist=gauss std=1` (`agauss`, `gauss`) or `dist=unif N=1` (`aunif`, `unif`, `limit`) in
either block, is read back as the text `f(a…)`. Then:
- the `parameters v`, its `vary` line and the `real rack_*` definitions are dropped;
- SPICE → Spectre → SPICE is the identity on expressions.

If the pattern does not hold, because someone edited the variable, nothing is rewritten.
`rack_*` stays an ordinary `FuncDef` and `v` an ordinary statistics entry. The result is
still correct, just not sugared.

**(b) Native Spectre statistics.** These go into the IR table (§10.1). A SPICE writer
lowers each `vary` of parameter `p`, where `v` is `p`'s declared value:

| Spectre | SPICE (ngspice/HSPICE/Xyce) |
|---|---|
| `process { vary p dist=gauss std=s }` | `.param p__mc = agauss(v, s, 1)` then `.param p = p__mc`. Two layers, so one draw per run shared by every reference (§1.2, measured) |
| `percent=yes` | `agauss(v, (v)*(s)/100, 1)` |
| `dist=unif N=n` | `aunif(v, n)`; with percent, `aunif(v, (v)*(n)/100)` |
| `dist=lnorm std=s` | `exp(agauss(log(v), s, 1))`; `lnorm` with percent is ambiguous (§3), so the block stays raw and is reported |
| `mismatch { vary p … }` | in every subcircuit whose body references `p`, add `.param p__mm_r = agauss(p, s, 1)` and `.param p__mm = p__mm_r`, and rename `p` to `p__mm` in that body. Two layers inside the subcircuit, so one draw per instance (measured, `a.cir`) |
| `p` in both blocks | the process lowering at top, then the mismatch lowering around the process-sampled `p`. That is Spectre's "process value becomes the mean" rule |
| `correlate param=[a b …] cc=c`, gauss process, numeric `cc` | normalized two-layer `z_i = agauss(0,1,1)`, then a Cholesky factor `L` of the correlation matrix (unlisted pairs 0), `p_i = v_i + s_i·Σ_j L_ij z_j`. A 2×2 with an expression `cc`: `p_b = v_b + s_b·(cc·z_1 + sqrt(1-cc^2)·z_2)` |
| `correlate` with expression `cc` (n>2), or over non-gauss distributions | refuse (`CodeGenError::Other`), naming the statement |
| `correlate dev=[…]` (matched instances) | no SPICE form without restructuring the instances; refuse |
| `truncate tr=t` | none in SPICE. With `t >= 4`, write nothing and report it; with `t < 4`, refuse, since the std differs by more than 1% (e.g. 0.880× at 2σ, computed) |

## 7. VACASK output

- **The four native functions** go out verbatim. They draw exactly the §5.1 scopes (§4),
  so no lifting is needed.
- **`limit(nom, avar)`** becomes `((aunif(0, 1) > 0) ? (nom) + (avar) : (nom) - (avar))`.
  That is one call site in the same expression.
- **Spectre statistics from the IR table** are lowered as in §6, but one layer is enough,
  because a VACASK `parameters` call is already a value:
  - top-level `parameters p = agauss(v, s, 1)`;
  - per referencing subcircuit, `parameters p__mm = agauss(p, s, 1)`, with references
    renamed.
- **`Analysis::SpectreMonteCarlo`** becomes
  `mc mc1 samples=<numruns> seed=<seed> lh=0 … analysis … endmc` in the control block.
  `lh=0` matches Spectre's `sampling=standard`.
- **`variations`.** VACASK has no switch for it. `variations=all` lowers as written. When
  `process` or `mismatch` excludes a scope that the deck has variables in, refuse. Do not
  emit a silently different run. If this is ever needed, gate each lowered call's
  `sigma`/`avar` with a `rack_process`/`rack_mismatch` 0/1 parameter. VACASK's
  `<=0 → nom` rule makes that exact.
- **The nominal (non-`mc`) run** returns `nom` everywhere, the same as Spectre.

## 8. Existing converters

- **Keysight ADS Netlist Translator** (`https://edadownload.software.keysight.com/eedl/ads/2011/pdf/netlist.pdf`,
  pp.26, 142). `gauss()` gives "Unresolved reference error". `statistics{mismatch{}}` was
  "not supported … prior to ADS 2005A". "%x" statistics are not translated.
- **dan-fritchman/Netlist** (`netlist/write/spice.py:307-317`). It parses Spectre
  statistics blocks but writes them to SPICE only as an "Unsupported `StatisticsBlock`"
  comment.
- **Cadence `spp` and Spectre's SPICE reader.** No primary text was reachable.
- **SpiceRack today:**
  - It keeps a Spectre `statistics` block whole as a raw `Block`, which reaches Spectre
    only (`grammar/spectre.peg:114-119`).
  - It refuses the functions for Spectre, VACASK and LTspice (`src/codegen/mod.rs:182-201`).
  - It writes `montecarlo` with a note that "without one this runs `numruns` identical
    simulations" (`src/codegen/spectre/analyses.rs:349-351`).

## 9. Pitfalls

1. **The per-reference redraw** of one-layer random `.param`s in ngspice, HSPICE and Xyce
   (§1.2). The ngspice manual says otherwise. The adopted mapping draws once per scope.
2. **`gauss` is relative.**
   - `gauss(0,…)` ≡ 0, which kills sky130's GAUSS terms.
   - Translating `gauss` as `std=rvar/sigma` is wrong by a factor of `nom`.
3. **`sigma` is a divisor.** `agauss(0,1,3)` has std 1/3 (`CBLOG`). `sigma<=0` means "off",
   not a division by zero.
4. **`unif` N is the half-range**, both in Spectre (`N`) and in SPICE (`avar`, `nom·rvar`).
   The std is N/√3. Writing `std=` for a uniform is wrong.
5. **Truncation.** Spectre truncates gaussians at 4σ by rejection; SPICE and VACASK do not
   truncate.
6. **The nominal run.**
   - ngspice draws on every run, so a plain `op` is one random sample.
   - Spectre and VACASK use `nom` outside Monte Carlo, and so do HSPICE and Xyce.
   - Compare Spectre's nominal only with the SPICE deck with every call replaced by `nom`.
7. **Zero variance** (`std=0`, or `percent=yes` with mean 0) is a Spectre error
   (SFE-3224/3225). This happens when IHP's `mm_ok` gating is translated into `std=`.
8. **Default `variations=process`** in Spectre ignores mismatch variables. Write `all` for
   SPICE-origin decks.
9. **Xyce `LIMIT(x,y,z)` is a clamp**, not random, so detect `limit` by arity. The current
   by-name refusal (`STATISTICAL`) refuses Xyce's clamp for no reason.
10. **HSPICE's 4th argument** is rejected by ngspice.
11. **Seeds** do not carry sample streams across simulators. Compare statistics, never
    individual runs.
12. **PDK quirks** (sky130 `+EXP(AGAUSS)`, dead `GAUSS(0,…)`) must survive translation
    unchanged.

## 10. Adopted mapping

### 10.1 IR (`src/ir/circuit.rs`, schema in `schema/circuit-ir.schema.json`)

- **Expressions.** No change for SPICE-origin calls: they stay as text in expressions, the
  source of truth for SPICE writers. That keeps SPICE → IR → SPICE the identity.
- **New field `Subcircuit::statistics: Vec<Statistics>`**
  (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`). It is filled only by the
  Spectre importer, and is meaningful on `CircuitIR.top` and `LibSection.body`:

  ```rust
  /// One Spectre `statistics { }` block (R19 pp.175-179).
  pub struct Statistics {
      pub process: Vec<Vary>,
      pub mismatch: Vec<Vary>,
      pub correlations: Vec<Correlate>,
      /// `truncate tr=` per scope: (block, process, mismatch); `None` inherits (R19 p.179).
      pub truncate: [Option<String>; 3],
  }
  pub struct Vary { pub param: String, pub dist: Dist, pub spread: String, pub percent: bool }
  pub enum Dist { Gauss, Lnorm, Unif }      // `spread` is std= (gauss, lnorm) or N= (unif)
  pub struct Correlate { pub params: Vec<String>, pub devices: Vec<String>, pub cc: String }
  ```

- **`Analysis::SpectreMonteCarlo`** gains `variations: Option<String>`. It is written only
  when `Some`, so imported decks round-trip.

### 10.2 Spectre codegen (new transform module `src/codegen/statistics.rs`)

1. Walk the expressions with their owner and scope, as `expressions` does
   (`src/codegen/mod.rs:163-178`). Skip a 3-argument `limit` and an HSPICE 4-argument call.
2. Inline the body of every `.func` that contains a statistical call at its call sites.
3. Rewrite each call to `rack_<f>(args…, rack_<owner>_<field>_<k>)`. Record
   `(name, scope, Gauss|Unif, section)`, where scope is process for top-level evaluation and
   mismatch inside a subcircuit (§5.1).
4. Emit the five `real rack_*` functions (§5.2) once per output file, only if used.
5. Emit `parameters <names>=0` plus a `statistics { process { vary … dist=gauss std=1 | dist=unif N=1 } mismatch { … } }`
   block per deck top and per section.
6. Write the `Statistics` table entries verbatim as further blocks.
7. Write a SPICE-origin `montecarlo` with `variations=all`.
8. In `check_functions`, Spectre and VACASK become `has = true`, and the LTspice refusal
   stays. Update the stale VACASK claim in the comment at `:46-51`.

### 10.3 VACASK codegen

- The four functions are written verbatim. `limit` is lowered (§7).
- `Statistics` entries are lowered to one-layer `parameters … = agauss/aunif(…)` (§7).
- `SpectreMonteCarlo` becomes `mc … lh=0 … endmc`. Refuse a `variations` value that drops a
  populated scope.

### 10.4 Spectre import

- Parse `statistics { process { vary … } mismatch { … } correlate … truncate … }` into
  `Statistics`. A block that does not parse stays a raw `Block`, as today.
- Recognize the generated pattern (§6a) and rewrite it back to SPICE text. Drop the
  `rack_*` `FuncDef`s and their variables.
- Read `montecarlo`'s `variations=`.

### 10.5 SPICE codegen of `Statistics` (ngspice, HSPICE, Xyce)

- Lower as in §6(b). Refuse only `correlate dev=`, an n>2 `correlate` with an expression
  `cc`, a non-gauss `correlate`, `truncate tr<4`, and `lnorm` with `percent`.
- Lint: warn when a one-layer `.param` whose own text calls a statistical function is
  referenced more than once. ngspice draws each reference; Spectre and VACASK draw once.

### 10.6 Tests

**Golden text, no simulator:**

- **T-G1**: `r1 a 0 r='agauss(1k,100,3)'` writes:
  - `rack_agauss(1k, 100, 3, rack_r1_r_1)`;
  - `parameters rack_r1_r_1=0`;
  - `process { vary rack_r1_r_1 dist=gauss std=1 }`.
- **T-G2**: one case each for `gauss`, `aunif`, `unif` and `limit`, with the `dist=unif N=1`
  ones for the last three.
- **T-G3**: two calls in one expression, and the nested `agauss(agauss(0,1,1),1,1)`, give
  two variables.
- **T-G4**: placement.
  - A subcircuit `.param` local, a subcircuit `.model` and a subcircuit instance go to
    `mismatch`.
  - A top `.model` and a top `.param` go to `process`.
- **T-G5**: IHP `w='agauss(w, dw_mm, (mm_ok != 1 ? 0 : 1))'` keeps its arguments verbatim
  and produces no `std=0`.
- **T-G6**: the sky130 line `parameters_fet_tt.spice:21` keeps the nested `EXP` and the
  `+` composition.
- **T-G7**: a call inside `.lib tt` puts its `parameters` line and statistics block inside
  `section tt`.
- **T-G8**: a `.func` whose body calls `agauss` is inlined at each call site.
- **T-G9**: Xyce `limit(x,1,2)` is not treated as statistical. Xyce `agauss(1,2)` gets
  sigma 1.
- **T-G10**: Spectre and VACASK no longer refuse (invert the current refusal test). LTspice
  still refuses.

**Round trip:**

- **T-R1**: SPICE → Spectre → SPICE is the identity on every expression. Run it on a
  sky130, gf180 and IHP slice.
- **T-R2**: a Spectre statistics block with process, mismatch, `correlate param`,
  `truncate` and `percent` goes Spectre → IR → Spectre and comes back equal as a table.
- **T-R3**: the `Statistics` → ngspice lowering gives:
  - the process two-layer idiom;
  - the per-subcircuit mismatch two-layer idiom with renamed references;
  - `exp` for `lnorm`, scaling for `percent`, and `aunif` for `unif N`.
- **T-R4**: a 2×2 `correlate param` becomes Cholesky text. `correlate dev` is an error.
- **T-R5**: `pdk-models` converts sky130, gf180 and IHP to Spectre with no refusal. The
  count of `vary` lines equals the count of lifted call sites.

**Numeric, ngspice only (runs here).** The lowering is pure algebra, so its statistics can
be checked without Spectre:

- **T-N0**: write the lowered form back to ngspice. Each `rack_*` becomes a `.func`, and
  each `z` becomes a two-layer `agauss(0,1,1)` (or `aunif(0,1)`) in its scope. Then compare
  with the original deck over N=2000 runs.
  - Each `rack_*` `.func` must carry the §5.2 guards. ngspice's numparam takes `?:`
    (`grammar/ngspice.peg` already reads ternaries).
  - The deck is a divider: `v1 in 0 1`, `r1 in out <F>`, `r2 out 0 1k`.
  - Expected v(out) (linearized, computed):
    - `agauss(1k,100,3)`: mean 0.5, std 8.33e-3 (std(R1)=33.3).
    - `aunif(1k,100)`: std(R1)=57.7.
    - `limit(1k,50)`: exactly two values.
  - Tolerances: mean within 4·σ/√N, std within 4·σ/√(2N).
  - Use the `NG47` §18.5.2 "Monte-Carlo example 1" loop: seed per run, `mc_source`
    (§13.5.48), re-run.
- **T-N1 (mismatch scope)**: two instances of a subcircuit with a direct call have
  inter-instance correlation |ρ| < 4/√N. A top two-layer process param read by both has
  ρ = 1.

**Numeric, Spectre/VACASK (gated on the binary being present; neither is installed
here):**

- **T-S1 (nominal)**: with no `montecarlo`/`mc`, every node voltage of the T-G decks equals
  ngspice on the same deck with each call replaced by `nom`, to 1e-9 relative. For VACASK,
  run outside `mc`.
- **T-S2 (statistics)**: T-N0 decks under `montecarlo numruns=2000 variations=all` and
  `mc samples=2000 lh=0` match the ngspice mean and std within the T-N0 tolerances. Spectre's
  4σ truncation (−0.054% std) is far inside them.
- **T-S3 (mismatch)**: T-N1 holds under Spectre `variations=mismatch` and under VACASK.
  Nested `x1.xa` vs `x2.xa` are independent.
- **T-S4 (unverified Spectre behaviour)**: a call in a subcircuit parameter default draws
  per instance, and a lifted variable inside a structural `if` behaves as at top level.
  Record the result in this note.

## 11. As implemented

`src/codegen/statistics/` (`lift.rs`, `lower.rs`, `vacask.rs`), `src/ir/statistics.rs`,
`src/import/lower/statistics.rs`, the `Statistics` rules of `grammar/spectre.peg`, and the
lint rule in `src/lint.rs`. Tests: `tests/test_monte_carlo.rs` (T-G1 to T-G10, T-R1 to
T-R5, the lint), `src/codegen/statistics/tests.rs` (T-N0, T-N1 on ngspice), and the Spectre
decks and runner in `tests/spectre/` (T-S1 to T-S3; T-S4 not yet written). Where it departs
from §5 and §10:

- **`rack_limit` at `u = 0`** returns `nom`, not `nom - avar`, so the nominal run is exactly
  `nom` as §5.2 promises: `(u > 0) ? nom + avar : ((u < 0) ? nom - avar : nom)`. A draw is
  never exactly 0, so the two values are unchanged.
- **Mismatch variable names** carry the subcircuit: `rack_<subckt>_<owner>_<field>_<k>`. One
  local name (sky130's `swx_vth0_delta`) recurs across a PDK's subcircuits, and files are
  converted separately, so the owner alone would collide when they are included together.
- **The lowered tables** are always written braced (`.param p__mc={agauss(...)}`).
- **T-R1 and T-R5** run on the sky130 and gf180mcu files and sections whose `.model`s have a
  Spectre master today; resistor, capacitor and level-3 diode model cards have none yet,
  a gap of their own.

**Spectre results (T-S1 to T-S3).** Run on Spectre 21.1.0.460.isr10 (a licensed host), the decks in
`tests/spectre/` through `run.py`, N = 2000: all 18 checks pass.

| Deck | Result | Expected (tolerance) | ngspice |
|---|---|---|---|
| T-S1 nominal | v(out) = 0.5, exact | 0.5 (1e-9 rel) | 0.5 |
| T-S2 agauss | mean 5.0030e-01, std 8.255e-03 | 0.5 (7.45e-4), 8.333e-3 (5.27e-4) | 4.9988e-01, 8.403e-03 |
| T-S2 aunif | std 1.438e-02 | 1.4434e-2 (9.13e-4) | 1.4708e-02 |
| T-S2 limit | two values only, std 1.2509e-02 | 1.2508e-2 (7.91e-4) | 1.2505e-02 |
| T-S3 mismatch | ρ(o1, o2) = +0.027 | \|ρ\| < 0.089 | — |
| T-S3 nested | \|ρ\| ≤ 0.04 for x1.xa/x2.xa, x1.xa/x1.xb, x1.xb/x2.xb; every std in tolerance | \|ρ\| < 0.089 | — |

So Spectre samples `mismatch` per hierarchical instance (§5.1's medium-confidence claim
holds), and its 4σ truncation is invisible at N = 2000. Spectre 21.1 keeps only the last
iteration unless `savefamilyplots=yes`; `run.py` adds it to a copy of each deck.
