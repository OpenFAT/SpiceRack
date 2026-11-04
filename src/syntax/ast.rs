//! The parser's output: one row per node, `{tag: u8, main_token: u32, lhs:
//! u32, rhs: u32}` SoA, payloads too wide for two words in `extra_data`, the
//! names interned once, and the parse's diagnostics. Netlist statements,
//! analyses and expressions are all rows of one [`Ast`]; a node's text is
//! its tokens' ([`Tokens`]), so the spelling is never copied.
//!
//! Every [`Tag`] says how to read `main_token`, `lhs`, `rhs` and its extra
//! words. Three layouts recur, named in the tags' docs:
//! - **list at `k`**: `extra_data[k]` is a length `n`, then `n` node ids;
//! - **fields**: `lhs` is a list at `lhs`, the statement's items after its
//!   head, in source order (words, groups, delimited expressions, strings,
//!   `key=value` assignments); a header statement's fields follow its
//!   header;
//! - **header**: `lhs` is extra `[name, master, code, n, node0 .. node(n-1),
//!   m, arg0 .. arg(m-1)]`, the statement's [`Header`] then its fields list
//!   (a [`Tag::has_header`] statement: `name [(nodes)] master args`);
//! - **body**: `rhs` is [`NONE`] for a statement with no body, else
//!   `extra_data[rhs..]` is `[next, closer, n, s0 .. s(n-1)]`: the chained
//!   branch (an `If`'s `else`, else `NONE`), the statement that closed the
//!   block (`NONE` when the file ended first), and the body statements.
//!
//! A statement node's `main_token` is always its statement's first token, so
//! its tokens run from there to the next [`token::Tag::EndOfStatement`]
//! (a block's last token is its closer's).
//!
//! Cites: Zig 0.17 `lib/std/zig/Ast.zig` (rows, `extra_data`, `errors`, tag
//! docs).
//!
//! Contents:
//! - `pub enum Tag`, `Tag::has_header(self) -> bool`, `pub struct Node(pub u32)`,
//!   `pub const NONE: u32`, `pub struct Header<'a>`
//! - `pub struct Name(pub u32)`, `pub struct Mark`
//! - `pub struct Ast<'s>`
//! - `Ast::tag(&self, n: Node) -> Tag`, `Ast::main_token(&self, n: Node) -> u32`,
//!   `Ast::lhs(&self, n: Node) -> u32`, `Ast::rhs(&self, n: Node) -> u32`,
//!   `Ast::len(&self) -> usize`, `Ast::is_empty(&self) -> bool`,
//!   `Ast::root(&self) -> Node`, `Ast::list(&self, at: u32) -> &[u32]`,
//!   `Ast::extra(&self, at: u32) -> u32`, `Ast::fields(&self, n: Node) -> &[u32]`,
//!   `Ast::header(&self, n: Node) -> Header<'_>`,
//!   `Ast::body(&self, n: Node) -> &[u32]`, `Ast::next(&self, n: Node) -> Option<Node>`,
//!   `Ast::closer(&self, n: Node) -> Option<Node>`, `Ast::name(&self, id: Name) -> &str`,
//!   `Ast::diags(&self) -> &[Diag]`, `Ast::push(&mut self, ..) -> Node`,
//!   `Ast::push_list(&mut self, items: &[u32]) -> u32`, `Ast::push_extra(&mut self, words: &[u32]) -> u32`,
//!   `Ast::push_body(&mut self, next: u32, closer: u32, items: &[u32]) -> u32`,
//!   `Ast::push_header(&mut self, name: u32, master: u32, code: u32, nodes: &[u32], args: &[u32]) -> u32`,
//!   `Ast::set_extra(&mut self, at: u32, word: u32)`, `Ast::set_root(&mut self, list: u32)`,
//!   `Ast::set_rhs(&mut self, n: Node, rhs: u32)`, `Ast::new(tokens_len: usize) -> Ast<'s>`,
//!   `Ast::intern(&mut self, text: Cow<'s, str>) -> Name`, `Ast::diag(&mut self, code: Code, token: u32)`,
//!   `Ast::mark(&self) -> Mark`, `Ast::reset(&mut self, m: Mark)`

