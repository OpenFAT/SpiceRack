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
//! runs (T-N0, T-N1) are `test_monte_carlo_ngspice.rs`; the Spectre runs
//! (T-S1 to T-S3) are `tests/spectre/`, whose decks `render::named::lift`
//! checks byte for byte.
//!
//! Contents: none (tests only).

use std::path::Path;

use spicerack::syntax::Dialect;
use spicerack::syntax::diag::Code;
use spicerack::syntax::render::render_text;

// ---- private ----

/// `deck` (ngspice) written for Spectre, with no note on it.
fn spectre(deck: &str) -> String {
    let w = written(deck, Dialect::Ngspice, Dialect::Spectre);
    let codes: Vec<Code> = w.notes.diags.iter().map(|d| d.code).collect();
    assert_eq!(codes, [] as [Code; 0], "{}\n{deck}", w.notes.messages);
    w.text
}

/// `src` (read in `d`) written for `to`, which must not refuse it.
fn written(src: &str, d: Dialect, to: Dialect) -> spicerack::syntax::render::Written {
    render_text(src, d, "t", to).unwrap_or_else(|e| panic!("{to:?}: {}\n{src}", e.messages))
}

/// The codes `src` (read in `d`) is refused with on `to`.
fn refused(src: &str, d: Dialect, to: Dialect) -> Vec<Code> {
    match render_text(src, d, "t", to) {
        Ok(w) => panic!("{to:?} wrote it:\n{}", w.text),
        Err(e) => e.diags.iter().map(|d| d.code).collect(),
    }
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
    let scs = written(
        "t\nr1 a 0 r={limit(x,1,2)}\nr2 a 0 r={agauss(1k,10)}\n.op\n.end\n",
        Dialect::Xyce,
        Dialect::Spectre,
    )
    .text;
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
    let deck = "t\nr1 a 0 r='agauss(1k,100,3)+limit(0,5)'\n.op\n.end\n";
    spectre(deck);
    let sim = written(deck, Dialect::Ngspice, Dialect::Vacask).text;
    assert!(
        sim.contains("r=agauss(1k,100,3)+((aunif(0, 1) > 0) ? (0) + (5) : (0) - (5))"),
        "{sim}"
    );
    assert_eq!(
        refused(deck, Dialect::Ngspice, Dialect::Ltspice),
        [Code::W0003]
    );
}

/// T-R1: SPICE -> Spectre -> SPICE gives every expression back, on a deck
/// with every placement and on slices of sky130 and gf180mcu (the files
/// with the most call shapes; `slices`).
#[test]
fn t_r1_spice_spectre_spice_identity() {
    let placements = "t\n.param tp='agauss(0,1,1)'\n.subckt s a b\n.param loc='agauss(0,1,1)'\nR1 a b r='aunif(1k,10)+limit(0,1)'\n.ends s\nX1 a 0 s\n.op\n.end\n";
    for (name, deck) in
        std::iter::once(("placements".to_string(), placements.to_string())).chain(slices())
    {
        let sp = written(&deck, Dialect::Ngspice, Dialect::Ngspice).text;
        let scs = render_text(&deck, Dialect::Ngspice, "t", Dialect::Spectre)
            .unwrap_or_else(|e| panic!("{name}: {}", e.messages))
            .text;
        let unread = written(&scs, Dialect::Spectre, Dialect::Spectre)
            .notes
            .diags
            .iter()
            .filter(|d| d.code == Code::P0001)
            .count();
        assert_eq!(unread, 0, "{name}");
        let back = kept_out(&scs);
        assert_eq!(calls(&back.text), calls(&sp), "{name}");
        // No table and no `rack_` function is left: every call read back.
        assert!(
            !back.text.contains("rack_")
                && !back.text.contains("__mc=")
                && !back.text.contains("__mm"),
            "{name}"
        );
    }
}

/// `scs` (Spectre) written as ngspice, less the statements the reader
/// keeps as text (an instance of a master the file does not define, which
/// ngspice cannot be given as Spectre text: `W0001`, which names the first
/// such line), each dropped from `scs` in turn.
fn kept_out(scs: &str) -> spicerack::syntax::render::Written {
    let mut text = scs.to_string();
    for _ in 0..scs.lines().count() {
        let e = match render_text(&text, Dialect::Spectre, "t", Dialect::Ngspice) {
            Ok(w) => return w,
            Err(e) => e,
        };
        assert!(
            e.diags.iter().all(|d| d.code == Code::W0001),
            "{}",
            e.messages
        );
        let at: usize = e
            .messages
            .lines()
            .find_map(|m| m.split(':').nth(1)?.parse().ok())
            .expect("a line");
        text = text
            .lines()
            .enumerate()
            .filter(|&(i, _)| i + 1 != at)
            .map(|(_, l)| format!("{l}\n"))
            .collect();
    }
    panic!("every line kept as text:\n{scs}")
}

