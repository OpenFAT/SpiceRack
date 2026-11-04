# Phase 5 (diagnostics catalogue): status

Phase 5 of the rewrite (TESTING.md, "Golden gate"):
every refusal and drop in `src/syntax/` is a coded `Diag`, every code a
catalogue row, refusals reach the user as codes plus rendered messages, and
tests and the golden pin codes, never text. Gate: the refusal set is
unchanged (same decks, same constructs). Phase 4's state, and the Phase 7
list, stay in `PHASE4-STATUS.md`.

## The catalogue (`src/syntax/diag.rs`)

One table, `catalogue!`: each code's variant and its row
`{title, cite, explain, kind}` are one entry, so no code lacks a row and
`Code::ALL` holds every code. `Kind::Note` (the deck is still written) or
`Kind::Refusal` (it is not). Codes are never renumbered or reused.

| Class | Range | Codes | Kind |
|---|---|---|---|
| L (tokenize) | L0001–L0100 | none yet | |
| P (parse) | P0001–P0100 | P0001–P0009 (9) | Note |
| R (resolve) | R0001–R0100 | R0001–R0010 (10) | Note |
| W (render: SPICE and shared) | W0001–W0100 | W0001–W0020 (20) | Refusal |
| | | W0021–W0028 (8): an element as an error comment (CDL), a Verilog-A load, an output request, an option, a `.control` line, a waveform's extra values, text copied as written, CDL's LVS drops | Note |
| W (render: named) | W0101–W0200 | W0101–W0109 (9) | Refusal |
| | | W0110 (a sectionless `.lib` written `section=`), W0111 (a `level` that is no number, read as 1) | Note |

58 codes. Tests (`diag::tests`): every code has a row with a title, a
non-empty cite and an explanation longer than its title; every code is in
its stage's range, in stage order, ascending (a new code is appended), and
every non-`W` code is a note. `capability::tests`: every `Refuse` cell is a
`W` refusal code, every `Omit`/`Copy` cell a `W` note.

## What reaches the user

- `render`, `render_kit`, `render_text` return `Result<Written, Diags>`:
  `Written { text, notes }`, or the refusal. `Diags { diags, messages }`
  holds every diagnostic on the file (the parser's notes, or the kit's for
  its file: `P` and `R`; the reader's; the writer's), sorted by source
  position (`diag::sort`), and their messages (`diag::messages`:
  `<deck>:<line>:<column>: <code> <title>`, the line and column computed
  there, from the token). The deck is refused when any is a `Refusal`.
- A writer's capability cell carries a drop's code (`Alt::Omit(Code)`,
  `Alt::Copy(Code)`), and the writers' `ask` records it, as it records a
  refusal.
- `golden record --writer syntax` writes a `.refused` as `codes: <sorted
  codes>` then the messages; `golden diff` compares two such files by the
  code line alone (`diff::codes`, tested).

## What moved from free text (or nothing) to codes

- CDL: an element the ngspice card refuses was turned into an `* error:`
  comment and its codes dropped from the result; a Verilog-A load an
  `* error:` comment with no code. Both stay in the text (byte for byte)
  and are `W0021` notes.
- Silent drops, now notes (the bytes unchanged): LTspice's Verilog-A/OSDI
  loads (the capability cell was `Omit`, never asked) and a subcircuit's
  loads on every target but ESPice (`W0022`); `.print` on LTspice, Spectre,
  VACASK, `.save` on Xyce and VACASK, HSPICE's `.probe` under a noise run
  (`W0023`); VACASK's options on SPICE targets and Spectre, Xyce's on the
  others, VACASK's cosmetic options (`W0024`); a `.control` block's
  non-analysis lines (`W0025`, the reader's, `Deck::notes`); a waveform's
  values past its form's count (ngspice PULSE `NP`: `Wave::cut`, `W0026`);
  text kept raw copied into its own family unchecked (`W0027`); what CDL's
  layout does not write (`W0028`); a sectionless `.lib` on Spectre/VACASK
  (`W0110`); a non-numeric model `level` (`W0111`); an instance whose
  letter has no device row (`R0001`, which `resolve` dropped).
- Anchors: `W0002`, `W0003` (and `W0017`'s Xyce option) were at token 0;
  each is now at the statement holding it (`checks::expressions` returns
  each value with its statement's token).
- The parse and kit notes (`P`, `R`) never reached the render result; they
  are in it now (and in each `.refused` code line).
- The old pipeline's messages beside the codes they became, for review:
  `refusal-map.txt` (from the `--refusal-status-only` report at
  `799cf96`, before the switch: 47,845 keys, 1,080 message shapes, each
  with its code sets, counts and an example key).

## Gate

At the catalogue's last commit, `golden record --writer syntax`
(`/tmp/claude-1000/spicerack-golden/p5-3`) against `baseline-daa0ad0-fx9`
(`PHASE4-STATUS.md`: the extended baseline plus `func_stats.{sp,cir}`),
`--refusal-status-only`: IDENTICAL except the 26 listed xfails, every
target but Verilog (which stays on the old writer), the same
written/refused status per (input, target). `check-determinism --writer
syntax` IDENTICAL. The merge tier 470 tests (466 at `b05f4a0`). The record
ran in 783 s wall, most of it behind the shared heavy lock; peak RSS 3.3 GB
(2.3 GB at `daa0ad0`, before messages and notes; not profiled).

## Left

- **L class.** The tokenizer skips a `+` line with nothing to continue
  (`tokenize.rs`, `Line::Skip`) with no code: it makes no token there to
  anchor one. It needs a byte-anchored diagnostic or a zero-width token;
  no row is taken until then.
- `resolve`: an entry file past `FILES_MAX` appends nothing
  (`diag_from(None, R0009)`: no includer to anchor at).
- Checks that do not look everywhere (not drops): the `^` and statistical
  scans skip raw text, coupling, linear, POLY, switch and line cards,
  analyses and `.if` conditions (`spice/checks.rs`, `named/checks.rs`).
- Notes are not in the golden: a written deck's `.txt` is its text alone;
  `tests/test_render.rs` pins one note per code (`what_a_writer_leaves_out…`).
- A `Diag` carries no file: a kit's diagnostics are per file
  (`resolve::File::diags`), a render's are its file's.
- `named::cards::master` can note `W0111` once per call for one model (a
  model group's bins).
