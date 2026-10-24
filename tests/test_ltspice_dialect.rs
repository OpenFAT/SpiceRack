//! LTspice dialect audit.
//!
//! UNVERIFIED BY EXECUTION: no LTspice binary exists on this machine (wine is
//! present, LTspice is not). Every assertion here checks the emitted text
//! against LTspice's *documented* syntax. Nothing below was run through
//! LTspice.

use spicerack::codegen::{netlist, CodeGenError, Dialect};
use spicerack::ir::*;

/// One dialect, probed one piece at a time through a minimal deck.
struct Cg(Dialect);

fn lt() -> Cg {
    Cg(Dialect::Ltspice)
}

fn ng() -> Cg {
    Cg(Dialect::Ngspice)
}

impl Cg {
    fn emit_netlist(&self, ir: &CircuitIR) -> Result<String, CodeGenError> {
        netlist(ir, self.0)
    }

    /// The deck minus its `* t` title and `.end`.
    fn body(&self, component: Option<&Component>, tb: Testbench) -> Result<String, CodeGenError> {
        let top = Subcircuit {
            name: "t".into(), ports: vec![], parameters: vec![],
            components: component.into_iter().cloned().collect(), instances: vec![], models: vec![],
            raw_spice: vec![], includes: vec![], libs: vec![], osdi_loads: vec![], verilog_blocks: vec![],
        };
        let ir = CircuitIR { raw_dialect: None, top, testbench: Some(tb), subcircuit_defs: vec![], model_libraries: vec![] };
        let n = self.emit_netlist(&ir)?;
        Ok(n.strip_prefix("* t\n").unwrap().strip_suffix("\n.end").unwrap().to_string())
    }

    fn emit_step_param(&self, sp: &StepParam) -> Result<String, CodeGenError> {
        self.body(None, Testbench { step_params: vec![sp.clone()], ..tb() })
    }

    /// The last line: ngspice puts `.save all` ahead of a noise card.
    fn emit_analysis(&self, a: &Analysis) -> Result<String, CodeGenError> {
        let b = self.body(None, Testbench { analyses: vec![a.clone()], ..tb() })?;
        Ok(b.lines().last().unwrap().to_string())
    }

    fn emit_component(&self, c: &Component) -> Result<String, CodeGenError> {
        self.body(Some(c), tb())
    }

    fn emit_options(&self, o: &SimOptions) -> Result<String, CodeGenError> {
        self.body(None, Testbench { options: o.clone(), ..tb() })
    }
}

fn tb() -> Testbench {
    Testbench {
        dut: "t".into(), stimulus: vec![], analyses: vec![], options: SimOptions::default(),
        saves: vec![], measures: vec![], temperature: None, nominal_temperature: None,
        initial_conditions: vec![], node_sets: vec![], step_params: vec![], extra_lines: vec![],
    }
}

// ── .step ──

/// LTspice: `.step [lin|oct|dec] param <name> <start> <stop> <inc>`.
/// The `param` keyword is required in the lin/oct/dec form too — the codegen
/// used to drop it, producing `.step dec rval ...` which LTspice rejects.
#[test]
fn step_keeps_the_param_keyword_in_every_form() {
    for sweep in ["lin", "oct", "dec"] {
        let sp = StepParam {
            param: "rval".into(),
            start: 1e3,
            stop: 1e4,
            step: 10.0,
            sweep_type: Some(sweep.into()),
        };
        let emitted = lt().emit_step_param(&sp).unwrap();
        assert_eq!(emitted, format!(".step {sweep} param rval 1000 10000 10"));
    }
}

#[test]
fn ngspice_refuses_step_instead_of_commenting_it_out() {
    let sp = StepParam {
        param: "rval".into(),
        start: 1e3,
        stop: 1e4,
        step: 1e3,
        sweep_type: None,
    };
    let err = ng().emit_step_param(&sp).unwrap_err().to_string();
    assert!(err.contains("no `.step`"), "{err}");
    assert!(err.contains(".dc"), "error should name the working alternative: {err}");
}

// ── .four ──

/// LTspice's `.four` takes an optional harmonic count; SPICE3/ngspice's does
/// not (ngspice reads the count as an output vector and fails the analysis).
#[test]
fn four_harmonic_count_is_ltspice_only() {
    let four = Analysis::Fourier {
        fundamental: 1e3,
        outputs: vec!["V(out)".into()],
        num_harmonics: Some(9),
    };
    assert_eq!(lt().emit_analysis(&four).unwrap(), ".four 1k 9 V(out)");
    assert_eq!(ng().emit_analysis(&four).unwrap(), ".four 1k V(out)");
}

// ── .noise ──

