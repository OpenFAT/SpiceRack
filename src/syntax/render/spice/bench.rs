//! The testbench → SPICE-family cards: the deck's testbench statements
//! folded in read order (options merged into one card, the last `.temp`
//! winning), then options, temperature, initial conditions, saves, prints,
//! measures, `.step`s and the analyses, which an ngspice deck with a
//! `.meas` or `.four` runs from one `.control` block.
//!
//! Cites: ngspice manual §11.1-§11.3 (analyses), §15.4.3 (`.save`), §15.6
//! (`.meas`), §15.4.2 (`.four`, the `fourier` command), §17.5 (`-b -r`:
//! "No .measure possible in batch mode"); HSPICE `.probe`, `.option post`,
//! `.option delmax`, `START=`; Xyce RG pp.96-104 (`.options <pkg>`), p.155
//! (`.step`), Table 6-1; LTspice help (`.step param`, no `.print`).
//!
//! Contents:
//! - `fn is_four(line: &str) -> bool`, `fn is_continuation(line: &str) -> bool`
//! - `pub(super) struct Tb`, `W::fold(&self) -> Tb`
//! - `W::control_run(&self) -> Option<Vec<String>>`, `W::bench(&mut self, control: Option<&[String]>)`, `W::runs(&self) -> bool`

use super::W;
use crate::codegen::Dialect;
use crate::syntax::render::capability::{Action, Alt, Construct};
use crate::syntax::render::out::Space;
use crate::syntax::render::read::{Ana, Bench, BenchCard, Opt};
use crate::syntax::render::spelling::spelling;

/// The testbench, folded from its statements in read order.
#[derive(Default)]
pub(super) struct Tb {
    /// Each analysis, with its statement's first token.
    analyses: Vec<(Ana, u32)>,
    portable: Vec<(String, String)>,
    xyce: Vec<(String, String)>,
    temp: Option<f64>,
    tnom: Option<f64>,
    ics: Vec<(String, f64)>,
    nodesets: Vec<(String, f64)>,
    saves: Vec<String>,
    prints: Vec<(String, Vec<String>)>,
    measures: Vec<String>,
    /// Each `.step`'s sweep, name, numbers and first token.
    steps: Vec<(Option<&'static str>, String, [f64; 3], u32)>,
    /// VACASK control lines kept as text.
    pub(super) extra: Vec<String>,
}

/// Control commands that start an analysis: a `.control` block issuing one
/// owns the run.
const RUN_COMMANDS: [&str; 5] = ["run", "op", "ac", "tran", "dc"];

/// True if `line` is a `.four` card (any case).
pub(super) fn is_four(line: &str) -> bool {
    line.split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case(".four"))
}

/// True if `line` continues the card above it (`+ ...`).
pub(super) fn is_continuation(line: &str) -> bool {
    line.trim_start().starts_with('+')
}

impl W<'_, '_> {
    /// Whether an ngspice deck runs its analyses from a `.control` block,
    /// and the `fourier` commands to run there, one per `.four`.
    ///
    /// ngspice refuses `.measure` and ignores `.four` under `-b -r`, so a
    /// deck with either (a card or a raw line) runs every analysis from one
    /// block, in card order, where `.four` is the `fourier` command. `None`
    /// for other targets, when nothing needs a block, the deck has no
    /// `.ac`/`.tran`/`.dc` for a `.measure` to read, or a raw control block
    /// already runs the deck.
    pub(super) fn control_run(&self) -> Option<Vec<String>> {
        if self.d != Dialect::Ngspice {
            return None;
        }
        let tb = self.fold();
        let mut lines: Vec<String> = Vec::new();
        for b in self.deck.defs.iter().chain(std::iter::once(&self.deck.top)) {
            lines.extend(self.raw_lines(&b.raw));
        }
        lines.extend(tb.extra.iter().cloned());
        let raw = || lines.iter().map(String::as_str);
        let mut fourier = fourier_commands(raw());
        for (a, _) in &tb.analyses {
            if let Ana::Four(f, outs) = a {
                let mut cmd = format!("fourier {}", spelling(Dialect::Ngspice).number(*f, self.x));
                outs.iter().for_each(|o| cmd.push_str(&format!(" {o}")));
                fourier.push(cmd);
            }
        }
        if (tb.measures.is_empty() && !raw().any(is_meas) && fourier.is_empty())
            || runs_in_control(raw())
        {
            return None;
        }
        tb.analyses
            .iter()
            .any(|(a, _)| matches!(a, Ana::Ac(_) | Ana::Tran { .. } | Ana::Dc(_)))
            .then_some(fourier)
    }

