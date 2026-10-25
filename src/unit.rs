//! Values with units, for `value @ u_kOhm` in Python.

use std::fmt;

/// SI prefixes, largest first: (multiplier, SPICE suffix).
const PREFIXES: [(f64, &str); 10] = [
    (1e12, "t"),
    (1e9, "g"),
    (1e6, "meg"),
    (1e3, "k"),
    (1.0, ""),
    (1e-3, "m"),
    (1e-6, "u"),
    (1e-9, "n"),
    (1e-12, "p"),
    (1e-15, "f"),
];

/// SPICE scale suffixes, lowercase, as a reader tries them: `meg` and `mil`
/// before `m`, since the shorter match first turns a megohm into a milliohm.
pub const SPICE_SCALES: &[(&str, f64)] = &[
    ("meg", 1e6), ("mil", 25.4e-6),
    ("t", 1e12), ("g", 1e9), ("k", 1e3), ("m", 1e-3),
    ("u", 1e-6), ("n", 1e-9), ("p", 1e-12), ("f", 1e-15), ("a", 1e-18),
];

/// The value ngspice gives a number token: `INPevaluate` (ngspice
/// src/spicelib/parser/inpeval.c), digit for digit. The digits make an
/// integer mantissa, then one `pow(10, e)` scales it, rounded as ngspice
/// rounds, so `500n` and `0.5u` can land one ulp apart. Text after the scale
/// letter is ignored (`10v`, `1k5`); `None` when the token holds no number.
pub fn ngspice_value(t: &str) -> Option<f64> {
    let b = t.as_bytes();
    let mut i = 0;
    let sign = match b.first() {
        Some(b'+') => { i += 1; 1.0 }
        Some(b'-') => { i += 1; -1.0 }
        _ => 1.0,
    };
    if !b.get(i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
        return None;
    }
    let digit = |i: usize| b.get(i).filter(|c| c.is_ascii_digit()).map(|c| (c - b'0') as f64);
    let (mut mantis, mut expo1) = (0.0f64, 0i32);
    while let Some(d) = digit(i) {
        mantis = 10.0 * mantis + d;
        i += 1;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while let Some(d) = digit(i) {
            mantis = 10.0 * mantis + d;
            expo1 -= 1;
            i += 1;
        }
    }
    let (mut expo2, mut expsgn) = (0i32, 1i32);
    if matches!(b.get(i), Some(b'e' | b'E' | b'd' | b'D')) {
        i += 1;
        match b.get(i) {
            Some(b'+') => i += 1,
            Some(b'-') => { i += 1; expsgn = -1 }
            _ => {}
        }
        while let Some(d) = digit(i) {
            expo2 = 10 * expo2 + d as i32;
            i += 1;
        }
    }
    let rest = t[i..].to_ascii_lowercase();
    expo1 += match rest.as_bytes().first() {
        Some(b't') => 12,
        Some(b'g') => 9,
        Some(b'k') => 3,
        Some(b'u') => -6,
        Some(b'n') => -9,
        Some(b'p') => -12,
        Some(b'f') => -15,
        Some(b'a') => -18,
        Some(b'm') if rest.starts_with("meg") => 6,
        Some(b'm') if rest.starts_with("mil") => {
            mantis *= 25.4;
            -6
        }
        Some(b'm') => -3,
        _ => 0,
    };
    // C's pow, as ngspice calls it (Rust's powi multiplies, and rounds apart).
    Some(sign * mantis * 10f64.powf((expo1 + expsgn * expo2) as f64))
}

