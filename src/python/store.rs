//! The store (D3: the store is source files): a deck's or a subcircuit's
//! ngspice source text behind a first line naming its dialect, and that
//! text read back. A subcircuit reads back into the cards its calls wrote;
//! a testbench's deck into its top's statements, as written. Text in
//! another dialect is first written as ngspice (`syntax::render`).
//!
//! Cites: ngspice manual §2.4 (`.subckt`), §2.6 (`.include`), §2.7
//! (`.lib`), §16 (`.control`, `pre_osdi`).
//!
//! Contents:
//! - `pub(super) const HEADER: &str`
//! - `pub(super) fn to_source(b: &Body) -> Result<String, String>`
//! - `pub(super) fn from_source(text: &str, d: Option<Dialect>) -> Result<Body, String>`
//! - `pub(super) fn with_loads(text: &str, loads: &[String]) -> String`
//! - `pub(super) fn read_deck(text: &str, d: Option<Dialect>) -> Result<Body, String>`

use std::borrow::Cow;
use std::fmt::Write as _;

use super::model::{Body, Card, params};
use crate::syntax::ast::{Ast, Node, Tag};
use crate::syntax::lexicon::NGSPICE_GIVEN;
use crate::syntax::token::{Tag as T, Tokens};
use crate::syntax::{Dialect, parse, tokenize};

/// A stored text's first line, the name of the dialect after it following
/// (`* spicerack-source ngspice`): a comment in every SPICE dialect and in
/// Spectre.
pub(super) const HEADER: &str = "* spicerack-source ";

/// `b` as the stored text of an ngspice deck that defines it
/// ([`with_loads`]): `.subckt` with its includes, `.lib` calls, models,
/// cards in the order the deck writes them, and its lines given as text.
///
/// # Errors
/// A `verilog()` block, which has no source form until it is lowered (a
/// run lowers it).
pub(super) fn to_source(b: &Body) -> Result<String, String> {
    if !b.verilog.is_empty() {
        return Err(format!(
            "{}: a verilog() block has no source form; it is lowered when the deck is written",
            b.name
        ));
    }
    let mut out = format!("{}\n", b.name);
    let ps = match b.params.is_empty() {
        true => String::new(),
        false => format!(" PARAMS:{}", params(&b.params)),
    };
    let ports: String = b.ports.iter().map(|p| format!(" {p}")).collect();
    w(&mut out, format_args!(".subckt {}{ports}{ps}\n", b.name));
    b.includes
        .iter()
        .for_each(|i| w(&mut out, format_args!(".include {i}\n")));
    b.libs
        .iter()
        .for_each(|(p, s)| w(&mut out, format_args!(".lib {p} {s}\n")));
    let cards = b
        .models
        .iter()
        .chain(b.elements.iter().chain(&b.instances).map(|c| &c.text));
    cards
        .chain(&b.raw)
        .for_each(|l| w(&mut out, format_args!("{l}\n")));
    w(&mut out, format_args!(".ends {}\n.end\n", b.name));
    Ok(with_loads(&out, &b.osdi))
}

/// The subcircuit `text` defines: its first `.subckt`, and the loads at
/// its top. `d` is the text's dialect; `None` reads it off a [`HEADER`]
/// line, ngspice without one. A card keeps its text; one this API does not
/// write (a `.param`, an option) is kept as a line given as text.
///
/// # Errors
/// As [`read_deck`]; text that defines no subcircuit.
pub(super) fn from_source(text: &str, d: Option<Dialect>) -> Result<Body, String> {
    let text = ngspice_text(text, d)?;
    let t = tokenize(&text, &NGSPICE_GIVEN);
    let a = parse(&t, &NGSPICE_GIVEN);
    let top = a.fields(a.root());
    let sc = top.iter().copied().find(|&n| a.tag(Node(n)) == Tag::Subckt);
    let Some(sc) = sc else {
        return Err("the text defines no subcircuit (.subckt)".into());
    };
    let mut b = head(&statement(&a, &t, sc));
    for &n in top {
        if a.tag(Node(n)) == Tag::Control {
            b.osdi.extend(loads(&a, &t, n));
        }
    }
    for &n in a.body(Node(sc)) {
        let line = statement(&a, &t, n);
        let first = line
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let rest = || line[first.len()..].trim().to_string();
        match first.as_str() {
            ".model" => b.models.push(line.clone()),
            ".include" | ".inc" => b.includes.push(rest()),
            ".lib" => match rest().rsplit_once(' ') {
                Some((p, s)) => b.libs.push((p.trim().into(), s.into())),
                None => b.raw.push(line.clone()),
            },
            w if a.tag(Node(n)) == Tag::Instance => {
                let card = Card::element(&first[1..], line.clone());
                match w.starts_with('x') {
                    true => b.instances.push(card),
                    false => b.elements.push(card),
                }
            }
            _ => b.raw.push(line.clone()),
        }
    }
    Ok(b)
}