    /// The deck's testbench statements, folded in read order.
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
                self.fold_one(&mut tb, card, tok);
            }
        }
        tb
    }

    /// Whether the deck runs an analysis.
    pub(super) fn runs(&self) -> bool {
        !self.fold().analyses.is_empty()
    }

    /// The testbench's cards.
    pub(super) fn bench(&mut self, control: Option<&[String]>) {
        let tb = self.fold();
        if self.d == Dialect::Xyce {
            for l in self.xyce_options(&tb) {
                self.o.token(&l, Space::Line);
            }
        } else {
            let opts: Vec<String> = tb
                .portable
                .iter()
                .map(|(k, v)| {
                    let k = if k == "max_iterations" { "ITL1" } else { k };
                    if v.is_empty() {
                        k.to_string()
                    } else {
                        format!("{k}={v}")
                    }
                })
                .collect();
            if !opts.is_empty() {
                self.o
                    .token(&format!(".options {}", opts.join(" ")), Space::Line);
            }
            if let Some(t) = tb.temp {
                let t = self.sp.decimal(t, self.x);
                self.o.token(&format!(".temp {t}"), Space::Line);
            }
            if let Some(t) = tb.tnom {
                let t = self.sp.decimal(t, self.x);
                self.o.token(&format!(".options tnom={t}"), Space::Line);
            }
        }
        for (n, v) in &tb.ics {
            let v = self.sp.decimal(*v, self.x);
            self.o.token(&format!(".ic V({n})={v}"), Space::Line);
        }
        for (n, v) in &tb.nodesets {
            let v = self.sp.decimal(*v, self.x);
            self.o.token(&format!(".nodeset V({n})={v}"), Space::Line);
        }
        self.saves(&tb);
        if self.ask(Construct::Print, 0) == Action::Native {
            for (kind, outs) in &tb.prints {
                let sep = if kind.is_empty() { "" } else { " " };
                let line = format!(".print{sep}{kind} {}", outs.join(" "));
                self.o.token(&line, Space::Line);
            }
        }
        for m in &tb.measures {
            // `.MEASURE TRAN ...` is already a card, in any case.
            let card = m.get(..5).is_some_and(|p| p.eq_ignore_ascii_case(".meas"));
            let line = format!("{}{m}", if card { "" } else { ".meas " });
            self.o.token(&line, Space::Line);
        }
        for (sweep, param, [a, b, c], tok) in &tb.steps {
            if self.ask(Construct::Step, *tok) != Action::Native {
                continue;
            }
            let line = match (self.d, sweep) {
                (Dialect::Xyce, Some(s)) => format!(".step {s} {param} {a} {b} {c}"),
                (Dialect::Xyce, None) => format!(".step {param} {a} {b} {c}"),
                (_, Some(s)) => format!(".step {s} param {param} {a} {b} {c}"),
                (_, None) => format!(".step param {param} {a} {b} {c}"),
            };
            self.o.token(&line, Space::Line);
        }
        self.write_raw(&tb.extra);
        match control {
            Some(fourier) => {
                // `fourier` reads the current plot: it follows the first
                // transient (at the end of the block when there is none).
                let at = tb
                    .analyses
                    .iter()
                    .position(|(a, _)| matches!(a, Ana::Tran { .. }));
                self.o.token(".control", Space::Line);
                for (i, (a, tok)) in tb.analyses.iter().enumerate() {
                    // As a command, `.four` would run the transient again.
                    if !matches!(a, Ana::Four(..)) {
                        let card = self.analysis(a, *tok);
                        self.o.token(card.trim_start_matches('.'), Space::Line);
                    }
                    if at == Some(i) {
                        fourier.iter().for_each(|f| self.o.token(f, Space::Line));
                    }
                }
                if at.is_none() {
                    fourier.iter().for_each(|f| self.o.token(f, Space::Line));
                }
                self.o.token(".endc", Space::Line);
            }
            None => {
                for (a, tok) in &tb.analyses {
                    let card = self.analysis(a, *tok);
                    self.o.token(&card, Space::Line);
                }
            }
        }
    }
}