use std::borrow::Cow;
use std::collections::HashMap;

use crate::syntax::diag::{Code, Diag};
#[cfg(doc)]
use crate::syntax::token::{self, Tokens};

/// A node's kind. Statements first, then the items of a statement's fields,
/// then expressions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Tag {
    // ── structure ──
    /// The file. `lhs`: a list of the top statements. `main_token` 0.
    Root,
    /// The title line. `main_token`: the `Title` token.
    Title,
    /// `.end`. No fields.
    End,
    /// A statement the parser cannot read, kept as its tokens, always with a
    /// diagnostic. `lhs`: its last token; `rhs`: the [`Dialect`] index
    /// (`Dialect::ALL`) of the lexicon that read it.
    ///
    /// [`Dialect`]: crate::syntax::Dialect
    Verbatim,
    /// `.include`, `include`. Fields: the path.
    Include,
    /// SPICE `.lib file section` (LTspice `.lib file`). Fields.
    LibCall,
    /// `.subckt name ports [params:] k=v` … `.ends`, Spectre/VACASK
    /// `subckt`. Fields: the head's; body.
    Subckt,
    /// SPICE `.lib name` … `.endl`, Spectre/VACASK `section name` …
    /// `endsection`. Fields: the name; body.
    Section,
    /// Spectre `library name` … `endlibrary`. Fields; body.
    Library,
    /// `.if (c)`, `.elseif (c)`, Spectre `if (c) {`, VACASK `@if c`.
    /// Fields: the condition (one expression); body; `next` the `.elseif`
    /// (an `If`) or `.else` (an `Else`).
    If,
    /// `.else`, Spectre `} else {`, VACASK `@else`. Body.
    Else,
    /// ngspice `.control` … `.endc`, VACASK `control` … `endc`: a script.
    /// Body: each line a `Line`, but the analysis commands, read as their
    /// dot cards (an `Analysis` whose head is the command word).
    Control,
    /// Spectre `simulator lang=spice` … `simulator lang=spectre`: the region's
    /// statements, read by the SPICE lexicon. Fields: the head's; body.
    Region,
    /// Lines kept whole: `.data` … `.enddata`, `.variation` …
    /// `.end_variation`. Body: each line a `Line`.
    Data,
    /// A line of a block kept whole by design (a `.control` script line, a
    /// `.data` row): its tokens, with no diagnostic, since nothing in it is
    /// unread. `lhs`: its last token.
    Line,
    /// `.param`, `parameters`. Fields.
    Param,
    /// `.func name(args) [=] body`, Spectre `real name(real a, ...) {
    /// return e }`. Fields; Spectre's body.
    Func,
    /// `.model name kind params`, Spectre/VACASK `model name master params`.
    /// Fields; a Spectre model group's body holds its bins (`Card`s).
    Model,
    /// `.options`, `.option`, `.opt`, Spectre `opt1 options reltol=1e-3`,
    /// VACASK `options` (in `control`). A header: Spectre's name and master
    /// word, else `NONE`; `code` `NONE`. Fields.
    Options,
    /// `.save`, HSPICE `.probe`, Spectre/VACASK `save`. Fields.
    Save,
    /// `.print`, `.plot`. Fields.
    Print,
    /// `.meas`. Fields.
    Measure,
    /// `.ic`, Spectre `ic`. Fields.
    Ic,
    /// `.nodeset`. Fields.
    Nodeset,
    /// `.temp`. Fields.
    Temp,
    /// `.global`, `global`. Fields.
    Global,
    /// An analysis: SPICE `.op`, `.dc`, `.ac`, `.tran`, `.noise`, `.tf`,
    /// `.sens`, `.pz`, `.four`, `.sp` (and a `.control` command read as its
    /// card), Spectre `tran1 tran stop=1n`, VACASK `analysis op1 op`. A
    /// header: the name and the kind word (`NONE` in SPICE, whose kind is
    /// its keyword), `code` the [`Analysis`] kind (`NONE` for a kind word
    /// the dialect's table lacks), the nodes (Spectre `noise1 (out 0)
    /// noise`). Fields; a Spectre `sweep … {`'s body.
    ///
    /// [`Analysis`]: crate::syntax::statement::Analysis
    Analysis,
    /// A Spectre control statement: `alter1 alter param=x value=1`, `info`,
    /// `set`, `shell` …. A header: the name, the master word, `code` the
    /// [`Command`]. Fields; an `altergroup … {`'s body.
    ///
    /// [`Command`]: crate::syntax::statement::Command
    Command,
    /// `.step`. Fields.
    Step,
    /// Spectre `statistics {`. Body: `Process`, `Mismatch`, `Card`s
    /// (`correlate`, `truncate`).
    Statistics,
    /// Spectre `process {` / `mismatch {` inside `statistics`. Body: `Card`s
    /// (`vary`, `truncate`).
    Process,
    /// Any other statement with a keyword or directive head (`.mc`,
    /// `.backanno`, `load`, `vary`, a Spectre `} ` closer, a bin `1: …`, a
    /// VACASK `control` line): its head's token tag says which. Fields; a
    /// Spectre `{` block's body.
    Card,
    /// An element: SPICE `R1 a b 1k`, Spectre `r1 (a b) resistor r=1k`,
    /// split by its dialect's device row (`syntax::device`). A header:
    /// `name` its name `Word`, `master` the model, subcircuit or master
    /// `Word` (`NONE` when the card names none, as `R1 a b 1k`), `code` the
    /// [`Primitive`] (`NONE` when the parser cannot know it: a Spectre master
    /// that is a model or a subcircuit, a VACASK model), the nodes `Word`s;
    /// fields: the arguments after the nodes and master, positional values
    /// and `key=value`s, in source order; a Spectre `name master … {`'s
    /// body.
    ///
    /// [`Primitive`]: crate::syntax::device::Primitive
    Instance,

    // ── items ──
    /// A name or value as written: tokens with no gap between them (`net-1`,
    /// `vdd!`, `n<3>`, `10%3`), or across an HSPICE `\\` glue.
    /// `lhs`: the last token; `rhs`: its interned text ([`Name`]).
    Word,
    /// `key = value`. `lhs`: the key (a `Word`, or a `Group` such as
    /// `v(n)` in `.ic v(n)=1`); `rhs`: the value.
    Assign,
    /// `word(items)`: a waveform `sin(0 1 1k)`, `poly(2)`, an output
    /// `v(a)`. `lhs`: the `Word`; `rhs`: extra `[close, n, items...]`, `close`
    /// the `)` token (or the last token when the group is unclosed).
    Group,
    /// A value continued by blank- or comma-separated numbers: a model's
    /// matrix (`c=1e-10 -6e-11`), SPICE's comma vector (`ic=0,-5`). `lhs`: a
    /// list of its parts.
    Seq,
    /// `{expr}`, `'expr'`, HSPICE `"expr"`. `main_token`: the opening
    /// delimiter; `lhs`: the expression (`NONE` for an empty `{}` or
    /// `''`); `rhs`: the closing delimiter. An HSPICE `"…"` holding no
    /// expression is a `Bad` instead.
    Delimited,

    // ── expressions ──
    /// A number. `main_token`: the `Number` token (its value is
    /// [`Tokens::value`]).
    Num,
    /// A name in an expression. `lhs`: its [`Name`].
    Ident,
    /// `"…"`. `main_token`: the `String` token, or an HSPICE path's
    /// opening `"` ([`token::Tag::DoubleQuote`]); `lhs`: that path's
    /// closing `"`, else `NONE`.
    Str,
    /// `op x`. `main_token`: the operator; `lhs`: the operand.
    Unary,
    /// `x op y`. `main_token`: the operator; `lhs`, `rhs`: the operands.
    Binary,
    /// `c ? a : b`. `main_token`: `?`; `lhs`: `c`; `rhs`: extra `[a, b]`.
    Ternary,
    /// `f(args)`. `main_token`: the name; `lhs`: its [`Name`]; `rhs`: extra
    /// `[close, n, args...]`.
    Call,
    /// `v(a)`, `v(a,b)`, `i(v1)`: a probe of nodes or a device. Read as
    /// `Call`; the arguments are `Word`s.
    Probe,
    /// VACASK `x[i]`. `main_token`: `[`; `lhs`: `x`; `rhs`: extra `[i, close]`.
    Index,
    /// `(x)`. `main_token`: `(`; `lhs`: `x`; `rhs`: the `)` token.
    Paren,
    /// `[a b]` (Spectre), `[a, b]` (VACASK). `main_token`: `[`; `rhs`: extra
    /// `[close, n, items...]`.
    Vector,
    /// VACASK `{a, b}`. As `Vector`.
    List,
    /// An expression that does not parse, kept as its tokens (always with
    /// a diagnostic). `lhs`: its last token.
    Bad,
}

