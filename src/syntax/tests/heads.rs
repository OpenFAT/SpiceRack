//! The parser's header statements, one test per shape: SPICE elements split
//! by their device row, Spectre instances, analyses, control statements and
//! options by their master, VACASK `analysis` and `options`. Each names the
//! clause it reads, the tree derived by hand, the codes, and its checks
//! (printed as `mod.rs` prints trees: `Tag"head"<code name master|nodes>[args]`).
//!
//! Contents: none (tests only).

use super::read;
use crate::syntax::lexicon::{HSPICE, NGSPICE, SPECTRE, VACASK};

// ---- private ----

#[test]
fn spice_instance_x_master_either_side() {
    // ngspice manual §2.4.3: `XYYY N1 N2 SUBNAM p=v`; sky130's alias puts
    // the parameters first (`X1 a b w=1 sub`). Checks: 2.
    let (top, codes) = read("t\nX1 a b sub w=1\nX2 a b w=1 sub\n", &NGSPICE);
    assert_eq!(
        top,
        [
            "Instance\"X1\"<Subckt w\"X1\" w\"sub\"|w\"a\" w\"b\">[w\"w\"=n\"1\"]",
            "Instance\"X2\"<Subckt w\"X2\" w\"sub\"|w\"a\" w\"b\">[w\"w\"=n\"1\"]"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn element_cards_split_by_their_device_row() {
    // The general forms `syntax::device` cites: `RXXX n+ n- <value>
    // <mname>` (a model among the arguments: resolve decides), `KXXX LYYY
    // LZZZ value`, `VXXX N+ N- <DC> ...`, `EXXX N+ N- NC+ NC- VALUE`, `FXXX
    // N+ N- VNAM VALUE`, `DXXX n+ n- mname`, `QXXX nc nb ne <ns> mname`
    // (a flag ends the nodes), `MXXX nd ng ns nb mname` and HSPICE's
    // bulkless `Mxxx nd ng ns mname`, `WYYY N+ N- VNAM MODEL`, `TXXX N1 N2
    // N3 N4 Z0=VALUE`, ngspice's OSDI `NXXX n1 ... mname`. Checks: 2.
    let src = "t\nR1 a b 1k rmod\nK1 L1 L2 0.9\nV1 a 0 dc 1 ac 1\nE1 a b c d 2\nF1 a b vs 2\nD1 a k dmod area=2\nQ1 c b e s qmod off\nM1 d g s b nch w=1u\nM2 d g s nch\nW1 a b vc wmod\nT1 a 0 b 0 z0=50\nN1 a b c va\n";
    let (top, codes) = read(src, &NGSPICE);
    assert_eq!(
        top,
        [
            "Instance\"R1\"<Resistor w\"R1\" -|w\"a\" w\"b\">[w\"1k\" w\"rmod\"]",
            "Instance\"K1\"<Coupling w\"K1\" -|>[w\"L1\" w\"L2\" w\"0.9\"]",
            "Instance\"V1\"<VoltageSource w\"V1\" -|w\"a\" w\"0\">[w\"dc\" w\"1\" w\"ac\" w\"1\"]",
            "Instance\"E1\"<Vcvs w\"E1\" -|w\"a\" w\"b\" w\"c\" w\"d\">[w\"2\"]",
            "Instance\"F1\"<Cccs w\"F1\" -|w\"a\" w\"b\">[w\"vs\" w\"2\"]",
            "Instance\"D1\"<Diode w\"D1\" w\"dmod\"|w\"a\" w\"k\">[w\"area\"=n\"2\"]",
            "Instance\"Q1\"<Bjt w\"Q1\" w\"qmod\"|w\"c\" w\"b\" w\"e\" w\"s\">[w\"off\"]",
            "Instance\"M1\"<Mosfet w\"M1\" w\"nch\"|w\"d\" w\"g\" w\"s\" w\"b\">[w\"w\"=n\"1u\"]",
            "Instance\"M2\"<Mosfet w\"M2\" w\"nch\"|w\"d\" w\"g\" w\"s\">[]",
            "Instance\"W1\"<CurrentSwitch w\"W1\" w\"wmod\"|w\"a\" w\"b\">[w\"vc\"]",
            "Instance\"T1\"<TransmissionLine w\"T1\" -|w\"a\" w\"0\" w\"b\" w\"0\">[w\"z0\"=n\"50\"]",
            "Instance\"N1\"<Osdi w\"N1\" w\"va\"|w\"a\" w\"b\" w\"c\">[]"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn a_letter_no_table_holds_has_no_primitive() {
    // HSPICE reads only the twenty shared letters (`syntax::device::SPICE`);
    // its legal neighbour is ngspice's `N1` above. Checks: 1.
    let (top, _) = read("t\nN1 a b c va\n", &HSPICE);
    assert_eq!(
        top,
        ["Instance\"N1\"<- w\"N1\" -|>[w\"a\" w\"b\" w\"c\" w\"va\"]"]
    );
}

#[test]
fn spectre_analyses_controls_and_options_by_master() {
    // Spectre Reference 19.1: a statement is an analysis by its master
    // (p.416 `tran1 tran stop=1n`; p.181 `noise1 (out 0) noise`), a control
    // statement or `options` only with no nodes; p.406 `sweep` holds its
    // analyses in braces; the legal neighbour `x1 (a b) set` is an
    // instance. Checks: 2.
    let src = "t\ntran1 tran stop=1n\nnoise1 (out 0) noise iprobe=v1\nopt1 options reltol=1e-3\ni1 info what=models\nx1 (a b) set\nsw1 sweep param=temp values=[1 2] {\ntr2 tran stop=1n\n}\n";
    let (top, codes) = read(src, &SPECTRE);
    assert_eq!(
        top,
        [
            "Analysis\"tran1\"<Tran w\"tran1\" w\"tran\"|>[w\"stop\"=n\"1n\"]",
            "Analysis\"noise1\"<Noise w\"noise1\" w\"noise\"|w\"out\" w\"0\">[w\"iprobe\"=i\"v1\"]",
            "Options\"opt1\"<- w\"opt1\" w\"options\"|>[w\"reltol\"=n\"1e-3\"]",
            "Command\"i1\"<Info w\"i1\" w\"info\"|>[w\"what\"=i\"models\"]",
            "Instance\"x1\"<- w\"x1\" w\"set\"|w\"a\" w\"b\">[]",
            "Analysis\"sw1\"<Sweep w\"sw1\" w\"sweep\"|>[w\"param\"=i\"temp\" w\"values\"=Vector\"[\"[n\"1\" n\"2\"]]{Analysis\"tr2\"<Tran w\"tr2\" w\"tran\"|>[w\"stop\"=n\"1n\"]}"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn vacask_analysis_and_options_in_control() {
    // VACASK demo decks: `control` holds `options k=v` and `analysis <name>
    // <kind> k=v`; a kind the table lacks keeps its word, code `-`.
    // Checks: 2.
    let src = "t\ncontrol\noptions reltol=1e-3\nanalysis op1 op\nanalysis x1 zz\nendc\n";
    let (top, codes) = read(src, &VACASK);
    assert_eq!(
        top,
        [
            "Control\"control\"[]{Options\"options\"<- - -|>[w\"reltol\"=n\"1e-3\"] Analysis\"analysis\"<Op w\"op1\" w\"op\"|>[] Analysis\"analysis\"<- w\"x1\" w\"zz\"|>[]}"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn spectre_instance_with_and_without_parentheses() {
    // Spectre UG p.66: `name [(]nodes[)] master params`; without parentheses
    // the master is the last bare word before the first `name=`. Checks: 2.
    let (top, codes) = read(
        "t\nr1 (a b) resistor r=1k\nr2 a b resistor r=2k\n",
        &SPECTRE,
    );
    assert_eq!(
        top,
        [
            "Instance\"r1\"<Resistor w\"r1\" w\"resistor\"|w\"a\" w\"b\">[w\"r\"=n\"1k\"]",
            "Instance\"r2\"<Resistor w\"r2\" w\"resistor\"|w\"a\" w\"b\">[w\"r\"=n\"2k\"]"
        ]
    );
    assert!(codes.is_empty());
}
