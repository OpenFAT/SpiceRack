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

/// The prefix keeping `|v| / multiplier` in [1, 1000), clamped to t..f;
/// `(1.0, "")` for zero.
pub fn si_prefix(v: f64) -> (f64, &'static str) {
    if v == 0.0 {
        return (1.0, "");
    }
    PREFIXES.into_iter().find(|p| v.abs() >= p.0).unwrap_or(PREFIXES[9])
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
        write!(f, "{}{}{}", self.value / mult, suffix, self.symbol)
    }
}