/// `v` spelled for ngspice: the fewest digits (SI prefix, plain decimal or
/// exponent form) that [`ngspice_value`] reads back as exactly `v`, so a
/// value read from an ngspice deck is written back bit for bit. Zeros count
/// as digits there: `0.050000` is 50000e-6, another double than `0.05`.
pub fn ngspice_spelling(v: f64) -> String {
    // ponytail: per-thread memo, cleared at 64k entries. A netlist repeats few
    // distinct values (ibmpg3: 1.6M numbers, 466 distinct) and the search
    // below costs ~15 µs each; an LRU if a deck ever has 64k+ distinct ones.
    thread_local! {
        static MEMO: std::cell::RefCell<std::collections::HashMap<u64, String>> = Default::default();
    }
    MEMO.with_borrow_mut(|memo| {
        if memo.len() >= 1 << 16 {
            memo.clear();
        }
        memo.entry(v.to_bits()).or_insert_with(|| spell(v)).clone()
    })
}

fn spell(v: f64) -> String {
    for digits in 1..=17 {
        let sci = format!("{:.*e}", digits - 1, v);
        let Ok(x) = sci.parse::<f64>() else { break };
        let (m, e) = sci.split_once('e').unwrap_or((&sci, "0"));
        let e: i32 = e.parse().unwrap_or(0);
        let mut found = vec![plain(m, e), sci.clone()];
        // Past 2^53 ngspice's mantissa rounds as it accumulates, so the
        // spelling that lands on `v` may be a neighbour of `v`'s own digits.
        if digits >= 17 {
            let (sign, body) = m.strip_prefix('-').map_or(("", m), |r| ("-", r));
            let int: i128 = body.replace('.', "").parse().unwrap_or(0);
            for delta in (1..=64).flat_map(|k| [-k, k]) {
                let digs = (int + delta).to_string();
                if digs.len() == digits {
                    found.push(plain(&format!("{sign}{}.{}", &digs[..1], &digs[1..]), e));
                }
            }
        }
        if !m.ends_with('0') || digits == 1 {
            found.insert(0, crate::codegen::format_spice_number(x));
        }
        if let Some(s) = found.into_iter().find(|s| ngspice_value(s) == Some(v)) {
            return s;
        }
    }
    crate::codegen::format_spice_number(v)
}

/// The mantissa `m` (`-3.30`) times `10^e` as plain decimal, every digit kept.
fn plain(m: &str, e: i32) -> String {
    let (sign, m) = m.strip_prefix('-').map_or(("", m), |r| ("-", r));
    let digits: String = m.chars().filter(|c| c.is_ascii_digit()).collect();
    let point = 1 + e; // digits before the decimal point
    let body = if point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else if point as usize >= digits.len() {
        format!("{digits}{}", "0".repeat(point as usize - digits.len()))
    } else {
        format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
    };
    format!("{sign}{body}")
}

/// The prefix keeping `|v| / multiplier` in [1, 1000), clamped to t..f;
/// `(1.0, "")` for zero.
pub fn si_prefix(v: f64) -> (f64, &'static str) {
    if v == 0.0 {
        return (1.0, "");
    }
    PREFIXES.into_iter().find(|p| v.abs() >= p.0).unwrap_or(PREFIXES[9])
}

/// `v * mult` for a power-of-ten `mult`, rounded once: `100 @ u_ns` is the
/// f64 nearest 1e-7, where the float multiply lands an ulp above it.
pub fn scale(v: f64, mult: f64) -> f64 {
    shift(v, mult).unwrap_or(v * mult)
}

/// `v / mult` the same way: 1e-7 in ns is 100, not 99.99999999999999.
pub fn unscale(v: f64, mult: f64) -> f64 {
    shift(v, 1.0 / mult).unwrap_or(v / mult)
}

/// Moves the decimal point of `v`'s shortest exact spelling by `mult`'s
/// power of ten, then reads it back once. `None` if `mult` is no power of ten.
fn shift(v: f64, mult: f64) -> Option<f64> {
    let exp = mult.log10().round() as i32;
    if 10f64.powi(exp) != mult && 10f64.powi(-exp) != 1.0 / mult {
        return None;
    }
    let s = format!("{v:e}");
    let (m, e) = s.split_once('e')?;
    format!("{m}e{}", e.parse::<i32>().ok()? + exp).parse().ok()
}

