# AGENTS.md

SpiceRack: build SPICE circuits in Python, netlist them for any simulator
(ngspice, HSPICE, LTspice, VACASK, Spectre, ESPice), run them and read the
results back. The core is a Rust crate (edition 2024) with PyO3 bindings
shipped as the `spicerack` Python package. `README.md` is the overview;
`grammar/README.md` is the dialect readers' contract.

## Skills to load first

Load each with the Skill tool (other agents: read
`~/.claude/skills/<name>/SKILL.md` and the companion files it names). Source:
https://github.com/OmarSiwy/NixOSConfig/tree/main/skills

| Skill | Use it for here |
|---|---|
| `data-oriented-design` | `CircuitIR` (`src/ir`), the import AST and result tables: the six questions, SoA, narrowest widths, handles |
| `codebase-clean-code` | Owners: `import` reads, `ir` holds, `codegen` writes one dialect each, `backend` runs one simulator each; one spine per direction |
| `api-design` | Every `pub` item and the Python API (PySpice-close, backend neutral): usage code first, caller owns memory and flow |
| `code-style` | File order, ≤500 code lines per file, `check_style.py` |
| `simd-loops` | Hot loops over raw-file columns (`src/rawfile.rs`): the plain-loop forms, or explicit SIMD with a scalar oracle |
| `doc-comments` | Every `pub` item |
| `ponytail` (ultra) | How little code; never what shape the data takes |
| `assertions` | Contracts, capacities, bounded loops |
| `test-plan`, `tdd` | The oracles below, the gates, a feature's definition of done |

Language:

| Skill | Use it for here |
|---|---|
| `rust` | Types, memory, `zero-cost.md`: an abstraction enters a hot loop only with an `asm_probe` verdict; explicit SIMD is `fearless_simd` |

Project-specific:

| Skill | Use it for here |
|---|---|
| `compiler-construction` | `grammar/*.peg`, `src/import` and `src/codegen`: one schema every dialect captures into, lowering to `CircuitIR`, coded diagnostics, goldens |
| `spice-engine` | Reading a simulator's result: which oracle (ngspice, VACASK, analytic) and at what tolerance |

## The method

Binding on every change, in every project.

### Code

- Five commitments: lean storage, no runtime residue, cheap change,
  contained failure, proof by test.
- Answer the six data questions before writing code. Then: one array per
  field, the narrowest width the stated range allows, typed `u32` handles
  with a sentinel, existence tables instead of flags, hot/cold split, tables
  sized once from a named capacity, a compile-time size assertion on every
  hot struct.
- An abstraction on a hot path carries an assembly probe: FOLDED, IDENTICAL,
  EQUIVALENT or NEGATIVE pass; RESIDUE fails the build. A "no residue" claim
  cites the probe, a speed claim cites a benchmark. Dispatch per batch, never
  per element.
- A seam names what varies and needs a second case today; it sits on the
  cheapest rung (concrete, free function, table, closed enum with exhaustive
  switch, comptime, request table, function-pointer table, `dyn`, plugin).
- Tables, transforms, one spine per pipeline (ifs up, fors down), one owner
  writes each table and everyone else appends requests. Three likely changes
  each have a change radius of zero; the compiler lists every other site.
- APIs start from the caller's code. The caller owns memory and flow:
  requests instead of callbacks, an immediate twin for anything retained.
  Factor at the second real instance.
- Branches: delete, then branchless, then vectorize along a lane axis. The
  `W == 1` instantiation is the scalar oracle; vectorization is verified in
  the assembly.
- GPU: SoA always, the roofline ceiling named with its arithmetic before
  tuning, data kept on the device, fixed-order reductions, the dependency DAG
  recovered before porting. "This belongs on the CPU" is a valid verdict.
- An operating error returns to the caller that can decide; a bug hits an
  assertion. Two or more split, paired assertions per function, a bound on
  every loop, input validated once at the edge, fallible work before the
  commit.
- Pass order: boundary, placement, data, zero-cost proof, contracts, tests,
  branches, lanes. Data layout beats brevity. Every pass keeps NaN gates,
  allocation-failure paths, `errdefer`s, trust-boundary validation and
  tolerances.

