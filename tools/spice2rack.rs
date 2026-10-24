//! SPICE deck -> SpiceRack IR -> every dialect.
//!
//!   cargo run --no-default-features --bin spice2rack -- -o out/ deck.sp [deck2.sp ...]
//!
//! For each deck writes `out/<stem>.json` (the `CircuitIR`, SpiceRack's own
//! form) and one deck per backend: `.ngspice.sp`, `.hspice.sp`, `.ltspice.cir`,
//! `.spectre.scs`, `.vacask.sim`, `.espice.sp`. A backend that cannot express
//! the circuit (VACASK has no B sources, ngspice no `.step`) is reported and
//! skipped; the others are still written. The reader is `spicerack::import`.
//!
//! `--python` also writes `<stem>.py`: the deck as SpiceRack Python, using
//! the public API only. A construct the API cannot express goes through
//! `raw_spice()` under a `# gap:` comment, and `--python` ends with the gap
//! tally over all decks: the API's missing pieces, ranked by real decks.
//! Running the script prints `tb.netlist(<dialect>)`, which must hold the
//! same cards as the `.ngspice.sp` written beside it (`--check` runs it and
//! compares them as a multiset: a `raw_spice()` fallback moves its card into
//! the raw block, after the components).
//! `--dialect hspice` reads HSPICE decks (default ngspice).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use spicerack::codegen::{Dialect, card, netlist};
use spicerack::import::parse;
use spicerack::ir::*;

// ── Python ──

fn py_str(s: &str) -> String {
    format!("{s:?}")
}

fn py_num(v: f64) -> String {
    format!("{v:?}")
}

/// A value argument: a number, or an expression string.
fn py_value(v: &IrValue) -> Option<String> {
    match v {
        IrValue::Numeric { value } => Some(py_num(*value)),
        IrValue::Expression { expr } => Some(py_str(expr)),
        IrValue::Raw { .. } => None,
    }
}

/// `**{"k": "v", ...}` for parameters whose names need not be identifiers.
fn py_kwargs(params: &[(String, String)]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let items: Vec<String> = params.iter().map(|(k, v)| format!("{}: {}", py_str(k), py_str(v))).collect();
    format!(", **{{{}}}", items.join(", "))
}

/// The SpiceRack Python for one deck, and the API gaps it hit.
struct Py {
    out: String,
    gaps: Vec<&'static str>,
}

impl Py {
    fn line(&mut self, s: impl AsRef<str>) {
        self.out.push_str(s.as_ref());
        self.out.push('\n');
    }

    /// `obj.raw_spice(<card>)` under the reason no API call fits.
    fn gap(&mut self, obj: &str, why: &'static str, text: &str) {
        self.gaps.push(why);
        self.line(format!("{obj}.raw_spice({})  # gap: {why}", py_str(text)));
    }