/// The `spicerack.unit` constants: (Python name, multiplier, symbol).
pub const UNITS: &[(&str, f64, &str)] = &[
    ("u_V", 1.0, "V"),
    ("u_mV", 1e-3, "V"),
    ("u_uV", 1e-6, "V"),
    ("u_µV", 1e-6, "V"),
    ("u_A", 1.0, "A"),
    ("u_mA", 1e-3, "A"),
    ("u_uA", 1e-6, "A"),
    ("u_µA", 1e-6, "A"),
    ("u_nA", 1e-9, "A"),
    ("u_Ohm", 1.0, "Ohm"),
    ("u_Ω", 1.0, "Ohm"),
    ("u_kOhm", 1e3, "Ohm"),
    ("u_kΩ", 1e3, "Ohm"),
    ("u_MOhm", 1e6, "Ohm"),
    ("u_MΩ", 1e6, "Ohm"),
    ("u_F", 1.0, "F"),
    ("u_mF", 1e-3, "F"),
    ("u_uF", 1e-6, "F"),
    ("u_µF", 1e-6, "F"),
    ("u_nF", 1e-9, "F"),
    ("u_pF", 1e-12, "F"),
    ("u_fF", 1e-15, "F"),
    ("u_H", 1.0, "H"),
    ("u_mH", 1e-3, "H"),
    ("u_uH", 1e-6, "H"),
    ("u_µH", 1e-6, "H"),
    ("u_nH", 1e-9, "H"),
    ("u_Hz", 1.0, "Hz"),
    ("u_kHz", 1e3, "Hz"),
    ("u_MHz", 1e6, "Hz"),
    ("u_GHz", 1e9, "Hz"),
    ("u_s", 1.0, "s"),
    ("u_ms", 1e-3, "s"),
    ("u_us", 1e-6, "s"),
    ("u_µs", 1e-6, "s"),
    ("u_ns", 1e-9, "s"),
    ("u_ps", 1e-12, "s"),
    ("u_W", 1.0, "W"),
    ("u_mW", 1e-3, "W"),
    ("u_uW", 1e-6, "W"),
    ("u_µW", 1e-6, "W"),
    ("u_Degree", 1.0, "°C"),
];

/// A value in base SI units (1 kOhm is `value: 1000.0`) and its unit symbol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitValue {
    pub value: f64,
    pub symbol: &'static str,
}

impl UnitValue {
    /// SPICE number text: "1k", "10p", "3.3".
    pub fn str_spice(&self) -> String {
        crate::codegen::format_spice_number(self.value)
    }
}

/// Unrounded mantissa, SPICE suffix, symbol: "1kOhm", "3.3V".
impl fmt::Display for UnitValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (mult, suffix) = si_prefix(self.value);
        write!(f, "{}{}{}", unscale(self.value, mult), suffix, self.symbol)
    }
}

#[cfg(test)]
mod ngspice_numbers {
    use super::*;

    #[test]
    fn read_as_ngspice_reads_and_written_back_exactly() {
        assert_eq!(ngspice_value("1k5"), Some(1000.0));
        assert_eq!(ngspice_value("10%3"), Some(10.0));
        assert_eq!(ngspice_value("2mil"), Some(2.0 * 25.4 * 1e-6));
        assert_eq!(ngspice_value("abc"), None);
        assert_eq!(ngspice_value("0.5u"), Some(5.0 * 10f64.powf(-7.0)));
        for t in ["500n", "0.5u", "1.5meg", "4.7k", "0.1n", "-3.3", "1e-12", "108f", "2.2e-9", "0.000000001", "0.050000", "1.000000"] {
            let v = ngspice_value(t).unwrap();
            assert_eq!(ngspice_value(&ngspice_spelling(v)), Some(v), "{t}");
        }
        assert_eq!(ngspice_spelling(1000.0), "1k");
        assert_eq!(ngspice_value(&ngspice_spelling(0.7)), Some(0.7));
    }
}
