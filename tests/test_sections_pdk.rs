//! Real PDK model files through library sections: each read, written back
//! in its own dialect and run.
//!
//! Oracles: a line scan of each file for its section names; read -> write ->
//! read -> write as a text fixpoint; and the real simulator running a corner from the file
//! SpiceRack wrote against the same corner from the original, bit for bit:
//! ngspice on the pinned gf180mcu PDK (`$PDK_ROOT`), VACASK on its own corner
//! library (`$VACASK_CORPUS`). sky130's is read only (see its test).
//! Unsupported fails, never skips: run in `nix develop .#testing`.

use std::path::Path;

use spicerack::syntax::Dialect;
use spicerack::syntax::diag::Code;
use spicerack::syntax::render::{Written, render_text};
use spicerack::syntax::resolve::{Form, Role};

#[path = "common/sections.rs"]
mod sections;
use sections::meaning;

// ---- private ----

/// gf180mcu's model file read and written back by SpiceRack: every `.lib`
/// definition is a section (a line scan finds the same names), nothing is
/// kept as text (each `.param` in a `.subckt` is a local), nothing lands in
/// the top, and ngspice runs an inverter on the `typical` corner from the
/// written file to the same output voltage as from the PDK's own.
#[test]
fn gf180mcu_model_file_round_trips_through_sections() {
    let root = std::env::var("PDK_ROOT")
        .expect("PDK_ROOT not set: run the tests in `nix develop .#testing`");
    let pdk = Path::new(&root).join("gf180mcuD/libs.tech/ngspice");
    let src = std::fs::read_to_string(pdk.join("sm141064.ngspice")).unwrap();
    let w = written(&src, Dialect::Ngspice, "sm141064", Dialect::Ngspice);
    assert_eq!(codes(&w), [] as [Code; 0], "{}", w.notes.messages);
    // The `.param`s inside its `.subckt`s are the definitions' locals: as
    // many in the written file as in the source (one a card there).
    let locals = |text: &str| {
        let mut inside = false;
        let mut n = 0;
        for l in text.lines() {
            let l = l.trim_start().to_ascii_lowercase();
            inside = (inside || l.starts_with(".subckt")) && !l.starts_with(".ends");
            if inside && l.starts_with(".param") {
                n += split_params(&l[6..]);
            }
        }
        n
    };
    assert_eq!(locals(&w.text), locals(&src));
    let scanned: Vec<&str> = src
        .lines()
        .filter_map(|l| l.strip_prefix(".lib "))
        .map(str::trim)
        .filter(|w| !w.contains(char::is_whitespace))
        .collect();
    let read: Vec<String> = meaning(&w.text).into_iter().map(|m| m.name).collect();
    assert_eq!(read, scanned);
    assert!(
        w.text.starts_with("* sm141064\n\n.lib "),
        "nothing at the top"
    );
    let text = fixpoint(&src, Dialect::Ngspice);

    // The sections call `.lib 'sm141064.spice' <section>`: that name is the
    // written file; every other file is the PDK's own.
    let dir = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(&pdk).unwrap().flatten() {
        std::os::unix::fs::symlink(e.path(), dir.path().join(e.file_name())).unwrap();
    }
    for f in ["sm141064.spice", "sm141064.ngspice"] {
        std::fs::remove_file(dir.path().join(f)).unwrap();
        std::fs::write(dir.path().join(f), &text).unwrap();
    }
    let vout = |lib: &Path| {
        let deck = format!(
            "inverter\n.include '{}'\n.lib '{}' typical\nXp vout vin vdd vdd pfet_03v3 w=2e-6 l=0.28e-6\nXn vout vin 0 0 nfet_03v3 w=1e-6 l=0.28e-6\nVdd vdd 0 3.3\nVin vin 0 1.5\nRl vout 0 10k\n.op\n.end\n",
            lib.join("design.ngspice").display(),
            lib.join("sm141064.ngspice").display()
        );
        let raw = Dialect::Ngspice
            .run(&deck)
            .unwrap_or_else(|e| panic!("{e}\n{deck}"));
        let i = raw
            .variables
            .iter()
            .position(|v| {
                v.name.eq_ignore_ascii_case("v(vout)") || v.name.eq_ignore_ascii_case("vout")
            })
            .expect("vout");
        raw.real_data[i][0]
    };
    let (want, got) = (vout(&pdk), vout(dir.path()));
    assert!(want > 0.0 && want < 3.3, "{want}");
    assert_eq!(got.to_bits(), want.to_bits(), "{got} != {want}");
}