    fn component(&mut self, obj: &str, c: &Component) {
        let raw = card(c, Dialect::Ngspice).unwrap_or_default();
        let n = |s: &str| py_str(s);
        match c {
            Component::Resistor { name, n1, n2, value, params } | Component::Capacitor { name, n1, n2, value, params } | Component::Inductor { name, n1, n2, value, params } => {
                let letter = match c { Component::Resistor { .. } => "R", Component::Capacitor { .. } => "C", _ => "L" };
                match py_value(value) {
                    Some(v) => self.line(format!("{obj}.{letter}(name={}, positive={}, negative={}, value={v}{})", n(name), n(n1), n(n2), py_kwargs(params))),
                    None => self.gap(obj, "raw element value", &raw),
                }
            }
            Component::MutualInductor { name, inductor1, inductor2, coupling } => {
                self.line(format!("{obj}.K(name={}, inductor1={}, inductor2={}, coupling={})", n(name), n(inductor1), n(inductor2), py_num(*coupling)))
            }
            Component::VoltageSource { name, np, nm, value, ac_magnitude, ac_phase, waveform } | Component::CurrentSource { name, np, nm, value, ac_magnitude, ac_phase, waveform } => {
                let current = matches!(c, Component::CurrentSource { .. });
                let (letter, kind) = if current { ("I", "Current") } else { ("V", "Voltage") };
                let ac = match (ac_magnitude, ac_phase) {
                    (None, _) => String::new(),
                    (Some(m), None) => format!(", ac={}", py_num(*m)),
                    (Some(m), Some(p)) => format!(", ac={}, ac_phase={}", py_num(*m), py_num(*p)),
                };
                let dc = match value { IrValue::Numeric { value } => Some(*value), _ => None };
                let head = format!("name={}, positive={}, negative={}", n(name), n(np), n(nm));
                match (waveform, dc) {
                    (None, _) => match py_value(value) {
                        Some(v) => self.line(format!("{obj}.{letter}({head}, value={v}{ac})")),
                        None => self.gap(obj, "raw element value", &raw),
                    },
                    (_, None) => self.gap(obj, "waveform sources take no expression DC value", &raw),
                    (Some(IrWaveform::Pulse { initial, pulsed, delay, rise_time, fall_time, pulse_width, period }), Some(v)) => {
                        if !ac.is_empty() {
                            self.gap(obj, "pulse/PWL sources take no ac=", &raw)
                        } else if v != *initial {
                            self.gap(obj, "waveform sources take no DC value of their own", &raw)
                        } else {
                            self.line(format!(
                                "{obj}.Pulse{kind}Source({head}, initial_value={}, pulsed_value={}, delay={}, pulse_width={}, period={}, rise_time={}, fall_time={})",
                                py_num(*initial), py_num(*pulsed), py_num(*delay), py_num(*pulse_width), py_num(*period), py_num(*rise_time), py_num(*fall_time)
                            ))
                        }
                    }
                    (Some(IrWaveform::Pwl { values }), Some(v)) => {
                        let first = values.first().map_or(0.0, |p| p.1);
                        if current {
                            self.gap(obj, "no PWL current source on Subcircuit", &raw)
                        } else if !ac.is_empty() {
                            self.gap(obj, "pulse/PWL sources take no ac=", &raw)
                        } else if v != first {
                            self.gap(obj, "waveform sources take no DC value of their own", &raw)
                        } else {
                            let pts: Vec<String> = values.iter().map(|(t, y)| format!("({}, {})", py_num(*t), py_num(*y))).collect();
                            self.line(format!("{obj}.PieceWiseLinearVoltageSource({head}, values=[{}])", pts.join(", ")))
                        }
                    }
                    (Some(IrWaveform::Sin { offset, amplitude, frequency, delay, damping, phase }), Some(v)) => {
                        if *delay != 0.0 || *damping != 0.0 || *phase != 0.0 {
                            self.gap(obj, "sinusoidal sources take no delay/damping/phase", &raw)
                        } else if current && !ac.is_empty() {
                            self.gap(obj, "Subcircuit.SinusoidalCurrentSource takes no ac=", &raw)
                        } else {
                            self.line(format!(
                                "{obj}.Sinusoidal{kind}Source({head}, dc_offset={}, offset={}, amplitude={}, frequency={}{ac})",
                                py_num(v), py_num(*offset), py_num(*amplitude), py_num(*frequency)
                            ))
                        }
                    }
                    (Some(IrWaveform::Exp { .. } | IrWaveform::Sffm { .. } | IrWaveform::Am { .. }), _) => self.gap(obj, "no EXP/SFFM/AM source", &raw),
                }
            }
            Component::BehavioralVoltage { name, np, nm, expression } => self.line(format!("{obj}.BV(name={}, positive={}, negative={}, expression={})", n(name), n(np), n(nm), n(expression))),
            Component::BehavioralCurrent { name, np, nm, expression } => self.line(format!("{obj}.BI(name={}, positive={}, negative={}, expression={})", n(name), n(np), n(nm), n(expression))),
            Component::Vcvs { name, np, nm, ncp, ncm, gain } => self.line(format!("{obj}.E(name={}, positive={}, negative={}, control_positive={}, control_negative={}, voltage_gain={})", n(name), n(np), n(nm), n(ncp), n(ncm), py_num(*gain))),
            Component::Vccs { name, np, nm, ncp, ncm, transconductance } => self.line(format!("{obj}.G(name={}, positive={}, negative={}, control_positive={}, control_negative={}, transconductance={})", n(name), n(np), n(nm), n(ncp), n(ncm), py_num(*transconductance))),
            Component::Cccs { name, np, nm, vsense, gain } => self.line(format!("{obj}.F(name={}, positive={}, negative={}, vsense={}, current_gain={})", n(name), n(np), n(nm), n(vsense), py_num(*gain))),
            Component::Ccvs { name, np, nm, vsense, transresistance } => self.line(format!("{obj}.H(name={}, positive={}, negative={}, vsense={}, transresistance={})", n(name), n(np), n(nm), n(vsense), py_num(*transresistance))),
            Component::Diode { name, np, nm, model, params } => self.line(format!("{obj}.D(name={}, anode={}, cathode={}, model={}{})", n(name), n(np), n(nm), n(model), py_kwargs(params))),
            Component::Bjt { name, nc, nb, ne, extra_nodes, model, params } => {
                let extra: String = ["substrate", "thermal"].iter().zip(extra_nodes).map(|(k, v)| format!(", {k}={}", n(v))).collect();
                self.line(format!("{obj}.Q(name={}, collector={}, base={}, emitter={}, model={}{extra}{})", n(name), n(nc), n(nb), n(ne), n(model), py_kwargs(params)))
            }
            Component::Mosfet { name, nd, ng, ns, nb, model, params } => self.line(format!("{obj}.M(name={}, drain={}, gate={}, source={}, bulk={}, model={}{})", n(name), n(nd), n(ng), n(ns), n(nb), n(model), py_kwargs(params))),
            Component::Jfet { name, nd, ng, ns, model, params } => self.line(format!("{obj}.J(name={}, drain={}, gate={}, source={}, model={}{})", n(name), n(nd), n(ng), n(ns), n(model), py_kwargs(params))),
            Component::Mesfet { name, nd, ng, ns, model, params } => self.line(format!("{obj}.Z(name={}, drain={}, gate={}, source={}, model={}{})", n(name), n(nd), n(ng), n(ns), n(model), py_kwargs(params))),
            Component::VSwitch { name, np, nm, ncp, ncm, model } => self.line(format!("{obj}.S(name={}, positive={}, negative={}, control_positive={}, control_negative={}, model={})", n(name), n(np), n(nm), n(ncp), n(ncm), n(model))),
            Component::ISwitch { name, np, nm, vcontrol, model } => self.line(format!("{obj}.W(name={}, positive={}, negative={}, vcontrol={}, model={})", n(name), n(np), n(nm), n(vcontrol), n(model))),
            Component::TLine { name, inp, inm, outp, outm, z0, td } => self.line(format!("{obj}.T(name={}, input_positive={}, input_negative={}, output_positive={}, output_negative={}, Z0={}, TD={})", n(name), n(inp), n(inm), n(outp), n(outm), py_num(*z0), py_num(*td))),
            Component::Xspice { name, connections, model } => {
                let conns: Vec<String> = connections.iter().map(|c| n(c)).collect();
                self.line(format!("{obj}.A(name={}, connections=[{}], model={})", n(name), conns.join(", "), n(model)))
            }
            Component::RawSpice { line } => self.gap(obj, "card with no IR form", line),
        }
    }