/// `text`, an ngspice deck's source (its title on the first line), stored:
/// [`HEADER`] `ngspice`, the title, then `loads` as a `.control` block of
/// `pre_osdi` lines (ngspice's form, which [`read_deck`] reads back as
/// loads), then the rest.
pub(super) fn with_loads(text: &str, loads: &[String]) -> String {
    let (title, rest) = text.split_once('\n').unwrap_or((text, ""));
    let mut out = format!("{HEADER}ngspice\n{title}\n");
    if !loads.is_empty() {
        out.push_str(".control\n");
        loads
            .iter()
            .for_each(|p| w(&mut out, format_args!("pre_osdi {p}\n")));
        out.push_str(".endc\n");
    }
    out.push_str(rest);
    out
}

/// The deck stored `text` holds, as a top block: its title the name, a
/// top-level `.control` block of `pre_osdi` lines alone its loads, and
/// every other statement but `.end` a line given as text, its bytes as
/// written (comments and blank lines after it included), so [`with_loads`]
/// of the deck written from it is `text` again. `d` as for [`from_source`].
///
/// # Errors
/// A dialect with no reader; text the renderer refuses to write as
/// ngspice; a [`HEADER`] naming no dialect.
pub(super) fn read_deck(text: &str, d: Option<Dialect>) -> Result<Body, String> {
    let text = ngspice_text(text, d)?;
    let t = tokenize(&text, &NGSPICE_GIVEN);
    let a = parse(&t, &NGSPICE_GIVEN);
    let top = a.fields(a.root());
    let line_start = |n: u32| {
        let at = t.start(a.main_token(Node(n))) as usize;
        text[..at].rfind('\n').map_or(0, |i| i + 1)
    };
    let starts: Vec<usize> = top.iter().map(|&n| line_start(n)).collect();
    assert!(starts.is_sorted(), "the top statements are in source order");
    let mut b = Body {
        name: text.lines().next().unwrap_or_default().trim().to_string(),
        ..Body::default()
    };
    for (i, &n) in top.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(text.len());
        let span = &text[starts[i]..end];
        let kept = match a.tag(Node(n)) {
            Tag::End => continue,
            // What follows the title line, up to the first statement.
            Tag::Title => span.split_once('\n').map_or("", |(_, r)| r),
            // What follows the block's `.endc` line.
            Tag::Control if is_loads(&a, &t, n) => {
                b.osdi.extend(loads(&a, &t, n));
                let endc = span
                    .to_ascii_lowercase()
                    .find(".endc")
                    .unwrap_or(span.len());
                span[endc..].split_once('\n').map_or("", |(_, r)| r)
            }
            _ => span,
        };
        if !kept.is_empty() {
            b.raw
                .push(kept.strip_suffix('\n').unwrap_or(kept).to_string());
        }
    }
    Ok(b)
}

// ---- private ----

fn w(s: &mut String, args: std::fmt::Arguments<'_>) {
    s.write_fmt(args).expect("a String takes any write");
}

/// Statement `n`'s text: its first token to its last, continuation lines
/// joined as written.
fn statement(a: &Ast, t: &Tokens, n: u32) -> String {
    let first = a.main_token(Node(n));
    let eos = (first..t.len() as u32)
        .find(|&k| t.tags()[k as usize] == T::EndOfStatement)
        .expect("every statement ends");
    let last = (first..eos).last().unwrap_or(first);
    t.source()[t.start(first) as usize..t.end(last) as usize].to_string()
}

/// The `.subckt` line's name, ports and `PARAMS:` defaults (`k=v`).
fn head(line: &str) -> Body {
    let mut words = words(line).into_iter().skip(1);
    let name = words.next().unwrap_or_default();
    let mut b = Body {
        name,
        ..Body::default()
    };
    for w in words {
        if w.eq_ignore_ascii_case("params:") {
            continue;
        }
        match w.split_once('=') {
            Some((k, v)) => b.params.push((k.into(), v.into())),
            None => b.ports.push(w),
        }
    }
    b
}

