//! One circuit dumped in every dialect, read back by ESPice.
//!
//! ESPice reads ngspice, HSPICE and Spectre (`espice --tokenizer=<d>`), so it
//! is the oracle for those three and for its own deck: each must give the same
//! operating point. LTspice and VACASK decks are only checked to emit; ESPice
//! reads neither. The HSPICE card forms follow ESPice's
//! `tests/frontend/grammar/hspice*.peg` and `tests/conformance/tests/netlist/hspice`.
//!
//! The ESPice runs need `espice` on PATH (`$ESPICE` overrides): unsupported
//! fails, never skips, so outside `nix develop .#testing` they fail.
//!
//! Cites: real ESPice (`--tokenizer=<d>`) as the oracle for each dialect.
//!
//! Contents: none (tests only).

use spicerack::syntax::Dialect;
use spicerack::syntax::diag::Code;
use spicerack::syntax::render::{Diags, render_text};

/// Known failures: the dialects whose deck is not written or not read, with
/// the reason. A listed dialect that reads back fails the test until its
/// line goes.
const ESPICE_REFUSES: &[(Dialect, &str)] = &[
    (
        Dialect::Ngspice,
        "ESPice has no XSPICE code models; ngspice spells the Laplace block as an `s_xfer` A element",
    ),
    (
        Dialect::Spectre,
        "the renderer refuses a B source for Spectre (`v(mid)` has no safe Spectre spelling)",
    ),
];

/// v(mid) = 0.5 (a parameter expression over a parametrised subcircuit),
/// v(b) = 2 v(mid) + 1 = 2, v(lp) = DC gain of 1/(1 + 1e-3 s) = 1.
const CIRCUIT: &str = "\
.param rv=2k
.subckt load n PARAMS: r=1k
R1 n 0 {r}
.ends load
V1 in 0 1 AC 1
R1 in mid {rv}
B1 b 0 V=2*v(mid)+1
B2 lp 0 V=Laplace(v(in), 1/(1+s*1e-3))
X1 mid load r=2k
";

// ---- private ----

/// The circuit with `extra` cards after it (its analyses), as an ngspice
/// deck; `bsources` false leaves the B cards out.
fn circuit(extra: &str, bsources: bool) -> String {
    let body: String = CIRCUIT
        .lines()
        .filter(|l| bsources || !l.starts_with('B'))
        .map(|l| format!("{l}\n"))
        .collect();
    format!("* dialects\n{body}{extra}.end\n")
}

fn netlist(deck: &str, d: Dialect) -> Result<String, Diags> {
    render_text(deck, Dialect::Ngspice, "dialects", d).map(|w| w.text)
}

/// The codes `d`'s refusal of `deck` names.
fn refusal(deck: &str, d: Dialect) -> Vec<Code> {
    let e = netlist(deck, d).expect_err("refused");
    e.diags.iter().map(|x| x.code).collect()
}

#[test]
fn every_dialect_dumps() {
    // VACASK has no behavioural-source language; the rest is common ground.
    let deck = circuit(".op\n", false);
    for d in [
        Dialect::Ngspice,
        Dialect::Hspice,
        Dialect::Ltspice,
        Dialect::Vacask,
        Dialect::Spectre,
        Dialect::Espice,
    ] {
        let text = netlist(&deck, d).unwrap_or_else(|e| panic!("{d:?}: {}", e.messages));
        assert!(text.contains("mid"), "{d:?}:\n{text}");
    }
}

