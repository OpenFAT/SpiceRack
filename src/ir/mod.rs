//! `CircuitIR`: the backend-neutral circuit every reader (`import`, the
//! Python API, JSON) writes and every writer (`codegen`) reads. This file
//! holds the dialect names and re-exports the tables, one file each:
//! `circuit` (definitions, ports, instances, models), `component` (element
//! cards), `waveform` (values and source waveforms), `testbench` (analyses,
//! options, saves) and `features` (what a deck needs from a simulator).
//! JSON is `schema/circuit-ir.schema.json`, checked by `schema_check`.
//!
//! Cites: none here (each file cites its own manuals).
//!
//! Contents:
//! - `pub enum Dialect`, `Dialect::ALL: [Dialect; 9]`,
//!   `Dialect::ALIASES: [(&str, Dialect); 3]`, `Dialect::name(self) -> &'static str`,
//!   `Dialect::family(self) -> Dialect`, `Dialect::from_backend_name(&str) -> Option<Dialect>`,
//!   `impl FromStr for Dialect`
//! - `pub use circuit::*`, `component::*`, `features::*`, `testbench::*`, `waveform::*`

use serde::{Deserialize, Serialize};

mod circuit;
mod component;
mod features;
mod testbench;
mod waveform;

pub use circuit::*;
pub use component::*;
pub use features::*;
pub use testbench::*;
pub use waveform::*;

/// The netlist language a simulator reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
    Ngspice,
    Ltspice,
    Spectre,
    Vacask,
    /// ngspice text with Verilog-A loaded as `.hdl` instead of `pre_osdi`.
    Espice,
    /// HSPICE: `'expr'` quoting, `E`/`G` `vol=`/`cur=` behavioural sources,
    /// `.hdl` Verilog-A, and the card forms in ESPice's
    /// `tests/frontend/grammar/hspice.peg` and `tests/conformance/tests/netlist/hspice`.
    Hspice,
    /// Xyce: the xschem layout only (`codegen::xschem`).
    Xyce,
    /// CDL for LVS: the xschem layout only (`codegen::xschem`).
    Cdl,
    /// Structural Verilog: the xschem layout only (`codegen::xschem`).
    Verilog,
}

impl Dialect {
    pub const ALL: [Dialect; 9] = [
        Dialect::Ngspice,
        Dialect::Ltspice,
        Dialect::Spectre,
        Dialect::Vacask,
        Dialect::Espice,
        Dialect::Hspice,
        Dialect::Xyce,
        Dialect::Cdl,
        Dialect::Verilog,
    ];

    /// The dialect's name (`ngspice`, ...), as `FromStr` reads it.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Ngspice => "ngspice",
            Dialect::Ltspice => "ltspice",
            Dialect::Spectre => "spectre",
            Dialect::Vacask => "vacask",
            Dialect::Espice => "espice",
            Dialect::Hspice => "hspice",
            Dialect::Xyce => "xyce",
            Dialect::Cdl => "cdl",
            Dialect::Verilog => "verilog",
        }
    }

    /// The dialect whose text this one reads: ESPice reads ngspice decks.
    pub fn family(self) -> Dialect {
        if self == Dialect::Espice {
            Dialect::Ngspice
        } else {
            self
        }
    }

    /// Backend names kept as aliases of a dialect's own name, so scripts
    /// that name the old shared-library backends still run.
    pub const ALIASES: [(&'static str, Dialect); 3] = [
        ("ngspice-subprocess", Dialect::Ngspice),
        ("ngspice-shared", Dialect::Ngspice),
        ("vacask-shared", Dialect::Vacask),
    ];

    /// The dialect a backend name selects: its [`Self::name`] or one of
    /// [`Self::ALIASES`], spelled exactly (unlike `FromStr`, case matters).
    pub fn from_backend_name(s: &str) -> Option<Dialect> {
        let alias = Self::ALIASES.iter().find(|a| a.0 == s).map(|a| a.1);
        alias.or_else(|| Self::ALL.into_iter().find(|d| d.name() == s))
    }
}

impl std::str::FromStr for Dialect {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Dialect::ALL
            .into_iter()
            .find(|d| d.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown dialect '{s}'"))
    }
}

