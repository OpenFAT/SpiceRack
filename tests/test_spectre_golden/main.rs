//! Golden-string tests for the Spectre writer.
//!
//! Spectre is licence-gated and not installed, so nothing here can be executed
//! against the real simulator. The strongest evidence available is that the
//! text we emit matches the statement forms Cadence documents. Every assertion
//! below carries the citation it is checking.
//!
//! Sources:
//! * `[REF]` *Spectre Circuit Simulator Reference*, Product Version 19.1,
//!   January 2020 <https://ee.kpi.ua/~yv/edu/ok/book/spectre_refManual.pdf>
//! * `[CMP]` *Spectre Circuit Simulator Reference*, Product Version 5.0,
//!   September 2003 <http://eece.cu.edu.eg/~fhussien/Spectre_tutorial.pdf>
//!   (component chapters)
//! * `[KUN]` Kundert, *The Designer's Guide to SPICE and Spectre*, Appendix B
//!   <https://designers-guide.org/analysis/dg-spice/chB.pdf>
//!
//! Each deck is ngspice text written for Spectre by `syntax::render`.
//! These tests prove only "we emit what the manual describes". They do NOT
//! prove any simulation result.
//!
//! Contents: none (tests only).

use spicerack::syntax::diag::Code;
use spicerack::syntax::render::{Numbers, Source, render};
use spicerack::syntax::{Dialect, lexicon, parse, tokenize};

mod analyses;
mod components;
mod deck;

// ---- private ----

/// The ngspice deck `deck` (line 1 its title) written for Spectre, or the
/// codes of its refusal. Its numbers are read as the doubles they spell
/// (`lexicon::NGSPICE_GIVEN`, as the Python API's decks are), so `100n` is
/// 1e-7 and not ngspice's digit-by-digit 1.0000000000000001e-7.
fn spectre(deck: &str) -> Result<String, Vec<Code>> {
    spectre_with(deck, &[])
}

/// [`spectre`], with the top's Verilog-A loads given beside the text.
fn spectre_with(deck: &str, loads: &[String]) -> Result<String, Vec<Code>> {
    let lex = &lexicon::NGSPICE_GIVEN;
    let t = tokenize(deck, lex);
    let ast = parse(&t, lex);
    let src = Source {
        ast: &ast,
        tokens: &t,
        dialect: Dialect::Ngspice,
        name: deck.lines().next().unwrap_or_default(),
        kit: None,
        loads,
        numbers: Numbers::Given,
    };
    render(&src, Dialect::Spectre)
        .map(|w| w.text)
        .map_err(|d| d.diags.iter().map(|d| d.code).collect())
}

/// The written deck's last section: the analyses, or the components when the
/// deck has none. Neither contains a blank line.
fn last_block(deck: &str) -> Result<String, Vec<Code>> {
    let text = spectre(deck)?;
    Ok(text
        .rsplit("\n\n")
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_string())
}

/// The Spectre statement(s) of one ngspice analysis card.
fn analysis(card: &str) -> String {
    try_analysis(card).unwrap()
}

fn try_analysis(card: &str) -> Result<String, Vec<Code>> {
    last_block(&format!("t\n{card}\n.end\n"))
}

/// The Spectre instance of one ngspice element card.
fn component(card: &str) -> String {
    try_component(card).unwrap()
}

fn try_component(card: &str) -> Result<String, Vec<Code>> {
    last_block(&format!("t\n{card}\n.end\n"))
}
