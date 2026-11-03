# Compiler architecture reference for the transpiler rewrite

**The target.** SpiceRack becomes a transpiler shaped like the front half of
Zig:

1. A tokenizer, one lexical table per dialect.
2. One parser.
3. One AST.
4. A renderer that prints the AST in any dialect, netlists and analyses
   alike.

There is no optimization and no lowering.

**What this note covers.**

- §1–§4 read Zig's tokenizer, `Parse`, `Ast` and `Ast/Render` (`zig fmt`) in
  depth, including how comments and formatting survive a round trip.
- §5 covers AstGen's name resolution.
- §6 covers Sema, AIR and codegen briefly, because a transpiler can skip
  them.
- §7–§9 cover ESPice, VerA, LLVM, tree-sitter and Pandoc.
- §10 and §11 answer the two questions the brief asked.
- §12 lists what transfers to SpiceRack.

Every claim is cited. The path prefixes are:

| Prefix | Means |
|---|---|
| `zig/` | `/nix/store/90x7n96l24rm9qv03zfyz4zkb4dbj7g9-zig-0.17.0/lib/std/zig/` |
| `Z/` | `https://github.com/ziglang/zig/blob/0.17.0/` (tag 0.17.0 = commit `7647adab80dd`) |
| `ESPice/` | `/home/omare/Documents/Projects/EDA-Tools/Simulator/ESPice/` |
| `VerA/` | `/home/omare/Documents/Projects/EDA-Tools/Simulator/VerA/` |
| `SR/` | this repository at `9efaefc` |
| `P/` | `https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/` |

Three notes on sources:

- Zig 0.17 has no `render.zig`. The renderer is `zig/Ast/Render.zig`,
  exposed as `Ast.Render` (Ast.zig:229).
- The nix store's `parser_test.zig` is empty, and there is no `src/fmt.zig`
  in it. Both were read with `git show 0.17.0:` from a local clone and are
  cited as `Z/lib/std/zig/parser_test.zig` and `Z/src/fmt.zig`.
- A `Z/…#Lnnn` link points at the tagged blob GitHub serves. Its line
  numbers come from that tag but were not opened in a browser.

---

## 1. Tokenizer (`zig/tokenizer.zig`)

### 1.1 What it produces

**Input and output.** The input is a `[:0]const u8`, with a UTF-8 BOM
skipped in `init` (334-340). Each `next()` call returns one
`Token{tag, loc: Loc{start, end}}` (3-10).

**Keywords.** An identifier is lexed first, then looked up with
`getKeyword` (655-664) in a comptime `StaticStringMap(Tag)` (12, 61).

**Spelling is a function of the tag.** `Tag.lexeme()` returns the fixed
spelling of every fixed token. It returns `null` only for tags whose text
varies (184-196):

- invalid;
- identifier;
- string, multiline-string line, char;
- eof;
- builtin;
- number;
- doc_comment, container_doc_comment.

`symbol()` gives a display name for diagnostics (308).

### 1.2 Trivia

The state machine is a `State` enum (342-387) run as `state: switch` with
`continue :state .x` (399, 498). The states that handle trivia are `start`,
`expect_newline`, `slash`, `line_comment_start`, `line_comment`,
`doc_comment_start`, `doc_comment`, `backslash`,
`multiline_string_literal_line` and `invalid`.

**Whitespace** (`' ' \n \t \r`) only advances `loc.start` (413-417). A lone
`\r` must be followed by `\n` (`expect_newline`, 501-518).

**Comments** are told apart by their first bytes:

| Spelling | Result | Where |
|---|---|---|
| `//` | enters `line_comment_start` | 928-937 |
| `//!` | a `container_doc_comment` token | 954 |
| `//` then a newline | dropped | 957-961 |
| `///` | a `doc_comment` token | 970-987 |
| `////` | a plain comment again | 970-987 |

A plain comment produces no token. At its newline the tokenizer resets
`loc.start` and continues in `.start` (991-1015). Doc comments are tokens
because the parser attaches them to declarations. Control bytes inside any
comment send the tokenizer to `.invalid` (950-952, 1009-1011).

### 1.3 It never fails

`.invalid` eats bytes up to the next `\n`, then emits one `.invalid` token
(520-531). The doc comment on `next()` says: "After this returns invalid,
it will reset on the next newline … An eof token will always be returned at
the end" (389-390). A NUL byte before `buffer.len` is invalid; the NUL at
`buffer.len` is EOF (404-415).

### 1.4 Lexing is loose, validation comes later

Numbers are lexed loosely: the `.int` state accepts letters and `_`
(1029-1040). Errors such as a duplicate exponent are found later, in
AstGen (AstGen.zig:8605).

The SPICE analogue is that `1meg`, `1M`, `1e-3u` and `30M` all lex as
`Number`. What the suffix means belongs to the dialect table, read when
the number is rendered.

### 1.5 The Ast keeps only the start offset

`Ast.parse` tokenizes everything up front and stores
`TokenList = MultiArrayList(struct{tag, start: u32})` (Ast.zig:28-31,
149-170). It is pre-sized at `source.len/8` (153-155).

To get a token's text, `tokenSlice` returns `lexeme()` if the tag has one,
and otherwise re-runs the tokenizer from `start`, asserting that the tag
matches (Ast.zig:279-295). So each token costs 5 bytes, and its text and
end are derived from the source.

---

## 2. Parse (`zig/Parse.zig`)

### 2.1 Discipline and state

The parser is predictive LL(k) and "never allowed to backtrack", which
gives worst-case linear time (3-14). Its state is
`{gpa, source, tokens, tok_i, errors, nodes, extra_data, scratch, recover}`
(18-26). The error set is `ParseError || Allocator.Error` (16).

### 2.2 Entry points

- `Ast.parse(gpa, source, mode) Allocator.Error!Ast` is the public entry
  (Ast.zig:149). The only error it returns is out-of-memory. Syntax errors
  are data in `Ast.errors`.
- `parseRoot` appends the root node, index 0, then calls
  `parseContainerMembers` (264). It stores the members' span in `data[0]`
  and warns if the input does not end at eof (208-236).
- `parseZon` is a second mode (238).
- Member dispatch is a `switch` on the current token tag (285-330).

### 2.3 Lists: one scratch stack, then `listToSpan`

Every list is built on one shared `scratch` stack:

1. Record `scratch_top = p.scratch.items.len`.
2. `defer p.scratch.shrinkRetainingCapacity(scratch_top)`.
3. Append the children.
4. Slice `scratch.items[scratch_top..]`.

Examples are `parseBlock` (1946-1955) and `parseContainerMembers` (264-266,
494). A nested list stacks above its parent's slice, so no list allocates.

`listToSpan` copies the slice into `extra_data` and returns
`SubRange{start, end}` (69-75; Ast.zig:3889-3894).

### 2.4 Choosing the tag from the child count

`parseBlock` shows the pattern (1958-1974):

- **0–2 children** use the `*_two` tag, with both children inline as
  `opt_node_and_opt_node`.
- **More children** use the plain tag, with an `.extra_range` into
  `extra_data`.
- **The trailing separator** is read off the token stream:
  `tokenTag(p.tok_i - 2) == .semicolon` selects `block_two_semicolon` or
  `block_semicolon`.

Calls follow the same rule:

- `call_one_comma` / `call_one` (2984);
- `call_comma` / `call` (2992).

Container members return `Members{len, data, trailing}` (494-512).

`if` shows the payload choice:

- `if_simple` holds `node_and_node{cond, then}`.
- `@"if"` holds `node_and_extra{cond, addExtra(Node.If{then_expr, else_expr})}`.

Both are built in `expectIfStatement` (1151-1207). `Node.If` is at
Ast.zig:3896-3899.

### 2.5 `main_token` and `extra_data`

**`main_token`** is the anchor each tag's doc comment names:

- `{` for a block (Ast.zig:3777-3790);
- `(` for a call (3404, 3416);
- `fn` for a prototype (3533-3571);
- `if` (1165).

`firstToken`, `lastToken` and diagnostics hang off it.

**`addExtra(struct)`** reflects over a struct's fields with a comptime loop.
Each node, optional, token or extra index is stored as one u32. Any other
field type is a `@compileError` (105-123). `Ast.extraData(index, T)` is the
inverse (Ast.zig:305-322).

Example payload structs:

- `If{then_expr, else_expr}` (3896);
- `GlobalVarDecl{type_node, align_node, addrspace_node, section_node}`, all
  optional (3906-3915);
- `While` (3929);
- `FnProtoOne` (3945) and `FnProto` (3958).

`Node.Data` is an untagged union of 19 index-pair shapes, and the tag says
which applies (3843-3863). Comptime asserts hold `Data` to 8 bytes and
`Tag` to 1 byte (2978-2985).