    /// Everything a `Subcircuit` object holds, written as calls on `obj`.
    fn block(&mut self, obj: &str, sc: &Subcircuit, top: bool) {
        for p in &sc.includes {
            self.line(format!("{obj}.include({})", py_str(p)));
        }
        for (p, sec) in &sc.libs {
            self.line(format!("{obj}.lib({}, {})", py_str(p), py_str(sec)));
        }
        for p in &sc.osdi_loads {
            self.line(format!("{obj}.veriloga({})", py_str(p)));
        }
        if top {
            for p in &sc.parameters {
                self.line(format!("{obj}.parameter({}, {})", py_str(&p.name), py_str(p.default.as_deref().unwrap_or(""))));
            }
        }
        for m in &sc.models {
            self.line(format!("{obj}.model({}, {}{})", py_str(&m.name), py_str(&m.kind), py_kwargs(&m.parameters)));
        }
        for c in &sc.components {
            self.component(obj, c);
        }
        for i in &sc.instances {
            let nodes: String = i.port_mapping.iter().map(|p| format!(", {}", py_str(p))).collect();
            self.line(format!("{obj}.X({}, {}{nodes}{})", py_str(&i.name), py_str(&i.subcircuit), py_kwargs(&i.parameters)));
        }
        for l in &sc.raw_spice {
            self.gap(obj, "card with no IR form", l);
        }
    }