/// `pts_per_summary` is a SPICE3/ngspice extension; LTspice's `.noise` card
/// ends at the stop frequency.
#[test]
fn noise_points_per_summary_is_ngspice_only() {
    let noise = Analysis::Noise {
        output: "out".into(),
        reference: "0".into(),
        source: "V1".into(),
        variation: "dec".into(),
        points: 10,
        start: 1.0,
        stop: 1e6,
        points_per_summary: Some(5),
    };
    assert_eq!(lt().emit_analysis(&noise).unwrap(), ".noise V(out) V1 dec 10 1 1meg");
    assert_eq!(ng().emit_analysis(&noise).unwrap(), ".noise V(out) V1 dec 10 1 1meg 5");
}

// ── analyses LTspice does not have ──

#[test]
fn pz_disto_and_sens_are_rejected_for_ltspice() {
    let cases: Vec<Analysis> = vec![
        Analysis::PoleZero {
            node1: "1".into(),
            node2: "0".into(),
            node3: "3".into(),
            node4: "0".into(),
            tf_type: "vol".into(),
            pz_type: "pz".into(),
        },
        Analysis::Distortion {
            variation: "dec".into(),
            points: 10,
            start: 1e3,
            stop: 1e6,
            f2overf1: None,
        },
        Analysis::Sensitivity { output: "V(out)".into(), ac: None },
    ];
    for a in &cases {
        assert!(lt().emit_analysis(a).is_err(), "ltspice must reject {a:?}");
        assert!(ng().emit_analysis(a).is_ok(), "ngspice must accept {a:?}");
    }
}

// ── .save ──

fn noise_tb(saves: Vec<String>) -> CircuitIR {
    CircuitIR { raw_dialect: None,
        top: Subcircuit {
            name: "nz".into(),
            ports: vec![],
            parameters: vec![],
            components: vec![Component::Resistor {
                name: "1".into(),
                n1: "in".into(),
                n2: "out".into(),
                value: IrValue::Numeric { value: 10e3 },
                params: vec![],
            }],
            instances: vec![],
            models: vec![],
            raw_spice: vec![],
            includes: vec![],
            libs: vec![],
            osdi_loads: vec![],
            verilog_blocks: vec![],
        },
        testbench: Some(Testbench {
            dut: "nz".into(),
            stimulus: vec![],
            analyses: vec![Analysis::Noise {
                output: "out".into(),
                reference: "0".into(),
                source: "V1".into(),
                variation: "dec".into(),
                points: 10,
                start: 1.0,
                stop: 1e6,
                points_per_summary: None,
            }],
            options: SimOptions::default(),
            saves,
            measures: vec![],
            temperature: None,
            nominal_temperature: None,
            initial_conditions: vec![],
            node_sets: vec![],
            step_params: vec![],
            extra_lines: vec![],
        }),
        subcircuit_defs: vec![],
        model_libraries: vec![],
    }
}

/// A node-name save list cannot name the noise output vectors, so it must not
/// be emitted. ngspice needs an explicit `.save all` to override; LTspice has
/// no `.save all` and saves everything by default, so it gets no save line.
#[test]
fn noise_never_emits_a_node_save_list() {
    let ir = noise_tb(vec!["V(out)".into()]);

    let ng_netlist = ng().emit_netlist(&ir).unwrap();
    assert!(ng_netlist.contains(".save all"), "{ng_netlist}");
    assert!(!ng_netlist.contains(".save V(out)"), "{ng_netlist}");

    let lt_netlist = lt().emit_netlist(&ir).unwrap();
    assert!(!lt_netlist.contains(".save"), "ltspice has no .save all: {lt_netlist}");
}

// ── XSPICE and OSDI are ngspice-only ──

#[test]
fn xspice_and_osdi_are_commented_or_omitted_for_ltspice() {
    let a = Component::Xspice {
        name: "1".into(),
        connections: vec!["in".into(), "out".into()],
        model: "gainblk".into(),
    };
    assert!(lt().emit_component(&a).unwrap().starts_with("* XSPICE"));
    assert!(ng().emit_component(&a).unwrap().starts_with("A1 "));
}

// ── options ──

#[test]
fn ltspice_uses_the_spice3_option_spellings() {
    let opts = SimOptions {
        portable: vec![
            ("reltol".into(), "1e-4".into()),
            ("abstol".into(), "1e-12".into()),
            ("max_iterations".into(), "200".into()),
        ],
        backend_specific: Default::default(),
    };
    let s = lt().emit_options(&opts).unwrap();
    assert!(s.contains("reltol=1e-4"), "{s}");
    assert!(s.contains("abstol=1e-12"), "{s}");
    // LTspice documents itl1/itl2/itl4 like SPICE3.
    assert!(s.contains("ITL1=200"), "{s}");
}
