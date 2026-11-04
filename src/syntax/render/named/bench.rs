//! The testbench → Spectre statements: the deck's testbench items folded in
//! read order (options merged, the last temperature winning), then
//! `options`, `ic`, `nodeset`, `save`, the measures in a `simulator
//! lang=spice` region, and the analyses, each `.step` a `sweep` block
//! around them.
//!
//! Cites: Spectre Reference 19.1 `[REF]` pp.187-236 (`options`: `reltol`
//! #1, `vabstol` #3, `iabstol` #4, `temp` #6, `tnom` #7, `dcmaxiters` #56),
//! p.490 (`ic`), p.502 (`nodeset`), pp.517-518 (`save`), p.17, p.20
//! (`.measure`), p.64 (`dc`: `dev=`/`param=`), p.43 (`ac`), pp.416-417
//! (`tran`), pp.181-182 (`noise`), pp.438-439 (`xf`), p.524 (`sens`),
//! pp.387-389 (`sp`), pp.406-410 (`sweep`); `[CMP]` pp.323-325
//! (`fourier`); `[KUN]` B.2 (`simulator lang=`).
//!
//! Contents:
//! - `pub(super) struct Tb`, `pub(super) type Step`
//! - `W::bench(&mut self)`, `W::fold(&self) -> Tb`, `W::unwritten_outputs(&mut self, tb: &Tb)`
//! - `fn bare_node(s: &str) -> &str`

use super::W;
use crate::syntax::diag::Code;
use crate::syntax::render::capability::Construct;
use crate::syntax::render::read::{Ana, Bench, BenchCard, Opt, Sweep};

/// The testbench, folded from its statements in read order.
#[derive(Default)]
pub(super) struct Tb {
    /// Each analysis, with its statement's first token.
    pub(super) analyses: Vec<(Ana, u32)>,
    /// The portable options, with their statement's first token.
    pub(super) options: Vec<(String, String, u32)>,
    /// VACASK's own options, values as written, with their statement's
    /// first token.
    pub(super) vacask: Vec<(String, String, u32)>,
    pub(super) temp: Option<f64>,
    pub(super) tnom: Option<f64>,
    pub(super) ics: Vec<(String, f64)>,
    pub(super) nodesets: Vec<(String, f64)>,
    pub(super) saves: Vec<String>,
    /// The first save card's first token.
    pub(super) save_tok: Option<u32>,
    /// Each `.print`'s first token (neither target writes one).
    pub(super) prints: Vec<u32>,
    /// The first token of each options card holding Xyce's own options,
    /// which neither target writes.
    pub(super) xyce: Vec<u32>,
    /// Each `.meas` line, with its statement's first token.
    pub(super) measures: Vec<(String, u32)>,
    /// Each `.step`'s sweep, name, numbers and first token.
    pub(super) steps: Vec<Step>,
    /// VACASK control lines kept as text.
    pub(super) extra: Vec<String>,
}