// ---- private ----

impl W<'_, '_> {
    /// One testbench card into `tb`.
    fn fold_one(&self, tb: &mut Tb, card: BenchCard, tok: u32) {
        {
            match card {
                BenchCard::Ana(a) => tb.analyses.push((a, tok)),
                BenchCard::Options(opts) => {
                    for o in opts {
                        match o {
                            Opt::Portable(k, v) => tb.portable.push((k, v)),
                            Opt::Xyce(k, v) => tb.xyce.push((k, v)),
                            Opt::Temp(t) => tb.temp = Some(t),
                            Opt::Tnom(t) => tb.tnom = Some(t),
                            Opt::Vacask(..) => {}
                        }
                    }
                }
                BenchCard::Saves(s) => tb.saves.extend(s),
                BenchCard::Step { sweep, param, nums } => tb.steps.push((sweep, param, nums, tok)),
                BenchCard::Temp(t) => tb.temp = Some(t),
                BenchCard::NodeVals(true, v) => tb.ics.extend(v),
                BenchCard::NodeVals(false, v) => tb.nodesets.extend(v),
                BenchCard::Print(k, outs) => tb.prints.push((k, outs)),
                BenchCard::Meas(m) => tb.measures.push(m),
                BenchCard::Extra(l) => tb.extra.push(l),
            }
        }
    }

    /// The save list. A noise run produces `onoise_spectrum`, not node
    /// voltages, so a node list can never name it: ngspice then saves all
    /// (and the names beside it), the others none. HSPICE's `.save` saves
    /// simulator state; `.probe` selects outputs. Xyce's rawfile holds
    /// every vector.
    fn saves(&mut self, tb: &Tb) {
        let noise = tb
            .analyses
            .iter()
            .any(|(a, _)| matches!(a, Ana::Noise { .. }));
        let ngspice = matches!(self.d, Dialect::Ngspice | Dialect::Espice);
        match self.ask(Construct::Save, 0) {
            Action::Spell(Alt::Omit) => {}
            Action::Spell(Alt::Probe) if !noise => {
                for s in &tb.saves {
                    let line = match s.as_str() {
                        "all" => ".option post".to_string(),
                        s => format!(".probe {s}"),
                    };
                    self.o.token(&line, Space::Line);
                }
            }
            _ if ngspice && noise => {
                let mut line = String::from(".save all");
                for s in tb.saves.iter().filter(|s| *s != "all") {
                    line.push_str(&format!(" {s}"));
                }
                self.o.token(&line, Space::Line);
            }
            _ if !tb.saves.is_empty() && !noise => {
                self.o
                    .token(&format!(".save {}", tb.saves.join(" ")), Space::Line);
            }
            _ => {}
        }
    }

