//! The transpiler rewrite's front end (`docs/research/transpiler-architecture.md`):
//! text → [`token::Tokens`] by a dialect's [`lexicon::Lexicon`], so far.
//! Phase 1 of §5: not wired into `import`; `tests/test_syntax_tokenize.rs`
//! gates it against today's line stage and number readers.
//!
//! Contents:
//! - `pub mod lexicon`, `pub mod token`
//! - `pub use tokenize::tokenize`

pub mod lexicon;
mod number;
mod scan;
pub mod token;
mod tokenize;

pub use tokenize::tokenize;

// ---- private ----

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