/// A node index into [`Ast`]'s rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Node(pub u32);

/// An interned name: names are numbered in first-seen order, so the same
/// input gives the same ids.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Name(pub u32);

/// No node, no token: an absent optional word.
pub const NONE: u32 = u32::MAX;

/// A header statement's head (`Tag::has_header`): `name [(nodes)] master`.
#[derive(Clone, Copy, Debug)]
pub struct Header<'a> {
    /// The name `Word`, or `NONE`.
    pub name: u32,
    /// The master `Word`, or `NONE`.
    pub master: u32,
    /// What the master names, by the tag: a `Primitive` (`Instance`), an
    /// `Analysis` kind, a `Command`; `NONE` when unknown.
    pub code: u32,
    /// The node `Word`s.
    pub nodes: &'a [u32],
}

/// A point to undo appends back to: a speculative parse that lost.
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    nodes: usize,
    extra: usize,
    diags: usize,
}

/// A file's syntax tree.
///
/// Invariants: the four node columns have one row per node; node 0 is the
/// `Root`; every id in a row or in `extra_data` that names a node is below
/// the node count; `names` and `name_ids` hold the same names.
pub struct Ast<'s> {
    tags: Vec<Tag>,
    main_tokens: Vec<u32>,
    lhs: Vec<u32>,
    rhs: Vec<u32>,
    extra_data: Vec<u32>,
    names: Vec<Cow<'s, str>>,
    name_ids: HashMap<Cow<'s, str>, Name>,
    diags: Vec<Diag>,
}

