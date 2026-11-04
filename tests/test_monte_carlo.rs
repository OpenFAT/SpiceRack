//! SPICE statistical calls and Spectre `statistics` blocks across dialects:
//! the golden text of the Spectre lifting (T-G1 to T-G10) and the round
//! trips (T-R1 to T-R5).
//!
//! Oracles: the expected Spectre text, worked by hand from the mapping
//! (each call `rack_f(args, v)` of a variable `v` named after its owner and
//! field, varied `dist=gauss std=1` or `dist=unif N=1`, `process` at the top
//! and `mismatch` in a subcircuit; Spectre Reference 19.1 pp.175-179,
//! p.481, ngspice `xpressn.c:45-85`); for the round trips, the
//! source text itself (SPICE -> Spectre -> SPICE gives each expression
//! back) and the Spectre table read back equal; for the PDK slices, an
//! independent count of call sites in the source expressions. The ngspice
//! runs (T-N0, T-N1) are `src/codegen/statistics/tests.rs`; the Spectre
//! runs (T-S1 to T-S3) are `tests/spectre/`.
//!
//! Contents: none (tests only).

use std::path::Path;

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::parse;
use spicerack::ir::*;

// ---- private ----

fn spectre(deck: &str) -> String {
    let (ir, report) = parse(deck, "t", Dialect::Ngspice).unwrap();
    assert_eq!(report, Default::default(), "{deck}");
    netlist(&ir, Dialect::Spectre).unwrap_or_else(|e| panic!("{e}\n{deck}"))
}

/// The `vary` lines of a Spectre deck, `(scope, line)`.
fn varies(scs: &str) -> Vec<(String, String)> {
    let mut scope = String::new();
    let mut out = Vec::new();
    for l in scs.lines().map(str::trim) {
        if let Some(s) = l.strip_suffix(" {") {
            scope = s.to_string();
        } else if l.starts_with("vary ") {
            out.push((scope.clone(), l.to_string()));
        }
    }
    out
}

/// T-G1: `r1 a 0 r='agauss(1k,100,3)'`.
#[test]
fn t_g1_agauss_becomes_a_normalized_variable() {
    let scs = spectre("t\nr1 a 0 r='agauss(1k,100,3)'\n.op\n.end\n");
    assert!(
        scs.contains("r1 (a 0) resistor r=rack_agauss(1k,100,3, rack_r1_r_1)"),
        "{scs}"
    );
    assert!(scs.contains("rack_r1_r_1=0"), "{scs}");
    assert_eq!(
        varies(&scs),
        [("process".into(), "vary rack_r1_r_1 dist=gauss std=1".into())]
    );
    assert!(
        scs.contains("real rack_agauss(real nom, real avar, real sigma, real z) {"),
        "{scs}"
    );
}

/// T-G2: `gauss`, `aunif`, `unif`, `limit`; the last three uniform on [-1, 1].
#[test]
fn t_g2_each_function() {
    for (call, f, dist) in [
        (
            "gauss(1k,0.1,3)",
            "rack_gauss(1k,0.1,3, rack_r1_r_1)",
            "dist=gauss std=1",
        ),
        (
            "aunif(1k,100)",
            "rack_aunif(1k,100, rack_r1_r_1)",
            "dist=unif N=1",
        ),
        (
            "unif(1k,0.1)",
            "rack_unif(1k,0.1, rack_r1_r_1)",
            "dist=unif N=1",
        ),
        (
            "limit(1k,50)",
            "rack_limit(1k,50, rack_r1_r_1)",
            "dist=unif N=1",
        ),
    ] {
        let scs = spectre(&format!("t\nr1 a 0 r='{call}'\n.op\n.end\n"));
        assert!(scs.contains(f), "{scs}");
        assert_eq!(
            varies(&scs),
            [("process".into(), format!("vary rack_r1_r_1 {dist}"))]
        );
    }
}

