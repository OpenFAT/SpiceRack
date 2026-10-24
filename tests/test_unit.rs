use spicerack::unit::*;

/// `v @ <name>` as Python builds it.
fn at(v: f64, name: &str) -> UnitValue {
    let &(_, mult, symbol) = UNITS.iter().find(|u| u.0 == name).expect("unit in table");
    UnitValue { value: v * mult, symbol }
}

#[test]
fn every_unit_scales_and_prints() {
    let cases = [
        (3.3, "u_V", "3.3", "3.3V"),
        (100.0, "u_mV", "100m", "100mV"),
        (1.0, "u_µV", "1u", "1uV"),
        (5.0, "u_nA", "5n", "5nA"),
        (100.0, "u_Ohm", "100", "100Ohm"),
        (1.0, "u_kΩ", "1k", "1kOhm"),
        (1.0, "u_MOhm", "1meg", "1megOhm"),
        (100.0, "u_fF", "100f", "100fF"),
        (10.0, "u_pF", "10p", "10pF"),
        (1.0, "u_uH", "1u", "1uH"),
        (1.0, "u_GHz", "1g", "1gHz"),
        (100.0, "u_ns", "100n", "100ns"),
        (1.0, "u_ps", "1p", "1ps"),
        (100.0, "u_mW", "100m", "100mW"),
        (27.0, "u_Degree", "27", "27°C"),
    ];
    for (v, name, spice, shown) in cases {
        let uv = at(v, name);
        assert_eq!(uv.str_spice(), spice, "{name}");
        assert_eq!(uv.to_string(), shown, "{name}");
    }
    assert!((at(10.0, "u_pF").value - 10e-12).abs() < 1e-24);
}

#[test]
fn prefix_choice() {
    assert_eq!(si_prefix(1000.0), (1e3, "k"));
    assert_eq!(si_prefix(1e-12), (1e-12, "p"));
    assert_eq!(si_prefix(1e6), (1e6, "meg"));
    assert_eq!(si_prefix(0.001), (1e-3, "m"));
    assert_eq!(si_prefix(-2e-3), (1e-3, "m"));
    assert_eq!(si_prefix(0.0), (1.0, ""));
    assert_eq!(si_prefix(1e-18), (1e-15, "f"));
}