impl Tag {
    /// Whether the statement's `lhs` is a [`Header`] and its fields.
    pub fn has_header(self) -> bool {
        matches!(
            self,
            Tag::Instance | Tag::Analysis | Tag::Command | Tag::Options
        )
    }
}

impl<'s> Ast<'s> {
    /// An empty tree holding only its `Root` row (filled by `set_root`).
    pub fn new(tokens_len: usize) -> Ast<'s> {
        let mut a = Ast {
            tags: Vec::with_capacity(tokens_len / 2 + 1),
            main_tokens: Vec::with_capacity(tokens_len / 2 + 1),
            lhs: Vec::with_capacity(tokens_len / 2 + 1),
            rhs: Vec::with_capacity(tokens_len / 2 + 1),
            extra_data: Vec::new(),
            names: Vec::new(),
            name_ids: HashMap::new(),
            diags: Vec::new(),
        };
        a.push(Tag::Root, 0, NONE, NONE);
        a
    }

    /// Node `n`'s tag.
    pub fn tag(&self, n: Node) -> Tag {
        self.tags[n.0 as usize]
    }

    /// Node `n`'s anchor token (the statement's first token for statements).
    pub fn main_token(&self, n: Node) -> u32 {
        self.main_tokens[n.0 as usize]
    }

    /// Node `n`'s first data word, read as its tag says.
    pub fn lhs(&self, n: Node) -> u32 {
        self.lhs[n.0 as usize]
    }

    /// Node `n`'s second data word, read as its tag says.
    pub fn rhs(&self, n: Node) -> u32 {
        self.rhs[n.0 as usize]
    }

    /// The number of nodes, the root included.
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// Whether the tree holds no node at all (never: the root is always
    /// there).
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// The `Root` node.
    pub fn root(&self) -> Node {
        Node(0)
    }

    /// The list at `at`: `extra_data[at]` items after it.
    pub fn list(&self, at: u32) -> &[u32] {
        if at == NONE {
            return &[];
        }
        let n = self.extra_data[at as usize] as usize;
        &self.extra_data[at as usize + 1..at as usize + 1 + n]
    }

    /// One word of `extra_data`.
    pub fn extra(&self, at: u32) -> u32 {
        self.extra_data[at as usize]
    }

    /// A statement's fields (`Root`: its statements).
    ///
    /// # Panics
    /// When `n` is no node.
    pub fn fields(&self, n: Node) -> &[u32] {
        if !self.tag(n).has_header() {
            return self.list(self.lhs(n));
        }
        let h = self.lhs(n);
        self.list(h + 4 + self.extra(h + 3))
    }

    /// A header statement's head.
    ///
    /// # Panics
    /// When `n`'s tag has no header.
    pub fn header(&self, n: Node) -> Header<'_> {
        assert!(self.tag(n).has_header(), "a header statement");
        let h = self.lhs(n);
        Header {
            name: self.extra(h),
            master: self.extra(h + 1),
            code: self.extra(h + 2),
            nodes: self.list(h + 3),
        }
    }

    /// A block statement's body; empty for a statement with none.
    pub fn body(&self, n: Node) -> &[u32] {
        match self.rhs(n) {
            NONE => &[],
            at => self.list(at + 2),
        }
    }

    /// A block's chained branch: an `If`'s `.elseif` or `.else`.
    pub fn next(&self, n: Node) -> Option<Node> {
        self.body_word(n, 0)
    }

    /// The statement that closed block `n`; `None` when the file ended
    /// first.
    pub fn closer(&self, n: Node) -> Option<Node> {
        self.body_word(n, 1)
    }

    /// Interned name `id`'s text.
    pub fn name(&self, id: Name) -> &str {
        &self.names[id.0 as usize]
    }

    /// The parse's diagnostics, in the order they were found.
    pub fn diags(&self) -> &[Diag] {
        &self.diags
    }

    /// Appends a node and returns its id.
    pub fn push(&mut self, tag: Tag, main_token: u32, lhs: u32, rhs: u32) -> Node {
        let id = u32::try_from(self.tags.len()).expect("fewer than u32::MAX nodes");
        assert!(id != NONE, "a node id is never NONE");
        self.tags.push(tag);
        self.main_tokens.push(main_token);
        self.lhs.push(lhs);
        self.rhs.push(rhs);
        Node(id)
    }

    /// Appends the list `[len, items...]`; returns its offset.
    pub fn push_list(&mut self, items: &[u32]) -> u32 {
        let at = u32::try_from(self.extra_data.len()).expect("fewer than u32::MAX extra words");
        self.extra_data.push(items.len() as u32);
        self.extra_data.extend_from_slice(items);
        at
    }

    /// Appends `words` to `extra_data`; returns the offset of the first.
    pub fn push_extra(&mut self, words: &[u32]) -> u32 {
        let at = u32::try_from(self.extra_data.len()).expect("fewer than u32::MAX extra words");
        self.extra_data.extend_from_slice(words);
        at
    }

    /// Appends a body, `[next, closer, n, items...]`; returns its offset
    /// (a block's `rhs`).
    pub fn push_body(&mut self, next: u32, closer: u32, items: &[u32]) -> u32 {
        let at = self.push_extra(&[next, closer]);
        self.push_list(items);
        at
    }

    /// Appends a header and the fields after it; returns its offset (the
    /// statement's `lhs`).
    pub fn push_header(
        &mut self,
        name: u32,
        master: u32,
        code: u32,
        nodes: &[u32],
        args: &[u32],
    ) -> u32 {
        let at = self.push_extra(&[name, master, code]);
        self.push_list(nodes);
        self.push_list(args);
        at
    }

    /// Rewrites word `at` of `extra_data` (a block's `next` or `closer`,
    /// known only after its body).
    pub fn set_extra(&mut self, at: u32, word: u32) {
        self.extra_data[at as usize] = word;
    }

    /// Rewrites the root's statement list.
    pub fn set_root(&mut self, list: u32) {
        self.lhs[0] = list;
    }

    /// Rewrites node `n`'s `rhs` (a block's body, appended after its head).
    pub fn set_rhs(&mut self, n: Node, rhs: u32) {
        self.rhs[n.0 as usize] = rhs;
    }

    /// `text`'s id, a new one the first time it is seen.
    pub fn intern(&mut self, text: Cow<'s, str>) -> Name {
        if let Some(&id) = self.name_ids.get(text.as_ref()) {
            return id;
        }
        let id = Name(u32::try_from(self.names.len()).expect("fewer than u32::MAX names"));
        self.names.push(text.clone());
        self.name_ids.insert(text, id);
        debug_assert_eq!(self.names.len(), self.name_ids.len());
        id
    }

    /// Where the rows end now.
    pub fn mark(&self) -> Mark {
        Mark {
            nodes: self.tags.len(),
            extra: self.extra_data.len(),
            diags: self.diags.len(),
        }
    }

    /// Drops every node, extra word and diagnostic appended since `m`
    /// (interned names stay: ids are only ever added).
    pub fn reset(&mut self, m: Mark) {
        assert!(m.nodes >= 1, "the root stays");
        self.tags.truncate(m.nodes);
        self.main_tokens.truncate(m.nodes);
        self.lhs.truncate(m.nodes);
        self.rhs.truncate(m.nodes);
        self.extra_data.truncate(m.extra);
        self.diags.truncate(m.diags);
    }

    /// Appends a diagnostic anchored at `token`.
    pub fn diag(&mut self, code: Code, token: u32) {
        self.diags.push(Diag { code, token });
    }
}