#[test]
fn hspice_card_forms() {
    // The start spelled `1e-7`: ngspice reads `100n` as 100 * 10^-9, an ulp
    // above 1e-7, and HSPICE is written that exact double.
    let deck = circuit(
        ".lib corners.lib tt\n.tran 1n 1u 1e-7 5n UIC\n.noise V(mid) V1 dec 2 1k 100k 1\n",
        true,
    );
    let n = netlist(&deck, Dialect::Hspice).unwrap_or_else(|e| panic!("{}", e.messages));
    for want in [
        ".lib 'corners.lib' tt",
        ".subckt load n r=1k",
        "R1 n 0 'r'",
        "R1 in mid 'rv'",
        "E1 b 0 vol='2*v(mid)+1'",
        "EB2 lp 0 LAPLACE in 0 1e0 / 1e0 1e-3",
        ".option delmax=5n",
        ".tran 1n 1u START=100n UIC",
        ".ac dec 2 1k 100k\n.noise V(mid) V1 1",
    ] {
        assert!(n.contains(want), "missing `{want}` in:\n{n}");
    }
    // ESPice refuses these in an HSPICE deck (invalid/hspice_brace_*.sp).
    assert!(
        !n.contains('{') && !n.contains("PARAMS:") && !n.contains("\nB"),
        "{n}"
    );
    // HSPICE has no `.step` (it sweeps on the analysis card or `.alter`),
    // and no XSPICE `A` element.
    assert!(netlist(&circuit(".step rv 1k 3k 1k\n.op\n", true), Dialect::Hspice).is_err());
    assert!(netlist(&circuit("A1 a m\n.op\n", true), Dialect::Hspice).is_err());
}

/// Text kept as written (a card the reader has no form for), at the top or
/// inside a subcircuit, reaches its own family only: ngspice and ESPice
/// copy it (a `W0027` note), every other target refuses it (`W0001`)
/// rather than drop it.
#[test]
fn kept_text_is_refused_not_dropped() {
    let line = ".width out=80";
    for at_top in [true, false] {
        let deck = match at_top {
            true => circuit(&format!("{line}\n.op\n"), false),
            false => {
                circuit(".op\n", false).replace("R1 n 0 {r}\n", &format!("R1 n 0 {{r}}\n{line}\n"))
            }
        };
        for d in [
            Dialect::Spectre,
            Dialect::Vacask,
            Dialect::Hspice,
            Dialect::Ltspice,
        ] {
            assert!(
                refusal(&deck, d).contains(&Code::W0001),
                "{d:?} at_top={at_top}"
            );
        }
        for d in [Dialect::Ngspice, Dialect::Espice] {
            let w = render_text(&deck, Dialect::Ngspice, "dialects", d)
                .unwrap_or_else(|e| panic!("{d:?}: {}", e.messages));
            assert!(w.text.contains(line), "{d:?}:\n{}", w.text);
            assert!(w.notes.diags.iter().any(|x| x.code == Code::W0027), "{d:?}");
        }
    }
}

/// Spectre deck text that would parse but mean something else.
#[test]
fn spectre_keeps_spice_meaning() {
    let model = ".model qn NPN(IKF=30M)\n";
    let deck = circuit(&format!("{model}.save i(V1)\n.tran 1n 1u UIC\n"), false);
    let n = netlist(&deck, Dialect::Spectre).unwrap_or_else(|e| panic!("{}", e.messages));
    // SPICE 30M is milli; Spectre's M is mega, its m milli.
    assert!(n.contains("model qn bjt type=npn ikf=30m"), "{n}");
    assert!(n.contains("skipdc=yes"), "uic: {n}");
    assert!(n.contains("save v1:p"), "current save: {n}");
    // A nested `.dc` must not lose a sweep, and a VDMOS has no Spectre
    // master: both refused.
    let nested = circuit(&format!("{model}.dc V1 0 1 0.1 temp 0 1 0.1\n"), false);
    assert!(netlist(&nested, Dialect::Spectre).is_err());
    let vdmos = circuit(".model qn VDMOS(IKF=30M)\n.op\n", false);
    assert!(netlist(&vdmos, Dialect::Spectre).is_err());
}

/// An empty option value is a flag: bare in SPICE, refused by Spectre
/// (`W0101`), whose `options` statement takes `name=value` only.
#[test]
fn option_flags() {
    let deck = circuit(".options noacct reltol=1e-4\n.op\n", false);
    let n = netlist(&deck, Dialect::Ngspice).unwrap_or_else(|e| panic!("{}", e.messages));
    assert!(n.contains(".options noacct reltol=1e-4"), "{n}");
    assert!(refusal(&deck, Dialect::Spectre).contains(&Code::W0101));
}

