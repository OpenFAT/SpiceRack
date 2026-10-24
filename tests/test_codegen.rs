use std::collections::BTreeMap;
use spicerack::codegen::{netlist, CodeGenError, Dialect};
use spicerack::ir::*;

/// The card a single analysis becomes: the deck's line before `.end`.
fn analysis(d: Dialect, a: &Analysis) -> Result<String, CodeGenError> {
    let mut ir = resistor_divider_ir();
    ir.testbench.as_mut().unwrap().analyses = vec![a.clone()];
    let n = netlist(&ir, d)?;
    Ok(n.lines().rev().nth(1).unwrap().to_string())
}

// ── Shared test fixtures ──

fn resistor_divider_ir() -> CircuitIR {
    CircuitIR { raw_dialect: None, globals: vec![],
        top: Subcircuit {
            name: "Voltage Divider".into(),
            ports: vec![],
            parameters: vec![],
            components: vec![
                Component::VoltageSource {
                    name: "in".into(),
                    np: "input".into(),
                    nm: "0".into(),
                    value: IrValue::Numeric { value: 10.0 },
                    ac_magnitude: None,
                    ac_phase: None,
                    waveform: None,
                },
                Component::Resistor {
                    name: "1".into(),
                    n1: "input".into(),
                    n2: "output".into(),
                    value: IrValue::Numeric { value: 10000.0 },
                    params: vec![],
                },
                Component::Resistor {
                    name: "2".into(),
                    n1: "output".into(),
                    n2: "0".into(),
                    value: IrValue::Numeric { value: 10000.0 },
                    params: vec![],
                },
            ],
            instances: vec![],
            models: vec![],
            raw_spice: vec![],
            includes: vec![],
            libs: vec![],
            osdi_loads: vec![],
            verilog_blocks: vec![],
        },
        testbench: Some(Testbench { prints: vec![],
            dut: "Voltage Divider".into(),
            stimulus: vec![],
            analyses: vec![Analysis::Op],
            options: SimOptions::default(),
            saves: vec![],
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

fn mosfet_inverter_ir() -> CircuitIR {
    CircuitIR { raw_dialect: None, globals: vec![],
        top: Subcircuit {
            name: "CMOS Inverter".into(),
            ports: vec![],
            parameters: vec![],
            components: vec![
                Component::Mosfet {
                    name: "p1".into(),
                    nd: "out".into(),
                    ng: "in".into(),
                    ns: "vdd".into(),
                    nb: "vdd".into(),
                    model: "pmos_3p3".into(),
                    params: vec![("W".into(), "2u".into()), ("L".into(), "180n".into())],
                },
                Component::Mosfet {
                    name: "n1".into(),
                    nd: "out".into(),
                    ng: "in".into(),
                    ns: "0".into(),
                    nb: "0".into(),
                    model: "nmos_3p3".into(),
                    params: vec![("W".into(), "1u".into()), ("L".into(), "180n".into())],
                },
                Component::VoltageSource {
                    name: "dd".into(),
                    np: "vdd".into(),
                    nm: "0".into(),
                    value: IrValue::Numeric { value: 3.3 },
                    ac_magnitude: None,
                    ac_phase: None,
                    waveform: None,
                },
            ],
            instances: vec![],
            models: vec![
                ModelDef {
                    name: "nmos_3p3".into(),
                    kind: "NMOS".into(),
                    parameters: vec![("VTO".into(), "0.7".into())],
                },
                ModelDef {
                    name: "pmos_3p3".into(),
                    kind: "PMOS".into(),
                    parameters: vec![("VTO".into(), "-0.7".into())],
                },
            ],
            raw_spice: vec![],
            includes: vec![],
            libs: vec![],
            osdi_loads: vec![],
            verilog_blocks: vec![],
        },
        testbench: Some(Testbench { prints: vec![],
            dut: "CMOS Inverter".into(),
            stimulus: vec![],
            analyses: vec![
                Analysis::Dc { sweeps: vec![DcSweep { source: "Vin".into(), start: 0.0, stop: 3.3, step: 0.01 }] },
            ],
            options: SimOptions::default(),
            saves: vec!["V(out)".into()],
            measures: vec![],
            temperature: Some(27.0),
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

fn subcircuit_ir() -> CircuitIR {
    CircuitIR { raw_dialect: None, globals: vec![],
        top: Subcircuit {
            name: "Top Level".into(),
            ports: vec![],
            parameters: vec![],
            components: vec![
                Component::VoltageSource {
                    name: "dd".into(),
                    np: "vdd".into(),
                    nm: "0".into(),
                    value: IrValue::Numeric { value: 1.8 },
                    ac_magnitude: None,
                    ac_phase: None,
                    waveform: None,
                },
            ],
            instances: vec![
                Instance {
                    name: "inv1".into(),
                    subcircuit: "inverter".into(),
                    port_mapping: vec!["a".into(), "b".into(), "vdd".into(), "0".into()],
                    parameters: vec![],
                },
                Instance {
                    name: "inv2".into(),
                    subcircuit: "inverter".into(),
                    port_mapping: vec!["b".into(), "c".into(), "vdd".into(), "0".into()],
                    parameters: vec![("wp".into(), "4u".into())],
                },
            ],
            models: vec![],
            raw_spice: vec![],
            includes: vec![],
            libs: vec![],
            osdi_loads: vec![],
            verilog_blocks: vec![],
        },
        testbench: Some(Testbench { prints: vec![],
            dut: "Top Level".into(),
            stimulus: vec![],
            analyses: vec![Analysis::Op],
            options: SimOptions::default(),
            saves: vec![],
            measures: vec![],
            temperature: None,
            nominal_temperature: None,
            initial_conditions: vec![],
            node_sets: vec![],
            step_params: vec![],
            extra_lines: vec![],
        }),
        subcircuit_defs: vec![
            Subcircuit {
                name: "inverter".into(),
                ports: vec![
                    Port { name: "in".into(), direction: PortDirection::Input },
                    Port { name: "out".into(), direction: PortDirection::Output },
                    Port { name: "vdd".into(), direction: PortDirection::InOut },
                    Port { name: "vss".into(), direction: PortDirection::InOut },
                ],
                parameters: vec![
                    ParamDef { name: "wp".into(), default: Some("2u".into()) },
                    ParamDef { name: "wn".into(), default: Some("1u".into()) },
                ],
                components: vec![
                    Component::Mosfet {
                        name: "p".into(),
                        nd: "out".into(),
                        ng: "in".into(),
                        ns: "vdd".into(),
                        nb: "vdd".into(),
                        model: "pmos".into(),
                        params: vec![("W".into(), "{wp}".into()), ("L".into(), "180n".into())],
                    },
                    Component::Mosfet {
                        name: "n".into(),
                        nd: "out".into(),
                        ng: "in".into(),
                        ns: "vss".into(),
                        nb: "vss".into(),
                        model: "nmos".into(),
                        params: vec![("W".into(), "{wn}".into()), ("L".into(), "180n".into())],
                    },
                ],
                instances: vec![],
                models: vec![],
                raw_spice: vec![],
                includes: vec![],
                libs: vec![],
                osdi_loads: vec![],
                verilog_blocks: vec![],
            },
        ],
        model_libraries: vec![],
    }
}

fn waveform_ir() -> CircuitIR {
    CircuitIR { raw_dialect: None, globals: vec![],
        top: Subcircuit {
            name: "Waveform Test".into(),
            ports: vec![],
            parameters: vec![],
            components: vec![
                Component::VoltageSource {
                    name: "sin".into(),
                    np: "vsin".into(),
                    nm: "0".into(),
                    value: IrValue::Numeric { value: 0.0 },
                    ac_magnitude: None,
                    ac_phase: None,
                    waveform: Some(IrWaveform::Sin {
                        offset: 1.0,
                        amplitude: 0.5,
                        frequency: 1e6,
                        delay: 0.0,
                        damping: 0.0,
                        phase: 0.0,
                    }),
                },
                Component::VoltageSource {
                    name: "pul".into(),
                    np: "vpul".into(),
                    nm: "0".into(),
                    value: IrValue::Numeric { value: 0.0 },
                    ac_magnitude: None,
                    ac_phase: None,
                    waveform: Some(IrWaveform::Pulse {
                        initial: 0.0,
                        pulsed: 1.8,
                        delay: 1e-9,
                        rise_time: 100e-12,
                        fall_time: 100e-12,
                        pulse_width: 5e-7,
                        period: 1e-6,
                    }),
                },
                Component::VoltageSource {
                    name: "pw".into(),
                    np: "vpwl".into(),
                    nm: "0".into(),
                    value: IrValue::Numeric { value: 0.0 },
                    ac_magnitude: None,
                    ac_phase: None,
                    waveform: Some(IrWaveform::Pwl {
                        values: vec![(0.0, 0.0), (1e-6, 1.8), (2e-6, 0.0)],
                    }),
                },
            ],
            instances: vec![],
            models: vec![],
            raw_spice: vec![],
            includes: vec![],
            libs: vec![],
            osdi_loads: vec![],
            verilog_blocks: vec![],
        },
        testbench: Some(Testbench { prints: vec![],
            dut: "Waveform Test".into(),
            stimulus: vec![],
            analyses: vec![
                Analysis::Transient { step: 1e-9, stop: 10e-6, start: None, max_step: None, uic: false },
            ],
            options: SimOptions::default(),
            saves: vec![],
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

// ── Tests ──

#[test]
fn test_spice3_ngspice_resistor_divider() {
    let ir = resistor_divider_ir();
    let cg = Dialect::Ngspice;
    let netlist = netlist(&ir, cg).unwrap();

    assert!(netlist.contains("* Voltage Divider"));
    assert!(netlist.contains("Vin input 0 10"));
    assert!(netlist.contains("R1 input output 10k"));
    assert!(netlist.contains("R2 output 0 10k"));
    assert!(netlist.contains(".op"));
    assert!(netlist.ends_with(".end"));
}

#[test]
fn test_spectre_resistor_divider() {
    let ir = resistor_divider_ir();
    let cg = Dialect::Spectre;
    let netlist = netlist(&ir, cg).unwrap();

    assert!(netlist.contains("// Voltage Divider"));
    assert!(netlist.contains("vin (input 0) vsource dc=10"));
    assert!(netlist.contains("r1 (input output) resistor r=10k"));
    assert!(netlist.contains("r2 (output 0) resistor r=10k"));
    assert!(netlist.contains("op1 dc"));
    assert!(!netlist.contains(".end"));
}

#[test]
fn test_spice3_mosfet_circuit() {
    let ir = mosfet_inverter_ir();
    let cg = Dialect::Ngspice;
    let netlist = netlist(&ir, cg).unwrap();

    assert!(netlist.contains("Mp1 out in vdd vdd pmos_3p3 W=2u L=180n"));
    assert!(netlist.contains("Mn1 out in 0 0 nmos_3p3 W=1u L=180n"));
    assert!(netlist.contains(".model nmos_3p3 NMOS(VTO=0.7)"));
    assert!(netlist.contains(".model pmos_3p3 PMOS(VTO=-0.7)"));
    assert!(netlist.contains(".save V(out)"));
    assert!(netlist.contains(".temp 27"));
    assert!(netlist.contains(".dc Vin"));
}

#[test]
fn test_spectre_mosfet_circuit() {
    let ir = mosfet_inverter_ir();
    let cg = Dialect::Spectre;
    let netlist = netlist(&ir, cg).unwrap();

    assert!(netlist.contains("mp1 (out in vdd vdd) pmos_3p3 W=2u L=180n"), "mosfet: {}", netlist);
    assert!(netlist.contains("mn1 (out in 0 0) nmos_3p3 W=1u L=180n"), "mosfet n: {}", netlist);
    // SPICE NMOS level 1 is Spectre's mos1 with type=n; keys lowercased.
    assert!(netlist.contains("model nmos_3p3 mos1 type=n vto=0.7"), "model: {}", netlist);
    assert!(netlist.contains("dc1 dc"), "dc: {}", netlist);
    assert!(netlist.contains("mytemp options temp=27"), "temp: {}", netlist);
}

#[test]
fn test_spice3_subcircuit_instances() {
    let ir = subcircuit_ir();
    let cg = Dialect::Ngspice;
    let netlist = netlist(&ir, cg).unwrap();

    assert!(netlist.contains(".subckt inverter in out vdd vss PARAMS: wp=2u wn=1u"), "subckt: {}", netlist);
    assert!(netlist.contains("Mp out in vdd vdd pmos W={wp} L=180n"), "Mp: {}", netlist);
    assert!(netlist.contains("Mn out in vss vss nmos W={wn} L=180n"), "Mn: {}", netlist);
    assert!(netlist.contains(".ends inverter"), "ends: {}", netlist);
    assert!(netlist.contains("Xinv1 a b vdd 0 inverter"), "X1: {}", netlist);
    assert!(netlist.contains("Xinv2 b c vdd 0 inverter wp=4u"), "X2: {}", netlist);
}

#[test]
fn test_spice3_waveforms() {
    let ir = waveform_ir();
    let cg = Dialect::Ngspice;
    let netlist = netlist(&ir, cg).unwrap();

    assert!(netlist.contains("SIN(1 0.5 1000000"), "sin: {}", netlist);
    assert!(netlist.contains("PULSE(0 1.8"), "pulse: {}", netlist);
    assert!(netlist.contains("PWL(0 0"), "pwl: {}", netlist);
    assert!(netlist.contains(".tran 1n"), "tran: {}", netlist);
}

#[test]
fn test_spectre_waveforms() {
    let ir = waveform_ir();
    let cg = Dialect::Spectre;
    let netlist = netlist(&ir, cg).unwrap();

    assert!(netlist.contains("type=sine sinedc=1 ampl=500m freq=1M"), "sin: {}", netlist);
    assert!(netlist.contains("type=pulse val0=0 val1=1.8"), "pulse: {}", netlist);
    assert!(netlist.contains("type=pwl wave=[0 0"), "pwl: {}", netlist);
    assert!(netlist.contains("tran1 tran step=1n"), "tran: {}", netlist);
}

#[test]
fn test_spice3_all_analysis_types() {
    let cg = Dialect::Ngspice;

    assert_eq!(analysis(cg, &Analysis::Op).unwrap(), ".op");

    let dc = Analysis::Dc { sweeps: vec![
        DcSweep { source: "V1".into(), start: 0.0, stop: 5.0, step: 0.1 },
    ]};
    assert!(analysis(cg, &dc).unwrap().starts_with(".dc V1"));

    let ac = Analysis::Ac { variation: "dec".into(), points: 100, start: 1.0, stop: 1e9 };
    assert!(analysis(cg, &ac).unwrap().starts_with(".ac dec 100"));

    let tran = Analysis::Transient { step: 1e-9, stop: 1e-6, start: None, max_step: None, uic: false };
    assert!(analysis(cg, &tran).unwrap().starts_with(".tran 1n"));

    let tran_uic = Analysis::Transient { step: 1e-9, stop: 1e-6, start: None, max_step: None, uic: true };
    assert!(analysis(cg, &tran_uic).unwrap().contains("UIC"));

    let noise = Analysis::Noise {
        output: "out".into(), reference: "0".into(), source: "V1".into(),
        variation: "dec".into(), points: 10, start: 1.0, stop: 1e6,
        points_per_summary: None,
    };
    assert!(analysis(cg, &noise).unwrap().starts_with(".noise V(out) V1"));

    let tf = Analysis::Tf { output: "V(out)".into(), source: "Vin".into() };
    assert_eq!(analysis(cg, &tf).unwrap(), ".tf V(out) Vin");

    let sens = Analysis::Sensitivity { output: "V(out)".into(), ac: None };
    assert_eq!(analysis(cg, &sens).unwrap(), ".sens V(out)");

    let pz = Analysis::PoleZero {
        node1: "1".into(), node2: "0".into(), node3: "3".into(), node4: "0".into(),
        tf_type: "vol".into(), pz_type: "pz".into(),
    };
    assert_eq!(analysis(cg, &pz).unwrap(), ".pz 1 0 3 0 vol pz");

    let four = Analysis::Fourier {
        fundamental: 1e3, outputs: vec!["V(out)".into()], num_harmonics: Some(10),
    };
    assert!(analysis(cg, &four).unwrap().contains(".four 1k V(out)"));
}

#[test]
fn test_spectre_options() {
    let opts = SimOptions {
        portable: vec![
            ("reltol".into(), "1e-4".into()),
            ("max_iterations".into(), "150".into()),
        ],
        backend_specific: BTreeMap::new(),
    };

    let mut ir = resistor_divider_ir();
    ir.testbench.as_mut().unwrap().options = opts;
    let n = netlist(&ir, Dialect::Spectre).unwrap();
    let s = n.lines().find(|l| l.starts_with("myopts")).unwrap();
    assert!(s.contains("myopts options"));
    assert!(s.contains("reltol=1e-4"));
    assert!(s.contains("dcmaxiters=150"));
}