    fn analysis(&mut self, a: &Analysis) {
        match a {
            Analysis::Op => self.line("tb.add_operating_point()"),
            Analysis::Dc { sweeps } => {
                let kw: Vec<String> = sweeps.iter().map(|s| format!("{}: slice({}, {}, {})", py_str(&s.source), py_num(s.start), py_num(s.stop), py_num(s.step))).collect();
                self.line(format!("tb.add_dc(**{{{}}})", kw.join(", ")))
            }
            Analysis::Ac { variation, points, start, stop } => self.line(format!("tb.add_ac(variation={}, number_of_points={points}, start_frequency={}, stop_frequency={})", py_str(variation), py_num(*start), py_num(*stop))),
            Analysis::Transient { step, stop, start, max_step, uic } => {
                let opt = |x: &Option<f64>| x.map_or("None".into(), py_num);
                self.line(format!("tb.add_transient(step_time={}, end_time={}, start_time={}, max_time={}, use_initial_condition={})", py_num(*step), py_num(*stop), opt(start), opt(max_step), if *uic { "True" } else { "False" }))
            }
            Analysis::Noise { output, reference, source, variation, points, start, stop, points_per_summary } => {
                let pps = points_per_summary.map_or("None".into(), |p| p.to_string());
                self.line(format!("tb.add_noise({}, {}, {}, variation={}, points={points}, start_frequency={}, stop_frequency={}, points_per_summary={pps})", py_str(output), py_str(reference), py_str(source), py_str(variation), py_num(*start), py_num(*stop)))
            }
            Analysis::Fourier { fundamental, outputs, num_harmonics } => {
                let outs: Vec<String> = outputs.iter().map(|o| py_str(o)).collect();
                let nh = num_harmonics.map_or("None".into(), |n| n.to_string());
                self.line(format!("tb.add_fourier({}, [{}], num_harmonics={nh})", py_num(*fundamental), outs.join(", ")))
            }
            other => {
                // `tb.tf()`, `tb.polezero()` ... run at once; there is no `add_` form.
                self.gaps.push("analysis with no add_* builder (tf, sens, pz, disto, ...)");
                let text = netlist(&CircuitIR { top: Subcircuit { name: String::new(), ports: vec![], parameters: vec![], components: vec![], instances: vec![], models: vec![], raw_spice: vec![], includes: vec![], libs: vec![], osdi_loads: vec![], verilog_blocks: vec![] }, testbench: Some(Testbench { dut: String::new(), stimulus: vec![], analyses: vec![other.clone()], options: SimOptions::default(), saves: vec![], measures: vec![], temperature: None, nominal_temperature: None, initial_conditions: vec![], node_sets: vec![], step_params: vec![], extra_lines: vec![] }), subcircuit_defs: vec![], model_libraries: vec![] }, Dialect::Ngspice).unwrap_or_default();
                let card = text.lines().find(|l| l.starts_with('.') && *l != ".end").unwrap_or("").to_string();
                self.line(format!("tb.extra_line({})  # gap: analysis with no add_* builder (tf, sens, pz, disto, ...)", py_str(&card)));
            }
        }
    }
}