// ── ESPice as the reader ──

fn espice() -> String {
    let bin = std::env::var("ESPICE").unwrap_or_else(|_| "espice".into());
    let ok = std::process::Command::new(&bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(
        ok,
        "`{bin} --version` failed: run the tests in `nix develop .#testing`, which puts espice on PATH"
    );
    bin
}

/// The operating point ESPice reads from `deck`, or why it refused it.
fn op_point(bin: &str, deck: &str, tokenizer: &str) -> Result<spicerack::result::RawData, String> {
    let dir = tempfile::TempDir::new().unwrap();
    let (path, raw) = (dir.path().join("deck.sp"), dir.path().join("out.raw"));
    std::fs::write(&path, deck).unwrap();
    let out = std::process::Command::new(bin)
        .arg(format!("--tokenizer={tokenizer}"))
        .arg(&path)
        .arg("--rawfile")
        .arg(&raw)
        .arg("--format=binary")
        .output()
        .unwrap();
    if !out.status.success() {
        return Err(format!(
            "espice --tokenizer={tokenizer} refused the deck:\n{}\n{}\n{deck}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    Ok(spicerack::rawfile::parse_raw(&std::fs::read(&raw).unwrap()).unwrap())
}

fn node(raw: &spicerack::result::RawData, name: &str) -> f64 {
    let wrapped = format!("v({name})");
    let i = raw
        .variables
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case(name) || v.name.eq_ignore_ascii_case(&wrapped))
        .unwrap_or_else(|| {
            panic!(
                "no {name} in {:?}",
                raw.variables.iter().map(|v| &v.name).collect::<Vec<_>>()
            )
        });
    raw.real_data[i][0]
}

#[test]
fn espice_reads_each_dialect_to_the_same_op() {
    let bin = espice();
    let ir = circuit(".op\n", true);
    let gens = [
        (Dialect::Ngspice, "ngspice"),
        (Dialect::Hspice, "hspice"),
        (Dialect::Espice, "ngspice"),
        (Dialect::Spectre, "spectre"),
    ];
    let mut refused = Vec::new();
    for (g, tokenizer) in gens {
        let deck = match netlist(&ir, g) {
            Ok(deck) => deck,
            Err(e) => {
                refused.push((g, format!("the renderer refused: {}", e.messages)));
                continue;
            }
        };
        let raw = match op_point(&bin, &deck, tokenizer) {
            Ok(raw) => raw,
            Err(why) => {
                refused.push((g, why));
                continue;
            }
        };
        for (name, want) in [("mid", 0.5), ("b", 2.0), ("lp", 1.0)] {
            let got = node(&raw, name);
            assert!(
                (got - want).abs() < 1e-6,
                "{:?}: v({name}) = {got}, want {want}\n{deck}",
                g
            );
        }
    }
    let got: Vec<Dialect> = refused.iter().map(|(d, _)| *d).collect();
    let known: Vec<Dialect> = ESPICE_REFUSES.iter().map(|(d, _)| *d).collect();
    assert_eq!(got, known, "ESPICE_REFUSES is out of date:\n{refused:#?}");
}

/// `^` is power in ngspice but XOR in LTspice. An expression read as
/// LTspice carries that XOR meaning; written as ngspice (power) it would
/// silently change the deck, so the renderer refuses (`W0002`). Written back
/// to LTspice, where it came from, it is fine.
#[test]
fn caret_meaning_refused_across_dialects() {
    let deck = "* dialects\nR1 a 0 {a^b}\n.end\n";
    let to = |d| render_text(deck, Dialect::Ltspice, "dialects", d);
    let e = to(Dialect::Ngspice).expect_err("refused");
    assert!(
        e.diags.iter().any(|x| x.code == Code::W0002),
        "{}",
        e.messages
    );
    to(Dialect::Ltspice).expect("same dialect's own meaning is fine");
}
