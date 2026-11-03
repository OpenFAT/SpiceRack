//! Real PDK model files through library sections: each read, written back
//! in its own dialect and run.
//!
//! Oracles: a line scan of each file for its section names; read -> write ->
//! read as a fixpoint; and the real simulator running a corner from the file
//! SpiceRack wrote against the same corner from the original, bit for bit:
//! ngspice on the pinned gf180mcu PDK (`$PDK_ROOT`), VACASK on its own corner
//! library (`$VACASK_CORPUS`). sky130's is read only (see its test).
//! Unsupported fails, never skips: run in `nix develop .#testing`.

use std::path::Path;

use spicerack::codegen::{Dialect, netlist};
use spicerack::import::parse;
use spicerack::ir::*;

#[path = "common/sections.rs"]
mod sections;
use sections::meaning;

// ---- private ----

/// gf180mcu's model file read and written back by SpiceRack: every `.lib`
/// definition is a section (a line scan finds the same names), nothing is
/// unparsed or raw (each `.param` in a `.subckt` is a local), nothing lands
/// in the top, and ngspice runs an inverter on the
/// `typical` corner from the written file to the same output voltage as
/// from the PDK's own.
#[test]
fn gf180mcu_model_file_round_trips_through_sections() {
    let root = std::env::var("PDK_ROOT")
        .expect("PDK_ROOT not set: run the tests in `nix develop .#testing`");
    let pdk = Path::new(&root).join("gf180mcuD/libs.tech/ngspice");
    let src = std::fs::read_to_string(pdk.join("sm141064.ngspice")).unwrap();
    let (ir, report) = parse(&src, "sm141064", Dialect::Ngspice).unwrap();
    // The `.param`s inside its `.subckt`s are the definitions' locals.
    assert_eq!(report, Default::default());
    let locals: usize = ir.subcircuits().map(|s| s.locals.len()).sum();
    assert_eq!(
        locals,
        src.lines().filter(|l| l.starts_with(".param")).count()
            - ir.sections
                .iter()
                .map(|s| s.body.parameters.len())
                .sum::<usize>()
    );
    let scanned: Vec<&str> = src
        .lines()
        .filter_map(|l| l.strip_prefix(".lib "))
        .map(str::trim)
        .filter(|w| !w.contains(char::is_whitespace))
        .collect();
    let read: Vec<&str> = ir.sections.iter().map(|s| s.body.name.as_str()).collect();
    assert_eq!(read, scanned);
    let top = &ir.top;
    assert!(
        top.parameters.is_empty()
            && top.models.is_empty()
            && top.libs.is_empty()
            && top.raw_spice.is_empty()
            && ir.subcircuit_defs.is_empty()
    );
    let text = fixpoint(&ir, Dialect::Ngspice);

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

/// gf180mcu's model files, SPICE -> Spectre -> IR, a section at a time:
/// each section either crosses with the same meaning ([`meaning`]: a
/// definition's locals are Spectre `parameters`, which an instance could
/// set; numbers respelled, SPICE quotes dropped) or is refused for one of
/// the named reasons: a `.model` kind with no Spectre master mapped (`r`,
/// `c`, a level-3 `d`). The mismatch and statistical corners cross, their
/// `agauss` calls lifted and read back (tests/test_monte_carlo.rs).
#[test]
fn gf180mcu_sections_cross_to_spectre() {
    let root = std::env::var("PDK_ROOT")
        .expect("PDK_ROOT not set: run the tests in `nix develop .#testing`");
    let pdk = Path::new(&root).join("gf180mcuD/libs.tech/ngspice");
    let (mut crossed, mut refused) = (Vec::new(), std::collections::BTreeSet::new());
    for f in [
        "sm141064.ngspice",
        "sm141064_mim.ngspice",
        "smbb000149.ngspice",
    ] {
        let src = std::fs::read_to_string(pdk.join(f)).unwrap();
        let (ir, _) = parse(&src, "gf180", Dialect::Ngspice).unwrap();
        for s in &ir.sections {
            let one = CircuitIR {
                sections: vec![s.clone()],
                ..Default::default()
            };
            let scs = match netlist(&one, Dialect::Spectre) {
                Ok(t) => t,
                Err(e) => {
                    let e = e.to_string();
                    let why = [
                        "(r level 1): no Spectre master",
                        "(c level 1): no Spectre master",
                        "(d level 3): no Spectre master",
                    ]
                    .into_iter()
                    .find(|w| e.contains(w))
                    .unwrap_or_else(|| panic!("{f} {}: {e}", s.body.name));
                    refused.insert(why);
                    continue;
                }
            };
            let (back, report) = parse(&scs, "gf180", Dialect::Spectre).unwrap();
            assert_eq!(report, Default::default(), "{f} {}:\n{scs}", s.body.name);
            assert_eq!(meaning(&back), meaning(&one), "{f} {}:\n{scs}", s.body.name);
            crossed.push(format!("{f}:{}", s.body.name));
        }
    }
    assert_eq!(refused.len(), 3, "{refused:?}");
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
    let (ir, report) = parse(&src, "sky130", Dialect::Ngspice).unwrap();
    assert_eq!(report, Default::default());
    let scanned: Vec<&str> = src
        .lines()
        .filter_map(|l| l.strip_prefix(".lib "))
        .map(str::trim)
        .collect();
    assert_eq!(
        ir.sections
            .iter()
            .map(|s| s.body.name.as_str())
            .collect::<Vec<_>>(),
        scanned
    );
    assert_eq!(
        ir.top,
        Subcircuit {
            name: "sky130".into(),
            ..Default::default()
        }
    );
    let tt = &ir.sections[0].body;
    assert_eq!((tt.parameters.len(), tt.includes.len()), (2, 4), "{tt:?}");
    fixpoint(&ir, Dialect::Ngspice);
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
    let (mut todo, mut seen) = (vec![entry.to_path_buf()], Vec::new());
    let mut raw = Vec::new();
    // ponytail: a linear `seen` scan; the tree is ~200 files.
    while let Some(rel) = todo.pop() {
        let rel = normalized(&rel);
        if seen.contains(&rel) {
            continue;
        }
        let src = std::fs::read_to_string(pdk.join(&rel))
            .unwrap_or_else(|e| panic!("{}: {e}", rel.display()));
        // An included file has no title line: a dummy one keeps its first card.
        let (ir, report) = parse(&format!("*\n{src}"), "f", Dialect::Ngspice).unwrap();
        assert!(
            report.unparsed.is_empty(),
            "{}: {:?}",
            rel.display(),
            report.unparsed
        );
        for sc in ir.subcircuits() {
            raw.extend(
                sc.raw_spice
                    .iter()
                    .map(|l| format!("{}: {l}", rel.display())),
            );
            for c in &sc.components {
                if let Component::RawSpice { line } = c {
                    raw.push(format!(
                        "{}: {}",
                        rel.display(),
                        &line[..line.find(' ').unwrap_or(line.len())]
                    ));
                }
            }
            let dir = rel.parent().unwrap();
            todo.extend(
                sc.includes
                    .iter()
                    .chain(sc.libs.iter().map(|(p, _)| p))
                    .map(|p| dir.join(p)),
            );
        }
        let text = fixpoint(&ir, Dialect::Ngspice);
        let hs = netlist(&ir, Dialect::Hspice)
            .unwrap_or_else(|e| panic!("{}: HSPICE: {e}", rel.display()));
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
    let (ir, report) = parse(&src, "cmos180n", Dialect::Vacask).unwrap();
    // `*` lines between the sections: VACASK skips everything outside a
    // section while it looks for one (lib/dfllexer.l, `SECLOOK`), so these
    // are text no VACASK statement reads, kept verbatim.
    assert_eq!(
        report.unparsed,
        [
            "* Worst power",
            "* Worst speed",
            "* Worst one",
            "* Worst zero",
            "* Model definition"
        ]
    );
    let scanned: Vec<&str> = src
        .lines()
        .filter_map(|l| l.trim().strip_prefix("section "))
        .map(str::trim)
        .collect();
    assert_eq!(
        ir.sections
            .iter()
            .map(|s| s.body.name.as_str())
            .collect::<Vec<_>>(),
        scanned
    );
    let text = fixpoint(&ir, Dialect::Vacask);

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

/// Emitting `ir` in `d` and reading it back gives `ir` again, and emitting
/// that gives the same text. Returns the text.
fn fixpoint(ir: &CircuitIR, d: Dialect) -> String {
    let text = netlist(ir, d).unwrap_or_else(|e| panic!("{d:?}: {e}"));
    let (again, _) = parse(&text, &ir.top.name, d).unwrap();
    assert_eq!(&again.sections, &ir.sections, "{d:?}:\n{text}");
    assert_eq!(&again, ir, "{d:?}:\n{text}");
    assert_eq!(netlist(&again, d).unwrap(), text, "{d:?}");
    text
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

/// `p` with its `.` and `..` parts resolved, lexically.
fn normalized(p: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}