/// gf180mcu's model files, SPICE -> Spectre -> SPICE, a section at a
/// time: each section either crosses with the same meaning ([`meaning`]: a
/// definition's locals are Spectre `parameters`, which an instance could
/// set; numbers respelled, SPICE quotes dropped) or is refused for its one
/// named reason, a `.model` kind with no Spectre master mapped (`W0104`: an
/// `r`, a `c`, a level-3 `d`, which a line scan finds in exactly those
/// sections; `W0006` on each R or C card naming one). The mismatch and statistical corners cross, their `agauss`
/// calls lifted and read back (tests/test_monte_carlo.rs).
#[test]
fn gf180mcu_sections_cross_to_spectre() {
    let root = std::env::var("PDK_ROOT")
        .expect("PDK_ROOT not set: run the tests in `nix develop .#testing`");
    let pdk = Path::new(&root).join("gf180mcuD/libs.tech/ngspice");
    let (mut crossed, mut refused) = (Vec::new(), Vec::new());
    for f in [
        "sm141064.ngspice",
        "sm141064_mim.ngspice",
        "smbb000149.ngspice",
    ] {
        let src = std::fs::read_to_string(pdk.join(f)).unwrap();
        for (name, block) in sections(&src) {
            let one = format!("*\n{block}");
            let scs = match render_text(&one, Dialect::Ngspice, "gf180", Dialect::Spectre) {
                Ok(w) => w.text,
                Err(e) => {
                    let got: Vec<Code> = e.diags.iter().map(|d| d.code).collect();
                    // The model, and each R or C card naming it (`W0006`).
                    assert!(
                        got.contains(&Code::W0104)
                            && got.iter().all(|&c| c == Code::W0104 || c == Code::W0006),
                        "{f} {name}: {}",
                        e.messages
                    );
                    refused.push(format!("{f}:{name}"));
                    continue;
                }
            };
            let back = written(&scs, Dialect::Spectre, "gf180", Dialect::Ngspice);
            assert_eq!(codes(&back), [] as [Code; 0], "{f} {name}:\n{scs}");
            let spice = written(&one, Dialect::Ngspice, "gf180", Dialect::Ngspice);
            assert_eq!(
                meaning(&back.text),
                meaning(&spice.text),
                "{f} {name}:\n{scs}"
            );
            crossed.push(format!("{f}:{name}"));
        }
    }
    // The refused sections are those holding a `.model` of kind r, c, or d
    // at level 3 (a line scan of each section).
    let mut unmapped = Vec::new();
    for f in [
        "sm141064.ngspice",
        "sm141064_mim.ngspice",
        "smbb000149.ngspice",
    ] {
        let src = std::fs::read_to_string(pdk.join(f)).unwrap();
        for (name, block) in sections(&src) {
            // Each `.model` card joined with its `+` lines.
            let mut cards: Vec<String> = Vec::new();
            for l in block.lines().map(str::to_ascii_lowercase) {
                match (l.trim_start().strip_prefix('+'), cards.last_mut()) {
                    (Some(more), Some(c)) => c.push_str(more),
                    _ => cards.push(l),
                }
            }
            let none = cards.iter().any(|c| {
                let w: Vec<&str> = c.split_whitespace().collect();
                w.first() == Some(&".model")
                    && w.get(2).is_some_and(|&k| {
                        k == "r" || k == "c" || k == "d" && c.replace(' ', "").contains("level=3")
                    })
            });
            if none {
                unmapped.push(format!("{f}:{name}"));
            }
        }
    }
    assert!(!refused.is_empty());
    assert_eq!(refused, unmapped);
    for name in [
        "sm141064.ngspice:statistical",
        "sm141064.ngspice:fets_mm",
        "sm141064.ngspice:bjt_mc",
    ] {
        assert!(
            crossed.iter().any(|c| c == name),
            "{name} did not cross: {crossed:?}"
        );
    }
    // The sections with subcircuits and models among those that cross.
    for name in [
        "sm141064.ngspice:nfet_06v0_nvt_t",
        "sm141064.ngspice:moscap",
        "sm141064.ngspice:efuse",
        "smbb000149.ngspice:pfet_10v0_asym_t",
    ] {
        assert!(
            crossed.iter().any(|c| c == name),
            "{name} did not cross: {crossed:?}"
        );
    }
}

