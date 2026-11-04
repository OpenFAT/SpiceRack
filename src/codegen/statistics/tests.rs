//! T-N0 and T-N1 on real ngspice: the Spectre lifting is pure algebra, so lifting a deck and
//! lowering its normalized variables back to ngspice draws (`lift`, then
//! `lower`) must give the original deck's statistics. Oracles: the closed
//! forms (a divider's linearized mean and std; `limit`'s
//! two values), at its tolerances (mean within 4 sigma/sqrt(N), std within
//! 4 sigma/sqrt(2N)), and ngspice's own run of the original deck at the same
//! tolerances; for the scopes, the inter-instance correlation (|rho| <
//! 4/sqrt(N) apart, 1 for a shared process draw).
//!
//! Cites: ngspice manual §13.5.48, §18.5.2 (the Monte Carlo loop).
//!
//! Contents: none (tests only).

use super::{lift, lower};
use crate::codegen::{Dialect, netlist};
use crate::import::parse;

const N: usize = 2000;

// ---- private ----

/// `v(out)` of a divider `r1 in out <r>`, `r2 out 0 1k`, `v1 in 0 1`: the
/// original deck's runs and the lifted-and-lowered deck's.
fn divider(r: &str) -> [Vec<Vec<f64>>; 2] {
    let deck = format!("t\nv1 in 0 1\nr1 in out r='{r}'\nr2 out 0 1k\n.end\n");
    both(&deck, &["out"])
}

/// `deck` run `N` times as written and as lifted then lowered, the probed
/// node voltages of each run.
fn both(deck: &str, probes: &[&str]) -> [Vec<Vec<f64>>; 2] {
    let (ir, _) = parse(deck, "t", Dialect::Ngspice).unwrap();
    let lifted = lift(&ir)
        .unwrap()
        .expect("the deck calls a statistical function");
    let lowered = lower(&lifted, false)
        .unwrap()
        .expect("lifting made a table");
    let text = netlist(&lowered, Dialect::Ngspice).unwrap();
    assert!(text.contains("rack_"), "{text}");
    [runs(deck, probes), runs(&text, probes)]
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

/// T-N0: agauss(1k,100,3) has std(R1) = 33.3, so v(out) has mean 0.5 and
/// std 2.5e-4 * 33.3 = 8.33e-3; aunif(1k,100) std(R1) = 100/sqrt(3) = 57.7,
/// std 1.443e-2; limit(1k,50) is exactly 1k/2050 or 1k/1950.
#[test]
fn t_n0_lowered_draws_match_the_original() {
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
}

/// T-N1: a subcircuit's own draw is independent per instance; a top
/// two-layer process draw read by both instances is shared (rho = 1).
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
}