/// T-R2: a Spectre `statistics` block with process, mismatch, `correlate`,
/// `truncate` and `percent` reads into the table and back equal.
#[test]
fn t_r2_spectre_table_round_trip() {
    let scs = "// t\nsimulator lang=spectre\nparameters p=1k q=2 r=3\nstatistics {\n  process {\n    vary p dist=gauss std=10 percent=yes\n    vary q dist=lnorm std=0.1\n    vary r dist=unif N=0.2\n    truncate tr=5\n  }\n  mismatch { vary q dist=gauss std=0.05 }\n  correlate param=[p r] cc=0.6\n  correlate dev=[x1 x2] cc=0.9\n  truncate tr=4\n}\nr1 (a 0) resistor r=p\n";
    let w = written(scs, Dialect::Spectre, Dialect::Spectre);
    let codes: Vec<Code> = w.notes.diags.iter().map(|d| d.code).collect();
    assert_eq!(codes, [] as [Code; 0], "{}", w.notes.messages);
    // Each `vary`, `correlate` and `truncate` of the source, in its block,
    // is in the table written back; reading that back writes it again.
    let lines = |t: &str| -> Vec<(String, String)> {
        let t = t.replace('{', "\n{\n").replace('}', "\n}\n");
        let (mut scope, mut last, mut out) = (Vec::new(), String::new(), Vec::new());
        for l in t.lines().map(str::trim).filter(|l| !l.is_empty()) {
            match l {
                "{" => scope.push(last.clone()),
                "}" => {
                    scope.pop();
                }
                _ if ["vary ", "correlate ", "truncate "]
                    .iter()
                    .any(|k| l.starts_with(k)) =>
                {
                    // Values as numbers: the writer respells `0.05` as `50m`.
                    let words: Vec<String> = l.split(' ').map(number).collect();
                    out.push((scope.last().cloned().unwrap_or_default(), words.join(" ")));
                }
                _ => last = l.to_string(),
            }
        }
        out.sort();
        out
    };
    assert_eq!(lines(&w.text), lines(scs), "{}", w.text);
    // Three process `vary`s and its `truncate`, one mismatch `vary`, two
    // `correlate`s, the block's `truncate`.
    assert_eq!(lines(&w.text).len(), 8);
    assert_eq!(
        written(&w.text, Dialect::Spectre, Dialect::Spectre).text,
        w.text
    );
}

/// `k=v` with `v` as a number when it is one; any other word as is.
fn number(w: &str) -> String {
    let Some((k, v)) = w.split_once('=') else {
        return w.to_string();
    };
    match spicerack::unit::parse_spice_number(v) {
        Some((x, end, scale)) if end + scale.len() == v.len() => format!("{k}={x:e}"),
        _ => w.to_string(),
    }
}