/// The deck as a SpiceRack Python script.
fn python(ir: &CircuitIR) -> Py {
    let mut py = Py { out: String::new(), gaps: Vec::new() };
    py.line(format!("# {} — generated by spice2rack from the deck's IR", ir.top.name));
    py.line("import sys\n\nimport spicerack as ps\n");
    for (i, sc) in ir.subcircuit_defs.iter().enumerate() {
        let ports: Vec<String> = sc.ports.iter().map(|p| py_str(&p.name)).collect();
        let params: Vec<(String, String)> = sc.parameters.iter().map(|p| (p.name.clone(), p.default.clone().unwrap_or_default())).collect();
        py.line(format!("sub{i} = ps.Subcircuit({}, [{}]{})", py_str(&sc.name), ports.join(", "), py_kwargs(&params)));
        py.block(&format!("sub{i}"), sc, false);
        py.line("");
    }
    py.line(format!("dut = ps.Subcircuit({})", py_str(&ir.top.name)));
    py.block("dut", &ir.top, true);
    py.line("\ntb = ps.Testbench(dut)");
    for i in 0..ir.subcircuit_defs.len() {
        py.line(format!("tb.add_subcircuit(sub{i})"));
    }
    if let Some(tb) = &ir.testbench {
        for c in &tb.stimulus {
            py.component("tb", c);
        }
        if !tb.options.portable.is_empty() {
            py.line(format!("tb.options({})", &py_kwargs(&tb.options.portable)[2..]));
        }
        if let Some(t) = tb.temperature {
            py.line(format!("tb.temperature = {}", py_num(t)));
        }
        let volts = |v: &[(String, f64)]| v.iter().map(|(k, x)| format!("{}: {}", py_str(k), py_num(*x))).collect::<Vec<_>>().join(", ");
        if !tb.initial_conditions.is_empty() {
            py.line(format!("tb.initial_condition(**{{{}}})", volts(&tb.initial_conditions)));
        }
        if !tb.node_sets.is_empty() {
            py.line(format!("tb.node_set(**{{{}}})", volts(&tb.node_sets)));
        }
        if !tb.saves.is_empty() {
            py.line(format!("tb.save({})", tb.saves.iter().map(|s| py_str(s)).collect::<Vec<_>>().join(", ")));
        }
        for m in &tb.measures {
            py.line(format!("tb.measure({})", py_str(m)));
        }
        for l in &tb.extra_lines {
            py.line(format!("tb.extra_line({})", py_str(l)));
        }
        for a in &tb.analyses {
            py.analysis(a);
        }
    }
    py.line("\nif __name__ == \"__main__\":\n    print(tb.netlist(sys.argv[1] if len(sys.argv) > 1 else \"ngspice\"))");
    py
}

// ── Output ──

const BACKENDS: [(Dialect, &str); 6] = [
    (Dialect::Ngspice, "ngspice.sp"),
    (Dialect::Hspice, "hspice.sp"),
    (Dialect::Ltspice, "ltspice.cir"),
    (Dialect::Spectre, "spectre.scs"),
    (Dialect::Vacask, "vacask.sim"),
    (Dialect::Espice, "espice.sp"),
];

struct Opts {
    out: PathBuf,
    dialect: Dialect,
    python: bool,
    check: bool,
}