#[cfg(test)]
mod schema_check;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// A deck whose top is `top`, with no testbench.
    fn deck(top: Subcircuit) -> CircuitIR {
        CircuitIR {
            top,
            ..Default::default()
        }
    }

    fn resistor() -> Component {
        Component::Resistor {
            name: "1".into(),
            n1: "out".into(),
            n2: "0".into(),
            value: IrValue::Numeric { value: 1000.0 },
            model: None,
            params: vec![],
        }
    }

    /// The JSON `"type"` tag of `x` is its variant's name.
    fn assert_tagged<T: Serialize + std::fmt::Debug>(x: &T) {
        let name = format!("{x:?}");
        assert_eq!(
            serde_json::to_value(x).unwrap()["type"],
            name.split([' ', '{']).next().unwrap(),
            "{name}"
        );
    }

    #[test]
    fn test_compute_features_xspice() {
        let xspice = Component::Xspice {
            name: "1".into(),
            connections: vec!["in".into(), "out".into()],
            model: "d_and".into(),
        };
        let features = deck(Subcircuit {
            components: vec![xspice, resistor()],
            ..Default::default()
        })
        .compute_features();
        assert!(features.has(feature::XSPICE));
        assert!(!features.has(feature::OSDI));
        assert!(!features.has(feature::MEASURES));
        assert!(!features.has(feature::LAPLACE));
        assert_eq!(features.element_count, 2);
    }

    #[test]
    fn test_compute_features_osdi_and_laplace() {
        let b = Component::BehavioralVoltage {
            name: "1".into(),
            np: "out".into(),
            nm: "0".into(),
            expression: "laplace(V(in), {1/(1+s*1e-6)})".into(),
        };
        let features = deck(Subcircuit {
            components: vec![b],
            osdi_loads: vec!["model.osdi".into()],
            ..Default::default()
        })
        .compute_features();
        assert!(!features.has(feature::XSPICE));
        assert!(features.has(feature::OSDI));
        assert!(features.has(feature::LAPLACE));
        assert_eq!(features.element_count, 1);
    }

    #[test]
    fn test_compute_features_control_blocks() {
        let raw_spice = vec![".control".into(), "run".into(), ".endc".into()];
        assert!(
            deck(Subcircuit {
                raw_spice,
                ..Default::default()
            })
            .compute_features()
            .has(feature::CONTROL_BLOCKS)
        );
    }

    #[test]
    fn test_compute_features_measures_and_step() {
        let step = StepParam {
            param: "R1".into(),
            start: 100.0,
            stop: 10000.0,
            step: 100.0,
            sweep_type: None,
        };
        let tb = Testbench {
            measures: vec!["tran v_max MAX V(out)".into()],
            step_params: vec![step],
            ..Default::default()
        };
        let features = CircuitIR {
            testbench: Some(tb),
            ..Default::default()
        }
        .compute_features();
        assert!(features.has(feature::MEASURES));
        assert!(features.has(feature::STEP_PARAMS));
    }

    #[cfg(feature = "backend")]
    #[test]
    fn test_check_backend_osdi_against_ltspice() {
        let ir = deck(Subcircuit {
            osdi_loads: vec!["model.osdi".into()],
            ..Default::default()
        });
        let issues = ir.check_backend("ltspice");
        assert!(
            issues
                .iter()
                .any(|i| i.severity == IssueSeverity::Error && i.message.contains("OSDI"))
        );
        // Spectre supports OSDI
        assert!(ir.check_backend("spectre").is_empty());
    }

    /// For every feature bit, a deck that needs it: `check_backend` and the
    /// `select` predicate (`backend::runs`) agree on every runnable dialect.
    #[cfg(feature = "backend")]
    #[test]
    fn check_backend_and_select_agree_on_every_feature() {
        let bench = |tb: Testbench| CircuitIR {
            testbench: Some(tb),
            ..Default::default()
        };
        let laplace = Component::BehavioralVoltage {
            name: "1".into(),
            np: "out".into(),
            nm: "0".into(),
            expression: "laplace(V(in), {1/(1+s*1e-6)})".into(),
        };
        let cosim = VerilogBlock {
            source: "m.v".into(),
            mode: VerilogMode::Simulate,
            instance_name: "u1".into(),
            connections: BTreeMap::new(),
            pdk: None,
            liberty: None,
            spice_models: None,
        };
        let decks = [
            (
                feature::XSPICE,
                deck(Subcircuit {
                    components: vec![Component::Xspice {
                        name: "1".into(),
                        connections: vec!["a".into()],
                        model: "m".into(),
                    }],
                    ..Default::default()
                }),
            ),
            (
                feature::OSDI,
                deck(Subcircuit {
                    osdi_loads: vec!["m.osdi".into()],
                    ..Default::default()
                }),
            ),
            (
                feature::MEASURES,
                bench(Testbench {
                    measures: vec!["tran x max v(a)".into()],
                    ..Default::default()
                }),
            ),
            (
                feature::STEP_PARAMS,
                bench(Testbench {
                    step_params: vec![StepParam {
                        param: "r".into(),
                        start: 1.0,
                        stop: 2.0,
                        step: 1.0,
                        sweep_type: None,
                    }],
                    ..Default::default()
                }),
            ),
            (
                feature::CONTROL_BLOCKS,
                deck(Subcircuit {
                    raw_spice: vec![".control".into(), ".endc".into()],
                    ..Default::default()
                }),
            ),
            (
                feature::LAPLACE,
                deck(Subcircuit {
                    components: vec![laplace],
                    ..Default::default()
                }),
            ),
            (
                feature::VERILOG_COSIM,
                deck(Subcircuit {
                    verilog_blocks: vec![cosim],
                    ..Default::default()
                }),
            ),
        ];
        assert_eq!(
            decks.iter().fold(0, |m, (f, _)| m | f),
            feature::ALL,
            "one deck per feature"
        );
        for (f, ir) in &decks {
            assert_eq!(ir.compute_features().needs, *f, "deck for bit {f:#04b}");
            for d in crate::backend::RUNNABLE {
                let issues = ir.check_backend(d.name());
                assert_eq!(
                    issues.is_empty(),
                    crate::backend::runs(d, *f),
                    "{} on bit {f:#04b}: {issues:?}",
                    d.name()
                );
            }
        }
    }

    #[cfg(feature = "backend")]
    #[test]
    fn test_check_backend_no_issues_for_clean_circuit() {
        let ir = deck(Subcircuit {
            components: vec![resistor()],
            ..Default::default()
        });
        for backend in &["ngspice", "ltspice", "vacask", "spectre"] {
            assert!(
                ir.check_backend(backend).is_empty(),
                "Expected no issues for {}",
                backend
            );
        }
    }

    // ── JSON serialization tests ──

    /// `ir` as JSON breaks nothing in `schema/circuit-ir.schema.json`.
    fn assert_schema(ir: &CircuitIR) {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schema/circuit-ir.schema.json")).unwrap();
        let errs = super::schema_check::errors(&schema, &serde_json::to_value(ir).unwrap());
        assert!(
            errs.is_empty(),
            "{} schema errors:\n{}",
            errs.len(),
            errs.join("\n")
        );
    }

    /// Every field an IR can hold, set (a PWL's repeat/delay as numbers and
    /// expressions, every port direction and Verilog form), the shipped
    /// example, and what the importers make of a deck per dialect: the JSON
    /// the schema promises Schemify. Regression: the schema had no R/C/L
    /// `model`, no `verilog_blocks` and an `IrNum` that was only a number.
    #[test]
    fn every_ir_field_and_imported_deck_matches_the_schema() {
        let s = |x: &str| x.to_string();
        let kv = || vec![(s("k"), s("v"))];
        let x = |e: &str| IrNum::Expr(e.into());
        let pwl = |repeat, delay| IrWaveform::Pwl {
            values: vec![(0.0.into(), x("v0")), (x("t1"), 1.0.into())],
            repeat,
            delay,
        };
        let src = |name: &str, waveform| Component::VoltageSource {
            name: s(name),
            np: s("a"),
            nm: s("0"),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0),
            ac_phase: Some(90.0),
            waveform: Some(waveform),
            port: Some(RfPort { num: 1, z0: 50.0 }),
        };
        let rcl = |value: IrValue| {
            [
                Component::Resistor {
                    name: s("1"),
                    n1: s("a"),
                    n2: s("b"),
                    value: value.clone(),
                    model: Some(s("rm")),
                    params: kv(),
                },
                Component::Capacitor {
                    name: s("1"),
                    n1: s("a"),
                    n2: s("b"),
                    value: value.clone(),
                    model: Some(s("cm")),
                    params: kv(),
                },
                Component::Inductor {
                    name: s("1"),
                    n1: s("a"),
                    n2: s("b"),
                    value,
                    model: Some(s("lm")),
                    params: kv(),
                },
            ]
        };
        let connections = [
            (s("a"), VerilogConnection::Single(s("n1"))),
            (s("b"), VerilogConnection::Bus(vec![s("d0"), s("d1")])),
        ]
        .into();
        let verilog = |mode, opt: Option<String>| VerilogBlock {
            source: s("module m; endmodule"),
            mode,
            instance_name: s("u1"),
            connections: BTreeMap::clone(&connections),
            pdk: opt.clone(),
            liberty: opt.clone(),
            spice_models: opt,
        };
        let full = Subcircuit {
            name: s("full"),
            ports: [
                PortDirection::InOut,
                PortDirection::Input,
                PortDirection::Output,
            ]
            .into_iter()
            .map(|direction| Port {
                name: s("p"),
                direction,
            })
            .collect(),
            parameters: vec![
                ParamDef {
                    name: s("w"),
                    default: Some(s("1u")),
                },
                ParamDef {
                    name: s("l"),
                    default: None,
                },
            ],
            components: rcl(IrValue::Raw {
                text: String::new(),
            })
            .into_iter()
            .chain([
                src("1", pwl(Some(0.0.into()), Some(x("td")))),
                src("2", pwl(Some(x("r")), Some(1e-3.into()))),
            ])
            .collect(),
            instances: vec![Instance {
                name: s("x1"),
                subcircuit: s("full"),
                port_mapping: vec![s("a")],
                parameters: kv(),
                at: Some(0),
            }],
            models: vec![ModelDef {
                name: s("rm"),
                kind: s("r"),
                parameters: kv(),
            }],
            raw_spice: vec![s(".probe v(a)")],
            includes: vec![s("a.inc")],
            libs: vec![(s("a.lib"), s("tt"))],
            osdi_loads: vec![s("a.osdi")],
            verilog_blocks: vec![
                verilog(VerilogMode::Simulate, None),
                verilog(VerilogMode::Synthesize, Some(s("x"))),
            ],
        };
        let tb = Testbench {
            stimulus: rcl(IrValue::Expression { expr: s("r") }).into(),
            analyses: vec![Analysis::Op],
            options: SimOptions {
                portable: kv(),
                backend_specific: [(s("xyce"), kv())].into(),
            },
            saves: vec![s("v(a)")],
            measures: vec![s("tran m max v(a)")],
            temperature: Some(27.0),
            nominal_temperature: Some(25.0),
            initial_conditions: vec![(s("a"), 1.0)],
            node_sets: vec![(s("a"), 0.5)],
            step_params: vec![
                StepParam {
                    param: s("r"),
                    start: 1.0,
                    stop: 2.0,
                    step: 1.0,
                    sweep_type: Some(Sweep::Lin),
                },
                StepParam {
                    param: s("c"),
                    start: 1.0,
                    stop: 2.0,
                    step: 1.0,
                    sweep_type: None,
                },
            ],
            extra_lines: vec![s(".option x")],
            prints: vec![PrintCard {
                analysis: s("tran"),
                outputs: vec![s("v(a)")],
            }],
            ngbehavior: Some(s("hsa")),
        };
        let library = ModelLibrary {
            name: s("sky"),
            path: s("sky.lib"),
            corner: Some(s("tt")),
            backend_paths: [(s("vacask"), s("sky.inc"))].into(),
            setup_includes: vec![s("a.inc")],
        };
        let ir = CircuitIR {
            top: full.clone(),
            testbench: Some(tb),
            subcircuit_defs: vec![full, Subcircuit::default()],
            model_libraries: vec![library],
            raw_dialect: Some(Dialect::Ngspice),
            globals: vec![s("vdd")],
            functions: vec![FuncDef {
                name: s("f"),
                args: vec![s("x")],
                body: s("x*2"),
            }],
        };
        assert_schema(&ir);
        assert_schema(&CircuitIR {
            testbench: None,
            ..ir
        });
        assert_schema(
            &serde_json::from_str(include_str!(
                "../../schema/examples/voltage_divider.circuit.json"
            ))
            .unwrap(),
        );
        for (d, deck) in [
            (
                Dialect::Ngspice,
                "t\n.param w=1u\n.func f(x) {x*2}\n.global vdd\n.model rm r\nR1 a b rm l=1u\nV1 a 0 pwl(0 0 1m {w}) r=0 td=1m\n.probe v(a)\n.tran 1u 1m\n.end\n",
            ),
            (
                Dialect::Spectre,
                "// t\nsimulator lang=spectre\nv1 (a 0) vsource type=pwl wave=[0 0 1m 1 2m 0] pwlperiod=2m\nr1 (a 0) resistor r=1k\ntr tran step=1u stop=1m\n",
            ),
            (
                Dialect::Vacask,
                "t\nload \"spice/resistor.osdi\"\nmodel r sp_resistor\nr1 (a 0) r r=1k\ncontrol\n  analysis op1 op\nendc\n",
            ),
        ] {
            assert_schema(&crate::import::parse(deck, "t", d).unwrap().0);
        }
    }

    #[test]
    fn test_load_example_voltage_divider() {
        let json = include_str!("../../schema/examples/voltage_divider.circuit.json");
        let ir: CircuitIR = serde_json::from_str(json).unwrap();
        assert_eq!(ir.top.name, "Voltage Divider");
        assert_eq!(ir.top.components.len(), 3);
        assert!(ir.testbench.is_some());
        let tb = ir.testbench.as_ref().unwrap();
        assert_eq!(tb.analyses.len(), 1);
        match &tb.analyses[0] {
            Analysis::Op => {}
            other => panic!("Expected Op analysis, got {:?}", other),
        }
    }

    /// The flat `variation`/`points`/`start`/`stop` and `pss_*` fields that
    /// `FreqSweep` and `PssBase` now hold. Every line is the JSON the string
    /// `variation` code wrote (0.1.2, taken from the test suite's own decks);
    /// it loads, and writes back byte for byte.
    #[test]
    fn sweep_json_from_0_1_2_round_trips_byte_for_byte() {
        let old = [
            r#"{"type":"Ac","variation":"dec","points":50,"start":1.0,"stop":10000000000.0}"#,
            r#"{"type":"Distortion","variation":"dec","points":10,"start":1.0,"stop":1000.0,"f2overf1":null}"#,
            r#"{"type":"Noise","output":"p2","reference":"0","source":"Vp1","variation":"dec","points":50,"start":1000000.0,"stop":10000000000.0,"points_per_summary":null}"#,
            r#"{"type":"SPar","variation":"dec","points":20,"start":1000000.0,"stop":1000000000.0}"#,
            r#"{"type":"Sensitivity","output":"out","ac":{"variation":"dec","points":10,"start":1.0,"stop":1000000.0}}"#,
            r#"{"type":"SpectrePac","pss_fundamental":1000000000.0,"pss_stabilization":1e-7,"pss_harmonics":10,"variation":"dec","points":100,"start":1.0,"stop":1000000000.0,"sweep_type":"relative"}"#,
            r#"{"type":"SpectrePnoise","pss_fundamental":1000000000.0,"pss_stabilization":1e-7,"pss_harmonics":10,"output":"out","reference":"0","variation":"dec","points":10,"start":1.0,"stop":1000000.0}"#,
            r#"{"type":"SpectrePstb","pss_fundamental":1000000000.0,"pss_stabilization":1e-7,"pss_harmonics":10,"probe":"IPRB0","variation":"dec","points":10,"start":1.0,"stop":1000000.0}"#,
            r#"{"type":"SpectrePxf","pss_fundamental":1000000000.0,"pss_stabilization":1e-7,"pss_harmonics":10,"output":"out","source":"Vin","variation":"dec","points":10,"start":1.0,"stop":1000000.0}"#,
            r#"{"type":"Stability","probe":"IPRB0","variation":"dec","points":20,"start":1.0,"stop":1000000000.0}"#,
            r#"{"type":"Noise","output":"o","reference":"0","source":"V1","variation":"oct","points":3,"start":1.0,"stop":8.0,"points_per_summary":2}"#,
            r#"{"type":"Ac","variation":"lin","points":2,"start":1000.0,"stop":2000.0}"#,
            r#"{"type":"Ac","variation":"log","points":7,"start":1.0,"stop":1000.0}"#,
        ];
        for s in old {
            let a: Analysis = serde_json::from_str(s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(serde_json::to_string(&a).unwrap(), s);
        }
    }

    /// A PWL written before `repeat`/`delay` existed loads with neither and
    /// writes back byte for byte; a set one is written after the values.
    #[test]
    fn pwl_json_without_repeat_or_delay_round_trips_byte_for_byte() {
        let old = r#"{"type":"Pwl","values":[[0.0,0.0],[0.001,"vdd"]]}"#;
        let w: IrWaveform = serde_json::from_str(old).unwrap();
        assert_eq!(
            w,
            IrWaveform::Pwl {
                values: vec![
                    (0.0.into(), 0.0.into()),
                    (0.001.into(), IrNum::Expr("vdd".into()))
                ],
                repeat: None,
                delay: None
            }
        );
        assert_eq!(serde_json::to_string(&w).unwrap(), old);
        let IrWaveform::Pwl { values, .. } = w else {
            unreachable!()
        };
        let set = IrWaveform::Pwl {
            values,
            repeat: Some(0.0.into()),
            delay: Some(IrNum::Expr("td".into())),
        };
        assert_eq!(
            serde_json::to_string(&set).unwrap(),
            r#"{"type":"Pwl","values":[[0.0,0.0],[0.001,"vdd"]],"repeat":0.0,"delay":"td"}"#
        );
    }

    /// The keyword fields that were strings (`.pz` types, a `.step` sweep,
    /// a Spectre child analysis) load from the JSON the string code wrote
    /// (0.1.2, the test suite's own values) and write it back byte for byte;
    /// a keyword no simulator has fails the load.
    #[test]
    fn keyword_json_from_0_1_2_round_trips_byte_for_byte() {
        let old = [
            r#"{"type":"PoleZero","node1":"in","node2":"0","node3":"out","node4":"0","tf_type":"vol","pz_type":"pz"}"#,
            r#"{"type":"PoleZero","node1":"in","node2":"0","node3":"out","node4":"0","tf_type":"cur","pz_type":"pol"}"#,
            r#"{"type":"PoleZero","node1":"in","node2":"0","node3":"out","node4":"0","tf_type":"vol","pz_type":"zer"}"#,
            r#"{"type":"SpectreSweep","param":"R1","start":1000.0,"stop":10000.0,"step":1000.0,"inner":"dc1","inner_type":"dc"}"#,
            r#"{"type":"SpectreSweep","param":"rload","start":1000.0,"stop":10000.0,"step":1000.0,"inner":"ac1","inner_type":"ac"}"#,
            r#"{"type":"SpectreMonteCarlo","iterations":100,"inner":"tran1","inner_type":"tran","seed":42}"#,
        ];
        for s in old {
            let a: Analysis = serde_json::from_str(s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(serde_json::to_string(&a).unwrap(), s);
        }
        let pz: Analysis = serde_json::from_str(old[1]).unwrap();
        assert!(matches!(
            pz,
            Analysis::PoleZero {
                tf_type: PzTransfer::Cur,
                pz_type: PzKind::Pol,
                ..
            }
        ));
        let mc: Analysis = serde_json::from_str(old[5]).unwrap();
        assert!(matches!(
            mc,
            Analysis::SpectreMonteCarlo {
                inner_type: SpectreAnalysis::Tran,
                ..
            }
        ));
        for s in [
            r#"{"param":"R1","start":100.0,"stop":10000.0,"step":100.0,"sweep_type":"dec"}"#,
            r#"{"param":"R1","start":100.0,"stop":10000.0,"step":100.0,"sweep_type":null}"#,
        ] {
            let sp: StepParam = serde_json::from_str(s).unwrap();
            assert_eq!(serde_json::to_string(&sp).unwrap(), s);
        }
        let err = serde_json::from_str::<Analysis>(&old[0].replace("\"vol\"", "\"volt\""))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("unknown transfer type 'volt' (expected vol or cur)"),
            "{err}"
        );
        let err = serde_json::from_str::<Analysis>(&old[5].replace("\"tran\"", "\"op\""))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown Spectre analysis 'op'"), "{err}");
    }

    /// A `.pss` and a V source written before the shooting controls and RF
    /// ports existed load with them unset and write back byte for byte.
    #[test]
    fn pss_and_source_json_without_the_new_fields_round_trip_byte_for_byte() {
        let pss = r#"{"type":"Pss","fundamental":1000000000.0,"stabilization":1e-7,"observe_node":"out","points_per_period":128,"harmonics":10}"#;
        let a: Analysis = serde_json::from_str(pss).unwrap();
        assert!(matches!(
            a,
            Analysis::Pss {
                sc_iter: None,
                steady_coeff: None,
                uic: false,
                ..
            }
        ));
        assert_eq!(serde_json::to_string(&a).unwrap(), pss);
        let v = r#"{"type":"VoltageSource","name":"1","np":"in","nm":"0","value":{"type":"Numeric","value":0.0},"ac_magnitude":1.0,"ac_phase":null,"waveform":null}"#;
        let c: Component = serde_json::from_str(v).unwrap();
        assert!(matches!(c, Component::VoltageSource { port: None, .. }));
        assert_eq!(serde_json::to_string(&c).unwrap(), v);
        let Component::VoltageSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform,
            ..
        } = c
        else {
            unreachable!()
        };
        let rf = Component::VoltageSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform,
            port: Some(RfPort { num: 1, z0: 50.0 }),
        };
        assert!(
            serde_json::to_string(&rf)
                .unwrap()
                .ends_with(r#""waveform":null,"port":{"num":1,"z0":50.0}}"#)
        );
    }

    /// TRRANDOM's type is its deck number in JSON; one ngspice does not
    /// define fails the load.
    #[test]
    fn trrandom_kind_is_its_type_number() {
        let j = r#"{"type":"TrRandom","kind":2,"ts":1e-6,"td":0.0,"p1":1.0,"p2":0.0}"#;
        let w: IrWaveform = serde_json::from_str(j).unwrap();
        assert!(matches!(
            w,
            IrWaveform::TrRandom {
                kind: RandomKind::Gaussian,
                ..
            }
        ));
        assert_eq!(serde_json::to_string(&w).unwrap(), j);
        let err = serde_json::from_str::<IrWaveform>(&j.replace(":2,", ":7,"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown TRRANDOM type 7"), "{err}");
    }

    /// The sweep name is parsed where JSON is read: any case loads, and a
    /// name no simulator has fails the load instead of reaching a deck.
    #[test]
    fn sweep_names_parse_once_at_load() {
        let ac = |v: &str| {
            serde_json::from_str::<Analysis>(&format!(
                r#"{{"type":"Ac","variation":"{v}","points":5,"start":1.0,"stop":10.0}}"#
            ))
        };
        assert_eq!(
            ac("DEC").unwrap(),
            Analysis::Ac {
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 5,
                    start: 1.0,
                    stop: 10.0
                }
            }
        );
        let err = ac("decade").unwrap_err().to_string();
        assert!(err.contains("unknown sweep 'decade'"), "{err}");
    }

    /// A testbench written before 0.1.3 names its DUT in `dut`; the field is
    /// gone, and such JSON still loads (no `deny_unknown_fields`).
    #[test]
    fn testbench_json_with_dut_still_loads() {
        let old = r#"{"dut":"top","stimulus":[],"analyses":[{"type":"Op"}],"options":{"portable":[],"backend_specific":{}},
            "saves":[],"measures":[],"temperature":null,"nominal_temperature":null,"initial_conditions":[],
            "node_sets":[],"step_params":[],"extra_lines":[]}"#;
        let tb: Testbench = serde_json::from_str(old).unwrap();
        assert_eq!(tb.analyses, vec![Analysis::Op]);
        assert!(!serde_json::to_string(&tb).unwrap().contains("dut"));
    }

    #[test]
    fn test_all_component_variants_roundtrip() {
        // Build an IR containing every Component variant to ensure full roundtrip coverage
        let all_components = vec![
            Component::Resistor {
                name: "1".into(),
                n1: "a".into(),
                n2: "b".into(),
                value: IrValue::Numeric { value: 1e3 },
                model: None,
                params: vec![("tc1".into(), "0.001".into())],
            },
            Component::Capacitor {
                name: "1".into(),
                n1: "a".into(),
                n2: "b".into(),
                value: IrValue::Expression {
                    expr: "cap_val".into(),
                },
                model: None,
                params: vec![],
            },
            Component::Inductor {
                name: "1".into(),
                n1: "a".into(),
                n2: "b".into(),
                value: IrValue::Raw { text: "10u".into() },
                model: None,
                params: vec![],
            },
            Component::MutualInductor {
                name: "1".into(),
                inductor1: "L1".into(),
                inductor2: "L2".into(),
                coupling: 0.95,
            },
            Component::VoltageSource {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                value: IrValue::Numeric { value: 5.0 },
                ac_magnitude: None,
                ac_phase: None,
                waveform: None,
                port: None,
            },
            Component::CurrentSource {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                value: IrValue::Numeric { value: 1e-3 },
                ac_magnitude: None,
                ac_phase: None,
                waveform: Some(IrWaveform::Pulse {
                    initial: 0.0.into(),
                    pulsed: 1e-3.into(),
                    delay: 0.0.into(),
                    rise_time: 1e-9.into(),
                    fall_time: 1e-9.into(),
                    pulse_width: 5e-6.into(),
                    period: 10e-6.into(),
                }),
            },
            Component::BehavioralVoltage {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                expression: "V(x)*2+1".into(),
            },
            Component::BehavioralCurrent {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                expression: "I(V1)*10".into(),
            },
            Component::Vcvs {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                ncp: "c".into(),
                ncm: "d".into(),
                gain: 100.0,
            },
            Component::Vccs {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                ncp: "c".into(),
                ncm: "d".into(),
                transconductance: 0.01,
            },
            Component::Cccs {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                vsense: "Vs".into(),
                gain: 50.0,
            },
            Component::Ccvs {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                vsense: "Vs".into(),
                transresistance: 500.0,
            },
            Component::Diode {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                model: "D1".into(),
                params: vec![],
            },
            Component::Bjt {
                name: "1".into(),
                nc: "c".into(),
                nb: "b".into(),
                ne: "e".into(),
                extra_nodes: vec![],
                model: "Q1".into(),
                params: vec![("area".into(), "2".into())],
            },
            Component::Mosfet {
                name: "1".into(),
                nd: "d".into(),
                ng: "g".into(),
                ns: "s".into(),
                nb: Some("b".into()),
                extra_nodes: vec![],
                model: "M1".into(),
                params: vec![("W".into(), "1u".into()), ("L".into(), "180n".into())],
            },
            Component::Jfet {
                name: "1".into(),
                nd: "d".into(),
                ng: "g".into(),
                ns: "s".into(),
                model: "J1".into(),
                params: vec![],
            },
            Component::Mesfet {
                name: "1".into(),
                nd: "d".into(),
                ng: "g".into(),
                ns: "s".into(),
                model: "Z1".into(),
                params: vec![],
            },
            Component::VSwitch {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                ncp: "c".into(),
                ncm: "d".into(),
                model: "SW".into(),
            },
            Component::ISwitch {
                name: "1".into(),
                np: "a".into(),
                nm: "b".into(),
                vcontrol: "Vs".into(),
                model: "CSW".into(),
            },
            Component::TLine {
                name: "1".into(),
                inp: "i1".into(),
                inm: "i2".into(),
                outp: "o1".into(),
                outm: "o2".into(),
                z0: 50.0,
                td: 1e-9,
            },
            Component::Xspice {
                name: "1".into(),
                connections: vec!["[din]".into(), "[dout]".into()],
                model: "d_buf".into(),
            },
            Component::Poly {
                name: "1".into(),
                letter: 'E',
                np: "a".into(),
                nm: "b".into(),
                controls: vec!["c".into(), "d".into()],
                coeffs: vec![0.0, 2.0],
            },
            Component::RawSpice {
                line: ".probe V(out)".into(),
            },
        ];
        let x = || IrNum::Expr("vdd/2".into());
        let waveforms = [
            IrWaveform::Sin {
                offset: x(),
                amplitude: 1.0.into(),
                frequency: 1e3.into(),
                delay: 0.0.into(),
                damping: 0.0.into(),
                phase: 90.0.into(),
            },
            IrWaveform::Pulse {
                initial: 0.0.into(),
                pulsed: x(),
                delay: 0.0.into(),
                rise_time: 1e-9.into(),
                fall_time: 1e-9.into(),
                pulse_width: 5e-6.into(),
                period: 10e-6.into(),
            },
            IrWaveform::Pwl {
                values: vec![(0.0.into(), 0.0.into()), (1e-6.into(), x())],
                repeat: None,
                delay: None,
            },
            IrWaveform::Exp {
                initial: 0.0.into(),
                pulsed: 1.0.into(),
                rise_delay: x(),
                rise_tau: 1e-9.into(),
                fall_delay: 1e-6.into(),
                fall_tau: 1e-9.into(),
            },
            IrWaveform::Sffm {
                offset: 0.0.into(),
                amplitude: 1.0.into(),
                carrier_freq: 1e6.into(),
                modulation_index: x(),
                signal_freq: 1e3.into(),
            },
            IrWaveform::Am {
                amplitude: 1.0.into(),
                offset: 0.0.into(),
                modulating_freq: 1e3.into(),
                carrier_freq: 1e6.into(),
                delay: x(),
            },
            IrWaveform::TrNoise {
                na: x(),
                nt: 1e-9.into(),
                nalpha: 1.0.into(),
                namp: 1e-3.into(),
                rtsam: 0.0.into(),
                rtscapt: 0.0.into(),
                rtsemt: 0.0.into(),
            },
            IrWaveform::TrRandom {
                kind: RandomKind::Exponential,
                ts: 1e-6.into(),
                td: 0.0.into(),
                p1: x(),
                p2: 0.5.into(),
            },
        ];
        let sources = waveforms.into_iter().map(|w| Component::VoltageSource {
            name: "w".into(),
            np: "a".into(),
            nm: "0".into(),
            value: w.at_zero(),
            ac_magnitude: Some(1.0),
            ac_phase: None,
            waveform: Some(w),
            port: None,
        });
        let all_components: Vec<Component> = all_components.into_iter().chain(sources).collect();

        // The JSON tags each variant by name: every component, waveform and value.
        for c in &all_components {
            assert_tagged(c);
            match c {
                Component::VoltageSource {
                    value, waveform, ..
                }
                | Component::CurrentSource {
                    value, waveform, ..
                } => {
                    assert_tagged(value);
                    waveform.iter().for_each(assert_tagged);
                }
                Component::Resistor { value, .. }
                | Component::Capacitor { value, .. }
                | Component::Inductor { value, .. } => assert_tagged(value),
                _ => {}
            }
            // The two expression walkers visit the same fields.
            let mut m = c.clone();
            assert_eq!(
                c.exprs(),
                m.exprs_mut()
                    .into_iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>(),
                "{c:?}"
            );
        }
        let ir = CircuitIR {
            top: Subcircuit {
                name: "all_components".into(),
                components: all_components,
                ..Default::default()
            },
            ..Default::default()
        };

        let json = serde_json::to_string_pretty(&ir).unwrap();
        let ir2: CircuitIR = serde_json::from_str(&json).unwrap();
        assert_eq!(ir, ir2);
        assert_schema(&ir);
    }

    #[test]
    fn test_all_analysis_variants_roundtrip() {
        let analyses = vec![
            Analysis::Op,
            Analysis::Dc {
                sweeps: vec![DcSweep {
                    source: "V1".into(),
                    start: 0.0,
                    stop: 5.0,
                    step: 0.01,
                }],
            },
            Analysis::Ac {
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
            Analysis::Transient {
                step: 1e-9,
                stop: 1e-3,
                start: Some(0.0),
                max_step: Some(1e-10),
                uic: true,
            },
            Analysis::Noise {
                output: "V(out)".into(),
                reference: "0".into(),
                source: "V1".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6,
                },
                points_per_summary: None,
            },
            Analysis::Tf {
                output: "V(out)".into(),
                source: "Vin".into(),
            },
            Analysis::Sensitivity {
                output: "V(out)".into(),
                ac: Some(FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6,
                }),
            },
            Analysis::PoleZero {
                node1: "in".into(),
                node2: "0".into(),
                node3: "out".into(),
                node4: "0".into(),
                tf_type: PzTransfer::Vol,
                pz_type: PzKind::Pz,
            },
            Analysis::Distortion {
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6,
                },
                f2overf1: Some(0.9),
            },
            Analysis::Pss {
                fundamental: 1e6,
                stabilization: 100e-6,
                observe_node: "out".into(),
                points_per_period: 100,
                harmonics: 10,
                sc_iter: None,
                steady_coeff: None,
                uic: false,
            },
            Analysis::HarmonicBalance {
                frequencies: vec![1e6, 2e6],
                harmonics: vec![5, 3],
            },
            Analysis::SPar {
                sweep: FreqSweep {
                    variation: Sweep::Lin,
                    points: 201,
                    start: 1e6,
                    stop: 10e9,
                },
            },
            Analysis::Stability {
                probe: "iprobe".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
            Analysis::TransientNoise {
                step: 1e-9,
                stop: 1e-6,
            },
            Analysis::Fourier {
                fundamental: 1e3,
                outputs: vec!["V(out)".into()],
                num_harmonics: Some(10),
            },
            Analysis::SpectreSweep {
                param: "R1".into(),
                start: 1e3,
                stop: 10e3,
                step: 1e3,
                inner: "dc1".into(),
                inner_type: SpectreAnalysis::Dc,
            },
            Analysis::SpectreMonteCarlo {
                iterations: 100,
                inner: "tran1".into(),
                inner_type: SpectreAnalysis::Tran,
                seed: Some(42),
            },
            Analysis::SpectrePac {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 100e-6,
                    pss_harmonics: 10,
                },
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
                sweep_type: "relative".into(),
            },
            Analysis::SpectrePnoise {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 100e-6,
                    pss_harmonics: 10,
                },
                output: "out".into(),
                reference: "0".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
            Analysis::SpectrePxf {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 100e-6,
                    pss_harmonics: 10,
                },
                output: "out".into(),
                source: "V1".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
            Analysis::SpectrePstb {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 100e-6,
                    pss_harmonics: 10,
                },
                probe: "iprobe".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 100,
                    start: 1.0,
                    stop: 1e9,
                },
            },
        ];

        analyses.iter().for_each(assert_tagged);
        let ir = CircuitIR {
            top: Subcircuit {
                name: "analysis_test".into(),
                ..Default::default()
            },
            testbench: Some(Testbench {
                analyses,
                ..Default::default()
            }),
            ..Default::default()
        };

        let json = serde_json::to_string_pretty(&ir).unwrap();
        let ir2: CircuitIR = serde_json::from_str(&json).unwrap();
        assert_eq!(ir, ir2);
        assert_schema(&ir);
    }

    #[test]
    fn elements_keep_card_order() {
        let r = |n: &str| Component::RawSpice { line: n.into() };
        let x = |n: &str, at: Option<u32>| Instance {
            name: n.into(),
            at,
            ..Default::default()
        };
        let sc = Subcircuit {
            components: vec![r("a"), r("b")],
            instances: vec![x("late", None), x("first", Some(0)), x("mid", Some(1))],
            ..Default::default()
        };
        let names: Vec<&str> = sc
            .elements()
            .into_iter()
            .map(|e| match e {
                Element::Component(Component::RawSpice { line }) => line.as_str(),
                Element::Instance(i) => i.name.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(names, ["first", "a", "mid", "b", "late"]);
    }

    // ── Issue 02: Analysis::kind_str() ──

    #[test]
    fn test_analysis_kind_str() {
        assert_eq!(Analysis::Op.kind_str(), "op");
        assert_eq!(Analysis::Dc { sweeps: vec![] }.kind_str(), "dc");
        assert_eq!(
            Analysis::Ac {
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6
                }
            }
            .kind_str(),
            "ac"
        );
        assert_eq!(
            Analysis::Transient {
                step: 1e-9,
                stop: 1e-6,
                start: None,
                max_step: None,
                uic: false
            }
            .kind_str(),
            "tran"
        );
        assert_eq!(
            Analysis::Noise {
                output: "out".into(),
                reference: "0".into(),
                source: "V1".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6
                },
                points_per_summary: None
            }
            .kind_str(),
            "noise"
        );
        assert_eq!(
            Analysis::Pss {
                fundamental: 1e6,
                stabilization: 10e-6,
                observe_node: "out".into(),
                points_per_period: 100,
                harmonics: 10,
                sc_iter: None,
                steady_coeff: None,
                uic: false
            }
            .kind_str(),
            "pss"
        );
        assert_eq!(
            Analysis::HarmonicBalance {
                frequencies: vec![1e6],
                harmonics: vec![5]
            }
            .kind_str(),
            "hb"
        );
        assert_eq!(
            Analysis::Stability {
                probe: "p".into(),
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6
                }
            }
            .kind_str(),
            "stb"
        );
        assert_eq!(
            Analysis::TransientNoise {
                step: 1e-9,
                stop: 1e-6
            }
            .kind_str(),
            "trannoise"
        );
        assert_eq!(
            Analysis::SpectrePac {
                pss: PssBase {
                    pss_fundamental: 1e6,
                    pss_stabilization: 10e-6,
                    pss_harmonics: 10
                },
                sweep: FreqSweep {
                    variation: Sweep::Dec,
                    points: 10,
                    start: 1.0,
                    stop: 1e6
                },
                sweep_type: "relative".into()
            }
            .kind_str(),
            "pac"
        );
    }

    #[test]
    fn test_ac_magnitude_serde_roundtrip() {
        let v_with_ac = Component::VoltageSource {
            name: "ac".into(),
            np: "in".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 0.0 },
            ac_magnitude: Some(1.0),
            ac_phase: Some(45.0),
            waveform: None,
            port: None,
        };
        let json = serde_json::to_string(&v_with_ac).unwrap();
        assert!(
            json.contains("\"ac_magnitude\":1.0"),
            "json has ac_magnitude: {}",
            json
        );
        assert!(
            json.contains("\"ac_phase\":45.0"),
            "json has ac_phase: {}",
            json
        );

        let roundtripped: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(v_with_ac, roundtripped);

        // Without AC — fields should be absent (serde skip_serializing_none via default)
        let v_no_ac = Component::VoltageSource {
            name: "dc".into(),
            np: "vdd".into(),
            nm: "0".into(),
            value: IrValue::Numeric { value: 3.3 },
            ac_magnitude: None,
            ac_phase: None,
            waveform: None,
            port: None,
        };
        let json_no_ac = serde_json::to_string(&v_no_ac).unwrap();
        let rt_no_ac: Component = serde_json::from_str(&json_no_ac).unwrap();
        assert_eq!(v_no_ac, rt_no_ac);

        // Deserialize JSON without ac fields (backwards compat)
        let old_json = r#"{"type":"VoltageSource","name":"old","np":"a","nm":"0","value":{"type":"Numeric","value":5.0},"waveform":null}"#;
        let old_v: Component = serde_json::from_str(old_json).unwrap();
        match old_v {
            Component::VoltageSource {
                ac_magnitude,
                ac_phase,
                ..
            } => {
                assert_eq!(ac_magnitude, None);
                assert_eq!(ac_phase, None);
            }
            _ => panic!("Expected VoltageSource"),
        }
    }
}