    /// `a`'s card (HSPICE's `.tran` with a step cap and `.noise` take two
    /// lines); an analysis the target lacks is refused, written empty.
    fn analysis(&mut self, a: &Ana, tok: u32) -> String {
        let num = |w: &Self, v: f64| w.sp.number(v, w.x);
        let hs = self.d == Dialect::Hspice;
        let sweep = |w: &Self, s: &crate::syntax::render::read::Sweep| {
            format!(
                "{} {} {} {}",
                s.variation,
                s.points,
                num(w, s.start),
                num(w, s.stop)
            )
        };
        match a {
            Ana::Op => ".op".into(),
            Ana::Dc(sweeps) => {
                let mut s = String::from(".dc");
                for (src, a, b, c) in sweeps {
                    s.push_str(&format!(
                        " {src} {} {} {}",
                        num(self, *a),
                        num(self, *b),
                        num(self, *c)
                    ));
                }
                s
            }
            Ana::Ac(sw) => format!(".ac {}", sweep(self, sw)),
            // HSPICE reads extra positional fields as further step/stop
            // segments, so the start is `START=` and the step cap `delmax`.
            Ana::Tran {
                step,
                stop,
                start,
                max_step,
                uic,
            } if hs => {
                let mut s = String::new();
                if let Some(ms) = max_step {
                    s.push_str(&format!(".option delmax={}\n", num(self, *ms)));
                }
                s.push_str(&format!(".tran {} {}", num(self, *step), num(self, *stop)));
                if let Some(st) = start {
                    s.push_str(&format!(" START={}", num(self, *st)));
                }
                if *uic {
                    s.push_str(" UIC");
                }
                s
            }
            Ana::Tran {
                step,
                stop,
                start,
                max_step,
                uic,
            } => {
                let mut s = format!(".tran {} {}", num(self, *step), num(self, *stop));
                if let Some(st) = start {
                    s.push_str(&format!(" {}", num(self, *st)));
                }
                if let Some(ms) = max_step {
                    if start.is_none() {
                        s.push_str(" 0");
                    }
                    s.push_str(&format!(" {}", num(self, *ms)));
                }
                if *uic {
                    s.push_str(" UIC");
                }
                s
            }
            Ana::Noise {
                output,
                reference,
                source,
                sweep: sw,
                pps,
            } => {
                let out = if reference.is_empty() || reference == "0" {
                    format!("V({output})")
                } else {
                    format!("V({output},{reference})")
                };
                // HSPICE: `.noise ov src inter` takes its frequencies from
                // `.ac`.
                if hs {
                    return format!(
                        ".ac {}\n.noise {out} {source} {}",
                        sweep(self, sw),
                        pps.unwrap_or(0)
                    );
                }
                let mut s = format!(".noise {out} {source} {}", sweep(self, sw));
                // `pts_per_summary` is an ngspice/SPICE3 extension; LTspice's
                // `.noise` card stops at the stop frequency.
                let ng = matches!(self.d, Dialect::Ngspice | Dialect::Espice | Dialect::Xyce);
                if let (Some(p), true) = (pps, ng) {
                    s.push_str(&format!(" {p}"));
                }
                s
            }
            Ana::Tf(out, src) => match self.ask(Construct::Tf, tok) {
                Action::Native => format!(".tf {out} {src}"),
                _ => String::new(),
            },
            Ana::Four(f, outs) => {
                let mut s = format!(".four {}", num(self, *f));
                for o in outs {
                    s.push_str(&format!(" {o}"));
                }
                s
            }
            Ana::Sens(out, ac) => {
                let c = if ac.is_some() {
                    Construct::SensAc
                } else {
                    Construct::Sens
                };
                if self.ask(c, tok) != Action::Native {
                    return String::new();
                }
                let mut s = format!(".sens {out}");
                if let Some(sw) = ac {
                    s.push_str(&format!(" AC {}", sweep(self, sw)));
                }
                s
            }
            Ana::Pz(nodes, tf, pz) => match self.ask(Construct::PoleZero, tok) {
                Action::Native => format!(".pz {} {tf} {pz}", nodes.join(" ")),
                _ => String::new(),
            },
            Ana::Sp(sw) => match self.ask(Construct::SParameter, tok) {
                // ponytail: Rust `{}` numbers, byte for byte what the old
                // writer sent ngspice for this card.
                Action::Native => format!(
                    ".sp {} {} {} {}",
                    sw.variation, sw.points, sw.start, sw.stop
                ),
                _ => String::new(),
            },
        }
    }

