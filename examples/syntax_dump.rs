//! Prints the transpiler parser's tree for a deck: one line per statement
//! and item, indented by depth, with the diagnostics (debugging the
//! parser and `syntax::ast_to_ir`).
//!
//!   cargo run --example syntax_dump -- <dialect> <deck>
//!
//! Contents: none (an example; `main` is its entry).

use spicerack::syntax::ast::{Ast, NONE, Node, Tag};
use spicerack::syntax::token::Tokens;

// ---- private ----

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [d, path] = args.as_slice() else {
        eprintln!("usage: syntax_dump <dialect> <deck>");
        std::process::exit(2);
    };
    let d: spicerack::codegen::Dialect = d.parse().expect("a dialect");
    let src = spicerack::import::decode(&std::fs::read(path).expect("a readable deck"));
    let lex = spicerack::syntax::lexicon::of(d).expect("a dialect with a reader");
    let t = spicerack::syntax::tokenize(&src, lex);
    let a = spicerack::syntax::parse(&t, lex);
    let mut work: Vec<(u32, usize)> = a.fields(a.root()).iter().rev().map(|&n| (n, 0)).collect();
    while let Some((n, depth)) = work.pop() {
        let node = Node(n);
        println!(
            "{}{:?} {:?}",
            "  ".repeat(depth),
            a.tag(node),
            head(&a, &t, node)
        );
        let kids: Vec<u32> = match a.tag(node) {
            Tag::Instance
            | Tag::Card
            | Tag::Model
            | Tag::Param
            | Tag::Options
            | Tag::Save
            | Tag::Print
            | Tag::Analysis
            | Tag::Ic
            | Tag::Nodeset
            | Tag::Temp
            | Tag::Global
            | Tag::Include
            | Tag::LibCall
            | Tag::Func
            | Tag::Measure
            | Tag::Step => a.fields(node).to_vec(),
            Tag::Subckt
            | Tag::Section
            | Tag::Library
            | Tag::If
            | Tag::Else
            | Tag::Control
            | Tag::Region
            | Tag::Data
            | Tag::Statistics
            | Tag::Process => {
                let mut k = a.fields(node).to_vec();
                k.extend(a.body(node));
                k.extend(a.next(node).map(|x| x.0));
                k
            }
            Tag::Assign => vec![a.lhs(node), a.rhs(node)],
            Tag::Seq => a.list(a.lhs(node)).to_vec(),
            Tag::Group => a.list(a.rhs(node) + 1).to_vec(),
            _ => vec![],
        };
        work.extend(
            kids.into_iter()
                .filter(|&k| k != NONE)
                .rev()
                .map(|k| (k, depth + 1)),
        );
    }
    for e in a.diags() {
        let at = t.location(e.token);
        println!("{:?} at {}:{}", e.code, at.line, at.column);
    }
}

/// The node's anchor token's text.
fn head<'s>(a: &Ast, t: &Tokens<'s>, n: Node) -> &'s str {
    t.text(a.main_token(n))
}
