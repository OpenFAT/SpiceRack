//! Variable name normalization for cross-backend consistency.
//!
//! Every backend's vector names map to one lowercase canonical form:
//! - node voltages lose their `v()` wrapper and reference node: `V(OUT,0)` -> `out`
//! - branch currents become `i(name)`: `I(V1)`, `v1#branch`, Spectre/VACASK `V1:p` -> `i(v1)`
//! - LTspice's `:` hierarchy separator becomes `.`: `V(x1:internal)` -> `x1.internal`
//! - anything else (`time`, `frequency`, `onoise_spectrum`, bare Spectre nodes) is lowercased

/// Canonical form of a raw vector name from `backend`.
pub fn normalize_var_name(name: &str, backend: &str) -> String {
    let lower = name.to_lowercase();
    if let Some(current) = as_current(&lower, backend) {
        return current;
    }
    let wrapped = lower.strip_prefix("v(").or_else(|| lower.strip_prefix("v ("));
    let node = match wrapped.and_then(|s| s.strip_suffix(')')) {
        Some(inner) => inner.split(',').next().unwrap_or(inner).trim(),
        None => &lower,
    };
    hierarchy(node, backend)
}

/// True if `name` is a branch/terminal current in `backend`'s naming.
pub fn is_current_name(name: &str, backend: &str) -> bool {
    as_current(&name.to_lowercase(), backend).is_some()
}

/// `Some("i(dev)")` when lowercased `lower` names a current, else `None`.
fn as_current(lower: &str, backend: &str) -> Option<String> {
    if let Some(inner) = lower.strip_prefix("i(").and_then(|s| s.strip_suffix(')')) {
        return Some(format!("i({})", hierarchy(inner, backend)));
    }
    if let Some((dev, _)) = lower.split_once("#branch") {
        return Some(format!("i({dev})"));
    }
    // Spectre/VACASK terminal currents are `<device>:<terminal>`; inside v() a `:` is hierarchy.
    if matches!(backend, "spectre" | "vacask" | "vacask-shared") && !lower.starts_with("v(") {
        return lower.split_once(':').map(|(dev, _)| format!("i({dev})"));
    }
    None
}