/// T-G3: two calls in one expression, and one nested in another, give two
/// variables each.
#[test]
fn t_g3_two_calls_two_variables() {
    for e in ["agauss(1k,1,1)+aunif(0,2)", "agauss(agauss(0,1,1),1,1)"] {
        let scs = spectre(&format!("t\n.param p='{e}'\nr1 a 0 {{p}}\n.op\n.end\n"));
        let names: Vec<String> = varies(&scs)
            .into_iter()
            .map(|(_, l)| l.split(' ').nth(1).unwrap().to_string())
            .collect();
        assert_eq!(names, ["rack_p_1", "rack_p_2"], "{scs}");
    }
    let scs = spectre("t\n.param p='agauss(agauss(0,1,1),1,1)'\nr1 a 0 {p}\n.op\n.end\n");
    assert!(
        scs.contains("p=rack_agauss(rack_agauss(0,1,1, rack_p_1),1,1, rack_p_2)"),
        "{scs}"
    );
}

/// T-G4: a subcircuit's local, model and instance are `mismatch`; a top
/// model and a top `.param` are `process`.
#[test]
fn t_g4_placement() {
    let scs = spectre(
        "t\n.param tp='agauss(0,1,1)'\n.model tm nmos level=1 vto='agauss(1,0.1,1)'\n.subckt s a b\n.param loc='agauss(0,1,1)'\n.model sm nmos level=1 vto='agauss(1,0.1,1)'\nR1 a b r='agauss(1k,10,1)'\nX1 a b leaf w='agauss(1,0.1,1)'\n.ends s\n.subckt leaf a b w=1\nR1 a b {w*1k}\n.ends leaf\nX1 a 0 s\n.op\n.end\n",
    );
    let got = varies(&scs);
    let scope = |name: &str| {
        got.iter()
            .find(|(_, l)| l.contains(name))
            .map(|(s, _)| s.as_str())
            .unwrap_or_else(|| panic!("{name}:\n{scs}"))
    };
    assert_eq!(scope("rack_tp_1 "), "process");
    assert_eq!(scope("rack_tm_vto_1 "), "process");
    assert_eq!(scope("rack_s_loc_1 "), "mismatch");
    assert_eq!(scope("rack_s_sm_vto_1 "), "mismatch");
    assert_eq!(scope("rack_s_r1_r_1 "), "mismatch");
    assert_eq!(scope("rack_s_x1_w_1 "), "mismatch");
}

/// T-G5: IHP's gated mismatch keeps its arguments and writes no `std=0`.
#[test]
fn t_g5_gated_sigma_stays_an_argument() {
    let scs = spectre(
        "t\n.subckt n d g w=1u dw_mm=1n mm_ok=0\nR1 d g r={1k*w} w='agauss(w, dw_mm, (mm_ok != 1 ? 0 : 1))'\n.ends n\nX1 a 0 n\n.op\n.end\n",
    );
    assert!(
        scs.contains("rack_agauss(w, dw_mm, (mm_ok != 1 ? 0 : 1), rack_n_r1_w_1)"),
        "{scs}"
    );
    assert!(!scs.contains("std=0"), "{scs}");
}

/// T-G6: sky130 `parameters_fet_tt.spice:21` keeps the nested `EXP` and the
/// `+` composition.
#[test]
fn t_g6_sky130_exp_composition() {
    let scs = spectre(
        "t\n.param corner_factor=1 process_mc_factor=1 MC_PR_SWITCH=0\n.param sw_nw_rs_mult = {1.0+corner_factor*0.0} + process_mc_factor*MC_PR_SWITCH*EXP(AGAUSS(0,0.12,1))\nr1 a 0 {sw_nw_rs_mult}\n.op\n.end\n",
    );
    assert!(
        scs.contains("sw_nw_rs_mult=(1.0+corner_factor*0.0) + process_mc_factor*MC_PR_SWITCH*EXP(rack_agauss(0,0.12,1, rack_sw_nw_rs_mult_1))"),
        "{scs}"
    );
}

/// T-G7: a call inside `.lib tt` declares and varies its variable inside
/// `section tt`; the functions stay at the file's top.
#[test]
fn t_g7_section_placement() {
    let scs = spectre("t\n.lib tt\n.param p='agauss(0,1,1)'\n.endl tt\n");
    let at = |s: &str| scs.find(s).unwrap_or_else(|| panic!("{s}:\n{scs}"));
    assert!(at("real rack_agauss") < at("section tt"));
    assert!(at("section tt") < at("rack_p_1=0"));
    assert!(at("rack_p_1=0") < at("statistics {"));
    assert!(at("statistics {") < at("endsection tt"));
}