/// sky130's corner file: every `.lib` definition is a section, none is
/// unparsed (each was, its cards leaking into the top), and it is a
/// fixpoint. Its sections only include other files; loading a corner takes
/// ngspice 34 s, so the simulation oracle is gf180mcu's.
#[test]
fn sky130_corner_file_reads_into_sections() {
    let root = std::env::var("PDK_ROOT")
        .expect("PDK_ROOT not set: run the tests in `nix develop .#testing`");
    let src = std::fs::read_to_string(
        Path::new(&root).join("sky130A/libs.tech/ngspice/sky130.lib.spice"),
    )
    .unwrap();
    let w = written(&src, Dialect::Ngspice, "sky130", Dialect::Ngspice);
    assert_eq!(codes(&w), [] as [Code; 0], "{}", w.notes.messages);
    let scanned: Vec<&str> = src
        .lines()
        .filter_map(|l| l.strip_prefix(".lib "))
        .map(str::trim)
        .collect();
    let m = meaning(&w.text);
    assert_eq!(
        m.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        scanned
    );
    assert!(
        w.text.starts_with("* sky130\n\n.lib "),
        "nothing at the top"
    );
    assert_eq!(
        (m[0].params.len(), m[0].includes.len()),
        (2, 4),
        "{:?}",
        m[0]
    );
    fixpoint(&src, Dialect::Ngspice);
}

