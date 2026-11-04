//! Deck text in any dialect -> [`CircuitIR`], through the transpiler's front
//! end (`syntax`): the dialect's
//! lexicon tokenizes it (`syntax::tokenize`), one parser builds the AST
//! (`syntax::parse`), and the temporary bridge `syntax::ast_to_ir` builds
//! today's `CircuitIR` from it, exactly as the reader before it did (the
//! golden gate's `daa0ad0` oracle).
//!
//! A card the old card grammar refused is [`Report::unparsed`]; one the IR
//! has no field for is [`Report::raw`]. Both stay verbatim lines, so they
//! reach the deck's own dialect unchanged. `src/bin/spice2rack/` is the CLI.
//!
//! Contents:
//! - `pub struct Report` (what a deck lost: `unparsed` lines, `raw` count)
//! - `pub fn decode(bytes: &[u8]) -> String`
//! - `pub fn parse(src: &str, name: &str, d: Dialect) -> Result<(CircuitIR, Report), String>`
//! - `pub fn parse_kit(kit: &Kit, file: FileId, name: &str) -> (CircuitIR, Report)`
//! - `pub fn dump(src: &str, d: Dialect) -> Result<String, String>`

use crate::codegen::Dialect;
use crate::ir::*;
use crate::syntax::ast::{Ast, NONE, Node, Tag};
use crate::syntax::resolve::{FileId, Kit, Role};
use crate::syntax::{self, lexicon};

/// What a deck lost on the way to IR.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    /// Cards the dialect grammar refused, kept as raw lines.
    pub unparsed: Vec<String>,
    /// Cards the grammar read that no IR field holds, kept as raw lines.
    pub raw: usize,
}

/// Deck bytes as text: UTF-16LE with a BOM (LTspice), UTF-8, else Latin-1
/// (LTspice's older netlists, SSR "Encoding"), so a `µ` survives.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// Parses deck text in dialect `d` (line 1 is the title) into IR named `name`.
///
/// # Errors
/// `no <d> reader yet` for a dialect with no lexicon (CDL, Verilog); the
/// front end itself never fails, it keeps what it cannot read as raw lines.
pub fn parse(src: &str, name: &str, d: Dialect) -> Result<(CircuitIR, Report), String> {
    let lex = lexicon::of(d).ok_or_else(|| format!("no {d:?} reader yet"))?;
    let t = syntax::tokenize(src, lex);
    let ast = syntax::parse(&t, lex);
    Ok(syntax::ast_to_ir(&ast, &t, d, name, None))
}

/// Parses file `file` of `kit` (its [`File::text`], in the kit's dialect)
/// into IR named `name`, as [`parse`] does, but for what the file alone
/// leaves open, which `kit` answers: a master another file declares, an
/// R/C/L card's model word, a range card's node split.
///
/// # Panics
/// When `file` is a Verilog-A source or unread (it has no tree).
///
/// [`File::text`]: crate::syntax::resolve::File::text
pub fn parse_kit(kit: &Kit, file: FileId, name: &str) -> (CircuitIR, Report) {
    let f = kit.file(file);
    assert!(
        matches!(f.role, Role::Deck | Role::Library),
        "parse_kit: a file the kit read"
    );
    let lex = lexicon::of(kit.dialect()).expect("a kit's dialect has a reader");
    let t = syntax::tokenize(&f.text, lex);
    let ast = syntax::parse(&t, lex);
    syntax::ast_to_ir(&ast, &t, kit.dialect(), name, Some((kit, file)))
}

/// The AST of `src` (debugging the parser): one row per statement and
/// item, indented by depth, each its tag and its anchor token's text, then
/// the parse's diagnostics with their line and column.
///
/// # Errors
/// As [`parse`]: a dialect with no lexicon.
pub fn dump(src: &str, d: Dialect) -> Result<String, String> {
    let lex = lexicon::of(d).ok_or_else(|| format!("no {d:?} reader yet"))?;
    let t = syntax::tokenize(src, lex);
    let a = syntax::parse(&t, lex);
    let mut out = String::new();
    let mut work: Vec<(u32, usize)> = a.fields(a.root()).iter().rev().map(|&n| (n, 0)).collect();
    while let Some((n, depth)) = work.pop() {
        let node = Node(n);
        let head = t.text(a.main_token(node));
        out.push_str(&format!(
            "{}{:?} {head:?}\n",
            "  ".repeat(depth),
            a.tag(node)
        ));
        work.extend(
            children(&a, node)
                .into_iter()
                .filter(|&k| k != NONE)
                .rev()
                .map(|k| (k, depth + 1)),
        );
    }
    for e in a.diags() {
        let at = t.location(e.token);
        out.push_str(&format!("{:?} at {}:{}\n", e.code, at.line, at.column));
    }
    Ok(out)
}

// ---- private ----

/// The rows [`dump`] prints under `n`: a header's name, master and nodes,
/// the fields, a block's body and its chained branch; an item's parts.
fn children(a: &Ast, n: Node) -> Vec<u32> {
    let mut k = Vec::new();
    match a.tag(n) {
        tag if tag.has_header() => {
            let h = a.header(n);
            k.extend([h.name, h.master]);
            k.extend(h.nodes);
        }
        Tag::Assign => return vec![a.lhs(n), a.rhs(n)],
        Tag::Seq => return a.list(a.lhs(n)).to_vec(),
        Tag::Group => return a.list(a.rhs(n) + 1).to_vec(),
        Tag::Delimited => return vec![a.lhs(n)],
        Tag::Root
        | Tag::Title
        | Tag::End
        | Tag::Verbatim
        | Tag::Line
        | Tag::Word
        | Tag::Bad
        | Tag::Num
        | Tag::Ident
        | Tag::Str
        | Tag::Unary
        | Tag::Binary
        | Tag::Ternary
        | Tag::Call
        | Tag::Probe
        | Tag::Index
        | Tag::Paren
        | Tag::Vector
        | Tag::List => return k,
        // else: the other statements: fields, then a block's body below.
        _ => {}
    }
    k.extend(a.fields(n));
    if a.rhs(n) != NONE {
        k.extend(a.body(n));
        k.extend(a.next(n).map(|x| x.0));
    }
    k
}

#[cfg(test)]
mod tests;