/// T-G8: a `.func` whose body calls `agauss` is inlined at each call site:
/// two calls, two variables.
#[test]
fn t_g8_func_inlined() {
    let scs = spectre(
        "t\n.func jit(x) {x+agauss(0,1,1)}\nr1 a 0 r={jit(1k)}\nr2 a 0 r={jit(2k)}\n.op\n.end\n",
    );
    assert!(!scs.contains("real jit"), "{scs}");
    assert!(
        scs.contains("r=((1k)+rack_agauss(0,1,1, rack_r1_r_1))"),
        "{scs}"
    );
    assert!(
        scs.contains("r=((2k)+rack_agauss(0,1,1, rack_r2_r_1))"),
        "{scs}"
    );
}

/// T-G9: Xyce's `limit(x,1,2)` is a clamp, not a draw; Xyce's two-argument
/// `agauss` takes sigma 1.
#[test]
fn t_g9_xyce_forms() {
    let (ir, _) = parse(
        "t\nr1 a 0 r={limit(x,1,2)}\nr2 a 0 r={agauss(1k,10)}\n.op\n.end\n",
        "t",
        Dialect::Xyce,
    )
    .unwrap();
    let scs = netlist(&ir, Dialect::Spectre).unwrap();
    assert!(
        scs.contains("r1 (a 0) resistor r=min(max(x, 1), 2)"),
        "{scs}"
    );
    assert!(scs.contains("rack_agauss(1k,10,1, rack_r2_r_1)"), "{scs}");
    assert_eq!(varies(&scs).len(), 1);
}

/// T-G10: Spectre and VACASK write the calls; LTspice still refuses.
#[test]
fn t_g10_spectre_and_vacask_write_ltspice_refuses() {
    let (ir, _) = parse(
        "t\nr1 a 0 r='agauss(1k,100,3)+limit(0,5)'\n.op\n.end\n",
        "t",
        Dialect::Ngspice,
    )
    .unwrap();
    assert!(netlist(&ir, Dialect::Spectre).is_ok());
    let sim = netlist(&ir, Dialect::Vacask).unwrap();
    assert!(
        sim.contains("r=agauss(1k,100,3)+((aunif(0, 1) > 0) ? (0) + (5) : (0) - (5))"),
        "{sim}"
    );
    let err = netlist(&ir, Dialect::Ltspice).unwrap_err().to_string();
    assert!(err.contains("statistical function"), "{err}");
}

/// T-R1: SPICE -> Spectre -> SPICE gives every expression back, on a deck
/// with every placement and on slices of sky130 and gf180mcu (the files
/// with the most call shapes; `slices`).
#[test]
fn t_r1_spice_spectre_spice_identity() {
    let (placements, _) = parse(
        "t\n.param tp='agauss(0,1,1)'\n.subckt s a b\n.param loc='agauss(0,1,1)'\nR1 a b r='aunif(1k,10)+limit(0,1)'\n.ends s\nX1 a 0 s\n.op\n.end\n",
        "t",
        Dialect::Ngspice,
    )
    .unwrap();
    for (name, ir) in std::iter::once(("placements".to_string(), placements)).chain(slices()) {
        let scs = netlist(&ir, Dialect::Spectre).unwrap_or_else(|e| panic!("{name}: {e}"));
        let (back, report) = parse(&scs, "t", Dialect::Spectre).unwrap();
        assert!(report.unparsed.is_empty(), "{name}: {:?}", report.unparsed);
        assert_eq!(calls(&back), calls(&ir), "{name}");
        assert!(
            back.top.statistics.is_empty()
                && back.sections.iter().all(|s| s.body.statistics.is_empty()),
            "{name}"
        );
        assert!(
            !back.functions.iter().any(|f| f.name.starts_with("rack_")),
            "{name}"
        );
    }
}