/// A `.step`: its sweep keyword, parameter, start, stop and step, and its
/// statement's first token.
pub(super) type Step = (Option<&'static str>, String, [f64; 3], u32);

impl W<'_, '_> {
    /// The testbench's statements.
    pub(super) fn bench(&mut self) {
        let tb = self.fold();
        self.unwritten_outputs(&tb);
        // VACASK's own options are not Spectre's.
        let mut vacask: Vec<u32> = tb.vacask.iter().map(|o| o.2).collect();
        vacask.dedup();
        for tok in vacask {
            self.note(Code::W0024, tok);
        }
        // `[REF]` p.236: `o1 options scale=1.2`; options take `name=value`
        // only. A portable option is SPICE's: its name lowercased and
        // mapped, its number respelled (SPICE `VNTOL=1M` is milli).
        let mut opts = Vec::new();
        for (k, v, tok) in &tb.options {
            if v.is_empty() {
                self.refuse(Code::W0101, *tok);
                continue;
            }
            let k = k.to_ascii_lowercase();
            let k = match k.as_str() {
                "abstol" => "iabstol",
                "vntol" => "vabstol",
                "max_iterations" => "dcmaxiters",
                other => other,
            };
            opts.push(format!("{k}={}", self.value_text(v)));
        }
        if !opts.is_empty() {
            self.line(&format!("myopts options {}", opts.join(" ")));
        }
        if let Some(t) = tb.temp {
            self.line(&format!("mytemp options temp={}", self.num(t)));
        }
        if let Some(t) = tb.tnom {
            self.line(&format!("mytnom options tnom={}", self.num(t)));
        }
        for (word, vals) in [("ic", &tb.ics), ("nodeset", &tb.nodesets)] {
            for (node, v) in vals {
                let node = bare_node(&self.signal(node)).to_string();
                self.line(&format!("{word} {node}={}", self.num(*v)));
            }
        }
        for s in &tb.saves {
            // `i(V1)` is the source's current: its terminal, under the
            // name the instance is written with.
            let save = self.signal(s);
            let t = save.trim();
            let current = t
                .strip_prefix("i(")
                .or_else(|| t.strip_prefix("I("))
                .and_then(|r| r.strip_suffix(')'));
            let line = match current {
                Some(dev) => format!("save {}:p", self.inst_ref(dev.trim())),
                None => format!("save {}", bare_node(&save)),
            };
            self.line(&line);
        }
        // `.measure` is SPICE syntax: fenced by language switches.
        for (m, tok) in &tb.measures {
            self.ask(Construct::Measure, *tok);
            let body = m.trim();
            let body = match body.starts_with('.') {
                true => body.to_string(),
                false => format!(".measure {body}"),
            };
            self.line("simulator lang=spice");
            self.line(&body);
            self.line("simulator lang=spectre");
        }
        for l in &tb.extra {
            self.line(l);
        }
        self.line("");
        let mut block: Vec<String> = Vec::new();
        for (a, tok) in &tb.analyses {
            if let Some(s) = self.analysis(a, *tok) {
                block.extend(s.split('\n').map(str::to_string));
            }
        }
        // One `sweep` block per step parameter, the first outermost.
        for (idx, step) in tb.steps.iter().enumerate().rev() {
            let Some(head) = self.sweep_head(idx, step) else {
                continue;
            };
            let mut wrapped = vec![head];
            wrapped.extend(block.into_iter().map(|l| format!("  {l}")));
            wrapped.push("}".into());
            block = wrapped;
        }
        for l in &block {
            self.line(l);
        }
    }
}

impl W<'_, '_> {
    /// Notes what the testbench holds that neither target writes: each
    /// `.print`, Xyce's own options, the save list where the target saves
    /// every node.
    pub(super) fn unwritten_outputs(&mut self, tb: &Tb) {
        for &tok in &tb.prints {
            self.ask(Construct::Print, tok);
        }
        for &tok in &tb.xyce {
            self.note(Code::W0024, tok);
        }
        if let Some(tok) = tb.save_tok {
            self.ask(Construct::Save, tok);
        }
    }

    /// The deck's testbench items, folded in read order.
    pub(super) fn fold(&self) -> Tb {
        let mut tb = Tb::default();
        for &b in &self.deck.bench {
            let tok = match b {
                Bench::Card(n)
                | Bench::Meas(n)
                | Bench::Save(n)
                | Bench::Analysis(n)
                | Bench::Options(n)
                | Bench::NodeVals(n)
                | Bench::Control(n) => self.rd.main(n),
            };
            for card in self.rd.bench_cards(b) {
                match card {
                    BenchCard::Ana(a) => tb.analyses.push((a, tok)),
                    BenchCard::Options(opts) => {
                        for o in opts {
                            match o {
                                Opt::Portable(k, v) => tb.options.push((k, v, tok)),
                                Opt::Temp(t) => tb.temp = Some(t),
                                Opt::Tnom(t) => tb.tnom = Some(t),
                                Opt::Vacask(k, v) => tb.vacask.push((k, v, tok)),
                                Opt::Xyce(..) => {
                                    if !tb.xyce.contains(&tok) {
                                        tb.xyce.push(tok);
                                    }
                                }
                            }
                        }
                    }
                    BenchCard::Saves(s) => {
                        tb.save_tok.get_or_insert(tok);
                        tb.saves.extend(s);
                    }
                    BenchCard::Step { sweep, param, nums } => {
                        tb.steps.push((sweep, param, nums, tok))
                    }
                    BenchCard::Temp(t) => tb.temp = Some(t),
                    BenchCard::NodeVals(true, v) => tb.ics.extend(v),
                    BenchCard::NodeVals(false, v) => tb.nodesets.extend(v),
                    // `Construct::Print` is `Spell(Omit)` here (a quirk):
                    // asked, and noted, by [`W::unwritten_outputs`].
                    BenchCard::Print(..) => tb.prints.push(tok),
                    BenchCard::Meas(m) => tb.measures.push((m, tok)),
                    BenchCard::Extra(l) => tb.extra.push(l),
                }
            }
        }
        tb
    }
}

/// A SPICE signal (`V(out)`, `i(v1)`) as the bare node or device name;
/// other text trimmed (`[REF]` p.502: `nodeset 7=0 out=1`).
pub(super) fn bare_node(s: &str) -> &str {
    let t = s.trim();
    let inner = ["V(", "v(", "I(", "i("]
        .into_iter()
        .find_map(|p| t.strip_prefix(p))
        .and_then(|r| r.strip_suffix(')'));
    inner.map_or(t, str::trim)
}

// ---- private ----

impl W<'_, '_> {
    /// A signal (`v(x)`, `i(x)`, bare `x`), the name inside its wrapper in
    /// Spectre's spelling.
    fn signal(&self, s: &str) -> String {
        let t = s.trim();
        let wrapped = t
            .split_once('(')
            .filter(|(w, r)| matches!(*w, "v" | "V" | "i" | "I") && r.ends_with(')'));
        match wrapped {
            Some((w, r)) => format!("{w}({})", self.node(&r[..r.len() - 1])),
            None => self.node(t),
        }
    }

    /// `a`'s statement (several lines `\n`-joined where Spectre needs a
    /// companion analysis, or one statement per output); `None` when it is
    /// refused.
    fn analysis(&mut self, a: &Ana, tok: u32) -> Option<String> {
        let n = |w: &Self, v: f64| w.num(v);
        Some(match a {
            // `[REF]`: `dc1 dc`, a `dc` with no sweep is the operating point.
            Ana::Op => "op1 dc".into(),
            // A second sweep needs a `sweep` block around the first.
            Ana::Dc(sweeps) if sweeps.len() > 1 => {
                self.refuse(Code::W0015, tok);
                return None;
            }
            Ana::Dc(sweeps) => match sweeps.first() {
                Some((src, a, b, c)) => format!(
                    "dc1 dc {} start={} stop={} step={}",
                    dc_selector(src),
                    n(self, *a),
                    n(self, *b),
                    n(self, *c)
                ),
                None => "dc1 dc".into(),
            },
            Ana::Ac(sw) => format!(
                "ac1 ac start={} stop={} {}",
                n(self, sw.start),
                n(self, sw.stop),
                self.points(sw, tok)?
            ),
            // `step` (#9) is the closest analogue of SPICE's tstep.
            Ana::Tran {
                step,
                stop,
                start,
                max_step,
                uic,
            } => {
                let mut s = format!("tran1 tran step={} stop={}", n(self, *step), n(self, *stop));
                if let Some(st) = start {
                    s.push_str(&format!(" start={}", n(self, *st)));
                }
                if let Some(ms) = max_step {
                    s.push_str(&format!(" maxstep={}", n(self, *ms)));
                }
                if *uic {
                    s.push_str(" skipdc=yes");
                }
                s
            }
            // The output is the terminal list; `iprobe` names the input
            // source, a component.
            Ana::Noise {
                output,
                reference,
                source,
                sweep,
                ..
            } => {
                let neg = match reference.is_empty() {
                    true => "0",
                    false => bare_node(reference),
                };
                format!(
                    "noise1 ({} {neg}) noise start={} stop={} {} iprobe={}",
                    bare_node(output),
                    n(self, sweep.start),
                    n(self, sweep.stop),
                    self.points(sweep, tok)?,
                    self.inst_ref(source)
                )
            }
            // INFERRED (the old writer's choice): SPICE `.tf` is a DC
            // transfer function, Spectre's xf a small-signal one at `freq=0`.
            Ana::Tf(out, _) => format!("xf1 ({} 0) xf freq=0", bare_node(out)),
            // `sens` is a control statement over a named analysis, which is
            // written too.
            Ana::Sens(out, Some(sw)) => format!(
                "sensac1 ac start={} stop={} {}\nsens ({}) for (sensac1)",
                n(self, sw.start),
                n(self, sw.stop),
                self.points(sw, tok)?,
                bare_node(out)
            ),
            Ana::Sens(out, None) => format!("sensdc1 dc\nsens ({}) for (sensdc1)", bare_node(out)),
            Ana::Sp(sw) => format!(
                "sp1 sp start={} stop={} {}",
                n(self, sw.start),
                n(self, sw.stop),
                self.points(sw, tok)?
            ),
            // `fourier` is a component, one per output.
            Ana::Four(f, outs) => outs
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    format!(
                        "four{} ({} 0) fourier fund={}",
                        i + 1,
                        bare_node(o),
                        n(self, *f)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Ana::Pz(..) => {
                self.ask(Construct::PoleZero, tok);
                return None;
            }
        })
    }

    /// `dec=10`: a sweep's points (Spectre has no `oct`).
    fn points(&mut self, sw: &Sweep, tok: u32) -> Option<String> {
        match sw.variation {
            "oct" => {
                self.refuse(Code::W0015, tok);
                None
            }
            v => Some(format!("{v}={}", sw.points)),
        }
    }

    /// `sweep<k>_<name> sweep param=p start= stop= step= {` for `.step`
    /// `idx`; `None` when it is refused.
    fn sweep_head(
        &mut self,
        idx: usize,
        (sweep, param, [start, stop, step], tok): &Step,
    ) -> Option<String> {
        let suffix: String = param
            .chars()
            .map(|c| match c.is_ascii_alphanumeric() {
                true => c.to_ascii_lowercase(),
                false => '_',
            })
            .collect();
        let by = match sweep {
            Some("dec") => "dec",
            Some("log") => "log",
            Some("oct") => {
                self.refuse(Code::W0016, *tok);
                return None;
            }
            _ => "step",
        };
        Some(format!(
            "sweep{}_{suffix} sweep param={param} start={} stop={} {by}={} {{",
            idx + 1,
            self.num(*start),
            self.num(*stop),
            self.num(*step)
        ))
    }
}

/// SPICE `.dc Vin 0 5 0.1` sweeps a source's dc value: `dev=vin
/// param=dc`; `temp` is `param=temp`; anything else a netlist parameter
/// (`[REF]` p.64).
fn dc_selector(source: &str) -> String {
    let lower = source.to_lowercase();
    if lower == "temp" {
        return "param=temp".into();
    }
    match lower.as_bytes().first() {
        Some(b'v' | b'i') => format!("dev={lower} param=dc"),
        _ => format!("param={source}"),
    }
}
