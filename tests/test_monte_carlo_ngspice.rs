//! T-N0 and T-N1 on real ngspice: the Spectre lifting is pure algebra, so a
//! deck written for Spectre (its calls lifted into normalized `rack_*`
//! variables) and back for ngspice must draw as the original deck does, and
//! a Spectre `statistics` table written for ngspice must draw as the table
//! says. Oracles: the closed forms (a divider's linearized mean and std;
//! `limit`'s two values), at their tolerances (mean within 4 sigma/sqrt(N),
//! std within 4 sigma/sqrt(2N)), for the original deck and the written one
//! alike; for the scopes, the inter-instance correlation (|rho| <
//! 4/sqrt(N) apart, 1 for a shared process draw). Moved from the old
//! statistics passes' tests (phase 6 deleted the passes), now through
//! `syntax::render`.
//!
//! Cites: ngspice manual §13.5.48, §18.5.2 (the Monte Carlo loop); Spectre
//! Reference 19.1 pp.175-179 (`statistics`).
//!
//! Contents: none (tests only).

use spicerack::syntax::Dialect;
use spicerack::syntax::render::render_text;

/// The runs of each Monte Carlo loop.
const N: usize = 2000;

// ---- private ----

/// `src` (read in `d`) written for `to`, which must not refuse it.
fn written(src: &str, d: Dialect, to: Dialect) -> String {
    render_text(src, d, "t", to)
        .unwrap_or_else(|e| panic!("{to:?}: {}\n{src}", e.messages))
        .text
}

/// T-N0: agauss(1k,100,3) has std(R1) = 33.3, so v(out) has mean 0.5 and
/// std 2.5e-4 * 33.3 = 8.33e-3; aunif(1k,100) std(R1) = 100/sqrt(3) = 57.7,
/// std 1.443e-2; limit(1k,50) is exactly 1k/2050 or 1k/1950. Each on real
/// ngspice as written and as written back from its Spectre text (the calls
/// lifted into `rack_*` variables, then read back), at the closed form's
/// tolerances (mean within 4 sigma/sqrt(N), std within 4 sigma/sqrt(2N)).
/// A Spectre table's draws lowered to ngspice (Spectre Reference 19.1
/// pp.175-179: `gauss` std, `unif` half-width N) meet the same closed
/// forms: std 50 is 1.25e-2, N=100 is 1.443e-2.
#[test]
fn t_n0_draws_through_spectre_match_the_closed_forms() {
    for (r, sigma) in [
        ("agauss(1k,100,3)", 2.5e-4 * 100.0 / 3.0),
        ("aunif(1k,100)", 2.5e-4 * 100.0 / 3f64.sqrt()),
    ] {
        for (k, rows) in divider(r).iter().enumerate() {
            matches(&rows[0], 0.5, sigma, &format!("{r} deck {k}"));
        }
    }
    for rows in divider("limit(1k,50)") {
        let two = [1e3 / 2050.0, 1e3 / 1950.0];
        assert!(
            rows[0]
                .iter()
                .all(|v| two.iter().any(|t| (v - t).abs() < 1e-6)),
            "{:?}",
            &rows[0][..8]
        );
        assert!(
            two.iter()
                .all(|t| rows[0].iter().any(|v| (v - t).abs() < 1e-6))
        );
    }
    for (vary, sigma) in [
        ("dist=gauss std=50", 2.5e-4 * 50.0),
        ("dist=unif N=100", 2.5e-4 * 100.0 / 3f64.sqrt()),
    ] {
        let scs = format!(
            "// t\nsimulator lang=spectre\nparameters rv=1k\nstatistics {{\n  process {{ vary rv {vary} }}\n}}\nv1 (in 0) vsource dc=1\nr1 (in out) resistor r=rv\nr2 (out 0) resistor r=1k\n"
        );
        let sp = written(&scs, Dialect::Spectre, Dialect::Ngspice);
        assert!(sp.contains("rv__mc="), "{sp}");
        matches(&runs(&sp, &["out"])[0], 0.5, sigma, vary);
    }
}