/// T-R2: a Spectre `statistics` block with process, mismatch, `correlate`,
/// `truncate` and `percent` reads into the table and back equal.
#[test]
fn t_r2_spectre_table_round_trip() {
    let scs = "// t\nsimulator lang=spectre\nparameters p=1k q=2 r=3\nstatistics {\n  process {\n    vary p dist=gauss std=10 percent=yes\n    vary q dist=lnorm std=0.1\n    vary r dist=unif N=0.2\n    truncate tr=5\n  }\n  mismatch { vary q dist=gauss std=0.05 }\n  correlate param=[p r] cc=0.6\n  correlate dev=[x1 x2] cc=0.9\n  truncate tr=4\n}\nr1 (a 0) resistor r=p\n";
    let (ir, report) = parse(scs, "t", Dialect::Spectre).unwrap();
    assert_eq!(report, Default::default());
    let st = &ir.top.statistics[0];
    assert_eq!(
        st.process
            .iter()
            .map(|v| (v.param.as_str(), v.dist, v.percent))
            .collect::<Vec<_>>(),
        [
            ("p", Dist::Gauss, true),
            ("q", Dist::Lnorm, false),
            ("r", Dist::Unif, false)
        ]
    );
    assert_eq!(st.mismatch[0].param, "q");
    assert_eq!(st.correlations[1].devices, ["x1", "x2"]);
    assert_eq!(st.truncate, [Some("4".into()), Some("5".into()), None]);
    let again = netlist(&ir, Dialect::Spectre).unwrap();
    let (back, _) = parse(&again, "t", Dialect::Spectre).unwrap();
    assert_eq!(back.top.statistics, ir.top.statistics, "{again}");
}

/// T-R3: the table lowered to ngspice: the process two-layer idiom, the
/// mismatch two-layer idiom per reading subcircuit with renamed references,
/// `exp` for `lnorm`, scaling for `percent`, `aunif` for `unif N`.
#[test]
fn t_r3_table_to_ngspice() {
    let scs = "// t\nsimulator lang=spectre\nparameters p=1k q=2 r=3\nstatistics {\n  process {\n    vary p dist=gauss std=10 percent=yes\n    vary q dist=lnorm std=0.1\n    vary r dist=unif N=0.2\n  }\n  mismatch { vary q dist=gauss std=0.05 }\n}\nsubckt s (a b)\nr1 (a b) resistor r=q*1k\nends s\nr1 (a 0) resistor r=p\nx1 (a 0) s\n";
    let (ir, _) = parse(scs, "t", Dialect::Spectre).unwrap();
    let sp = netlist(&ir, Dialect::Ngspice).unwrap();
    for want in [
        ".param p__mc={agauss(1k, (1k)*(10)/100, 1)}\n.param p=p__mc\n",
        ".param q__mc={exp(agauss(log(2), 100m, 1))}\n.param q=q__mc\n",
        ".param r__mc={aunif(3, 200m)}\n.param r=r__mc\n",
        ".subckt s a b\n.param q__mm_r={agauss(q, 50m, 1)}\n.param q__mm=q__mm_r\nR1 a b {q__mm*1k}\n",
    ] {
        assert!(sp.contains(want), "{want}\n---\n{sp}");
    }
}

/// T-R4: a 2x2 `correlate param` is the Cholesky combination; `correlate
/// dev` has no SPICE form and is refused by name.
#[test]
fn t_r4_correlate() {
    let base = "// t\nsimulator lang=spectre\nparameters a=1 b=2\nstatistics {\n  process {\n    vary a dist=gauss std=0.1\n    vary b dist=gauss std=0.2\n  }\n  correlate CORR cc=0.6\n}\nr1 (x 0) resistor r=a+b\n";
    let (ir, _) = parse(&base.replace("CORR", "param=[a b]"), "t", Dialect::Spectre).unwrap();
    let sp = netlist(&ir, Dialect::Ngspice).unwrap();
    assert!(sp.contains(".param a={(1) + (100m)*((1e0)*a__z)}"), "{sp}");
    assert!(
        sp.contains(".param b={(2) + (200m)*((6e-1)*a__z + (8e-1)*b__z)}"),
        "{sp}"
    );
    let (dev, _) = parse(&base.replace("CORR", "dev=[x1 x2]"), "t", Dialect::Spectre).unwrap();
    let err = netlist(&dev, Dialect::Ngspice).unwrap_err().to_string();
    assert!(err.contains("correlate dev=[x1 x2]"), "{err}");
}