### 2.6 Error recovery

**`warnMsg`** appends an error and continues.

- For the `expected_*` family, if the offending token starts a new line,
  the error is re-anchored to the previous token with
  `token_is_prev = true`. The error then points at the end of the line that
  lacks the `;` (139-182).
- With `recover == false`, every warning becomes `error.ParseError` (183).

**`fail*`** calls `warnMsg`, then returns `error.ParseError` (187-206).

**`expectSemicolon(tag, recoverable)`** warns and carries on as though the
`;` were there, unless the caller marks it unrecoverable (3390-3397).

**Resynchronisation.**

- `findNextContainerMember` (515-575) skips tokens, tracking bracket depth,
  until a declaration starts.
- `findNextStmt` (578-608) tracks `{}` depth. It stops after a level-0 `;`,
  or before a level-0 `}` or eof.
- `expectStatementRecoverable` loops: on `ParseError` it calls
  `findNextStmt`, returns null at `}`, propagates the error at eof, and
  otherwise continues (1135-1149).
- `parseContainerMembers` catches `ParseError` and resyncs (311-313, 483).
- `parseRoot` swallows a top-level failure after
  `assert(p.errors.items.len > 0)` (217-221).

**Zombie nodes.** `reserveNode` lets a parent be allocated before its
children. On failure, `unreserveNode` turns the slot into an
`unreachable_literal` (83-103).

**`ParseOptions.recover`** (default true) is the one switch between "collect
everything" and "stop at the first error" (Ast.zig:141-144).

---

## 3. Ast (`zig/Ast.zig`)

### 3.1 Layout

- `Ast{source, tokens, nodes, extra_data, mode, errors}` (17-25).
- `Node{tag, main_token, data}` (2914-2917). The node list is pre-sized at
  (tokens + 2) / 2 (194-197).
- `Index`: the root is 0. `OptionalIndex`: none is maxInt. `Offset` and
  `OptionalOffset` are relative i32s (2920-2976).
- `rootDecls` is the root's `extra_range` slice (325-331).

### 3.2 Each tag documents itself

Every tag's doc comment states three things:

- its syntax;
- which `data` variant it uses and what each slot means;
- what `main_token` is.

For example, `call_one` reads: "`a(b)`… The `data` field is a
`.node_and_opt_node`: 1. … function expression. 2. … first argument… The
`main_token` field is the `(` token." (3397-3404).

The `*_comma` / `*_semicolon` variants say only "Same as X except there is
known to be a trailing comma" (3405-3407, 3417-3419, 3778-3790). The enum
says why they exist: "The FooComma/FooSemicolon variants exist to ease the
implementation of `Ast.lastToken()`" (2987-2988).

A leaf literal's `data` is unused. "Most identifiers will not have explicit
AST nodes" (3589-3601): an identifier is usually just a token index inside
another node.

### 3.3 `firstToken` and `lastToken`

Both are a `while (true) switch (tag)` that descends to the leftmost or
rightmost child (587, 863). An `end_offset` counts the closing punctuation
that is not itself a node:

- `call_one` adds 1 "for the rparen" and descends into its argument
  (1090-1096).
- The `*_two_comma` / `*_semicolon` group adds 2 "for the comma/semicolon +
  rbrace/rparen" (1133-1150).

The tag variants are what let this work with no stored end token.

### 3.4 `full.*` views

`pub const full` (2449) holds decoded structs such as `VarDecl`, `If`, `Call`
and `FnProto`.

- `fullIf` maps both `if_simple` and `@"if"` to one `full.If` (2296-2302, by
  way of `ifSimple` 1376 and `ifFull` 1387).
- `fullCall` takes a caller's `buffer: *[1]Node.Index`, so a one-argument
  call needs no allocation (2424; `callOne` 1957).
- Render consumes only these views, e.g. `renderIf(r, Ast.full.If, …)`
  (Render.zig:1351) and `renderArrayInit(r, Ast.full.ArrayInit, …)`
  (Render.zig:2104).

So the compact encoding has several tags per construct, but there is one
render path per construct.

### 3.5 Errors and locations

**`Ast.Error`.** `Error{tag, is_note, token_is_prev, token, extra:
union{none, expected_tag, offset}}` (2830-2841).

- It has 58 tags (2843-2908). The last plain ones are notes
  (`previous_field`, `next_field`, …).
- `renderError(tree, err, w)` is the only place error text is made: one
  `switch` on the tag (337).
- `errorOffset` adds the token's length when `token_is_prev` is set
  (236-238).

**Locations.**

- `tokenLocation(start_offset, token)` scans for newlines and returns
  `{line, column, line_start, line_end}` (240-275).
- `nodeToSpan` is `tokensToSpan(first, last, main)` (3980-3987). For a node
  that spans lines, it trims to the end that shares a line with the main
  token (3993-4011).
- `tokensOnSameLine` checks for a `\n` between two token starts
  (1299-1302).
- `getNodeSource` slices a node's source text (1304-1310).

---

## 4. Render (`zig/Ast/Render.zig`): `zig fmt`

### 4.1 Top-level structure

`Ast.render(tree, gpa, w, fixups)` (Ast.zig:231) calls `renderTree`. Its
first line is `assert(tree.errors.len == 0) // Cannot render an invalid
tree.` (86-87). It then:

1. renders the comments before token 0, `renderComments(0, tokenStart(0))`
   (97-99);
2. renders the container doc comments (101-103);
3. renders `renderMembers(rootDecls)` (105-107);
4. copies the rest of the source raw if a `zig fmt: off` region is still
   open at EOF (116-118).

The state is `{gpa, ais, tree, fixups}` (13-16). Indentation is 4 spaces, or
2 inside asm (18-19).

**Members.** `renderMembers` calls `renderExtraNewline` between members
(122-133). `renderMember` does three things in order (141-151):

1. checks the `omit_nodes` fixup;
2. renders the doc comments, `renderDocComments(firstToken(decl))`;
3. switches on the tag.

Leading `pub`/`extern`/`export`/`inline` are found by scanning token tags
backwards from `fn` (156-175). The AST stores no modifier list.

**Expressions.** `renderExpression` checks the `replace_nodes_with_*`
fixups, then switches on the tag (314-324).

### 4.2 Tokens are the only output primitive

`renderToken(tok, space)` writes `tokenSliceForRender`, which is the
source bytes with doc comments trimmed at the end (3162-3171). It then calls
`renderSpace` (2714-2720).

`Space` is the whole spacing vocabulary (2686-2712): `none`, `space`,
`newline`, `comma`, `comma_space`, `semicolon`, `maybe_space`,
`comma_maybe_space`, `skip`. `.comma` means "insert a comma if the next
token is not one, then a newline". `renderSpace` writes `,` when it is
missing (2741-2743) and consumes it when it is present (2760-2764).

### 4.3 Comments live in the gaps between tokens

The AST stores no comments at all. `renderSpace` calls
`renderComments(tokenEnd, nextTokenStart)` on the byte gap after every
token (2747-2751). If that returns true, the caller's own space or newline
is suppressed (2752-2756).

**`renderComments`** (3005-3067):

- finds each `//` in the gap and writes it with trailing whitespace trimmed;
  the text is otherwise unchanged (3010-3017, 3055-3056);
- emits one blank line for two or more newlines before the first comment;
- keeps a newline directly before a comment;
- gives a comment on the same line as the previous token exactly one space
  (3019-3035);
- keeps a blank line after the last comment, except at EOF (3059-3064).

**Probes.**

- `hasComment(a, b)` looks for a `/` byte in any inter-token gap in [a, b)
  (2972-2981). This works because in those gaps a `/` can only start a
  comment.
- `hasTrailingComment` checks for a `/` before the next `\n` (3173-3183).
- `hasDocComment`, `hasMultilineString` and `anythingBetween` are nearby
  (2972-3005, 3173-3189).

**Doc comments.** `renderDocComments(end_token)` walks backwards over
`doc_comment` tokens. They are owned by no node. It renders each with
`.newline` (3102-3131). `renderContainerDocComments` is the container
version (3133-3145).

The token gaps plus the source bytes are therefore the comment side table.

### 4.4 Blank lines

`renderExtraNewlineToken` scans backwards from a token through whitespace.
Two `\n` emit one blank line, so runs of blank lines collapse to one. It
skips the check when a comment directly precedes the token, because
`renderComments` already handled it (3074-3099).

It is asked before members (133), struct-init fields (2076), array rows
(2284) and doc comments (3122-3124).

### 4.5 One-line versus multi-line is the author's choice

The layout decision is read from the tokens, never stored:

| Construct | Multi-line when | Where |
|---|---|---|
| Struct init | `trailing_comma or hasComment(lbrace, rbrace)`, else one line with no trailing comma | 2067-2101 |
| Array init, one element with nothing before `}` | never; stays inline | 2129-2139 |
| Array init | a trailing comma, a comment, or a multiline string; else one line | 2141-2157 |
| Fn proto | `trailing_comma or hasComment(lparen, rparen)` | 1746 |
| Container | anything but fields with no comments or doc comments | 2310-2335 |

After `=` or an infix operator, a line break in the source is kept:
`rhs_seperate_line = !tree.tokensOnSameLine(op_token, op_token + 1)`
(500-502, 541).

`rendersMultiline` asks whether a node would wrap. It test-renders the node
into a writer that rejects newlines (966-998).

### 4.6 Array column alignment (2159-2292)

1. Render each element into a sub-stream. Its width is the byte length
   without the trailing comma (2165-2211).
2. The row size is the number of elements on the first source line. A
   trailing comma with everything on one line forces a row size of 1
   (2215-2229).
3. Sections end at an element that has a trailing comment (2232-2250).
4. Column widths are the per-column maximum (2257-2272).
5. Write each element with `.comma_space` plus `col_width - w` spaces of
   padding, and the row's last element with `.comma` (2274-2292).