/// `line` split at blanks outside brackets and quotes.
fn words(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut quote) = (Vec::new(), String::new(), 0i32, false);
    for c in line.chars() {
        match c {
            '\'' | '"' => quote = !quote,
            '(' | '{' | '[' if !quote => depth += 1,
            ')' | '}' | ']' if !quote => depth -= 1,
            c if c.is_whitespace() && depth == 0 && !quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `text` as ngspice text: its [`HEADER`] line left out, read in `d` (or
/// the header's dialect, or ngspice), another dialect's written as ngspice.
fn ngspice_text(text: &str, d: Option<Dialect>) -> Result<Cow<'_, str>, String> {
    let (named, body) = match text.strip_prefix(HEADER) {
        Some(rest) => {
            let (name, body) = rest.split_once('\n').unwrap_or((rest, ""));
            (Some(name.trim().parse::<Dialect>()?), body)
        }
        None => (None, text),
    };
    let d = d.or(named).unwrap_or(Dialect::Ngspice);
    if d.family() == Dialect::Ngspice {
        return Ok(Cow::Borrowed(body));
    }
    if crate::syntax::lexicon::of(d).is_none() {
        return Err(format!("no {} reader", d.name()));
    }
    let w = crate::syntax::render::render_text(body, d, "source", Dialect::Ngspice);
    Ok(Cow::Owned(w.map_err(|e| e.messages)?.text))
}

/// Whether `.control` block `n` holds `pre_osdi` lines and nothing else.
fn is_loads(a: &Ast, t: &Tokens, n: u32) -> bool {
    let body = a.body(Node(n));
    !body.is_empty() && loads(a, t, n).len() == body.len()
}

/// The paths a `.control` block's `pre_osdi` lines load.
fn loads(a: &Ast, t: &Tokens, n: u32) -> Vec<String> {
    let lines = a.body(Node(n)).iter().map(|&k| statement(a, t, k));
    let paths = lines.filter_map(|l| {
        let (cmd, path) = l.split_once(char::is_whitespace)?;
        cmd.eq_ignore_ascii_case("pre_osdi")
            .then(|| path.trim().to_string())
    });
    paths.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Body {
        Body {
            name: "amp".into(),
            ports: vec!["in".into(), "out".into()],
            params: vec![("w".into(), "1u".into())],
            elements: vec![Card::element("1", "R1 in out {w*1k}".into())],
            instances: vec![Card::element("u1", "Xu1 out 0 load r=2k".into())],
            models: vec![".model dm D (is=1e-14)".into()],
            raw: vec![".options reltol=1e-6".into()],
            includes: vec!["/m.lib".into()],
            libs: vec![("/pdk.lib".into(), "tt".into())],
            osdi: vec!["/m.osdi".into()],
            ..Body::default()
        }
    }

    #[test]
    fn a_subcircuit_reads_back_as_the_same_cards() {
        // The store's oracle is its own text: read back and written again,
        // byte for byte (D3: the source is the lossless form). Checks: 3.
        let text = to_source(&sample()).unwrap();
        assert!(text.starts_with("* spicerack-source ngspice\namp\n.control\n"));
        let again = from_source(&text, None).unwrap();
        assert_eq!(to_source(&again).unwrap(), text);
        assert_eq!(again.instances[0].name, "u1");
    }

    #[test]
    fn a_deck_reads_back_as_its_statements_and_loads() {
        // ngspice manual §16 (`pre_osdi` in `.control`): the loads block
        // comes back as loads, every other statement as written, the blank
        // line before `.subckt` kept. Checks: 4.
        let deck = "tb\n\n.global vdd\n\n.subckt c a\nR1 a 0 1k\n.ends c\nX1 n c\nsimulator lang=spectre\nhb1 hb fundfreqs=[1G] maxharms=[5]\nsimulator lang=spice\n.end\n";
        let text = with_loads(deck, &["/m.osdi".into()]);
        let b = read_deck(&text, None).unwrap();
        assert_eq!(b.name, "tb");
        assert_eq!(b.osdi, ["/m.osdi"]);
        assert_eq!(
            b.raw.len(),
            5,
            "the blank line after the loads: {:?}",
            b.raw
        );
        let raw: String = b.raw.iter().map(|l| format!("{l}\n")).collect();
        assert_eq!(with_loads(&format!("tb\n{raw}.end\n"), &b.osdi), text);
    }

    #[test]
    fn a_header_names_the_dialect_and_text_needs_a_subcircuit() {
        assert!(from_source("t\nR1 a 0 1k\n.end\n", None).is_err());
        assert!(
            from_source("t\n", Some(Dialect::Cdl)).is_err(),
            "no CDL reader"
        );
        assert!(from_source("* spicerack-source nonesuch\nt\n", None).is_err());
        // Spectre text is written as ngspice first (Spectre UG p.94); the
        // header names it, and an argument overrides the header.
        let spectre = "// t\nsubckt cell (a b)\nr1 (a b) resistor r=1k\nends cell\n";
        for (text, d) in [
            (format!("{HEADER}spectre\n{spectre}"), None),
            (
                format!("{HEADER}ngspice\n{spectre}"),
                Some(Dialect::Spectre),
            ),
        ] {
            let b = from_source(&text, d).unwrap();
            assert_eq!(
                (b.name.as_str(), b.ports.len(), b.elements.len()),
                ("cell", 2, 1)
            );
        }
    }
}