### Testing

- The oracle is named before the code and never recomputes the answer the
  way the code does. Cheapest first: assertions, a reference
  implementation, a round trip, a certificate, a metamorphic relation, a
  model, a golden with recorded provenance, a spec example.
- The main bug class is a wrong number with no crash, which coverage and
  fuzzing cannot see. A bug found by a person is a hole in the oracle set:
  it lands with its regression test and the oracle or generator that would
  have caught its class.
- A new test goes red on the bug before it is trusted green.
- Unsupported fails and never skips. A known failure is a named marker in
  the tree; an unexpected pass fails until the marker goes. Name lists are
  compared, never counts.
- A tolerance follows the oracle's own accuracy. Determinism is a test:
  1 job and N jobs give the same bytes, and the order is fixed, not the test.
- A flaky test is fixed or quarantined the same day, never retried
  silently.
- Tiers: pre-commit under 10 s, merge under 15 min, nightly, release. A
  release ships on numbers a script checks. The map lives in `TESTING.md`.

### Builds

- A build is a cache keyed on its declared inputs. Every producing step has
  an output argument, every checking step an exit-code check, every file
  read at run time is declared (`addFileInput`) or the step says
  `has_side_effects`. Keys hash contents, never absolute paths.
- Cut the artifact graph where edits land: one object per generated unit,
  narrow imports for shared ones, kernels out of host graphs.
- Zig edit loop: `zig build check --watch -fincremental`, one watch per
  artifact across all worktrees; the result that counts comes from a plain
  build. One global cache per toolchain store path; a worktree symlinks its
  `.zig-cache` to the main checkout's.
- Rust: flags live in `.cargo/config.toml`, never a one-off `RUSTFLAGS`;
  `check`, `build`, `test` and rust-analyzer agree on features, profile and
  target directory (rust-analyzer gets its own).
- Nothing collects a cache. Keep a byte budget per cache and delete old
  entries only while no build runs (the `long-term-garbage-collection`
  skill); a warm cache is kept, never wiped by routine.
- Gate on the exit code: `cmd | tail` swallows it, so use
  `exit "${PIPESTATUS[0]}"`.

### Maintaining

- `main` is green by machine: work lands through a pull request whose merge
  result passed the required checks. No `dev` branch; topic branches are
  deleted after the merge.
- Bots never push to `main`. Generated output ships as a release asset or
  through a gated pull request.
- Every dependency is pinned by commit and content hash, the toolchain by
  `flake.lock`, actions by SHA. A third-party input you hash is mirrored
  somewhere you control.
- The changelog is written by hand, one line per change citing its issue,
  every number in it measured. A release refuses a version mismatch, builds
  every target, checks every artifact and smokes the bundle.
- CI holds: the merge gate, every target compiled, docs that execute,
  reference suites gated by name, a nightly, the downstream contracts, the
  release job.

## Building

- Every build runs inside `nix develop -c …`; `nix develop .#testing` adds
  ngspice, OpenVAF, VACASK and the sky130/gf180 PDKs the integration tests
  run against. CI runs both suites in `.#testing`.
- Rust: `cargo test` (name one test target with `--test <file>` while
  iterating). Python: in a venv with `--system-site-packages`,
  `maturin develop` then `python3 -m pytest`.
- A change to the public Python API updates `docs/` and the user-facing
  skill in `skills/spicerack/` in the same commit.
- Run one cargo process per agent, and don't build while `free -g` shows
  under 6 GB available.

## Testing

- Every test names its oracle: the independent thing that says the output is
  wrong. A netlist is checked by running it on the real simulator or against
  a golden with provenance, never by re-deriving it the way codegen does.
- Unsupported fails and never skips: a missing simulator in `.#testing` is a
  failure.
- A golden that disagrees with an independent oracle is a bug in the code
  that wrote it. Fix the code, regenerate the golden, and say so in the commit.

## Committing

Commit at every green step: both suites passing, `cargo fmt --check` clean.
The message says what changed and in which owner. Never commit red to `main`.
