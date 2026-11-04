//! The transpiler: text → [`token::Tokens`] by a dialect's
//! [`lexicon::Lexicon`] → [`ast::Ast`] by one parser → a [`resolve::Kit`]
//! (a kit's files and its symbol table) → deck text in any dialect
//! ([`render`]), with every stage's diagnostics coded in [`diag`].
//!
//! Using it (a file read from disk: `crate::import`):
//! - a kit: `import::kit(&[entry], Form::Library, Dialect::Spectre, root)`
//!   (or [`resolve::Kit::load`] with a loader of your own) reads the entry
//!   and every file it includes, in read order (`kit.files()`), each with
//!   its role and diagnostics (`kit.file(id).diags`), its definitions in
//!   `kit.definitions()`;
//! - a file in another dialect: `render::render_kit(&kit, id, name,
//!   Dialect::Ngspice)` for a kit file, `render::render_text(text, from,
//!   name, to)` for text read alone; either is `Ok(Written { text, notes })`
//!   or `Err(Diags)`, a refusal;
//! - the diagnostics: `Diags { diags, messages }`, every `P`, `R` and `W`
//!   row on the file sorted by position, and one line per row
//!   (`<deck>:<line>:<column>: <code> <title>`); `diag::info(code)` is a
//!   code's title, clause and explanation, `diag::Kind` whether it refuses.
//!
//! "The old reader" in this module's comments is the PEG reader and its
//! lowering, at `9d1d6bb` (`git show 9d1d6bb:grammar/spice.peg`); "the old
//! writer" the `codegen` writers phase 6 deleted, at `949be79`. Their files
//! are gone, so a citation names the rule it means; `golden diff` against
//! the recorded `9d1d6bb` baseline gates every change (TESTING.md, "Golden
//! gate").
//!
//! Contents:
//! - `pub mod ast`, `pub mod device`, `pub mod diag`, `pub mod lexicon`, `pub mod render`,
//!   `pub mod resolve`, `pub mod statement`, `pub mod token`
//! - `pub use dialect::Dialect`, `pub use tokenize::tokenize`, `pub use parse::{parse, DEPTH_MAX}`

pub mod ast;
pub mod device;
pub mod diag;
mod dialect;
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

pub use dialect::Dialect;
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
