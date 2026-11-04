//! Files and text → the transpiler's front end (`syntax`): bytes decoded
//! as a deck's text, the text read into its tokens and tree, a kit's files
//! read from disk into a `syntax::resolve::Kit`. Thin wrappers: the reading
//! is `syntax`'s; this owns only the file system and the text encoding.
//! `src/bin/spice2rack/` is the CLI.
//!
//! Cites: none (LTspice's UTF-16 and Latin-1 netlists are why `decode`
//! guesses).
//!
//! Contents:
//! - `pub fn decode(bytes: &[u8]) -> String`
//! - `pub fn parse(src: &str, d: Dialect) -> Result<(Tokens<'_>, Ast<'_>), String>`
//! - `pub fn kit(entries: &[PathBuf], form: Form, d: Dialect, root: &Path) -> Kit`
//! - `pub fn dump(src: &str, d: Dialect) -> Result<String, String>`

use std::path::{Path, PathBuf};

use crate::syntax::ast::{Ast, NONE, Node, Tag};
use crate::syntax::resolve::{Form, Kit, LoadError};
use crate::syntax::token::Tokens;
use crate::syntax::{self, Dialect, lexicon};

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

/// Deck text in dialect `d` (line 1 its title) as its tokens and tree; the
/// tree's diagnostics are `Ast::diags`. Both borrow `src`.
///
/// # Errors
/// `no <d> reader yet` for a dialect with no lexicon (CDL, Verilog); the
/// front end itself never fails, it keeps what it cannot read as
/// `Verbatim` statements with a `P` diagnostic.
pub fn parse(src: &str, d: Dialect) -> Result<(Tokens<'_>, Ast<'_>), String> {
    let lex = lexicon::of(d).ok_or_else(|| format!("no {d:?} reader yet"))?;
    let t = syntax::tokenize(src, lex);
    let ast = syntax::parse(&t, lex);
    Ok((t, ast))
}

/// The kit `entries` reach, read from disk in dialect `d`: each file
/// decoded as [`decode`] does, and only under `root` (a path outside it is
/// refused, `R0007`; a missing one is `R0006`).
///
/// # Panics
/// When `d` has no reader (CDL, Verilog).
pub fn kit(entries: &[PathBuf], form: Form, d: Dialect, root: &Path) -> Kit {
    let mut load = |p: &Path| {
        if !p.starts_with(root) {
            return Err(LoadError::Refused);
        }
        if !p.is_file() {
            return Err(LoadError::NotFound);
        }
        std::fs::read(p)
            .map(|b| decode(&b))
            .map_err(|_| LoadError::NotFound)
    };
    Kit::load(entries, form, d, &mut load)
}

/// The AST of `src` (debugging the parser): one row per statement and
/// item, indented by depth, each its tag and its anchor token's text, then
/// the parse's diagnostics with their line and column.
///
/// # Errors
/// As [`parse`]: a dialect with no lexicon.
pub fn dump(src: &str, d: Dialect) -> Result<String, String> {
    let (t, a) = parse(src, d)?;
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
mod tests {
    use super::*;

    #[test]
    fn decode_reads_utf16_utf8_and_latin1() {
        assert_eq!(decode(&[0xFF, 0xFE, b'R', 0, b'1', 0]), "R1");
        assert_eq!(decode("\u{feff}V1 a 0 1".as_bytes()), "V1 a 0 1");
        assert_eq!(decode(&[b'1', 0xB5]), "1\u{b5}", "Latin-1 micro");
    }

    #[test]
    fn parse_refuses_a_dialect_with_no_reader() {
        assert!(parse("t\nR1 a b 1k\n", Dialect::Cdl).is_err());
        let (t, a) = parse("t\nR1 a b 1k\n", Dialect::Ngspice).unwrap();
        assert!(a.diags().is_empty());
        assert!(t.len() > 4);
    }
}