    /// Xyce's options: `.options <pkg> k=v` per package. Xyce's own keep
    /// their `pkg.key`; portable ones map through the RG tables, and one
    /// with no Xyce spelling is refused. Consecutive entries of one package
    /// share a line, so a deck's own order reads back unchanged.
    fn xyce_options(&mut self, tb: &Tb) -> Vec<String> {
        let mut items: Vec<(String, String)> = Vec::new();
        let kv = |k: &str, v: &str| {
            if v.is_empty() {
                k.to_string()
            } else {
                format!("{k}={v}")
            }
        };
        for (k, v) in &tb.portable {
            let (pkg, key) = match k.to_ascii_lowercase().as_str() {
                "reltol" => ("timeint", "reltol"),
                "vntol" => ("nonlin", "abstol"),
                "gmin" => ("device", "gmin"),
                "tnom" => ("device", "tnom"),
                "temp" => ("device", "temp"),
                "itl1" | "max_iterations" => ("nonlin", "maxstep"),
                "itl4" => ("nonlin-tran", "maxstep"),
                _ => {
                    self.ask(Construct::UnmappedOption, 0);
                    continue;
                }
            };
            items.push((pkg.into(), kv(key, v)));
        }
        let xyce = spelling(Dialect::Xyce);
        if let Some(t) = tb.temp {
            items.push(("device".into(), kv("temp", &xyce.number(t, false))));
        }
        if let Some(t) = tb.tnom {
            items.push(("device".into(), kv("tnom", &xyce.number(t, false))));
        }
        for (pk, v) in &tb.xyce {
            match pk.split_once('.') {
                Some((pkg, "")) => items.push((pkg.into(), String::new())),
                Some((pkg, k)) => items.push((pkg.into(), kv(k, v))),
                None => items.push((String::new(), kv(pk, v))),
            }
        }
        let mut out: Vec<(String, String)> = Vec::new();
        for (pkg, item) in items {
            match out.last_mut() {
                // A bare package (`.options linsol`) keeps its own line.
                Some((p, line)) if *p == pkg && !item.is_empty() && line != p => {
                    line.push(' ');
                    line.push_str(&item);
                }
                _ => out.push((
                    pkg.clone(),
                    if pkg.is_empty() {
                        item
                    } else if item.is_empty() {
                        pkg
                    } else {
                        format!("{pkg} {item}")
                    },
                )),
            }
        }
        out.into_iter()
            .map(|(_, l)| format!(".options {l}"))
            .collect()
    }
}

/// True if a `.control`/`.endc` block among `lines` issues a run command.
fn runs_in_control<'a>(lines: impl IntoIterator<Item = &'a str>) -> bool {
    let mut inside = false;
    for line in lines {
        let word = line.split_whitespace().next().unwrap_or("");
        if word.eq_ignore_ascii_case(".control") {
            inside = true;
        } else if word.eq_ignore_ascii_case(".endc") {
            inside = false;
        } else if inside && RUN_COMMANDS.iter().any(|c| word.eq_ignore_ascii_case(c)) {
            return true;
        }
    }
    false
}

/// True if `line` is a `.meas`/`.measure` card (any case).
fn is_meas(line: &str) -> bool {
    line.split_whitespace()
        .next()
        .and_then(|w| w.get(..5))
        .is_some_and(|w| w.eq_ignore_ascii_case(".meas"))
}

/// The `.four` cards among `lines`, each joined with its `+` continuation
/// lines, as `fourier` commands.
fn fourier_commands<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut open = false;
    for l in lines {
        if is_continuation(l) {
            if let (true, Some(cmd)) = (open, out.last_mut()) {
                l.trim_start()[1..]
                    .split_whitespace()
                    .for_each(|t| cmd.push_str(&format!(" {t}")));
            }
            continue;
        }
        open = is_four(l);
        if open {
            let mut cmd = String::from("fourier");
            l.split_whitespace()
                .skip(1)
                .for_each(|t| cmd.push_str(&format!(" {t}")));
            out.push(cmd);
        }
    }
    out
}
