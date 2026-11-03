//! IR identifiers -> Spectre identifiers: case folding, `\` escapes, signal
//! wrappers. Sources as in `spectre/mod.rs`.

use std::borrow::Cow;
use std::fmt::Write as _;

/// How the IR's identifiers reach the deck. `[KUN]` B.2: Spectre is
/// case-sensitive, and a SPICE-origin IR's device identifiers are
/// case-blind (an F card may sense `Vin` as `vin`), so every instance and
/// device name, and each reference to one, is folded to lowercase (`fold`)
/// to stay consistent with `inst_ref`; a Spectre-origin IR keeps its case.
/// Every identifier is then [`Name`]-escaped. Applied as each is written, so
/// `netlist` renames nothing in a copy of the IR.
#[derive(Clone, Copy)]
pub(crate) struct Names {
    pub(super) fold: bool,
    pub(super) escape: bool,
}

/// An identifier in Spectre's spelling, lowercased first when `.1`: any byte
/// outside `[A-Za-z0-9_!]` gets a `\` (UG p.62). SPICE names carry `:` `/`
/// `#` `[ ]` `.` `-` freely (post-layout `xssram/X1/M1:DRN`, KiCad
/// `Net-_R1-Pad2_`).
pub(super) struct Name<'a>(&'a str, bool, Names);

impl Names {
    /// Identifiers written exactly as the IR holds them (the xschem layout).
    pub(crate) const VERBATIM: Names = Names {
        fold: false,
        escape: false,
    };

    /// A node, model or subcircuit name.
    pub(super) fn n(self, s: &str) -> Name<'_> {
        Name(s, false, self)
    }

    /// A device or instance name, or a reference to one.
    pub(super) fn d(self, s: &str) -> Name<'_> {
        Name(s, self.fold, self)
    }
}

impl std::fmt::Display for Name<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let lower = self.1
            && !self
                .0
                .bytes()
                .all(|b| b.is_ascii() && !b.is_ascii_uppercase());
        let name: Cow<str> = if lower {
            self.0.to_lowercase().into()
        } else {
            self.0.into()
        };
        if !self.2.escape {
            return f.write_str(&name);
        }
        let plain = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '!';
        // A name that starts with a digit is read as digits only (`spectre.peg`
        // Node), so one needing escapes escapes its first digit too: `\15\+`.
        if !name.chars().all(plain) && name.starts_with(|c: char| c.is_ascii_digit()) {
            f.write_char('\\')?;
        }
        for c in name.chars() {
            if !plain(c) {
                f.write_char('\\')?;
            }
            f.write_char(c)?;
        }
        Ok(())
    }
}

/// A signal (`v(x)`, `i(x)`, bare `x`) with the name inside its wrapper
/// escaped.
pub(super) fn signal(s: &str, n: Names) -> String {
    let t = s.trim();
    match t
        .split_once('(')
        .filter(|(w, r)| matches!(*w, "v" | "V" | "i" | "I") && r.ends_with(')'))
    {
        Some((w, r)) => format!("{w}({})", n.n(&r[..r.len() - 1])),
        None => n.n(t).to_string(),
    }
}

/// Reference to another instance in the deck (probe, swept device, ...).
///
/// `[KUN]` B.2: "The language becomes case-sensitive". A SPICE-origin IR's
/// names are case-insensitive (`Vin`, `R1`), so every instance this file
/// emits is lowercased and references to them must match; a Spectre-origin
/// IR already carries the exact case Spectre used and keeps it: `keep_case`.
pub(super) fn inst_ref(name: &str, keep_case: bool) -> String {
    if keep_case {
        name.to_string()
    } else {
        name.to_lowercase()
    }
}
