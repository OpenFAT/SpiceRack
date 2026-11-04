//! Each SPICE device VACASK runs through its `sp_*` modules (diode, BJT,
//! level-1 MOSFET, JFET, coupled inductors, controlled sources), emitted and
//! run.
//!
//! Cites: real VACASK; the closed forms named in each test.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

#[test]
fn a_diode_runs_under_the_spice_device_set() {
    let deck = "diode\n.model dmod D (IS=1e-14 N=1.0)\nR1 vin a 1k\nD1 a 0 dmod\nV1 vin 0 1\n.temp 27\n.op\n.end\n";
    let netlist = emit(deck).unwrap();
    assert!(netlist.contains("load \"spice/diode.osdi\""), "{}", netlist);
    assert!(netlist.contains("model dmod sp_diode"), "{}", netlist);
    let raw = run(deck);
    // 1 V through 1k into a diode: a forward drop somewhere in the 0.4-0.8 V
    // band, and the rest across the resistor.
    let va = col(&raw, "a")[0];
    assert!(
        (0.4..0.8).contains(&va),
        "diode drop {} V is not physical",
        va
    );
}

#[test]
fn a_bjt_runs_and_carries_its_polarity() {
    let deck = "bjt\n.model qn NPN (IS=1e-15 BF=100)\nRb vb base 10k\nRc vcc coll 1k\nQ1 coll base 0 qn\nVcc vcc 0 5\nVb vb 0 1\n.op\n.end\n";
    let netlist = emit(deck).unwrap();
    assert!(netlist.contains("model qn sp_bjt (type=1"), "{}", netlist);
    let raw = run(deck);
    let vbe = col(&raw, "base")[0];
    assert!(
        (0.5..0.9).contains(&vbe),
        "Vbe {} V is not a forward-biased NPN",
        vbe
    );
    // With beta=100 the stage is on, so the collector is pulled well below Vcc.
    assert!(col(&raw, "coll")[0] < 4.9);
}

#[test]
fn a_level_1_mosfet_runs() {
    let deck = "mos\n.model nm NMOS (LEVEL=1 VTO=1.0 KP=2e-5 LAMBDA=0.05)\nRd vdd drain 1k\nM1 drain gate 0 0 nm W=10u L=1u\nVdd vdd 0 5\nVg gate 0 3\n.op\n.end\n";
    let netlist = emit(deck).unwrap();
    assert!(netlist.contains("load \"spice/mos1.osdi\""), "{}", netlist);
    assert!(netlist.contains("model nm sp_mos1 (type=1"), "{}", netlist);
    assert!(netlist.contains("w=10u l=1u"), "{}", netlist);
    let raw = run(deck);
    let vd = col(&raw, "drain")[0];
    assert!(
        vd < 5.0 && vd > 0.0,
        "drain {} V is not a conducting NMOS",
        vd
    );
}

#[test]
fn a_jfet_runs() {
    let deck = "jfet\n.model jn NJF (VTO=-2 BETA=1e-3)\nRd vdd drain 1k\nJ1 drain 0 0 jn\nVdd vdd 0 5\n.op\n.end\n";
    let netlist = emit(deck).unwrap();
    assert!(netlist.contains("load \"spice/jfet1.osdi\""), "{}", netlist);
    let raw = run(deck);
    assert!(col(&raw, "drain")[0] < 5.0);
}

#[test]
fn an_inductor_and_its_mutual_coupling_run() {
    let deck = "xfmr\nL1 p 0 1m\nL2 s 0 4m\nK1 L1 L2 0.99\nR1 drive p 1\nR2 s 0 1meg\nV1 drive 0 0 AC 1\n.ac dec 5 1k 100k\n.end\n";
    let netlist = emit(deck).unwrap();
    assert!(
        netlist.contains("k1 () mutual k=9.9e-1 ind1=\"l1\" ind2=\"l2\""),
        "{}",
        netlist
    );
    let raw = run(deck);
    // Turns ratio sqrt(4m/1m) = 2, so the secondary rises above the primary.
    let p = cplx(&raw, "p");
    let s = cplx(&raw, "s");
    let ratio = s.last().unwrap().norm() / p.last().unwrap().norm();
    close(ratio, 2.0 * 0.99, 2e-2);
}

#[test]
fn controlled_sources_run() {
    let deck = "ctl\nR0 in 0 1k\nE1 e 0 in 0 2\nR1 e 0 1k\nG1 0 g in 0 2m\nR2 g 0 1k\nF1 0 f V1 -2\nR3 f 0 1k\nH1 h 0 V1 -2k\nR4 h 0 1k\nV1 in 0 2\n.op\n.end\n";
    let raw = run(deck);
    close(col(&raw, "e")[0], 4.0, 1e-9); // VCVS: 2 V * 2
    close(col(&raw, "g")[0], 2.0 * 2e-3 * 1e3, 1e-9); // VCCS: 2 V * 2 mS * 1k
    // i(V1) = -2 mA, so the CCCS drives -2 * -2 mA = 4 mA into 1k.
    close(col(&raw, "f")[0], 4.0, 1e-9);
    close(col(&raw, "h")[0], 4.0, 1e-9);
}
