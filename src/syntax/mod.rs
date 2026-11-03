//! The transpiler's front end (`docs/research/transpiler-architecture.md`):
//! text → [`token::Tokens`] by a dialect's [`lexicon::Lexicon`] → [`ast::Ast`]
//! by one parser → a [`resolve::Kit`] (a kit's files and its symbol table)
//! → deck text in any dialect ([`render`]), and the temporary bridge
//! [`ast_to_ir`] from the AST to
//! today's `CircuitIR`. `import::parse` reads every deck through it (phase
//! 2b of §5); `golden diff` against the recorded `daa0ad0` baseline, the
//! PEG reader it replaced, gates every change (TESTING.md, "Golden gate").
//!
//! "The old reader" in this module's comments is that PEG reader and its
//! lowering, at `daa0ad0` (`git show daa0ad0:grammar/spice.peg`); its files
//! are gone, so a citation names the rule it means.
//!
//! Contents:
//! - `pub mod ast`, `pub mod device`, `pub mod diag`, `pub mod lexicon`, `pub mod render`,
//!   `pub mod resolve`, `pub mod statement`, `pub mod token`
//! - `pub use tokenize::tokenize`, `pub use parse::{parse, DEPTH_MAX}`,
//!   `pub use adapter::ast_to_ir`

mod adapter;
pub mod ast;
pub mod device;
pub mod diag;
mod expr;
mod fields;
pub mod lexicon;
mod named;
mod number;
mod parse;
pub mod render;
pub mod resolve;
mod scan;
mod spice;
pub mod statement;
pub mod token;
mod tokenize;

pub use adapter::ast_to_ir;
pub use parse::{DEPTH_MAX, parse};
pub use tokenize::tokenize;

// ---- private ----

#[cfg(test)]
mod tests;

/// The logical statements of `src` but the title and `.end`, each its
/// tokens' text joined by one blank.
#[cfg(test)]
fn test_words(src: &str, lex: &'static lexicon::Lexicon) -> Vec<String> {
    use token::Tag;
    let t = tokenize(src, lex);
    let mut out = Vec::new();
    let mut line: Vec<&str> = Vec::new();
    for i in 0..t.len() as u32 {
        match t.tags()[i as usize] {
            Tag::EndOfStatement => {
                if !line.is_empty() {
                    out.push(line.join(" "));
                }
                line.clear();
            }
            Tag::Title | Tag::KwEnd | Tag::LangSpice | Tag::LangSpectre => {}
            _ => line.push(t.text(i)),
        }
    }
    out
}

/// The values of `src`'s number tokens, in order.
#[cfg(test)]
fn test_numbers(src: &str, lex: &'static lexicon::Lexicon) -> Vec<f64> {
    let t = tokenize(src, lex);
    (0..t.len() as u32)
        .filter(|&i| t.tags()[i as usize] == token::Tag::Number)
        .map(|i| t.value(i))
        .collect()
}