/// Converts one deck; returns the Python gaps it hit (empty without `--python`).
fn convert(deck: &Path, o: &Opts) -> std::io::Result<Vec<&'static str>> {
    let src = String::from_utf8_lossy(&std::fs::read(deck)?).into_owned();
    let stem = deck.file_stem().and_then(|s| s.to_str()).unwrap_or("deck");
    let (ir, report) = parse(&src, stem, o.dialect).map_err(std::io::Error::other)?;
    if report.raw > 0 {
        eprintln!("{}: {} card(s) kept verbatim (no IR form)", deck.display(), report.raw);
    }
    for card in &report.unparsed {
        eprintln!("{}: grammar refused, kept verbatim: {card}", deck.display());
    }
    std::fs::write(o.out.join(format!("{stem}.json")), serde_json::to_string_pretty(&ir).map_err(std::io::Error::other)?)?;
    for (d, ext) in BACKENDS {
        match netlist(&ir, d) {
            Ok(text) => std::fs::write(o.out.join(format!("{stem}.{ext}")), text + "\n")?,
            Err(e) => eprintln!("{}: {d:?} skipped: {e}", deck.display()),
        }
    }
    if !o.python {
        return Ok(vec![]);
    }
    let py = python(&ir);
    let script = o.out.join(format!("{stem}.py"));
    std::fs::write(&script, &py.out)?;
    if o.check {
        let want = netlist(&ir, Dialect::Ngspice).map_err(std::io::Error::other)?;
        let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".into());
        let run = std::process::Command::new(python).arg(&script).output()?;
        let got = String::from_utf8_lossy(&run.stdout);
        let cards = |t: &str| {
            let mut v: Vec<String> = t.lines().map(str::to_string).collect();
            v.sort();
            v
        };
        if cards(&got) != cards(&want) {
            let err = String::from_utf8_lossy(&run.stderr);
            return Err(std::io::Error::other(format!("{} prints a different netlist{}", script.display(), err.lines().last().map(|l| format!(": {l}")).unwrap_or_default())));
        }
    }
    Ok(py.gaps)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |f: &str| args.iter().position(|a| a == f).map(|i| args.remove(i)).is_some();
    let (python, check) = (flag("--python"), flag("--check"));
    let mut value = |f: &str| args.iter().position(|a| a == f).filter(|i| i + 1 < args.len()).map(|i| {
        let v = args.remove(i + 1);
        args.remove(i);
        v
    });
    let out = PathBuf::from(value("-o").unwrap_or_else(|| ".".into()));
    let dialect = match value("--dialect").as_deref() {
        None | Some("ngspice") => Dialect::Ngspice,
        Some("hspice") => Dialect::Hspice,
        Some(other) => {
            eprintln!("--dialect {other}: reads ngspice or hspice");
            std::process::exit(2);
        }
    };
    if args.is_empty() {
        eprintln!("usage: spice2rack [-o DIR] [--dialect ngspice|hspice] [--python [--check]] DECK...");
        std::process::exit(2);
    }
    std::fs::create_dir_all(&out).expect("output directory");
    let o = Opts { out, dialect, python: python || check, check };
    let (mut failed, mut tally) = (0, BTreeMap::<&str, (usize, usize)>::new());
    for deck in &args {
        match convert(Path::new(deck), &o) {
            Ok(gaps) => {
                let mut seen: Vec<&str> = gaps.clone();
                seen.sort();
                seen.dedup();
                for g in &gaps {
                    tally.entry(g).or_default().0 += 1;
                }
                for g in seen {
                    tally.entry(g).or_default().1 += 1;
                }
            }
            Err(e) => {
                eprintln!("{deck}: {e}");
                failed += 1;
            }
        }
    }
    if o.python {
        let mut rows: Vec<_> = tally.into_iter().collect();
        rows.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        let mut report = format!("Python API gaps over {} decks (cards, decks):\n", args.len());
        for (g, (cards, decks)) in rows {
            let _ = writeln!(report, "  {decks:5} decks {cards:7} cards  {g}");
        }
        eprint!("{report}");
    }
    if failed > 0 {
        eprintln!("{failed} deck(s) failed");
    }
    std::process::exit((failed > 0) as i32);
}
