
    use super::*;
    use crate::codegen::{Dialect, netlist};

    const DECK: &str = "\
divider bench
* a comment
.param rv=2k
.model dm D(is=1e-14 n=1.5)
.model nch nmos level=1 vto=0.7 kp=100u
.subckt load n params: r=1k
R1 n 0 {r}
.ends load
V1 in 0 dc 1 ac 1
+ pulse(0 1 1n 1p 1p 5n 10n)
R1 in mid {rv} $ tail comment
X1 mid load r=2k
D1 mid dn dm
R2 dn 0 10k
E1 b 0 mid 0 2
B1 c 0 V = 2*v(mid)+1
M1 dd mid 0 0 nch w=10u l=1u
Rd in dd 10k
Gp 0 p poly(1) mid 0 0 1m
Rp p 0 1k
.option reltol=1e-4
.op
.end
";

    #[test]
    fn deck_to_ir() {
        let ir = parse(DECK, "divider", Dialect::Ngspice).unwrap().0;
        // A bare word naming a `.model` is a model reference, not `{rtest}`.
        let m = parse("t\n.model rtest r r=1k\nR1 1 0 rtest\nR2 1 0 rv\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(m.top.raw_spice.is_empty());
        assert!(matches!(&m.top.components[0], Component::Resistor { value: IrValue::Raw { text }, model: Some(model), .. } if text.is_empty() && model == "rtest"));
        assert!(matches!(&m.top.components[1], Component::Resistor { value: IrValue::Expression { expr }, model: None, .. } if expr == "rv"));
        // Nested definitions are local: two `sub`s, hoisted under their parents.
        let sc = parse("t\n.subckt a n\n.subckt sub n\nR1 n 0 1k\n.ends\nX1 n sub\n.ends\n.subckt b n\n.subckt sub n\nR1 n 0 2k\n.ends\nX1 n sub\n.ends\n", "t", Dialect::Ngspice).unwrap().0;
        let names: Vec<&str> = sc.subcircuit_defs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a__sub", "a", "b__sub", "b"]);
        assert_eq!(sc.subcircuit_defs[1].instances[0].subcircuit, "a__sub");
        assert_eq!(sc.subcircuit_defs[3].instances[0].subcircuit, "b__sub");
        // Nested `.if` blocks stay one raw block.
        let n = parse("t\n.if (a == 1)\n.if (b == 1)\nR1 1 0 1\n.endif\n.else\nR1 1 0 2\n.endif\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(n.top.components.is_empty() && n.top.raw_spice.len() == 7, "{:?}", n.top);
        assert_eq!(ir.subcircuit_defs[0].parameters[0], ParamDef { name: "r".into(), default: Some("1k".into()) });
        assert_eq!(ir.top.instances[0].parameters, vec![("r".into(), "2k".into())]);
        assert!(ir.top.components.iter().any(|c| matches!(c, Component::Resistor { value: IrValue::Expression { expr }, .. } if expr == "rv")));
        assert!(ir.top.components.iter().any(|c| matches!(c, Component::VoltageSource { waveform: Some(IrWaveform::Pulse { period, .. }), .. } if *period == 10e-9)));
        assert!(ir.top.components.iter().any(|c| matches!(c, Component::BehavioralVoltage { expression, .. } if expression == "2*v(mid)+1")));
        assert_eq!(ir.top.models[1].parameters.len(), 3);
        // POLY is a component now; nothing in this deck stays raw.
        assert!(ir.top.raw_spice.is_empty(), "{:?}", ir.top.raw_spice);
        // `.control` analyses become dot cards; the scripting goes.
        let c = parse("t\nR1 a 0 1k\n.control\nTRAN 1n 10n\nplot v(a)\n.endc\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(matches!(c.testbench.as_ref().unwrap().analyses[..], [Analysis::Transient { stop, .. }] if stop == 10e-9));
        // `.func`, as ngspice and HSPICE spell it.
        let f = parse("t\n.func dbl(x) {x*2}\nR1 a 0 {dbl(500)}\n", "t", Dialect::Ngspice).unwrap().0;
        assert_eq!(f.functions, vec![FuncDef { name: "dbl".into(), args: vec!["x".into()], body: "x*2".into() }]);
        assert!(netlist(&f, Dialect::Ngspice).unwrap().contains(".func dbl(x) {x*2}"));
        assert!(netlist(&f, Dialect::Hspice).unwrap().contains(".param dbl(x)='x*2'"));
        // POLY sources, voltage- and current-controlled.
        let p = parse("t\nE1 o 0 poly(2) a 0 b 0 1 2 3 4 5 6\nF1 f 0 poly(1) vs 0 1m\nVs s 0 0\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(p.top.raw_spice.is_empty(), "{:?}", p.top.raw_spice);
        assert!(matches!(&p.top.components[0], Component::Poly { letter: 'E', controls, coeffs, .. } if controls.len() == 4 && coeffs.len() == 6));
        assert!(netlist(&p, Dialect::Ngspice).unwrap().contains("E1 o 0 POLY(2) a 0 b 0 1 2 3 4 5 6"));
        // A waveform over parameters is a structured source in every dialect.
        let pw = parse("t\n.param vdd=1.8 tr=1n\nV1 a 0 PULSE(0 {vdd} 0 {tr} {tr*2} 5n 10n)\nR1 a 0 1k\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(pw.top.raw_spice.is_empty(), "{:?}", pw.top.raw_spice);
        assert!(matches!(&pw.top.components[0], Component::VoltageSource { value: IrValue::Numeric { value: 0.0 }, waveform: Some(IrWaveform::Pulse { pulsed: IrNum::Expr(p), fall_time: IrNum::Expr(f), .. }), .. } if p == "vdd" && f == "tr*2"));
        assert!(netlist(&pw, Dialect::Ngspice).unwrap().contains("PULSE(0 {vdd} 0 {tr} {tr*2} "));
        assert!(netlist(&pw, Dialect::Hspice).unwrap().contains("PULSE(0 'vdd' 0 'tr' 'tr*2'"));
        assert!(netlist(&pw, Dialect::Spectre).unwrap().contains("val1=vdd delay=0 rise=tr fall=tr*2"));
        assert!(netlist(&pw, Dialect::Vacask).unwrap().contains("val1=vdd delay=0 rise=tr fall=tr*2"));
        // A B expression keeps its own braces and spacing.
        let bx = parse("t\nB1 0 0 I={Cjo}/(1+max(V(b),-.5*{Vj})/{Vj})**{m}\n", "t", Dialect::Ltspice).unwrap().0;
        assert!(matches!(&bx.top.components[0], Component::BehavioralCurrent { expression, .. } if expression == "{Cjo}/(1+max(V(b),-.5*{Vj})/{Vj})**{m}"), "{:?}", bx.top.components);
        // A 3-terminal VDMOS, a model flag and an instance flag.
        let v = parse("t\n.model vdm vdmos pchan vto=1\n.model nch nmos level=1\nM1 d g s vdm\nM2 d g s b nch w=1u off\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(v.top.raw_spice.is_empty(), "{:?}", v.top.raw_spice);
        assert_eq!(v.top.models[0].parameters[0], ("pchan".to_string(), String::new()));
        assert!(matches!(&v.top.components[0], Component::Mosfet { nb: None, .. }));
        assert!(matches!(&v.top.components[1], Component::Mosfet { params, .. } if params.contains(&("off".into(), String::new()))));
        let text = netlist(&v, Dialect::Ngspice).unwrap();
        assert!(text.contains("M1 d g s vdm\n") && text.contains("M2 d g s b nch w=1u off") && text.contains("vdmos(pchan vto=1)"), "{text}");
        assert!(netlist(&v, Dialect::Spectre).is_err());
        // Optional waveform parentheses, `sine`, `.pz`, `.sens`.
        let w = parse("t\nV1 a 0 pulse 0 5 1n 1n 1n 5n 10n\nV2 b 0 dc 0 Sine(0 1 1k)\nR1 a 0 1k\n.pz a 0 b 0 vol pz\n.sens v(a) ac dec 10 1 1meg\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(w.top.raw_spice.is_empty(), "{:?}", w.top.raw_spice);
        assert!(matches!(&w.top.components[0], Component::VoltageSource { waveform: Some(IrWaveform::Pulse { pulsed, .. }), .. } if *pulsed == 5.0));
        assert!(matches!(&w.top.components[1], Component::VoltageSource { waveform: Some(IrWaveform::Sin { frequency, .. }), .. } if *frequency == 1e3));
        let a = &w.testbench.as_ref().unwrap().analyses;
        assert!(matches!(a[..], [Analysis::PoleZero { .. }, Analysis::Sensitivity { ac: Some(_), .. }]), "{a:?}");
        // A blank before the waveform's `(` is still the waveform.
        let b = parse("t\nV1 a 0 pwl (0 0 1n 1)\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(matches!(&b.top.components[0], Component::VoltageSource { waveform: Some(IrWaveform::Pwl { .. }), .. }), "{:?}", b.top);
        // A waveform-only source keeps its t=0 value as the DC value.
        let w = parse("t\nV2 x 0 pulse(3 0 1n 1n 1n 5n 10n)\n", "t", Dialect::Ngspice).unwrap().0;
        assert!(matches!(w.top.components[0], Component::VoltageSource { value: IrValue::Numeric { value: 3.0 }, .. }));
        // Numbers: the grammar's scale rows, read as one decimal.
        let n = parse("t\nR1 a 0 1.5meg\nC1 a 0 10pF\nR2 a 0 abc\nC2 a 0 0.1n\n", "t", Dialect::Ngspice).unwrap().0;
        let vals: Vec<&IrValue> = n.top.components.iter().map(|c| match c { Component::Resistor { value, .. } | Component::Capacitor { value, .. } => value, _ => unreachable!() }).collect();
        assert_eq!(vals, [&IrValue::Numeric { value: 1.5e6 }, &IrValue::Numeric { value: 10e-12 }, &IrValue::Expression { expr: "abc".into() }, &IrValue::Numeric { value: 1e-10 }]);
    }

    /// The converted ngspice deck must solve to the original deck's operating
    /// point. Skipped without ngspice on PATH.
    #[test]
    fn ngspice_round_trip_keeps_the_op() {
        let run = |deck: &str| -> Option<crate::result::RawData> {
            let dir = tempfile::TempDir::new().unwrap();
            let (p, r) = (dir.path().join("d.sp"), dir.path().join("d.raw"));
            std::fs::write(&p, deck).unwrap();
            let out = std::process::Command::new("ngspice").arg("-b").arg("-r").arg(&r).arg(&p).output().ok()?;
            assert!(out.status.success(), "{}\n{deck}", String::from_utf8_lossy(&out.stdout));
            Some(crate::rawfile::parse_raw(&std::fs::read(&r).unwrap()).unwrap())
        };
        let converted = netlist(&parse(DECK, "divider", Dialect::Ngspice).unwrap().0, Dialect::Ngspice).unwrap();
        let (Some(a), Some(b)) = (run(DECK), run(&converted)) else {
            eprintln!("ngspice not on PATH — skipping");
            return;
        };
        for node in ["v(mid)", "v(dn)", "v(b)", "v(c)", "v(dd)", "v(p)"] {
            let at = |r: &crate::result::RawData| r.variables.iter().position(|v| v.name.eq_ignore_ascii_case(node)).map(|i| r.real_data[i][0]);
            let (x, y) = (at(&a).unwrap(), at(&b).unwrap_or_else(|| panic!("{node} missing:\n{converted}")));
            assert!((x - y).abs() <= 1e-9 * x.abs().max(1.0), "{node}: {x} vs {y}\n{converted}");
        }
    }

    #[test]
    fn ltspice_inductor_gets_implicit_rser_unless_given_or_coupled() {
        let deck = "t\nL1 a 0 1u\nL2 b 0 1u Rser=5\nL3 c 0 1u\nL4 d 0 1u\nK1 L3 L4 0.5\n";
        let ir = parse(deck, "t", Dialect::Ltspice).unwrap().0;
        let rser = |name: &str| {
            ir.top.components.iter().find_map(|c| match c {
                Component::Inductor { name: n, params, .. } if n == name => Some(params.iter().find(|(k, _)| k.eq_ignore_ascii_case("rser")).map(|(_, v)| v.clone())),
                _ => None,
            })
        };
        assert_eq!(rser("1"), Some(Some("1m".to_string())));
        assert_eq!(rser("2"), Some(Some("5".to_string())));
        assert_eq!(rser("3"), Some(None));
        assert_eq!(rser("4"), Some(None));
    }

    /// Every dialect's grammar compiles and captures only AST tags.
    #[test]
    fn every_grammar_compiles() {
        for d in [Dialect::Ngspice, Dialect::Hspice, Dialect::Ltspice, Dialect::Xyce, Dialect::Spectre, Dialect::Vacask] {
            lang(d).unwrap();
        }
    }
