//! A number token's spelling → its `f64`, by the lexicon's [`Numbers`]
//! table: the one place a number is decoded (`transpiler-architecture.md`
//! §2.1). The tokenizer calls it once per [`Tag::Number`] token.
//!
//! The grammar each [`Tail`] reads is today's `Num` rule of that dialect's
//! `grammar/*.peg`; the arithmetic is today's lowering
//! (`import/lower/values.rs` `num_value`): ngspice's `INPevaluate`
//! ([`crate::unit::ngspice_value`]), or the mantissa and shifted exponent
//! read as one decimal. `tests/test_syntax_tokenize.rs` checks every number
//! in the corpora against that oracle.
//!
//! Contents:
//! - `pub(super) fn value(text: &str, lex: &Lexicon) -> f64`
//!
//! [`Tag::Number`]: crate::syntax::token::Tag::Number

use crate::syntax::lexicon::{Arithmetic, Lexicon, Numbers, Scale, Tail};

/// `text`'s value; NaN when the lexicon's number grammar does not read all
/// of it (`1k5` in Spectre, `0x1F` in VACASK, `1µ` in ngspice).
pub(super) fn value(text: &str, lex: &Lexicon) -> f64 {
    decode(text, &lex.numbers, lex.fold_case).unwrap_or(f64::NAN)
}

// ---- private ----

fn decode(text: &str, n: &Numbers, fold: bool) -> Option<f64> {
    let b = text.as_bytes();
    let digits = |q: usize| b[q..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut mag = digits(0);
    if b.get(mag) == Some(&b'.') {
        mag += 1 + digits(mag + 1);
    }
    let mantissa = mag;
    if b.get(mag).is_some_and(|c| n.exponent.contains(c)) {
        let sign = usize::from(matches!(b.get(mag + 1), Some(b'+' | b'-')));
        let exp = digits(mag + 1 + sign);
        if exp > 0 {
            mag += 1 + sign + exp;
        }
    }
    if let Tail::Letters { hex: true, .. } = n.tail
        && matches!(b, [b'0', b'x' | b'X', h, ..] if h.is_ascii_hexdigit())
    {
        return None;
    }
    // VACASK L594-L693: a scale or a name byte right after an exponent
    // makes the exponent no exponent, and the number no number.
    if let Tail::Letters {
        after_exponent: false,
        ..
    } = n.tail
        && mag > mantissa
        && mag < b.len()
    {
        mag = mantissa;
    }
    let rest = &text[mag..];
    let scale = n.scales.iter().find(|(s, _)| {
        rest.get(..s.len()).is_some_and(|p| {
            if fold {
                p.eq_ignore_ascii_case(s)
            } else {
                p == *s
            }
        })
    });
    let after = &rest[scale.map_or(0, |(s, _)| s.len())..];
    let frac = match n.tail {
        Tail::Spice { dot } => {
            let unit = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || (dot && c == b'.');
            if !rest.bytes().all(unit) {
                return None;
            }
            None
        }
        Tail::Ltspice => {
            if scale.is_some() && !after.is_empty() && after.bytes().all(|c| c.is_ascii_digit()) {
                Some(after)
            } else if rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == 'µ')
            {
                None
            } else {
                return None;
            }
        }
        Tail::Letters { .. } => {
            let letters = after.bytes().all(|c| c.is_ascii_alphabetic() || c == b'_');
            if !(rest.is_empty() || (scale.is_some() && letters)) {
                return None;
            }
            None
        }
    };
    match n.arithmetic {
        Arithmetic::Ngspice => crate::unit::ngspice_value(text),
        Arithmetic::Decimal => {
            // values.rs `num_value`, step for step: so `0.1n` is exactly
            // 1e-10 and a decoded number equals today's to the bit.
            let mut m = text[..mag].to_ascii_lowercase().replace('d', "e");
            if let Some(f) = frac {
                m = format!("{m}.{f}");
            }
            let (base, exp) = match m.split_once('e') {
                Some((base, e)) => (base, e.parse::<i32>().ok()?),
                None => (m.as_str(), 0),
            };
            let shift = match scale.map(|(_, s)| *s) {
                Some(Scale::Mil) => return Some(m.parse::<f64>().ok()? * 25.4e-6),
                Some(Scale::Exp(k)) => k,
                None => 0,
            };
            format!("{base}e{}", exp + shift).parse().ok()
        }
    }
}