/// sky130's combined library (`libs.tech/combined/sky130.lib.spice`) and
/// every file it reaches through `.include` and `.lib` (followed from the
/// IR): each reads with nothing unparsed and nothing raw, is a fixpoint,
/// and is written as ngspice and as HSPICE (no refusal) into two mirrors of
/// the tree. ngspice runs an NMOS on the `tt` corner to the same drain
/// current from the PDK's own files, the ngspice mirror and the HSPICE
/// mirror (read in its HSPICE mode, `hsa`), bit for bit.
///
/// Known gap: ESPice 0.3.0 reads the HSPICE mirror (`--tokenizer=hspice`)
/// and runs it, but to a current 5.1% above ngspice's (1.8498e-4 against
/// 1.7608e-4 A, measured; 2.3% at l=0.5); ngspice reading the same text
/// agrees bit for bit, so the gap is ESPice's reading, unexplained. The
/// test asserts the gap's band, so a fix fails it until this note goes.
#[test]
fn sky130_combined_library_converts_losslessly() {
    let root = std::env::var("PDK_ROOT")
        .expect("PDK_ROOT not set: run the tests in `nix develop .#testing`");
    let pdk = Path::new(&root).join("sky130A");
    let mirror = tempfile::tempdir().unwrap();
    let hspice = tempfile::tempdir().unwrap();
    let entry = Path::new("libs.tech/combined/sky130.lib.spice");
    // Every file the library reaches through `.include` and `.lib`, as a
    // kit reads them; each written alone (behind a dummy title line, as an
    // included file has none).
    let kit = spicerack::import::kit(&[pdk.join(entry)], Form::Library, Dialect::Ngspice, &pdk);
    let (mut seen, mut raw) = (Vec::new(), Vec::new());
    for f in kit.files() {
        assert!(
            matches!(f.role, Role::Library | Role::Deck),
            "{}: {:?}",
            f.path.display(),
            f.role
        );
        let rel = f.path.strip_prefix(&pdk).unwrap().to_path_buf();
        let w = written(&f.text, Dialect::Ngspice, "f", Dialect::Ngspice);
        raw.extend(
            w.notes
                .diags
                .iter()
                .filter(|d| d.code == Code::W0027 || format!("{:?}", d.code).starts_with('P'))
                .map(|d| format!("{}: {:?}", rel.display(), d.code)),
        );
        let text = fixpoint(&f.text, Dialect::Ngspice);
        let hs = render_text(&f.text, Dialect::Ngspice, "f", Dialect::Hspice)
            .unwrap_or_else(|e| panic!("{}: HSPICE: {}", rel.display(), e.messages))
            .text;
        for (root, text) in [(mirror.path(), text), (hspice.path(), hs)] {
            let out = root.join(&rel);
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            std::fs::write(out, text).unwrap();
        }
        seen.push(rel);
    }
    assert!(raw.is_empty(), "{raw:#?}");
    assert!(seen.len() > 100, "{} files reached", seen.len());
    let deck = |root: &Path| {
        format!(
            "sky130\n*spicerack ngbehavior=hsa\n.lib '{}' tt\nXn d g 0 0 sky130_fd_pr__nfet_01v8 w=1 l=0.15\nVd d 0 1.8\nVg g 0 1.2\n.op\n.end\n",
            root.join(entry).display()
        )
    };
    let id = |raw: spicerack::result::RawData| {
        let i = raw
            .variables
            .iter()
            .position(|v| {
                v.name.eq_ignore_ascii_case("vd#branch") || v.name.eq_ignore_ascii_case("i(vd)")
            })
            .unwrap_or_else(|| panic!("{:?}", raw.variables));
        -raw.real_data[i][0]
    };
    let ngspice = |root: &Path| {
        let deck = deck(root);
        id(Dialect::Ngspice
            .run(&deck)
            .unwrap_or_else(|e| panic!("{e}\n{deck}")))
    };
    let want = ngspice(&pdk);
    assert!(want > 1e-6, "{want}");
    for root in [mirror.path(), hspice.path()] {
        let got = ngspice(root);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "{}: {got} != {want}",
            root.display()
        );
    }
    // `SPICERACK_KEEP_HSPICE=<dir>` keeps the HSPICE tree (tests/espice/).
    if let Ok(keep) = std::env::var("SPICERACK_KEEP_HSPICE") {
        for rel in &seen {
            let to = Path::new(&keep).join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(hspice.path().join(rel), to).unwrap();
        }
    }
    let espice = id(espice_hspice(&deck(hspice.path())));
    let gap = espice / want - 1.0;
    assert!(
        (0.04..0.06).contains(&gap),
        "ESPice {espice} against ngspice {want}: {gap:+.4}"
    );
}

