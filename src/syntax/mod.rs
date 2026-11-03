//! The transpiler rewrite's front end (`docs/research/transpiler-architecture.md`):
//! text → [`token::Tokens`] by a dialect's [`lexicon::Lexicon`] → [`ast::Ast`]
//! by one parser, and the temporary bridge [`ast_to_ir`] from the AST to
//! today's `CircuitIR`. Phases 1 and 2 of §5: not wired into `import`;
//! `tests/test_syntax_tokenize.rs` gates the tokenizer against today's line
//! stage and number readers, `tests/test_syntax_adapter.rs` and `golden
//! record --frontend syntax` gate `ast_to_ir(parse(tokenize(..)))` against
//! `import::parse`.
//!
//! Contents:
//! - `pub mod ast`, `pub mod device`, `pub mod diag`, `pub mod lexicon`, `pub mod token`
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
mod scan;
mod spice;
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