/// T-R5: the sky130 and gf180mcu slices convert to Spectre with no
/// refusal, and the `vary` lines are exactly the lifted call sites (counted
/// in the source expressions).
#[test]
fn t_r5_pdk_vary_count_is_the_call_count() {
    for (name, ir) in slices() {
        let sites: usize = calls(&ir).iter().map(|(_, n)| n).sum();
        assert!(sites > 0, "{name}");
        let scs = netlist(&ir, Dialect::Spectre).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(varies(&scs).len(), sites, "{name}");
    }
}

/// sky130's parameter, critical-parameter and FET model files, and the
/// sections of gf180mcu's `sm141064.ngspice` and `smbb000149.ngspice` that
/// hold its FETs and statistics. Left out: the files and sections with
/// resistor, capacitor or level-3 diode `.model`s, which have no Spectre
/// master yet (a gap of their own, not of the statistics).
fn slices() -> Vec<(String, CircuitIR)> {
    let root = std::env::var("PDK_ROOT")
        .expect("PDK_ROOT not set: run the tests in `nix develop .#testing`");
    let pdk = Path::new(&root);
    let read = |f: &str| {
        parse(
            &format!("*\n{}", std::fs::read_to_string(pdk.join(f)).unwrap()),
            "t",
            Dialect::Ngspice,
        )
        .unwrap()
        .0
    };
    let mut out: Vec<(String, CircuitIR)> = [
        "sky130A/libs.tech/combined/continuous/parameters_fet_tt.spice",
        "sky130A/libs.tech/combined/continuous/models_fet.spice",
        "sky130A/libs.tech/combined/parameters/critical.spice",
    ]
    .into_iter()
    .map(|f| (f.to_string(), read(f)))
    .collect();
    for f in [
        "gf180mcuD/libs.tech/ngspice/sm141064.ngspice",
        "gf180mcuD/libs.tech/ngspice/smbb000149.ngspice",
    ] {
        let mut ir = read(f);
        let unmapped = |m: &ModelDef| {
            let k = m.kind.to_ascii_lowercase();
            k == "r"
                || k == "c"
                || k == "d"
                    && m.parameters
                        .iter()
                        .any(|(p, v)| p.eq_ignore_ascii_case("level") && v.trim() == "3")
        };
        ir.sections.retain(|s| {
            ["nfet_", "pfet_", "statistical", "typical", "fets_mm"]
                .iter()
                .any(|p| s.body.name.starts_with(p))
                && !std::iter::once(&s.body)
                    .chain(&s.subcircuit_defs)
                    .flat_map(|b| &b.models)
                    .any(unmapped)
        });
        out.push((f.to_string(), ir));
    }
    out
}

/// Lint: a one-layer random `.param` read twice is flagged; read
/// once, or through the two-layer idiom, it is not.
#[test]
fn lint_flags_a_one_layer_random_param_read_twice() {
    let lint = |d: &str| {
        spicerack::lint::lint_netlist(d, None)
            .warnings
            .into_iter()
            .filter(|w| w.message.contains("random .param"))
            .count()
    };
    assert_eq!(
        lint("t\n.param g=agauss(0,1,1)\nv1 a 0 {g}\nv2 b 0 {g}\nr1 a 0 1\nr2 b 0 1\n.end\n"),
        1
    );
    assert_eq!(
        lint("t\n.param g=agauss(0,1,1)\nv1 a 0 {g}\nr1 a 0 1\n.end\n"),
        0
    );
    assert_eq!(
        lint(
            "t\n.param g_r=agauss(0,1,1)\n.param g=g_r\nv1 a 0 {g}\nv2 b 0 {g}\nr1 a 0 1\nr2 b 0 1\n.end\n"
        ),
        0
    );
}