/// T-N1: a subcircuit's own draw is independent per instance; a top
/// two-layer process draw read by both instances is shared (rho = 1, and
/// |rho| < 4/sqrt(N) apart): as written, written back from Spectre, and a
/// Spectre table's `mismatch` and `process` draws lowered to ngspice
/// (Spectre Reference 19.1 p.175: mismatch "sampled on a per subcircuit
/// instance basis", process "once per Monte Carlo iteration").
#[test]
fn t_n1_mismatch_per_instance_process_shared() {
    let sub = |r: &str| {
        format!(
            "t\n.param g_r='agauss(0,100,1)'\n.param g=g_r\n.subckt s a b\n.param rl='{r}'\nr1 a b {{rl}}\nr2 b 0 1k\n.ends s\nv1 in 0 1\nx1 in o1 s\nx2 in o2 s\n.end\n"
        )
    };
    let bound = 4.0 / (N as f64).sqrt();
    for rows in both(&sub("agauss(1k,100,1)"), &["o1", "o2"]) {
        let rho = corr(&rows[0], &rows[1]);
        assert!(rho.abs() < bound, "mismatch rho {rho}");
    }
    for rows in both(&sub("1k+g"), &["o1", "o2"]) {
        let rho = corr(&rows[0], &rows[1]);
        assert!(rho > 1.0 - 1e-9, "process rho {rho}");
    }
    for (block, independent) in [("mismatch", true), ("process", false)] {
        let scs = format!(
            "// t\nsimulator lang=spectre\nparameters rl=1k\nstatistics {{\n  {block} {{ vary rl dist=gauss std=100 }}\n}}\nsubckt s (a b)\nr1 (a b) resistor r=rl\nr2 (b 0) resistor r=1k\nends s\nv1 (in 0) vsource dc=1\nx1 (in o1) s\nx2 (in o2) s\n"
        );
        let sp = written(&scs, Dialect::Spectre, Dialect::Ngspice);
        let rows = runs(&sp, &["o1", "o2"]);
        let rho = corr(&rows[0], &rows[1]);
        match independent {
            true => assert!(rho.abs() < bound, "{block} rho {rho}\n{sp}"),
            false => assert!(rho > 1.0 - 1e-9, "{block} rho {rho}\n{sp}"),
        }
    }
}

/// `v(out)` of a divider `r1 in out <r>`, `r2 out 0 1k`, `v1 in 0 1`: the
/// original deck's runs and the deck written back from its Spectre text.
fn divider(r: &str) -> [Vec<Vec<f64>>; 2] {
    let deck = format!("t\nv1 in 0 1\nr1 in out r='{r}'\nr2 out 0 1k\n.end\n");
    both(&deck, &["out"])
}

/// `deck` run `N` times as written and as written for Spectre (its calls
/// lifted into normalized variables) and back for ngspice, the probed node
/// voltages of each run.
fn both(deck: &str, probes: &[&str]) -> [Vec<Vec<f64>>; 2] {
    let scs = written(deck, Dialect::Ngspice, Dialect::Spectre);
    assert!(
        scs.contains("rack_") && scs.contains("statistics {"),
        "{scs}"
    );
    let back = written(&scs, Dialect::Spectre, Dialect::Ngspice);
    [runs(deck, probes), runs(&back, probes)]
}

/// ngspice's operating point `N` times, the circuit re-read with fresh
/// draws between runs (`mc_source`, ngspice manual §13.5.48).
fn runs(deck: &str, probes: &[&str]) -> Vec<Vec<f64>> {
    let body = deck.trim_end().trim_end_matches(".end");
    let print: Vec<String> = probes.iter().map(|p| format!("v({p})")).collect();
    let text = format!(
        "{body}\n.control\nlet i = 0\ndowhile i < {N}\n  if i > 0\n    mc_source\n  end\n  op\n  print {}\n  let i = i + 1\nend\n.endc\n.end\n",
        print.join(" ")
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mc.sp");
    std::fs::write(&path, &text).unwrap();
    let out = std::process::Command::new("ngspice")
        .arg("-b")
        .arg(&path)
        .output()
        .expect("ngspice not on PATH: run the tests in `nix develop .#testing`");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut rows: Vec<Vec<f64>> = vec![Vec::new(); probes.len()];
    for l in stdout.lines() {
        if let Some((k, v)) = l.split_once(" = ")
            && let Some(i) = print.iter().position(|p| p == k.trim())
        {
            rows[i].push(v.trim().parse().unwrap());
        }
    }
    assert!(
        rows.iter().all(|r| r.len() == N),
        "{}\n{text}",
        String::from_utf8_lossy(&out.stderr)
    );
    rows
}

fn mean(x: &[f64]) -> f64 {
    x.iter().sum::<f64>() / x.len() as f64
}

fn std(x: &[f64]) -> f64 {
    let m = mean(x);
    (x.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / (x.len() - 1) as f64).sqrt()
}

fn corr(a: &[f64], b: &[f64]) -> f64 {
    let (ma, mb) = (mean(a), mean(b));
    let cov: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| (x - ma) * (y - mb))
        .sum::<f64>()
        / (a.len() - 1) as f64;
    cov / (std(a) * std(b))
}

/// The mean and std of `x` against the closed form `(mu, sigma)`.
fn matches(x: &[f64], mu: f64, sigma: f64, what: &str) {
    let n = x.len() as f64;
    assert!(
        (mean(x) - mu).abs() < 4.0 * sigma / n.sqrt(),
        "{what}: mean {} against {mu}",
        mean(x)
    );
    assert!(
        (std(x) - sigma).abs() < 4.0 * sigma / (2.0 * n).sqrt(),
        "{what}: std {} against {sigma}",
        std(x)
    );
}
