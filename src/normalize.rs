//! Variable name normalization for cross-backend consistency.
//!
//! Every backend's vector names map to one lowercase canonical form:
//! - node voltages lose their `v()` wrapper and reference node: `V(OUT,0)` -> `out`
//! - branch currents become `i(name)`: `I(V1)`, `v1#branch`, Spectre/VACASK `V1:p` -> `i(v1)`
//! - LTspice's `:` hierarchy separator becomes `.`: `V(x1:internal)` -> `x1.internal`
//! - anything else (`time`, `frequency`, `onoise_spectrum`, bare Spectre nodes) is lowercased

use crate::ir::Dialect;

/// Canonical form of a raw vector name written by `backend` (`None`: named
/// as ngspice names it), and whether it names a current. One lowercased copy
/// of `name`, plus one more for a name it rewrites.
pub fn canonical(name: &str, backend: Option<Dialect>) -> (String, bool) {
    let lower = name.to_lowercase();
    let lt = backend == Some(Dialect::Ltspice);
    let hierarchy = |s: &str| {
        if lt {
            s.replace(':', ".")
        } else {
            s.to_string()
        }
    };
    if let Some(inner) = lower.strip_prefix("i(").and_then(|s| s.strip_suffix(')')) {
        return (format!("i({})", hierarchy(inner)), true);
    }
    if let Some((dev, _)) = lower.split_once("#branch") {
        return (format!("i({dev})"), true);
    }
    // Spectre/VACASK terminal currents are `<device>:<terminal>`; inside v() a `:` is hierarchy.
    if matches!(backend, Some(Dialect::Spectre | Dialect::Vacask))
        && !lower.starts_with("v(")
        && let Some((dev, _)) = lower.split_once(':')
    {
        return (format!("i({dev})"), true);
    }
    let wrapped = lower
        .strip_prefix("v(")
        .or_else(|| lower.strip_prefix("v ("));
    match wrapped.and_then(|s| s.strip_suffix(')')) {
        Some(inner) => (
            hierarchy(inner.split(',').next().unwrap_or(inner).trim()),
            false,
        ),
        None if lt => (hierarchy(&lower), false),
        None => (lower, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(name: &str, backend: &str) -> String {
        canonical(name, Some(backend.parse().unwrap())).0
    }

    fn cur(name: &str, backend: &str) -> bool {
        canonical(name, Some(backend.parse().unwrap())).1
    }

    #[test]
    fn no_backend_reads_ngspice_names() {
        for name in ["v(x1:a)", "I(V1)", "v1#branch", "V1:p", "time"] {
            assert_eq!(
                canonical(name, None),
                canonical(name, Some(Dialect::Ngspice)),
                "{name}"
            );
        }
        assert_eq!(canonical("v1#branch", None), ("i(v1)".to_string(), true));
        assert_eq!(canonical("V1:p", None), ("v1:p".to_string(), false));
    }

    // ── ngspice ──

    #[test]
    fn test_ngspice_voltage() {
        assert_eq!(norm("v(out)", "ngspice"), "out");
        assert_eq!(norm("V(OUT)", "ngspice"), "out");
    }

    #[test]
    fn test_ngspice_voltage_with_ref() {
        assert_eq!(norm("v(out,0)", "ngspice"), "out");
    }

    #[test]
    fn test_ngspice_current() {
        assert_eq!(norm("i(v1)", "ngspice"), "i(v1)");
        assert_eq!(norm("I(V1)", "ngspice"), "i(v1)");
    }

    #[test]
    fn test_ngspice_hierarchy() {
        assert_eq!(norm("v(x1.internal)", "ngspice"), "x1.internal");
        assert_eq!(norm("V(X1.INTERNAL)", "ngspice"), "x1.internal");
    }

    // ── LTspice ──

    #[test]
    fn test_ltspice_voltage() {
        assert_eq!(norm("V(out)", "ltspice"), "out");
    }

    #[test]
    fn test_ltspice_hierarchy() {
        assert_eq!(norm("V(x1:internal)", "ltspice"), "x1.internal");
    }

    #[test]
    fn test_ltspice_current() {
        assert_eq!(norm("I(V1)", "ltspice"), "i(v1)");
    }

    // ── Spectre ──

    #[test]
    fn test_spectre_bare_node() {
        assert_eq!(norm("out", "spectre"), "out");
        assert_eq!(norm("OUT", "spectre"), "out");
    }

    #[test]
    fn test_spectre_current() {
        // Spectre uses "V1:p" for current through V1 positive terminal
        assert_eq!(norm("V1:p", "spectre"), "i(v1)");
    }

    #[test]
    fn test_spectre_current_isrc() {
        assert_eq!(norm("I0:src", "spectre"), "i(i0)");
    }

    #[test]
    fn test_spectre_hierarchy() {
        // Spectre uses `.` for hierarchy (same as canonical)
        assert_eq!(norm("x1.internal", "spectre"), "x1.internal");
    }

    // ── Vacask ──

    #[test]
    fn test_vacask_bare_node() {
        assert_eq!(norm("out", "vacask"), "out");
    }

    #[test]
    fn test_vacask_current() {
        assert_eq!(norm("V1:p", "vacask"), "i(v1)");
    }

    // ── Sweep variables ──

    #[test]
    fn test_sweep_vars_unchanged() {
        assert_eq!(norm("time", "ngspice"), "time");
        assert_eq!(norm("frequency", "ngspice"), "frequency");
    }

    // ── Noise spectra ──

    #[test]
    fn test_noise_vars_preserved() {
        assert_eq!(norm("inoise_spectrum", "ngspice"), "inoise_spectrum");
        assert_eq!(norm("onoise_spectrum", "ngspice"), "onoise_spectrum");
    }

    // ── is_current_name ──

    #[test]
    fn test_is_current_explicit() {
        assert!(cur("i(v1)", "ngspice"));
    }

    #[test]
    fn test_is_current_spectre_terminal() {
        assert!(cur("V1:p", "spectre"));
        assert!(cur("I0:src", "vacask"));
    }

    #[test]
    fn test_is_not_current_voltage() {
        assert!(!cur("v(out)", "ngspice"));
    }

    #[test]
    fn test_is_not_current_spectre_voltage_wrapper() {
        // v(x1:internal) in Spectre context -- the `:` is inside v(), so
        // it's hierarchy, not a current
        assert!(!cur("v(x1:internal)", "spectre"));
    }

    // ── Issue 06: Cross-backend result parity ──

    #[test]
    fn test_same_circuit_same_normalized_keys() {
        let names_ng = ["time", "v(out)", "v(in)", "i(V1)"];
        let names_lt = ["TIME", "V(OUT)", "V(IN)", "I(V1)"];
        let names_sp = ["time", "out", "in", "V1:p"];

        let norm_ng: Vec<String> = names_ng.iter().map(|n| norm(n, "ngspice")).collect();
        let norm_lt: Vec<String> = names_lt.iter().map(|n| norm(n, "ltspice")).collect();
        let norm_sp: Vec<String> = names_sp.iter().map(|n| norm(n, "spectre")).collect();

        assert_eq!(norm_ng, norm_lt, "ngspice vs ltspice");
        assert_eq!(norm_ng, norm_sp, "ngspice vs spectre");
    }
}