/// The Spectre decks of T-S1 to T-S3 (`tests/spectre/`, run on a real
/// Spectre by `tests/spectre/run.py`) are what SpiceRack writes today for
/// their SPICE sources; `SPICERACK_BLESS=1` rewrites them.
#[test]
fn spectre_run_decks_are_current() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/spectre");
    let mc = |variations: &str| Analysis::SpectreMonteCarlo {
        iterations: 2000,
        inner: "dcmc".into(),
        inner_type: SpectreAnalysis::Dc,
        seed: Some(1),
        variations: Some(variations.into()),
    };
    let divider = |r: &str| format!("t\nv1 in 0 1\nr1 in out r='{r}'\nr2 out 0 1k\n.op\n.end\n");
    let pair = "t\n.param g_r='agauss(0,100,1)'\n.param g=g_r\n.subckt s a b\n.param rl='agauss(1k,100,1)'\nr1 a b {rl+g}\nr2 b 0 1k\n.ends s\nv1 in 0 1\nx1 in o1 s\nx2 in o2 s\n.op\n.end\n";
    let nested = "t\n.subckt leaf a b\n.param rl='agauss(1k,100,1)'\nr1 a b {rl}\nr2 b 0 1k\n.ends leaf\n.subckt two a b c\nxa a b leaf\nxb a c leaf\n.ends two\nv1 in 0 1\nx1 in o1 o2 two\nx2 in o3 o4 two\n.op\n.end\n";
    let decks = [
        (
            "t_s1_nominal.scs",
            divider("agauss(1k,100,3)+aunif(0,100)+limit(0,50)"),
            None,
        ),
        (
            "t_s2_agauss.scs",
            divider("agauss(1k,100,3)"),
            Some(mc("all")),
        ),
        ("t_s2_aunif.scs", divider("aunif(1k,100)"), Some(mc("all"))),
        ("t_s2_limit.scs", divider("limit(1k,50)"), Some(mc("all"))),
        ("t_s3_mismatch.scs", pair.to_string(), Some(mc("mismatch"))),
        ("t_s3_nested.scs", nested.to_string(), Some(mc("mismatch"))),
    ];
    let bless = std::env::var("SPICERACK_BLESS").is_ok();
    for (file, deck, analysis) in decks {
        let (mut ir, _) = parse(&deck, file, Dialect::Ngspice).unwrap();
        ir.testbench.as_mut().unwrap().analyses.extend(analysis);
        let text = netlist(&ir, Dialect::Spectre).unwrap() + "\n";
        let path = dir.join(file);
        if bless {
            std::fs::write(&path, &text).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            text,
            "{file}: SPICERACK_BLESS=1 rewrites it"
        );
    }
}

/// Each statistical call in `ir`'s expressions, `(text, count)`: the call
/// with its arguments, delimiters and blanks dropped, so a SPICE and a
/// Spectre spelling compare equal.
fn calls(ir: &CircuitIR) -> Vec<(String, usize)> {
    let mut out: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut exprs: Vec<String> = Vec::new();
    for sc in ir.subcircuits() {
        exprs.extend(
            sc.parameters
                .iter()
                .chain(&sc.locals)
                .filter_map(|p| p.default.clone()),
        );
        exprs.extend(
            sc.models
                .iter()
                .flat_map(|m| m.parameters.iter().map(|(_, v)| v.clone())),
        );
        exprs.extend(
            sc.instances
                .iter()
                .flat_map(|i| i.parameters.iter().map(|(_, v)| v.clone())),
        );
        exprs.extend(
            sc.components
                .iter()
                .flat_map(|c| c.exprs().into_iter().map(str::to_string)),
        );
    }
    for e in exprs {
        let low = e.to_ascii_lowercase();
        let t: String = low
            .chars()
            .filter(|c| !c.is_whitespace() && !"'{}".contains(*c))
            .collect();
        for f in ["agauss(", "gauss(", "aunif(", "unif(", "limit("] {
            let mut rest = t.as_str();
            while let Some(i) = rest.find(f) {
                let pre = rest[..i].bytes().last();
                if pre.is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'_') {
                    *out.entry(t.clone()).or_default() += 1;
                }
                rest = &rest[i + f.len()..];
            }
        }
    }
    out.into_iter().collect()
}