The test "array literal with hint" shows the author's first-row line break
setting the grid (Z/lib/std/zig/parser_test.zig#L1903-L1960).

For SpiceRack this is the model for aligning `.param` or `.model` key
columns: render to a sub-stream, measure, then pad.

### 4.7 Multiline strings

Each `\\` line token is rendered with `.newline` (337-349), then the next
item is dedented (351-360). Many sites switch to `.none` or `maybe_space`
when the next token is a multiline line (415, 501, 541, 569).

### 4.8 `zig fmt: off` / `on`

1. On `// zig fmt: off`, `renderComments` writes the canonical spelling and
   sets `ais.disabled_offset = index` (3049-3051).
2. While `disabled_offset` is set, every write is a no-op (3349-3389, 3512),
   but indentation is still tracked (3299-3305).
3. On `// zig fmt: on`, the renderer writes
   `source[disabled_offset..comment_start]` raw through
   `writeFixingWhitespace`, which only turns tabs into spaces and drops
   `\r` (3191-3196). It then writes the canonical `// zig fmt: on\n`
   (3040-3048).

This is the exact model for SpiceRack's verbatim regions: an HSPICE block
the parser does not model, or a Spectre `simulator lang=` island rendered
back into its own dialect.

### 4.9 AutoIndentingStream (3280-3524)

**The contract.** Indentation rises by at most one level per line, however
many scopes open on that line (3280-3291). Comments are indented by the
space stack (3292-3295).

**Fields.** `disabled_offset`, `indent_count`, `indent_stack`,
`space_stack`, `space_mode`, `current_line_empty`, `applied_indent`
(3297-3315). `IndentType` is `normal`, `after_equals`, `binop` or
`field_access` (3317-3322).

**API.**

- `writeAll`, `print`, `writeByte`, `splatByteAll` (3348-3370);
- `insertNewline` and `maybeInsertNewline` (3388, 3394);
- `pushSpace` and `enableSpaceMode` (3451, 3460);
- `pushIndent`, which primes an indent without writing it (3479), and
  `popIndent` (3489);
- `applyIndent` (3509).

The renderer never counts columns itself.

**For SpiceRack.** The stream is where a dialect's line-length limit and
continuation style belong: SPICE's `+` prefix, Spectre's trailing `\`. The
renderer asks for a break, and the stream spells it.

### 4.10 Fixups: edits as a side table, the AST untouched (28-84)

| Fixup | Key | Effect |
|---|---|---|
| `unused_var_decls` | mut token | insert `_ = x;` |
| `gut_functions` | fn node | the body becomes `@trap()` |
| `omit_nodes` | node | skip the node |
| `replace_nodes_with_string` | node | print a string instead |
| `append_string_after_node` | node | print a string after the node |
| `replace_nodes_with_node` | node | print another node |
| `rename_identifiers` | name | print a new name |
| `rebase_imported_paths` | — | prefix `@import` paths |

The hooks are at 149, 317-323, 935-941 and 2811-2815.

`zig fmt` passes `.{}`, no fixups (Z/src/fmt.zig#L370). The callers that use
them:

- `zig reduce` expresses each transformation as fixups and re-renders
  (`Z/lib/compiler/reduce.zig` L133-137, L180-183, L215-222, L312-339).
- `zig fetch --save` appends a dependency to `build.zig.zon` with
  `append_string_after_node` (`Z/lib/compiler/Maker.zig` L1837,
  L1930-1932).
- translate-c renders an Ast it built itself
  (`Z/lib/compiler/translate-c/Translator.zig` L317-322).

So Zig makes source-to-source edits without an AST rewriter: a pass
computes a side table, and the one renderer applies it.

### 4.11 What is canonicalised and what is preserved

**Preserved verbatim:**

- number, char and string literals, since they go through `renderToken`;
  `0x1.a827999fcef32p+1022` keeps its spelling (330-335;
  Z/lib/std/zig/parser_test.zig#L2365);
- comment text, apart from trailing whitespace;
- `zig fmt: off` regions;
- the author's one-line or multi-line choice (§4.5);
- the author's array row width (§4.6).

**Canonicalised:**

- `@"name"` is unquoted when its contents are a valid identifier that is
  not a keyword (2800-2912);
- escapes are normalised (2911-2913);
- commas and semicolons are inserted (2741, 2797);
- indentation and spacing within a line;
- runs of blank lines collapse to one.

### 4.12 The round-trip gate

**`zig fmt` refuses broken input.** A file with parse errors is never
rendered: its errors are printed and the process exits 2
(Z/src/fmt.zig#L155-L157, #L318). `--ast-check` also runs AstGen and
reports its errors (#L132-L152).

**`--check` compares bytes.**

- On stdin, `--check` exits with
  `@intFromBool(!mem.eql(u8, formatted, source_code))`
  (Z/src/fmt.zig#L159-L164).
- For a path, equal bytes leave the file untouched. Otherwise `--check`
  prints the path and fails, and normal mode writes the file atomically
  (#L370-L386).

**The tests** are in `Z/lib/std/zig/parser_test.zig`:

- `testParse` renders, then sets `anything_changed = !mem.eql(formatted,
  source)` (#L7363-L7391).
- `testTransform(src, expected)` asserts the output bytes, and that the
  change flag matches what was expected (#L7394-L7421).
- `testCanonical(src) = testTransform(src, src)` (#L7422-L7424): a
  canonical input must come back byte for byte.
- `testError` asserts the exact list of error tags (#L7428-L7441).
- Both transforms run under `checkAllAllocationFailures` (#L7417-L7420).
- A fuzz test feeds generated token streams to `parseTokens`
  (#L7443-L7453).
- There are 265 `testCanonical`, 87 `testTransform` and 44 `testError`
  calls.

No separate test asserts `render(render(x)) == render(x)`. Idempotence holds
because each `testTransform` expectation is itself canonical input.

### 4.13 How Zig keeps comments and formatting through a round trip

1. **Tokens keep offsets into the original bytes.** They store no text and
   no end (§1.5).
2. **Comments are never parsed into the tree.** They are re-read from the
   byte gap between consecutive tokens while rendering (§4.3).
3. **Blank lines are re-read the same way, and collapsed to one** (§4.4).
4. **Layout intent is re-read from tokens:** a trailing comma, a comment
   inside the brackets, a line break after `=`, the first-row width (§4.5,
   §4.6).
5. **Literals are written as spelled** (§4.11).
6. **Anything the formatter must not touch is a raw byte range** (§4.8).
7. **Edits are a side table, so unedited nodes keep all of the above**
   (§4.10).

---

## 5. AstGen's name resolution: a pass that resolves and does not rewrite

This section bears on question (1) in §10.

### 5.1 The scope chain

`Scope{tag}` is an intrusive linked list (AstGen.zig:11103-11164). Its tags
are `gen_zir`, `local_val`, `local_ptr`, `defer_normal`, `defer_error`,
`namespace` and `top`. `parent()` walks the list (11125-11134).

- **`LocalVal`** holds `{parent, inst, token_src, used, discarded,
  name: interned, id_cat}` (11178-11199).
- **`LocalPtr`** adds `used_as_lvalue` and `maybe_comptime` (11202-11225).
- **`Namespace`** holds `decls: map(name → decl node)`, kept "for the
  purposes of reporting name shadowing", plus `captures` (11238-11262).
- **`IdCat`** gives user-visible kinds such as "function parameter" and
  "local constant" (11166-11173).

### 5.2 The pre-scan makes forward references work

`scanContainer` runs before a container's members are lowered (struct
4900, union 5200, enum 5420, opaque 5578). It puts every declaration name
into `namespace.decls` "even if there are compile errors. This avoids
incorrect 'undeclared identifier' errors later on." (13098-13102).

From syntax and interned names alone, it reports:

- duplicate names, with a note on each repeat (13104-13119, 13198-13228);
- "name shadows primitive" (13128-13139);
- "declaration 'x' shadows local from outer scope" (13141-13175).

### 5.3 Lookup

`identifier()` resolves primitive names and `iN`/`uN` from a table before
any scope lookup (8106-8135). `localVarRef` then walks the scope chain by
interned name (8194-8348):

- **A local hit** marks the local `used` and returns its instruction
  (8213-8285).
- **A namespace hit** is recorded, and the walk continues. A second hit is
  an "ambiguous reference" (8291-8305).
- **No hit by `top`** is "use of undeclared identifier" (8310-8313).
- **A declaration found** is emitted as `decl_ref` or `decl_val` by name
  string, not by index (8315-8347). The comment: "Decl references happen
  by name rather than ZIR index so that when unrelated decls are modified,
  ZIR code … can be unmodified" (Zir.zig:434-440).

### 5.4 Local checks

`detectLocalShadowing` (12788-12875) and `checkUsed` at scope exit
(3023-3055) report "unused …", "pointless discard" and "local variable is
never mutated".

### 5.5 What is left to later stages

AstGen leaves to Sema everything that needs types, field resolution or
another file:

- `a.b` is emitted by name (`addFieldAccess`, 6017-6037).
- `@import` only checks that its operand is a string literal and records it
  in the `imports` table (9073-9103; Zir.zig:53-62).

Zig's name pass is therefore purely syntactic and per file. It answers
"which declaration does this name denote, and is it declared and unique?"
with no types, and it records cross-file references as names for a later
stage to resolve.

---

## 6. Sema, AIR and codegen: what a transpiler skips

**Sema** turns untyped ZIR into typed AIR. It does type checking,
compile-time evaluation and safety-check generation
([Z/src/Sema.zig#L1-L6](https://github.com/ziglang/zig/blob/0.17.0/src/Sema.zig#L1-L6)).
It interns every type and value in `InternPool`, so equality is index
equality
([InternPool.zig#L3938-L3945](https://github.com/ziglang/zig/blob/0.17.0/src/InternPool.zig#L3938-L3945)).
It records each error in a table keyed by analysis unit, and a separate
`transitive_failed_analysis` set stops cascades
([Sema.zig#L2697-L2734](https://github.com/ziglang/zig/blob/0.17.0/src/Sema.zig#L2697-L2734),
[#L35631-L35646](https://github.com/ziglang/zig/blob/0.17.0/src/Sema.zig#L35631-L35646)).

**AIR** is one SoA instruction list per function. Its tags carry meaning
(`add`, `add_wrap`, `add_sat`), not syntax
([Air.zig#L46-L69](https://github.com/ziglang/zig/blob/0.17.0/src/Air.zig#L46-L69)).

**Codegen** runs several backends from the same AIR. Capability is data in
two places:

- a feature × backend table that Sema consults
  ([target.zig#L960-L997](https://github.com/ziglang/zig/blob/0.17.0/src/target.zig#L960-L997));
- a per-backend set of AIR rewrites applied before the backend runs
  ([codegen/c.zig#L26-L38](https://github.com/ziglang/zig/blob/0.17.0/src/codegen/c.zig#L26-L38),
  [Air/Legalize.zig#L330-L341](https://github.com/ziglang/zig/blob/0.17.0/src/Air/Legalize.zig#L330-L341)).

**Why a transpiler can skip all three.** A netlist transpiler never
evaluates expressions, infers types, or emits machine operations. Its
output is source text, so everything after AstGen's name pass only serves
execution.

**Two ideas survive the skip.**

1. Keep errors in a table, with transitive failures suppressed. That is
   §12.6.
2. Keep "can target T express construct C" as one table rather than
   branches scattered through the writers. That is §12.4.

---

## 7. ESPice: what its three dialects teach

ESPice is not "a tokenizer per dialect feeding one parser". It has a line
and field layer per dialect, one card reader instantiated per dialect, and a
Spectre-to-SPICE text translator in front of that reader.

**Selection.** `--tokenizer=` is a comptime `StaticStringMap`
(`ESPice/src/frontend/dialect.zig:16-22`). Without it, dialect detection is
heuristic and tested against the corpus (`dialect.zig:30-84, 110-139`).

**Per-dialect lexical facts.** Each of `lines_ngspice`, `lines_hspice` and
`lines_spectre` exports the same constants (`lines.zig:45-51`):

- `title_line`;
- `remainder_ops`;
- `unary_not`;
- `fold_case` (false for Spectre, `lines_spectre.zig:35`);
- `Split`, the quote and brace options (`lines_ngspice.zig:34`,
  `lines_hspice.zig:42`, `lines_spectre.zig:37`);
- a `Lines` iterator of 120–170 lines handling comments and continuation;
- `parseNum`.

**Shared layers.**

- Physical lines, with the comment "all dialects share physical lines;
  comment and continuation rules stay local" (`physical_lines.zig:47`).
- The field splitter, `Fields(quotes, braces)` (`fields.zig:25`).
- One dot-card table for all three dialects (`card_table.zig:51, 76`).
- One expression compiler, which sees the dialect only through three
  constants (`expr_compile.zig:131, 201, 235`).

**What went well.**

- The lexical differences turned out to be facts plus one small iterator.
- The old line readers were kept in-tree as a byte-exact oracle during the
  rewrite (`src/frontend/tests/lines_oracle.zig:1-3`).

**What went badly.**

- **Spectre is translated to SPICE text and parsed twice.** That gives two
  tokenizers, a parallel `kinds` column, numbers re-printed through `{e}`
  (`spectre.zig:1-8, 37-42, 134-135`), and lost spelling.
- **The dialect leaks below the line layer.** There are 37 runtime
  `dialect ==` checks in 18 files. The per-letter rules are copied between
  `reader_devices.zig:116-135` and `reader_replay.zig:169-180`.
- **The IR stores `Deck.dialect`** (`card_rows.zig:405`).
- **The include expander has its own line reader** and misses `+`
  continuations (`source_search.zig:24`; `docs/using/dialects.md:213`).
- **Errors stop at the first card,** with no codes or spans
  (`reader_directives.zig:357-361`).
- **The grammar oracle was deleted** (6c015c10), leaving about 50 dangling
  citations. Those grammars survive as `SR/grammar/*.peg`
  (`SR/grammar/README.md`, last paragraph).

---

## 8. VerA: catalogue, fact tables, refusals by clause

The catalogue is `VerA/lib/diag/code.zig`, with rows in
`lib/diag/codes/class*.zig`. There is no `lib/diag_code.zig`. The IR is in
`src/ir/`.

### 8.1 Catalogue

- `Code` is an exhaustive `enum(u16)` spelled `E`/`W` plus four digits
  (`code.zig:48-50, 64-181`).
- Severity comes from the first letter (`lib/diag/level.zig:130-134`).
- The first two digits are the stage class (`code.zig:57-63`).
- Codes are never renumbered or reused (`:52-56`).

**Rows.** A row is `Info{title, lrm, explain}`. The `lrm` clause is printed
under every diagnostic, so message text never spells the clause
(`code.zig:20-46`).

**Totality.** A comptime loop binds the rows into an
`EnumArray(Code, Info)`. A duplicate row or a code with no row is a
`@compileError` (`code.zig:216-232`).

**The sink** is one `Bag` per run (`lib/diag/bag.zig:47`):

- every stage reports a `Span` into the same text (`lib/diag/root.zig:4-6`);
- rows are deduplicated by (code, file, start) (`lib/diag/builder.zig:127-136`);
- at most 64 are kept, and the dropped count is printed "so a truncated run
  never looks complete" (`bag.zig:32-35`).

**Fixtures pin codes, not titles** (`tests/suite/Reject.zig:192-212`).
`reject-only` fails on a second, unnamed error (`Reject.zig:77-103`).

### 8.2 Fact tables

**Rule:** one row per enum tag, total, with no defaults.

**IR opcodes.** One `Info` per opcode replaces per-pass `switch` arms
(`src/ir/opcode.zig:201-234`). Its cross-row invariants are checked at
comptime (`:318-341`).

**Meaning and spelling have different owners.** "Target spellings live in
`src/codegen/device/codegen/opcode_zig.zig`: `ir` does not know the target
is Zig" (`src/ir/opcode.zig:5-6`). The writer side has its own total
`Spell` table (`opcode_zig.zig:1-25`).

**Dialect versions form one ordered chain.** Whether a word is reserved in
dialect S is one compare, `intro(W) <= S` (`lib/frontend/keyword.zig:21-33,
365-372`). The lexer ignores the dialect, and the parser decides whether a
keyword may stand as an identifier (`keyword.zig:28-32`).

### 8.3 Refusals by clause

- **Every refusal is a code whose row cites the clause.** For example,
  E1205 → "IEEE 1364-2005 12.2.1" (`lib/diag/codes/class12a.zig:60-63`).
- **A subset refusal reports, and the construct is parsed anyway,** "so the
  one error is the only one" (`lib/frontend/parser.zig:263-310`).
- **A backend can only raise codes from its own class.** `Emitter.refuse`
  asserts at comptime that the code is in E1200..E1278
  (`src/codegen/digital/rtlil_emitter.zig:175-181`).
- **SPICE cards inside a subckt are refused, never skipped,** because
  "skipping either would change the circuit, a dropped R being an open"
  (`lib/frontend/spice_cards.zig:20-25`).
- **Strict known-gap list.** `tools/vera.xfail` fails the run on an
  unlisted failure, and also on a listed test that now passes
  (`tools/vera.xfail:1-5`).

### 8.4 Shape

- **Lexer rows** are `{tag: u8, start: u32}`. It never fails except on
  out-of-memory (`lib/frontend/lexer.zig:1-16`).
- **AST** is SoA with interned `StrId`s and one shared u32 pool
  (`lib/frontend/ast.zig:5-9`).
- **Recovery** resyncs at `;` or a closing keyword (`parser.zig:549-569`).
- **SPICE is translated to Verilog-AMS text** (`spice_cards.zig:1-10`). This
  is safe only because VerA never writes SPICE back out.

---

## 9. LLVM, tree-sitter, Pandoc

Pins: LLVM `llvmorg-23.1.5`, tree-sitter `v0.27.0`, tree-sitter-python
`26855eab`, pandoc `3.13`, pandoc-types `1.23.1.2`.

### 9.1 LLVM: a capability table read by one generic pass

- `enum LegalizeAction { Legal, Promote, Expand, LibCall, Custom }`
  ([TargetLowering.h#L221](https://github.com/llvm/llvm-project/blob/llvmorg-23.1.5/llvm/include/llvm/CodeGen/TargetLowering.h#L221)).
- It is stored as a dense `OpActions[type][op]` array
  ([#L3883](https://github.com/llvm/llvm-project/blob/llvmorg-23.1.5/llvm/include/llvm/CodeGen/TargetLowering.h#L3883)).
- Each target fills the table once, gated on its features
  ([RISCVISelLowering.cpp#L413](https://github.com/llvm/llvm-project/blob/llvmorg-23.1.5/llvm/lib/Target/RISCV/RISCVISelLowering.cpp#L413)).
- One shared legalizer switches on the action. `Custom` is the only action
  that calls target code
  ([LegalizeDAG.cpp#L1357](https://github.com/llvm/llvm-project/blob/llvmorg-23.1.5/llvm/lib/CodeGen/SelectionDAG/LegalizeDAG.cpp#L1357),
  [RISCVISelLowering.cpp#L8134](https://github.com/llvm/llvm-project/blob/llvmorg-23.1.5/llvm/lib/Target/RISCV/RISCVISelLowering.cpp#L8134)).
- Feature names in diagnostics are generated from the same `.td` records
  ([RISCVAsmParser.cpp#L1817-L1820](https://github.com/llvm/llvm-project/blob/llvmorg-23.1.5/llvm/lib/Target/RISCV/AsmParser/RISCVAsmParser.cpp#L1817-L1820)).

### 9.2 tree-sitter: a declarative table plus a scanner for the context-sensitive part

- One `grammar.js` per language. Tokens a regular lexer cannot decide are
  listed in `externals`
  ([grammar.js#L70](https://github.com/tree-sitter/tree-sitter-python/blob/26855eabccb19c6abf499fbc5b8dc7cc9ab8bc64/grammar.js#L70)).
  A hand-written scanner produces them; Python's keeps an indent stack
  ([scanner.c#L86](https://github.com/tree-sitter/tree-sitter-python/blob/26855eabccb19c6abf499fbc5b8dc7cc9ab8bc64/src/scanner.c#L86)).
- The scanner receives a `valid_symbols` bitset from the parse state
  ([parser.h#L137-L142](https://github.com/tree-sitter/tree-sitter/blob/v0.27.0/lib/src/parser.h#L137-L142)).
- Errors become `ERROR` nodes and zero-width MISSING leaves, so the parse
  never stops
  ([parser.c#L1271](https://github.com/tree-sitter/tree-sitter/blob/v0.27.0/lib/src/parser.c#L1271),
  [#L1523](https://github.com/tree-sitter/tree-sitter/blob/v0.27.0/lib/src/parser.c#L1523)).

### 9.3 Pandoc: readers → one AST → writers

**The AST.**

- `Pandoc Meta [Block]`
  ([Definition.hs#L104](https://github.com/jgm/pandoc-types/blob/1.23.1.2/src/Text/Pandoc/Definition.hs#L104)).
- `Attr` carries format-specific extras on neutral `Div` and `Span` nodes
  ([#L199](https://github.com/jgm/pandoc-types/blob/1.23.1.2/src/Text/Pandoc/Definition.hs#L199),
  [#L306](https://github.com/jgm/pandoc-types/blob/1.23.1.2/src/Text/Pandoc/Definition.hs#L306)).
- `RawBlock Format Text` and `RawInline Format Text` hold what the AST
  cannot model, tagged with their source format
  ([#L283](https://github.com/jgm/pandoc-types/blob/1.23.1.2/src/Text/Pandoc/Definition.hs#L283),
  [#L363](https://github.com/jgm/pandoc-types/blob/1.23.1.2/src/Text/Pandoc/Definition.hs#L363)).

**Registries are name → function lists.**

- Five Markdown reader names share one `readMarkdown` and differ only in
  their extension sets
  ([P/Readers.hs#L138-L145](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Readers.hs#L138-L145)).
- `getDefaultExtensions` is one table keyed by format name
  ([P/Extensions.hs#L357-L478](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Extensions.hs#L357-L478)).
- One `pandocToMarkdown` serves four formats through `MarkdownVariant`
  ([P/Writers/Markdown.hs#L62-L95](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Markdown.hs#L62-L95)).

§11 covers how writers handle a construct they cannot express.

---

## 10. Question 1: the least semantic information cross-dialect rendering needs

### 10.1 What Zig shows

Zig needs no semantic information to render. Its renderer is same-language:
every token's text is printed back from the source (§4.2), and nothing it
prints depends on what a name means. AstGen's name pass (§5) exists for
code generation, not for `zig fmt`. `zig fmt` runs only the parser, and
runs AstGen only under `--ast-check` (Z/src/fmt.zig#L132-L157).

Cross-dialect rendering is different, because the target's spelling of a
construct can depend on facts the source card does not state. SpiceRack's
evidence falls into three classes.

### 10.2 Class (i): card-local; the renderer plus the token's dialect is enough

- **SPICE → Spectre model master.** The master comes from type plus
  `LEVEL`: 14/54 → bsim4, 8/49 → bsim3v3, and an unmapped type is refused
  (`SR/src/codegen/spectre/mod.rs:248-309`).
- **SPICE instances.** The X-versus-M choice is spelled in the text by the
  letter (`SR/grammar/spice.peg:168, 175, 197`).
- **Number suffixes.** SPICE `M` is milli and Spectre `M` is mega, so
  `30M` becomes `0.03`. The writer re-parses the value text as a SPICE
  number (`SR/src/codegen/spectre/values.rs:16-41`). It must know the
  token's origin dialect.
- **`^`.** It means power in some dialects and XOR in others. A
  cross-boundary `^` is refused using `ir.raw_dialect`
  (`SR/src/codegen/mod.rs:149-169, 257-259`).
- **Case.** Spectre is case-sensitive. A Spectre-origin IR keeps its case
  (`SR/src/codegen/spectre/names.rs:14-20, 97-105`).
- **Statistical functions.** LTspice has none, so such a call is refused by
  name (`SR/src/codegen/mod.rs:197-222`).

The only non-local input here is the dialect each token was read in. A
per-dialect tokenizer knows that for free.

### 10.3 Class (ii): needs a name → kind table, with no rewriting

**Spectre `x1 (a b) leaf` → X, M, Q or D.** SpiceRack already runs a
whole-file declaration pre-pass. `declarations()` returns a model
`name → kind` map plus the list of subckt names, "wherever they sit:
instances may come before both" (`SR/src/import/lower/named.rs:34-39`).
`Cx::inst` then picks the letter (`SR/src/import/lower/named_elements.rs:86-345`):

1. a built-in master is decided from the card text alone;
2. a subckt name becomes an `Instance`, which is X (`:277-285`);
3. otherwise `kinds.get(m)?` maps the model kind: d → D, npn/pnp → Q,
   nmos/pmos → M (`:287-340`).

This is AstGen's `scanContainer` plus lookup (§5.2–§5.3) without the
scope chain.

**The table stops at file scope.** `includes` and `libs` are recorded but
never read (`SR/src/import/lower/mod.rs:271-282`). A master that lives in
an included PDK file misses the lookup, and the card becomes a raw line
(`named.rs:140-146`; `mod.rs:230-232`). A raw line reaches only its own
dialect (`SR/src/codegen/mod.rs:171-173, 261-264`). The Interoperate
pdk-scan commits only count such lines, by keyword or by shape (5d6e255).

**References to devices are not resolved.**

- **The F/H control source.** A Spectre `probe=` is copied raw into
  `vsense` (`named_elements.rs:221-237`), and the SPICE writer emits it
  verbatim (`SR/src/codegen/spice3/components.rs:215-228`). But the IR
  strips the letter from a definition, so `r1 resistor` is stored as `1`,
  and the writer adds it back as `{letter}{name}`
  (`named_elements.rs:354-370`; `spice3/components.rs:57, 131`). For a
  Spectre source `src (x 0) vsource`, SPICE gets `Vsrc` while the F card
  still says `src`. This is inferred from the code and was not run.
- **`.dc` sweep targets.** Device versus parameter is guessed from the
  first letter: `v`/`i` → `dev=… param=dc`, anything else → `param=`
  (`SR/src/codegen/spectre/analyses.rs:577-592`).
- **Noise and `i()` saves.** `iprobe=` and `save i(V1)` → `v1:p` are name
  references with the same exposure
  (`analyses.rs:124-156`; `spectre/mod.rs:201-214`).

**Scope per expression.** A SPICE `.subckt`'s `.param` lines are locals,
while Spectre `parameters` belong to the definition itself. The reader
classifies them by scope, and the writer emits them separately
(`SR/src/codegen/mod.rs:324-340`; `spectre/mod.rs:342-344`).

### 10.4 Class (iii): needs a structural rewrite

**Monte Carlo** runs whole-IR passes before any writer
(`SR/src/codegen/mod.rs:52-66`).

**`lift`, SPICE → Spectre.** It clones the IR and inlines `.func`s. It
rewrites each agauss/gauss/aunif/unif/limit call into `rack_f(args, var)`.
It classifies each variable as mismatch or process by scope alone,
`mismatch: site.subckt.is_some()`. It then declares the variables,
appends function definitions, and edits the `SpectreMonteCarlo` analyses
(`SR/src/codegen/statistics/lift.rs:66-140`, `:104-109`).

**`lower`, Spectre `statistics` → SPICE.** It makes one draw per run for a
process variation and one draw per instance for a mismatch. It writes a
two-layer `.param`, because a one-layer `.param` would be redrawn at every
reference. It uses Cholesky for correlations, and refuses what SPICE cannot
hold (`SR/src/codegen/statistics/lower.rs:1-50`).

These passes move cards between scopes and synthesise new ones.

**VACASK device models** sit between (ii) and (iii). Every R/C/L needs a
model, so the writer scans the whole deck and adds `load` and `model`
header lines (`SR/src/codegen/vacask/models.rs:15-24`). This only appends;
nothing moves.

### 10.5 Answer

**The least semantic information is three things beyond the card's own
text:**

1. **The origin dialect of each token.** The tokenizer already has it.
2. **A symbol table.** It maps a master name to {subckt, model kind,
   built-in}, and a device name to its kind, across every file the deck
   reaches.
3. **The scope of each expression:** top, section, or subckt.

**This does justify a resolve pass with no rewriting.** Its model is
AstGen's `scanContainer` plus `localVarRef`:

- **Input:** the ASTs of the deck and of every file it includes, the PDK
  libraries included. This is the one place where includes must actually be
  read. Today they are recorded and ignored (`SR/src/import/lower/mod.rs:271-282`).
- **Output:** a side table, `resolved[name_token] → DeclRef{file, node,
  kind}`, plus a `scope[node]` column.
- **It rewrites nothing.** The AST stays exactly what was parsed, so the
  same-dialect render stays byte-identical.

**Why not inside the renderer?**

1. **Forward references.** Instances may come before both their subckt and
   their model (`named.rs:34-36`), and the definition may be in another
   file. Lookups inside the renderer would mean a parse of every include
   hidden inside a printing routine. Zig does a pre-scan for this reason
   (AstGen.zig:13098-13102).
2. **Errors.** "Undeclared master", "ambiguous master" and "reference to a
   renamed device" are diagnostics. In Zig they come from the name pass
   (AstGen.zig:8291-8313), and a renderer should not discover them. Zig's
   renderer cannot even run on an AST that has errors (Render.zig:86-87).
3. **Shared results.** The `vsense` and `.dc` bugs in §10.3 are the same
   lookup that the X/M choice needs. A shared table fixes all three. A
   renderer-side lookup would be written once per writer.

**A name that cannot be resolved** behaves as raw does today:

- same dialect: render verbatim;
- other dialect: refuse with a code, never guess. Zig records an unresolved
  import by name for a later stage (AstGen.zig:9073-9103), and a refused
  card in VerA is a coded refusal (`VerA/lib/frontend/spice_cards.zig:20-25`).

**Monte Carlo is the one real rewrite, and it fits Zig's Fixups model.**
`lift` and `lower` compute edits:

- omit a `.param`;
- replace a call with `rack_f(…)`;
- append a `statistics { … }` block after a node;
- append per-instance parameters.

A pass can emit these into a Fixups side table that the one renderer
applies. That is how `zig reduce` and `zig fetch --save` edit source without
an AST rewriter (Render.zig:28-84; Z/lib/compiler/reduce.zig L312-339;
Z/lib/compiler/Maker.zig L1930-1932). The AST is never mutated, and
everything the pass does not touch keeps its spelling and comments.

**So the pipeline is:**

1. tokenize;
2. parse;
3. resolve, which builds the side table;
4. for each source/target pair that needs one, a fixup pass that only
   emits fixups (Monte Carlo, VACASK model headers);
5. render.

No stage rewrites the AST.

---

## 11. Question 2: how Pandoc writers handle a construct the target cannot express

### 11.1 What happens to raw content

Raw content in another format is dropped with a log entry. Almost every
writer repeats the same rule by hand: pass the content through when its
`Format` matches the target, and otherwise `report (BlockNotRendered b)` and
emit nothing.

| Writer | Rule | Where |
|---|---|---|
| Man | `man` passes, all else dropped and logged | [Man.hs#L117-L121](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Man.hs#L117-L121), inline [#L308-L312](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Man.hs#L308-L312) |
| LaTeX | `latex`, `tex`, `beamer` pass | [LaTeX.hs#L570-L576](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/LaTeX.hs#L570-L576), inline [#L1100-L1108](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/LaTeX.hs#L1100-L1108) |
| Markdown | depends on variant and extensions; under `Ext_raw_attribute` a foreign raw block survives as ```` ```{=fmt} ````; otherwise `renderEmpty = mempty <$ report (BlockNotRendered b)` | [Markdown.hs#L485-L519](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Markdown.hs#L485-L519) |
| Org, JATS, Docx, HTML | same rule | [Org.hs#L172-L178](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Org.hs#L172-L178), [JATS.hs#L433-L436](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/JATS.hs#L433-L436), [Docx/OpenXML.hs#L555-L559](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Docx/OpenXML.hs#L555-L559), [HTML.hs#L993-L1004](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/HTML.hs#L993-L1004) |

**RST is the exception for raw blocks.** It never drops one: any format
becomes an `.. raw:: fmt` directive
([RST.hs#L286-L291](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/RST.hs#L286-L291)).

Counting the `BlockNotRendered` and `InlineNotRendered` sites per writer
gives 0–3 each. Nearly all are this raw rule. The few others:

- Markdown's `[TABLE]` last resort;
- an AsciiDoc table nested more than one level deep;
- a LaTeX `data:` image.

### 11.2 Constructs with no native equivalent are approximated, usually without a log entry

| Construct | Writer | Fallback | Where |
|---|---|---|---|
| Underline | Man | Emph, "Underline is not supported, so treat the same as Emph" | [Man.hs#L275-L277](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Man.hs#L275-L277) |
| Underline | RST, Ms, Texinfo | Emph | [RST.hs#L781-L783](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/RST.hs#L781-L783), [Ms.hs#L420-L421](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Ms.hs#L420-L421), [Texinfo.hs#L460-L462](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Texinfo.hs#L460-L462) |
| Underline | Markdown | ladder: `[..]{.underline}` → `<span class=underline>` → `<u>` → Emph | [Markdown/Inline.hs#L397-L410](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Markdown/Inline.hs#L397-L410) |
| Strikeout | Man, RST | literal `[STRIKEOUT:…]` | [Man.hs#L280-L282](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Man.hs#L280-L282), [RST.hs#L787-L789](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/RST.hs#L787-L789) |
| Strikeout | Ms | grey text, since it "seems quite hard to do in roff" | [Ms.hs#L424-L428](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Ms.hs#L424-L428) |
| SmallCaps | Man, RST, Org | styling dropped, contents kept (`-- not supported`) | [Man.hs#L289](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Man.hs#L289), [RST.hs#L796](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/RST.hs#L796), [Org.hs#L521](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Org.hs#L521) |
| SmallCaps | Markdown | span, else uppercase the text | [Markdown/Inline.hs#L467-L472](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Markdown/Inline.hs#L467-L472) |
| Div with attributes | Markdown | `:::` → `<div>` → contents only | [Markdown.hs#L411-L430](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Markdown.hs#L411-L430) |
| Cite | Man, AsciiDoc | citation text only, data lost | [Man.hs#L296-L297](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Man.hs#L296-L297), [AsciiDoc.hs#L684](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/AsciiDoc.hs#L684) |
| Multi-block footnote | AsciiDoc | literal `"[multiblock footnote omitted]"`, **no log** | [AsciiDoc.hs#L726](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/AsciiDoc.hs#L726) |
| Table row/col spans | Man (via shared `toLegacyTable`) | spanned cells cut into single cells, content top-left, **no log** | [Man.hs#L145](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Man.hs#L145), [Shared.hs#L748-L754](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Shared.hs#L748-L754) |
| Table | Markdown | ladder below, then a logged drop | [Markdown.hs#L684-L721](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Markdown.hs#L684-L721) |
| Math | shared `convertMath` | on failure: log `CouldNotConvertTeXMath`, emit raw `$tex$` | [Writers/Math.hs#L33-L50](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Math.hs#L33-L50) |

Markdown's table ladder is:

1. simple table;
2. pipe table;
3. multiline table;
4. grid table;
5. pipe table again;
6. an HTML table, made by calling the HTML writer on the one block (L707);
7. an approximate pipe table without spans, "better than nothing" (L712-716);
8. `report (BlockNotRendered t)` with a literal `[TABLE]` (L720-721).

### 11.3 No writer raises an error for content

Every `throwError` found in a writer guards an invariant or an I/O
problem:

- an unreachable heading level in Texinfo
  ([Texinfo.hs#L261](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Texinfo.hs#L261),
  guarded at
  [#L87](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Texinfo.hs#L87));
- a `PandocShouldNeverHappenError` in RTF
  ([RTF.hs#L57-L59](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/RTF.hs#L57-L59));
- the reference docx archive
  ([Docx.hs#L514](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Docx.hs#L514));
- an unknown option in HTML
  ([HTML.hs#L676](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/HTML.hs#L676)).

### 11.4 Severity

`InlineNotRendered` and `BlockNotRendered` carry the dropped node itself
([Logging.hs#L68-L69](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Logging.hs#L68-L69)).
`messageVerbosity` fixes both at INFO, so they are hidden by default
([#L447](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Logging.hs#L447),
[#L465-L466](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Logging.hs#L465-L466)).
`CouldNotConvertTeXMath` is WARNING
([#L474](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Logging.hs#L474)).

### 11.5 Answer

**Drop, approximate, or error?** Approximate first, mostly without a log
entry. Drop, logged at INFO, only when nothing approximates, which is
mostly foreign raw content. Never raise an error for content.

The policy is not uniform. AsciiDoc's footnote placeholder and
`toLegacyTable`'s span splitting lose content and report nothing.

**Per writer or a shared table?** Per writer. Each writer decides with its
own pattern-match ladder. Three things are shared:

- the **extension sets** the ladders consult (`getDefaultExtensions`);
- a small **library of helpers** that writers may choose to call
  (`convertMath`, `toLegacyTable`, `gridTable`, in
  [Shared.hs#L32-L45](https://github.com/jgm/pandoc/blob/3.13/src/Text/Pandoc/Writers/Shared.hs#L32-L45));
- the **message catalogue**, with each message's level fixed in one
  function.

No table maps (construct, writer) → action. The same decision, "Underline
becomes Emph", is written separately in Man, RST, Ms and Texinfo.

### 11.6 What that means for SpiceRack

Pandoc's tolerance suits prose, where Emph instead of Underline loses only
styling. A netlist is different: "skipping either would change the
circuit, a dropped R being an open" (`VerA/lib/frontend/spice_cards.zig:20-25`).

So SpiceRack should keep Pandoc's ordered ladder and its typed catalogue,
but change three things:

1. **The last rung is a coded refusal, never a silent drop.**
2. **An approximation is allowed only when it is exact,** such as SPICE
   `30M` → `0.03`. Anything that changes meaning is a refusal.
3. **The ladder lives in one table,** so that the same decision is not
   copied writer by writer, as "Underline → Emph" is copied four times in
   Pandoc. This is the LLVM shape: a table of actions, read by one generic
   pass (§9.1).

---

## 12. What transfers to SpiceRack

### 12.1 Tokenizer per dialect versus a lexical table

**Recommendation: one tokenizer driven by a per-dialect `Lexicon` table,
with a small hook for the context-sensitive rules.**

- **The facts are a table.** ESPice's dialect modules reduce to the same
  constants: title line, case folding, quote characters, the meaning of
  braces, the remainder operator, number reading
  (`ESPice/src/frontend/lines.zig:45-51`; §7).
- **The hook is like tree-sitter's external scanner** (§9.2). It covers:
  - HSPICE `$`, which starts a comment only after a separator
    (`ESPice/src/frontend/lines_hspice.zig:88-106`);
  - continuation forms;
  - Spectre's switch into SPICE rules inside `simulator lang=spice`
    (`lines_spectre.zig:25, 44`).
- **Tokens store `{tag: u8, start: u32}` only.** Their text is re-read from
  the source (§1.5). Each source file keeps its dialect, so every token's
  origin dialect is known. That answers the class (i) facts of §10.2.
- **Keywords are a lookup after lexing an identifier** (`zig/tokenizer.zig:655-664`).
  The lexer stays dialect-blind about whether a word is reserved, and the
  parser decides (`VerA/lib/frontend/keyword.zig:28-32`).
- **The tokenizer never fails.** A bad byte becomes an `Invalid` token, and
  lexing resyncs at the next line (§1.3).
- **Comments are not tokens.** The renderer recovers them from the byte
  gaps between tokens (§4.3). SPICE `*` lines, `;` and `$` tails, and
  Spectre `//` all become gaps. The tokenizer only has to skip them
  correctly. Doc-like comments the AST must own, if any appear, become
  tokens the way Zig's `///` does (§1.2).
- **Lex numbers loosely.** `1meg`, `30M` and `1e-3u` are each one `Number`
  token. The suffix's meaning comes from the token's dialect when it is
  rendered (§1.4).
- **No pre-pass reads raw text.** Include expansion and `.option search`
  read logical lines from the tokenizer. ESPice's separate expander misses
  continuations (`ESPice/docs/using/dialects.md:213`).

### 12.2 A single parser over a normalized token stream

**Normalize at the token level, never by rewriting text.**

- **Text translation fails a round-trip tool.** ESPice translates Spectre
  to SPICE text and loses spelling (§7). VerA does the same with SPICE,
  safely only because it never writes SPICE back (§8.4).
- **Token tags are shared across dialects:** `Ident`, `Number`, `String`,
  `ExprOpen`/`ExprClose` (`'…'` or `{…}` per the lexicon), `Eq`, `LParen`,
  `RParen`, `Comma`, `Directive` (`.foo` / a Spectre keyword),
  `EndOfStatement`.
- **Statement heads are the one real structural difference.** A SPICE card
  is letter-led and positional (`R1 a b 1k`). A Spectre or VACASK instance
  is `name (nodes) master params`.
  - The parser dispatches on the head shape.
  - Both shapes build the same node tags, such as
    `instance{name, nodes, master, params}` and `subckt`, `model`, `param`,
    `analysis`, `section`, `if`.
  - Everything below the head is shared: parameter lists, expressions,
    blocks.
- **Copy Zig's parser mechanics:**
  - `parse(src, lexicon) -> Ast`, which can fail only on out-of-memory;
    syntax errors are data (§2.2);
  - one scratch stack plus `list_to_span` into `extra: Vec<u32>` (§2.3);
  - `*_two` / plain tags by child count, plus variants recording a trailing
    separator or a source line break, so the renderer can see the author's
    layout (§2.4, §3.2);
  - `main_token` as the anchor (§2.5);
  - a doc comment on each tag naming its data slots (§3.2);
  - `full::*` views, so one render path covers each construct (§3.4).
- **Error recovery is simpler than Zig's.** The resync point is the next
  logical line, or the next `.ends` / `endsection` / `}` at depth 0, the
  analogue of `findNextStmt` (§2.6).
  - A card that fails to parse becomes an `unparsed` node over its token
    range. It keeps today's `Tag::Unparsed` behaviour
    (`SR/src/import/lower/mod.rs:236-240`), but it is an AST node, so it
    renders verbatim in its own dialect.
- **Relation to the existing PEG grammars.** The PEG grammars already
  capture into one shared tag set (`SR/src/import/mod.rs:6, 54`). The
  hand-written parser replaces them, with the lexicons absorbing what the
  dialect `.peg` files override.

### 12.3 The AST, plus side tables for original spelling and resolution

With no lowering, the AST is the IR. Neutral meaning lives in the node tags.
Spelling, layout and meaning that needs whole-deck knowledge go in side
tables.

- **Original spelling comes for free.** It is `source[token.start..]`
  re-lexed (§1.5). No spelling table needs to be built.
- **The same-dialect render is a Zig-style `renderToken`.**
  - Literals are written as spelled (§4.11).
  - Comments come from the gaps (§4.3).
  - Blank lines and the author's one-line or multi-line choice are re-read
    from tokens (§4.4–§4.5).
  - This makes `render(parse(x, d), d) == x` reachable for clean decks, the
    `testCanonical` property (§4.12).
- **The cross-dialect render needs canonical spelling only where the
  dialects differ:** numbers, quoting, letters, keywords. It still carries
  comments across, re-prefixed with the target's comment leader.
- **The resolve side table** comes from §10.5:
  - `resolved[name_token] → DeclRef{file, node, kind}`;
  - a `scope[node]` column.
- **Interned names** use `u32` handles, with slot 0 meaning empty
  (`zig/AstGen.zig:11012-11033`).
- **No `dialect` field on the deck.** The dialect belongs to the source
  file and its tokens. ESPice's `Deck.dialect` invited 37 downstream
  branches (§7).
- **Verbatim regions** are a source byte range on a node, rendered raw into
  their own dialect only. This is the `zig fmt: off` model (§4.8). It
  replaces the deck-wide `raw_dialect` (`SR/src/ir/circuit.rs:30-34`) with a
  per-node origin.

### 12.4 A capability table per dialect

**"Can dialect X express construct Y" is one function. One generic step
reads it.**

- **The sources.**
  - LLVM's `OpActions` table and its single legalizer, where only `Custom`
    calls target code (§9.1);
  - Zig's feature × backend table and per-backend rewrite sets (§6);
  - Pandoc's extension sets (§9.3);
  - VerA's total fact tables (§8.2).
- **SpiceRack shape.** `fn action(target: Dialect, c: Construct, origin:
  Dialect) -> Action`, one exhaustive `match`, where
  `Action = Native | Exact(Respell) | Fixup(Pass) | Verbatim | Refuse(Code)`.
  - `Construct` is the closed list of things whose spelling varies:
    behavioural source, Laplace source, `.func`, library section,
    conditional, measure, control block, OSDI load, port element, HSPICE
    SFFM order, MOS level 8, statistics, `^`, …
  - Rust has no `EnumArray`, but one exhaustive `match` in one function
    gives the same totality: a new variant does not compile until every
    dialect answers (`VerA/src/ir/opcode.zig:182-198`).
- **It answers Pandoc's weakness (§11.5).** The Pandoc ladder is kept, but
  as table rows, with a coded `Refuse` as the last rung (§11.6).
- **It removes most of the 371 `Dialect::` sites.** `grep -rc 'Dialect::' src`
  at `9efaefc` counts 371 sites in 32 files. 66 of them are in
  `SR/src/codegen/spice3/mod.rs` alone, e.g.
  `if d == Dialect::Hspice { "'" } else { "" }` at `:351`.
- **The 7-bit simulator mask stays.** `SR/src/backend/mod.rs:77-113` answers
  "can the simulator run it", which is a different question from "can the
  dialect spell it".

### 12.5 One renderer driven by per-dialect spelling tables

- **One renderer over the AST, the shape of `Ast/Render.zig`.**
  - It refuses a tree that has errors (Render.zig:86-87).
  - Every token goes through one `render_token(tok, space)`.
  - A closed `Space` vocabulary (§4.2).
  - Comments come from the gaps (§4.3).
- **A per-dialect `Spelling` table holds:**
  - comment leader;
  - expression quote (`'…'`, `{…}`, none);
  - continuation (`+` prefix, `\` suffix);
  - case;
  - `.lib` / `include` form;
  - parameter-function form;
  - number format;
  - directive keywords;
  - letter ↔ primitive name.
  - The fixed-token half of it works like `Tag::lexeme()` (§1.1).
- **Statement shape is a field.** The renderer branches on two head shapes,
  SPICE positional and Spectre named. That is one renderer with two head
  routines, not one renderer per dialect. Pandoc's `MarkdownVariant` (§9.3)
  is the precedent.
- **The output stream owns line length, continuation and indentation**
  (`AutoIndentingStream`, §4.9). Column alignment of `.param` and `.model`
  keys follows the array-init algorithm (§4.6).
- **Edits go through Fixups** (§4.10). Monte Carlo, VACASK model headers and
  device renames are fixup producers. The AST is never mutated.
- **Separate owners for meaning and spelling.** Node tags carry meaning, and
  `Spelling` is owned by the renderer (`VerA/src/ir/opcode.zig:5-6`).

### 12.6 The diagnostic catalogue

- **A `Code` enum whose rows are `{title, cite, explain}`,** where `cite`
  names the dialect manual section (`VerA/lib/diag/code.zig:20-46`).
  - Codes are stable: never renumbered or reused (`:52-56`).
  - Severity is fixed by the code, as `messageVerbosity` fixes it in Pandoc
    (§11.4).
  - The class encodes the stage: lex, parse, resolve, fixup, render.
  - A renderer may raise only its own class (`rtlil_emitter.zig:175-181`).
- **Errors are anchored to a token or node, and turned into text late.**
  This is Zig's `Ast.Error` plus `renderError`, with `token_is_prev` for a
  missing token at the end of a line (§2.6, §3.5).
- **Collect, don't stop.**
  - one sink per run (`VerA/lib/diag/bag.zig:47`);
  - dedupe (`VerA/lib/diag/builder.zig:127-136`);
  - a cap with a suppressed count (`bag.zig:32-35`);
  - one unresolved master reported once, not once per instance, the
    analogue of Zig's transitive-failure suppression (§6).
- **Every `Refuse` and every verbatim region withheld from another dialect
  emits a code.** Pandoc's INFO-level drops and its unlogged placeholders
  are the counterexample (§11.5).
- **Replace `report.unparsed: Vec<String>`** (`SR/src/import/mod.rs:43`) with
  coded, spanned rows.
- **Fixtures pin codes, not titles,** with `reject-only` semantics
  (`VerA/tests/suite/Reject.zig:77-103, 192-212`). This mirrors Zig's
  `testError`, which asserts the exact tag list (§4.12).

### 12.7 The golden byte-identical gate for the migration

1. **Freeze today's output first.** Before changing code, record
   `write(read(deck, d_in), d_out)` for every corpus deck and every
   `(d_in, d_out)` pair, together with the `unparsed` list. The corpora are
   spice-bench plus the ngspice, HSPICE, Xyce, VACASK and LTspice corpora
   pinned in `flake.nix` (`SR/TESTING.md:36`).
2. **Keep the old path in-tree as the oracle,** as ESPice's
   `lines_oracle.zig:1-3` does. Delete it in one final commit, together
   with every comment that cites it. ESPice's deleted oracle left about 50
   dangling citations (§7).
3. **List every difference in a strict `migration.xfail`.** An unlisted
   difference fails the run, and so does a listed entry that now matches
   (`VerA/tools/vera.xfail:1-5`).
4. **Add Zig's three properties as new gates:**
   - `testCanonical`: `render(parse(x, d), d) == x` for canonical decks.
   - `testTransform`: pinned expected bytes for each cross-dialect pair.
   - The `--check` exit code: byte equality as a CLI mode
     (Z/src/fmt.zig#L159-L164; Z/lib/std/zig/parser_test.zig#L7363-L7424).

   The first property is new. The current pipeline cannot pass it, because
   it does not keep comments or the author's layout.
5. **Migrate one layer at a time.** First the lexicons, while the PEG
   grammars still parse. Then the parser. Then the resolve pass, which
   fixes `vsense` and `.dc` (§10.3) and is visible in the xfail list as
   intended differences. Then the renderer. Each step lands only when the
   gate is green.