// ---- private ----

impl Ast<'_> {
    fn body_word(&self, n: Node, k: u32) -> Option<Node> {
        match self.rhs(n) {
            NONE => None,
            at => match self.extra(at + k) {
                NONE => None,
                w => Some(Node(w)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_thirteen_bytes_per_node() {
        // Zig `Ast.Node`: a tag byte, the main token and two data words.
        assert_eq!(size_of::<Tag>(), 1);
        assert_eq!(size_of::<Tag>() + 3 * size_of::<u32>(), 13);
        assert_eq!(size_of::<Node>(), 4);
        assert_eq!(size_of::<Name>(), 4);
    }

    #[test]
    fn names_are_numbered_in_first_seen_order() {
        let mut a = Ast::new(0);
        let x = a.intern("x".into());
        let y = a.intern("y".into());
        assert_eq!((x, y), (Name(0), Name(1)));
        assert_eq!(a.intern(String::from("x").into()), x);
        assert_eq!(a.name(y), "y");
    }

    #[test]
    fn lists_and_bodies_read_back() {
        let mut a = Ast::new(0);
        let l = a.push_list(&[1, 2, 3]);
        assert_eq!(a.list(l), [1, 2, 3]);
        assert_eq!(a.list(NONE), [] as [u32; 0]);
        let fields = a.push_list(&[]);
        let at = a.push_body(NONE, NONE, &[7]);
        let n = a.push(Tag::Subckt, 0, fields, at);
        assert_eq!(a.body(n), [7]);
        assert_eq!(a.closer(n), None);
        a.set_extra(at + 1, 5);
        assert_eq!(a.closer(n), Some(Node(5)));
    }
}
