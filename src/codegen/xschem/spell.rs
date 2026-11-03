//! Names, nets and instances -> their spelling in one xschem dialect.
//!
//! Contents:
//! - `pub fn word(d: Dialect) -> &'static str`
//! - `pub fn is_spice(d: Dialect) -> bool`
//! - `pub fn comment(d: Dialect) -> &'static str`
//! - `pub fn push_ident(out: &mut String, name: &str, d: Dialect)`
//! - `pub fn element_name(line: &str, d: Dialect) -> &str`
//! - `pub fn push_element_name(out: &mut String, name: &str, d: Dialect)`
//! - `pub fn push_instance<'a>(out: &mut String, name: &str, subcircuit: &str, nets: impl IntoIterator<Item = &'a str>, ports: impl IntoIterator<Item = &'a str>, parameters: impl IntoIterator<Item = (&'a str, &'a str)>, d: Dialect)`

use crate::codegen::Dialect;

/// The dialect word (`ngspice`, …).
pub fn word(d: Dialect) -> &'static str {
    d.name()
}

/// SPICE syntax: verbatim names, `*` comments, `.subckt` blocks.
pub fn is_spice(d: Dialect) -> bool {
    !matches!(d, Dialect::Spectre | Dialect::Vacask | Dialect::Verilog)
}

/// The comment leader.
pub fn comment(d: Dialect) -> &'static str {
    if is_spice(d) { "*" } else { "//" }
}

/// Appends `name` as `d` spells an identifier: SPICE dialects verbatim; Spectre
/// escapes every character outside `[A-Za-z0-9_]` with `\`; Verilog writes a name
/// outside `[A-Za-z_][A-Za-z0-9_$]*` as an escaped identifier (`\d[3] `).
#[inline]
pub fn push_ident(out: &mut String, name: &str, d: Dialect) {
    match d {
        Dialect::Spectre | Dialect::Vacask => {
            for c in name.chars() {
                if !(c.is_ascii_alphanumeric() || c == '_') {
                    out.push('\\');
                }
                out.push(c);
            }
        }
        Dialect::Verilog => {
            let b = name.as_bytes();
            let simple = !b.is_empty()
                && (b[0].is_ascii_alphabetic() || b[0] == b'_')
                && b.iter()
                    .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'$');
            if simple {
                out.push_str(name);
            } else {
                out.push('\\');
                out.push_str(name);
                out.push(' ');
            }
        }
        _ => out.push_str(name),
    }
}

/// The element name of a written element line, its sort key: the first token, the
/// second for Verilog (`<module> <instance> (…);`); empty for a blank line.
pub fn element_name(line: &str, d: Dialect) -> &str {
    let skip = usize::from(d == Dialect::Verilog);
    line.split_ascii_whitespace().nth(skip).unwrap_or("")
}

/// Appends the element name instance `name` is written with: SPICE dialects add the
/// subcircuit letter `X` unless the name starts with it (ASCII case-insensitively),
/// so `X1` stays `X1` and `M1` is `XM1`; Spectre and Verilog escape the name. A
/// caller's name map keys on this, so it cannot drift from the deck.
pub fn push_element_name(out: &mut String, name: &str, d: Dialect) {
    if !is_spice(d) {
        push_ident(out, name, d);
        return;
    }
    if !name
        .as_bytes()
        .first()
        .is_some_and(|b| b.eq_ignore_ascii_case(&b'X'))
    {
        out.push('X');
    }
    out.push_str(name);
}

/// Appends a subcircuit instance as [`write`](super::write) spells an [`Instance`](crate::ir::Instance), without one:
/// `X<name> <nets> <subckt>` (SPICE), `<name> (<nets>) <subckt>` (Spectre),
/// `<subckt> <name> (.<port>(<net>), …);` (Verilog, pairing `ports`, the definition's
/// port names, with `nets`; the other dialects ignore `ports`); parameters as ` k=v`
/// (none for Verilog). A net `?` (an unconnected pin) is written as is. No newline.
///
/// For a caller that renders its rows into [`Lines`](super::Lines): no `String` per connection.
pub fn push_instance<'a>(
    out: &mut String,
    name: &str,
    subcircuit: &str,
    nets: impl IntoIterator<Item = &'a str>,
    ports: impl IntoIterator<Item = &'a str>,
    parameters: impl IntoIterator<Item = (&'a str, &'a str)>,
    d: Dialect,
) {
    match d {
        Dialect::Spectre => {
            push_element_name(out, name, d);
            out.push_str(" (");
            for (k, n) in nets.into_iter().enumerate() {
                if k > 0 {
                    out.push(' ');
                }
                push_net(out, n, d);
            }
            out.push_str(") ");
            push_ident(out, subcircuit, d);
        }
        Dialect::Verilog => {
            push_ident(out, subcircuit, d);
            out.push(' ');
            push_element_name(out, name, d);
            out.push_str(" (");
            for (k, (p, n)) in ports.into_iter().zip(nets).enumerate() {
                if k > 0 {
                    out.push_str(", ");
                }
                out.push('.');
                push_ident(out, p, d);
                out.push('(');
                push_net(out, n, d);
                out.push(')');
            }
            out.push_str(");");
            return;
        }
        _ => {
            push_element_name(out, name, d);
            for n in nets {
                out.push(' ');
                push_net(out, n, d);
            }
            out.push(' ');
            push_ident(out, subcircuit, d);
        }
    }
    for (k, v) in parameters {
        out.push(' ');
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
}

// ---- private ----

/// Appends a net: `?` (an unconnected pin) as is, any other name as an identifier.
#[inline]
fn push_net(out: &mut String, net: &str, d: Dialect) {
    if net == "?" {
        out.push('?')
    } else {
        push_ident(out, net, d)
    }
}