/// T-R3: the table lowered to ngspice: the process two-layer idiom, the
/// mismatch two-layer idiom per reading subcircuit with renamed references,
/// `exp` for `lnorm`, scaling for `percent`, `aunif` for `unif N`.
#[test]
fn t_r3_table_to_ngspice() {
    let scs = "// t\nsimulator lang=spectre\nparameters p=1k q=2 r=3\nstatistics {\n  process {\n    vary p dist=gauss std=10 percent=yes\n    vary q dist=lnorm std=0.1\n    vary r dist=unif N=0.2\n  }\n  mismatch { vary q dist=gauss std=0.05 }\n}\nsubckt s (a b)\nr1 (a b) resistor r=q*1k\nends s\nr1 (a 0) resistor r=p\nx1 (a 0) s\n";
    let sp = written(scs, Dialect::Spectre, Dialect::Ngspice).text;
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
/// dev` has no SPICE form and is refused by code (W0018).
#[test]
fn t_r4_correlate() {
    let base = "// t\nsimulator lang=spectre\nparameters a=1 b=2\nstatistics {\n  process {\n    vary a dist=gauss std=0.1\n    vary b dist=gauss std=0.2\n  }\n  correlate CORR cc=0.6\n}\nr1 (x 0) resistor r=a+b\n";
    let sp = written(
        &base.replace("CORR", "param=[a b]"),
        Dialect::Spectre,
        Dialect::Ngspice,
    )
    .text;
    assert!(sp.contains(".param a={(1) + (100m)*((1e0)*a__z)}"), "{sp}");
    assert!(
        sp.contains(".param b={(2) + (200m)*((6e-1)*a__z + (8e-1)*b__z)}"),
        "{sp}"
    );
    let dev = base.replace("CORR", "dev=[x1 x2]");
    assert_eq!(
        refused(&dev, Dialect::Spectre, Dialect::Ngspice),
        [Code::W0018]
    );
}

/// T-R5: the sky130 and gf180mcu slices convert to Spectre with no
/// refusal, and the `vary` lines are exactly the lifted call sites (counted
/// in the source expressions).
#[test]
fn t_r5_pdk_vary_count_is_the_call_count() {
    for (name, deck) in slices() {
        let sp = written(&deck, Dialect::Ngspice, Dialect::Ngspice).text;
        let sites: usize = calls(&sp).iter().map(|(_, n)| n).sum();
        assert!(sites > 0, "{name}");
        let scs = render_text(&deck, Dialect::Ngspice, "t", Dialect::Spectre)
            .unwrap_or_else(|e| panic!("{name}: {}", e.messages))
            .text;
        assert_eq!(varies(&scs).len(), sites, "{name}");
    }
}

/// sky130's parameter, critical-parameter and FET model files, and the
/// sections of gf180mcu's `sm141064.ngspice` and `smbb000149.ngspice` that
/// hold its FETs and statistics. Left out: the files and sections with
/// resistor, capacitor or level-3 diode `.model`s, which have no Spectre
/// master yet (a gap of their own, not of the statistics).
fn slices() -> Vec<(String, String)> {
    let root = std::env::var("PDK_ROOT")
        .expect("PDK_ROOT not set: run the tests in `nix develop .#testing`");
    let pdk = Path::new(&root);
    let read = |f: &str| format!("*\n{}", std::fs::read_to_string(pdk.join(f)).unwrap());
    let mut out: Vec<(String, String)> = [
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
        // The sections kept, by a line scan: their `.lib name` … `.endl`
        // lines, each `.model` card joined with its `+` lines to find its
        // kind and LEVEL.
        let mut deck = String::from("*\n");
        let (mut block, mut open) = (String::new(), false);
        for l in read(f).lines() {
            let words: Vec<&str> = l.split_whitespace().collect();
            let head = words.first().map(|w| w.to_ascii_lowercase());
            if head.as_deref() == Some(".lib") && words.len() == 2 && !open {
                open = true;
                block.clear();
            }
            if open {
                block.push_str(l);
                block.push('\n');
            }
            if head.as_deref() == Some(".endl") && open {
                open = false;
                let name = block.split_whitespace().nth(1).unwrap_or("").to_string();
                let kept = ["nfet_", "pfet_", "statistical", "typical", "fets_mm"]
                    .iter()
                    .any(|p| name.starts_with(p));
                if kept && !unmapped(&block) {
                    deck.push_str(&block);
                }
            }
        }
        out.push((f.to_string(), deck));
    }
    out
}

/// Whether `block` holds a `.model` with no Spectre master yet: a resistor,
/// a capacitor, a level-3 diode.
fn unmapped(block: &str) -> bool {
    let mut cards: Vec<String> = Vec::new();
    for l in block.lines().map(str::to_ascii_lowercase) {
        match (l.trim_start().strip_prefix('+'), cards.last_mut()) {
            (Some(more), Some(c)) => c.push_str(more),
            _ => cards.push(l),
        }
    }
    cards.iter().any(|c| {
        let w: Vec<&str> = c.split_whitespace().collect();
        w.first() == Some(&".model")
            && w.get(2).is_some_and(|&k| {
                k == "r" || k == "c" || k == "d" && c.replace(' ', "").contains("level=3")
            })
    })
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

/// Each statistical call in a written deck, `(text, count)`: the call
/// with its arguments, delimiters and blanks dropped, so a SPICE and a
/// Spectre spelling compare equal. A `.func` body is not a call site (it is
/// inlined at each call).
fn calls(deck: &str) -> Vec<(String, usize)> {
    let mut out: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for line in deck.lines() {
        if line.trim_start().to_ascii_lowercase().starts_with(".func") {
            continue;
        }
        let t: String = line
            .to_ascii_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace() && !"'{}".contains(*c))
            .collect();
        let b = t.as_bytes();
        for f in ["agauss(", "gauss(", "aunif(", "unif(", "limit("] {
            for (i, _) in t.match_indices(f) {
                if i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
                    continue;
                }
                // The call: up to its matching `)`.
                let mut depth = 0;
                let end = t[i..]
                    .char_indices()
                    .find(|&(_, c)| {
                        depth += i32::from(c == '(') - i32::from(c == ')');
                        c == ')' && depth == 0
                    })
                    .map_or(t.len(), |(k, _)| i + k + 1);
                *out.entry(t[i..end].to_string()).or_default() += 1;
            }
        }
    }
    out.into_iter().collect()
}
