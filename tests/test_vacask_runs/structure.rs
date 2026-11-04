//! Deck structure on VACASK: subcircuits and instances, `.step` parameters,
//! initial conditions, and several analyses in one deck in order.
//!
//! Cites: real VACASK.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

#[test]
fn subcircuits_and_their_instances_run() {
    let deck = "with_subckt\n.subckt divider in out PARAMS: rtop=1k rbot=1k\nR1 in out {rtop}\nR2 out 0 {rbot}\n.ends divider\nX1 vdd mid divider rtop=1k rbot=3k\nVdd vdd 0 4\n.op\n.end\n";
    let raw = run(deck);
    // 4 V * 3k/(1k+3k) = 3 V.
    close(col(&raw, "mid")[0], 3.0, 1e-9);
}

#[test]
fn step_params_sweep_a_netlist_variable() {
    // A `var` declared in the control block IS visible to the netlist, so a
    // `.step` of a netlist parameter runs on VACASK.
    let deck = "stepped\n.param rtop=1k\nR1 vdd out {rtop}\nR2 out 0 1k\nVdd vdd 0 2\n.step rtop 1000 3000 1000\n.op\n.end\n";
    let raw = run(deck);
    let out = col(&raw, "out");
    assert_eq!(out.len(), 3, "{:?}", out);
    close(out[0], 2.0 * 1000.0 / 2000.0, 1e-9);
    close(out[1], 2.0 * 1000.0 / 3000.0, 1e-9);
    close(out[2], 2.0 * 1000.0 / 4000.0, 1e-9);
}

#[test]
fn initial_conditions_reach_the_analysis() {
    let deck = "ic\nR1 vin out 1k\nC1 out 0 1u\nV1 vin 0 1\n.ic V(out)=0.5\n.tran 25u 1m 0 25u UIC\n.end\n";
    let netlist = emit(deck).unwrap();
    assert!(netlist.contains("ic=[\"out\"; 5e-1]"), "{}", netlist);
    let raw = run(deck);
    close(col(&raw, "out")[0], 0.5, 1e-9);
}

#[test]
fn run_all_returns_every_analysis_in_deck_order() {
    // VACASK writes one raw file per analysis; run_all reads each, in the
    // order the deck declares them. Oracle: the divider's closed forms,
    // out = vdd/2 in every analysis (1.5 V at the 3 V rail).
    let deck = rc(".op\n.dc Vdd 0 2 1\n.tran 100u 1m 0 100u");
    let netlist = emit(&deck).expect("emit vacask deck");
    assert!(
        have_vacask(),
        "vacask not on PATH: run the tests in `nix develop .#testing`"
    );
    let plots = Dialect::Vacask
        .run_all(&netlist)
        .unwrap_or_else(|e| panic!("{e}\n{netlist}"));
    assert_eq!(
        plots.len(),
        3,
        "{:?}",
        plots.iter().map(|p| &p.plot_name).collect::<Vec<_>>()
    );
    close(col(&plots[0], "out")[0], 1.5, 1e-9);
    assert_eq!(col(&plots[1], "vsweep"), vec![0.0, 1.0, 2.0]);
    for (v, want) in col(&plots[1], "out").into_iter().zip([0.0, 0.5, 1.0]) {
        assert!((v - want).abs() <= 1e-9, "dc out {v}, want {want}");
    }
    let time = col(&plots[2], "time");
    close(*time.last().unwrap(), 1e-3, 1e-9);
    assert!(
        col(&plots[2], "out")
            .iter()
            .all(|v| (v - 1.5).abs() <= 1e-9)
    );
    // `run` is the first analysis's plot, as for every backend.
    assert_eq!(run(&deck).plot_name, plots[0].plot_name);
    assert_eq!(col(&run(&deck), "out").len(), 1);
}