fn hierarchy(name: &str, backend: &str) -> String {
    if backend == "ltspice" { name.replace(':', ".") } else { name.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ngspice ──

    #[test]
    fn test_ngspice_voltage() {
        assert_eq!(normalize_var_name("v(out)", "ngspice"), "out");
        assert_eq!(normalize_var_name("V(OUT)", "ngspice"), "out");
    }

    #[test]
    fn test_ngspice_voltage_with_ref() {
        assert_eq!(normalize_var_name("v(out,0)", "ngspice"), "out");
    }

    #[test]
    fn test_ngspice_current() {
        assert_eq!(normalize_var_name("i(v1)", "ngspice"), "i(v1)");
        assert_eq!(normalize_var_name("I(V1)", "ngspice"), "i(v1)");
    }

    #[test]
    fn test_ngspice_hierarchy() {
        assert_eq!(normalize_var_name("v(x1.internal)", "ngspice"), "x1.internal");
        assert_eq!(normalize_var_name("V(X1.INTERNAL)", "ngspice"), "x1.internal");
    }


    // ── LTspice ──

    #[test]
    fn test_ltspice_voltage() {
        assert_eq!(normalize_var_name("V(out)", "ltspice"), "out");
    }

    #[test]
    fn test_ltspice_hierarchy() {
        assert_eq!(normalize_var_name("V(x1:internal)", "ltspice"), "x1.internal");
    }

    #[test]
    fn test_ltspice_current() {
        assert_eq!(normalize_var_name("I(V1)", "ltspice"), "i(v1)");
    }

    // ── Spectre ──

    #[test]
    fn test_spectre_bare_node() {
        assert_eq!(normalize_var_name("out", "spectre"), "out");
        assert_eq!(normalize_var_name("OUT", "spectre"), "out");
    }

    #[test]
    fn test_spectre_current() {
        // Spectre uses "V1:p" for current through V1 positive terminal
        assert_eq!(normalize_var_name("V1:p", "spectre"), "i(v1)");
    }

    #[test]
    fn test_spectre_current_isrc() {
        assert_eq!(normalize_var_name("I0:src", "spectre"), "i(i0)");
    }

    #[test]
    fn test_spectre_hierarchy() {
        // Spectre uses `.` for hierarchy (same as canonical)
        assert_eq!(normalize_var_name("x1.internal", "spectre"), "x1.internal");
    }

    // ── Vacask ──

    #[test]
    fn test_vacask_bare_node() {
        assert_eq!(normalize_var_name("out", "vacask"), "out");
    }

    #[test]
    fn test_vacask_current() {
        assert_eq!(normalize_var_name("V1:p", "vacask"), "i(v1)");
    }

    // ── Sweep variables ──

    #[test]
    fn test_sweep_vars_unchanged() {
        assert_eq!(normalize_var_name("time", "ngspice"), "time");
        assert_eq!(normalize_var_name("frequency", "ngspice"), "frequency");
    }

    // ── Noise spectra ──

    #[test]
    fn test_noise_vars_preserved() {
        assert_eq!(normalize_var_name("inoise_spectrum", "ngspice"), "inoise_spectrum");
        assert_eq!(normalize_var_name("onoise_spectrum", "ngspice"), "onoise_spectrum");
    }

    // ── is_current_name ──

    #[test]
    fn test_is_current_explicit() {
        assert!(is_current_name("i(v1)", "ngspice"));
    }

    #[test]
    fn test_is_current_spectre_terminal() {
        assert!(is_current_name("V1:p", "spectre"));
        assert!(is_current_name("I0:src", "vacask"));
    }

    #[test]
    fn test_is_not_current_voltage() {
        assert!(!is_current_name("v(out)", "ngspice"));
    }

    #[test]
    fn test_is_not_current_spectre_voltage_wrapper() {
        // v(x1:internal) in Spectre context -- the `:` is inside v(), so
        // it's hierarchy, not a current
        assert!(!is_current_name("v(x1:internal)", "spectre"));
    }

    // ── Backward compatibility with clean_name behavior ──

    #[test]
    fn test_backward_compat_ngspice_defaults() {
        // These mirror the old clean_name behavior
        assert_eq!(normalize_var_name("v(out)", "ngspice"), "out");
        assert_eq!(normalize_var_name("V(NET1)", "ngspice"), "net1");
        assert_eq!(normalize_var_name("i(Vin)", "ngspice"), "i(vin)");
    }

    // ── ngspice-shared / ngspice-subprocess variants ──

    #[test]
    fn test_ngspice_variant_names() {
        assert_eq!(normalize_var_name("v(out)", "ngspice-subprocess"), "out");
        assert_eq!(normalize_var_name("v(out)", "ngspice-shared"), "out");
    }

    // ── Issue 06: Cross-backend result parity ──

    #[test]
    fn test_spectre_current_v1_p() {
        assert!(is_current_name("V1:p", "spectre"));
        assert_eq!(normalize_var_name("V1:p", "spectre"), "i(v1)");
    }

    #[test]
    fn test_same_circuit_same_normalized_keys() {
        let names_ng = ["time", "v(out)", "v(in)", "i(V1)"];
        let names_lt = ["TIME", "V(OUT)", "V(IN)", "I(V1)"];
        let names_sp = ["time", "out", "in", "V1:p"];

        let norm_ng: Vec<String> = names_ng.iter()
            .map(|n| normalize_var_name(n, "ngspice"))
            .collect();
        let norm_lt: Vec<String> = names_lt.iter()
            .map(|n| normalize_var_name(n, "ltspice"))
            .collect();
        let norm_sp: Vec<String> = names_sp.iter()
            .map(|n| normalize_var_name(n, "spectre"))
            .collect();

        assert_eq!(norm_ng, norm_lt, "ngspice vs ltspice");
        assert_eq!(norm_ng, norm_sp, "ngspice vs spectre");
    }
}