/// VACASK's own corner library (`demo/cmos180n.lib` in `$VACASK_CORPUS`):
/// every `section` is read (a line scan finds the same names), it is a
/// fixpoint, and VACASK runs an NMOS on the `tm` corner
/// from the written file to the same drain current as from the original.
#[test]
fn vacask_library_round_trips_through_sections() {
    let root = std::env::var("VACASK_CORPUS")
        .expect("VACASK_CORPUS not set: run the tests in `nix develop .#testing`");
    let lib = Path::new(&root).join("demo/cmos180n.lib");
    let src = std::fs::read_to_string(&lib).unwrap();
    let w = written(&src, Dialect::Vacask, "cmos180n", Dialect::Vacask);
    // `*` lines between the sections: VACASK skips everything outside a
    // section while it looks for one (lib/dfllexer.l, `SECLOOK`), so these
    // are text no VACASK statement reads, kept as written.
    let unread = codes(&w).iter().filter(|&&c| c == Code::P0001).count();
    assert_eq!(unread, 5, "{}", w.notes.messages);
    for l in [
        "* Worst power",
        "* Worst speed",
        "* Worst one",
        "* Worst zero",
        "* Model definition",
    ] {
        assert!(w.text.lines().any(|t| t.trim() == l), "{l}\n{}", w.text);
    }
    let scanned: Vec<&str> = src
        .lines()
        .filter_map(|l| l.trim().strip_prefix("section "))
        .map(str::trim)
        .collect();
    let read: Vec<&str> = w
        .text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("section "))
        .map(str::trim)
        .collect();
    assert_eq!(read, scanned);
    let text = fixpoint(&src, Dialect::Vacask);

    // `tm` includes `cmos180n.lib`'s other sections: that name is the
    // written file.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("cmos180n.lib"), &text).unwrap();
    let id = |lib: &Path| {
        let deck = format!(
            "nmos\nground 0\nload \"bsim3v3.osdi\"\ninclude \"{}\" section=tm\nmodel vsource vsource\nm1 (d g 0 0) nmosmod w=10u l=1u\nvd (d 0) vsource dc=1.8\nvg (g 0) vsource dc=1.2\ncontrol\n  analysis op1 op\nendc\n",
            lib.display()
        );
        let raw = Dialect::Vacask
            .run(&deck)
            .unwrap_or_else(|e| panic!("{e}\n{deck}"));
        let i = raw
            .variables
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case("vd:flow(br)"))
            .unwrap_or_else(|| panic!("{:?}", raw.variables));
        raw.real_data[i][0]
    };
    let (want, got) = (id(&lib), id(&dir.path().join("cmos180n.lib")));
    assert!(want.abs() > 1e-6, "{want}");
    assert_eq!(got.to_bits(), want.to_bits(), "{got} != {want}");
}

/// `src` (read in `d`) written for `to`, which must not refuse it.
fn written(src: &str, d: Dialect, name: &str, to: Dialect) -> Written {
    render_text(src, d, name, to).unwrap_or_else(|e| panic!("{to:?}: {}", e.messages))
}

/// The codes of the notes on a written deck, in source order.
fn codes(w: &Written) -> Vec<Code> {
    w.notes.diags.iter().map(|d| d.code).collect()
}

/// `src` written in `d`, read back and written again: the same text, which
/// is returned.
fn fixpoint(src: &str, d: Dialect) -> String {
    let text = written(src, d, "f", d).text;
    assert_eq!(written(&text, d, "f", d).text, text, "{d:?}");
    text
}

/// Each `.lib name` … `.endl` block of `src`, by a line scan: its name and
/// its lines.
fn sections(src: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut open = false;
    for l in src.lines() {
        let words: Vec<&str> = l.split_whitespace().collect();
        let head = words.first().map(|w| w.to_ascii_lowercase());
        match (head.as_deref(), words.len()) {
            (Some(".lib"), 2) if !open => {
                open = true;
                out.push((words[1].to_string(), String::new()));
            }
            (Some(".endl"), _) if open => open = false,
            _ => {}
        }
        if let Some((_, block)) = out
            .last_mut()
            .filter(|_| open || head.as_deref() == Some(".endl"))
        {
            block.push_str(l);
            block.push('\n');
        }
    }
    out
}

/// The assignments on a `.param` card's text.
fn split_params(s: &str) -> usize {
    s.matches('=').count()
        - s.matches("==").count() * 2
        - s.matches("!=").count()
        - s.matches("<=").count()
        - s.matches(">=").count()
}

/// ESPice reading `deck` as HSPICE (`--tokenizer=hspice`) and its run.
fn espice_hspice(deck: &str) -> spicerack::result::RawData {
    let dir = tempfile::tempdir().unwrap();
    let (path, raw) = (dir.path().join("deck.sp"), dir.path().join("out.raw"));
    std::fs::write(&path, deck).unwrap();
    let out = std::process::Command::new("espice")
        .arg("--tokenizer=hspice")
        .arg(&path)
        .arg("--rawfile")
        .arg(&raw)
        .arg("--format=binary")
        .output()
        .expect("espice not on PATH: run the tests in `nix develop .#testing`");
    assert!(
        out.status.success(),
        "{}\n{deck}",
        String::from_utf8_lossy(&out.stderr)
    );
    spicerack::rawfile::parse_raw(&std::fs::read(&raw).unwrap()).unwrap()
}
