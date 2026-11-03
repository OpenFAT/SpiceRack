//! The golden gate's inputs: every corpus deck, PDK kit file and test fixture,
//! each with the dialect it is read in and the form it is read as.
//!
//! Corpora come from the variables `nix develop .#testing` sets, walked and
//! filtered the way `tests/test_import_suite` walks them. A kit is its entry
//! files plus every file their includes and `.lib` calls reach, breadth first,
//! the way Interop's `pdk-models` walks one; each file is still parsed alone,
//! as the `daa0ad0` reader did, and the read order goes into the manifest.
//!
//! Cites: `tests/test_import_suite/main.rs` (`files`, `decks`, the corpus
//! filters); Interop `src/pdk_models.rs` (`references`, `convert`: the walk
//! and the empty title line).
//!
//! Contents:
//! - `pub(super) enum Form { Deck, Library }`
//! - `pub(super) struct Input { pub(super) set: &'static str, pub(super) rel: String, pub(super) path: PathBuf, pub(super) dialect: Dialect, pub(super) form: Form }`
//! - `pub(super) fn inputs(repo: &Path, manifest: &mut Vec<String>) -> Vec<Input>`
//! - `pub(super) fn source(text: &str, form: Form, path: &Path) -> (String, String)`
//! - `pub(super) struct KitSet { pub(super) set: &'static str, pub(super) root: PathBuf, pub(super) kit: resolve::Kit }`
//! - `pub(super) fn kits(repo: &Path) -> Vec<KitSet>`

use std::collections::{BTreeSet, VecDeque};
use std::path::{Component, Path, PathBuf};

use spicerack::codegen::Dialect;
use spicerack::import::{decode, parse};
use spicerack::ir::CircuitIR;
use spicerack::syntax::resolve::{self, LoadError};

/// How a file is read: a deck (line 1 is its title, the IR is named for the
/// file's stem) or a kit's library file (an empty title line is prepended
/// and the IR is named `pdk`, as Interop's `pdk-models` reads it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Form {
    Deck,
    Library,
}

/// One file to record.
pub(super) struct Input {
    /// The input set: a corpus, a kit or `fixtures`.
    pub(super) set: &'static str,
    /// The path under the set's root, `/`-separated.
    pub(super) rel: String,
    pub(super) path: PathBuf,
    pub(super) dialect: Dialect,
    pub(super) form: Form,
}

// `test_syntax` reads these; `record` does not yet (phase 3's behaviour commits).
#[allow(dead_code)]
/// One kit as `syntax::resolve` reads it: a PDK kit from its entries, or a
/// fixture deck from itself; `root` is what its paths are shown against.
pub(super) struct KitSet {
    pub(super) set: &'static str,
    pub(super) root: PathBuf,
    pub(super) kit: resolve::Kit,
}

/// A corpus: every `*.<ext>` under `$<var>` whose full path passes `keep`.
struct Corpus {
    set: &'static str,
    var: &'static str,
    dialect: Dialect,
    ext: &'static str,
    keep: fn(&str) -> bool,
    note: &'static str,
}

/// A kit: `entries` under `$<var>/<sub>`, and every file they reach.
struct Kit {
    set: &'static str,
    var: &'static str,
    sub: &'static str,
    dialect: Dialect,
    entries: fn(&Path) -> Vec<String>,
}

/// `tests/test_import_suite`'s corpora. `NGSPICE_CORPUS` is `SPICE_BENCH`
/// in `.#testing` (flake.nix), and the two walks keep the same decks, so
/// the ngspice corpus is recorded once, as spice-bench.
const CORPORA: [Corpus; 5] = [
    Corpus {
        set: "spice-bench",
        var: "SPICE_BENCH",
        dialect: Dialect::Ngspice,
        ext: "sp",
        keep: |p| !p.rsplit('/').next().unwrap_or(p).starts_with('.'),
        note: "also the ngspice corpus (NGSPICE_CORPUS == SPICE_BENCH)",
    },
    Corpus {
        set: "hspice",
        var: "HSPICE_CORPUS",
        dialect: Dialect::Hspice,
        ext: "sp",
        keep: |p| p.contains("hspice") && !p.contains("invalid"),
        note: "ESPice conformance tests/netlist",
    },
    Corpus {
        set: "xyce",
        var: "XYCE_CORPUS",
        dialect: Dialect::Xyce,
        ext: "cir",
        keep: |_| true,
        note: "Xyce_Regression Netlists",
    },
    Corpus {
        set: "vacask",
        var: "VACASK_CORPUS",
        dialect: Dialect::Vacask,
        ext: "sim",
        keep: |_| true,
        note: "VACASK sources",
    },
    Corpus {
        set: "ltspice",
        var: "LTSPICE_CORPUS",
        dialect: Dialect::Ltspice,
        ext: "net",
        keep: |_| true,
        note: "spice-datasets",
    },
];

/// The PDK kits. sky130 and gf180mcu are the `PDK_ROOT` `.#testing` pins
/// (`nix/pdks.nix`); IHP is a checkout named by `$IHP_PDK`, entered at its
/// corner libraries; VACASK's own library is its `demo/cmos180n.lib`.
const KITS: [Kit; 4] = [
    Kit {
        set: "pdk-sky130A",
        var: "PDK_ROOT",
        sub: "sky130A",
        dialect: Dialect::Ngspice,
        entries: |_| vec!["libs.tech/combined/sky130.lib.spice".into()],
    },
    Kit {
        set: "pdk-gf180mcuD",
        var: "PDK_ROOT",
        sub: "gf180mcuD",
        dialect: Dialect::Ngspice,
        entries: |_| vec!["libs.tech/ngspice/sm141064.ngspice".into()],
    },
    Kit {
        set: "pdk-ihp-sg13g2",
        var: "IHP_PDK",
        sub: "ihp-sg13g2",
        dialect: Dialect::Ngspice,
        entries: ihp_corners,
    },
    Kit {
        set: "pdk-vacask",
        var: "VACASK_CORPUS",
        sub: "",
        dialect: Dialect::Vacask,
        entries: |_| vec!["demo/cmos180n.lib".into()],
    },
];

/// Fixture extensions under `tests/` and the dialect each is read in.
const FIXTURES: [(&str, Dialect); 7] = [
    ("sp", Dialect::Ngspice),
    ("lib", Dialect::Ngspice),
    ("spice", Dialect::Ngspice),
    ("cir", Dialect::Xyce),
    ("scs", Dialect::Spectre),
    ("sim", Dialect::Vacask),
    ("net", Dialect::Ltspice),
];

/// Every input, sorted by set and path. Appends the manifest's set, kit and
/// read-order lines to `manifest`; an absent set is a line, not an error.
pub(super) fn inputs(repo: &Path, manifest: &mut Vec<String>) -> Vec<Input> {
    let mut all = Vec::new();
    for c in &CORPORA {
        let Some(root) = root(c.set, c.var, "", manifest) else {
            continue;
        };
        let mut found = Vec::new();
        files(&root, &mut found);
        found.retain(|p| p.extension() == Some(c.ext.as_ref()) && (c.keep)(&p.to_string_lossy()));
        manifest.push(format!(
            "set {} {} {} inputs: ${}, {}",
            c.set,
            c.dialect.name(),
            found.len(),
            c.var,
            c.note
        ));
        all.extend(found.into_iter().map(|path| Input {
            set: c.set,
            rel: rel(&root, &path),
            path,
            dialect: c.dialect,
            form: Form::Deck,
        }));
    }
    for k in &KITS {
        if let Some(root) = root(k.set, k.var, k.sub, manifest) {
            all.extend(kit(k, &root, manifest));
        }
    }
    let mut found = Vec::new();
    files(&repo.join("tests"), &mut found);
    let before = all.len();
    all.extend(found.into_iter().filter_map(|path| {
        let ext = path.extension()?.to_str()?;
        let &(_, dialect) = FIXTURES.iter().find(|f| f.0 == ext)?;
        Some(Input {
            set: "fixtures",
            rel: rel(repo, &path),
            path,
            dialect,
            form: Form::Deck,
        })
    }));
    manifest.push(format!(
        "set fixtures {} inputs: tests/**/*.{{sp,lib,spice,cir,scs,sim,net}}, dialect by extension",
        all.len() - before
    ));
    all.sort_by(|a, b| (a.set, &a.rel).cmp(&(b.set, &b.rel)));
    all
}

/// The text `parse` reads for a file and the name it gives the IR.
pub(super) fn source(text: &str, form: Form, path: &Path) -> (String, String) {
    match form {
        Form::Deck => {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("deck");
            (text.to_string(), stem.to_string())
        }
        Form::Library => (format!("\n{text}"), "pdk".to_string()),
    }
}

#[allow(dead_code)]
/// Every PDK kit (entered at its entries, read as libraries) and every
/// fixture deck (entered at itself, read as a deck, its includes followed
/// under `tests/`) as a [`resolve::Kit`]. An absent kit is left out.
pub(super) fn kits(repo: &Path) -> Vec<KitSet> {
    let mut out = Vec::new();
    for k in &KITS {
        let Some(root) = root(k.set, k.var, k.sub, &mut Vec::new()) else {
            continue;
        };
        let entries: Vec<PathBuf> = (k.entries)(&root).iter().map(|e| root.join(e)).collect();
        let kit = resolve::Kit::load(
            &entries,
            resolve::Form::Library,
            k.dialect,
            &mut loader(&root),
        );
        out.push(KitSet {
            set: k.set,
            root,
            kit,
        });
    }
    let tests = repo.join("tests");
    let mut found = Vec::new();
    files(&tests, &mut found);
    found.sort();
    for path in found {
        let Some(&(_, d)) = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|e| FIXTURES.iter().find(|f| f.0 == e))
        else {
            continue;
        };
        let kit = resolve::Kit::load(&[path], resolve::Form::Deck, d, &mut loader(&tests));
        out.push(KitSet {
            set: "fixtures",
            root: repo.to_path_buf(),
            kit,
        });
    }
    out
}

// ---- private ----

#[allow(dead_code)]
/// Reads a file under `root` (decoded as `import::decode` does); refuses
/// any other path.
fn loader(root: &Path) -> impl FnMut(&Path) -> Result<String, LoadError> + '_ {
    move |p: &Path| {
        if !p.starts_with(root) {
            return Err(LoadError::Refused);
        }
        if !p.is_file() {
            return Err(LoadError::NotFound);
        }
        std::fs::read(p)
            .map(|b| decode(&b))
            .map_err(|_| LoadError::NotFound)
    }
}

/// `$<var>/<sub>` if it is set and exists; else an `absent` manifest line.
fn root(set: &str, var: &str, sub: &str, manifest: &mut Vec<String>) -> Option<PathBuf> {
    let Ok(base) = std::env::var(var) else {
        manifest.push(format!("absent {set}: ${var} not set"));
        return None;
    };
    let root = Path::new(&base).join(sub);
    if !root.is_dir() {
        manifest.push(format!("absent {set}: ${var}/{sub} is not a directory"));
        return None;
    }
    Some(root)
}

/// Every file under `dir`, skipping `src*` directories (fetched sources and
/// their assets in spice-bench, as `test_import_suite` skips them).
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !e.file_name().to_string_lossy().starts_with("src") {
                files(&p, out);
            }
        } else {
            out.push(p);
        }
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// IHP's corner libraries, the files a deck `.lib`s (`cornerMOSlv.lib mos_tt`).
fn ihp_corners(root: &Path) -> Vec<String> {
    let models = root.join("libs.tech/ngspice/models");
    let mut found = Vec::new();
    files(&models, &mut found);
    let mut v: Vec<String> = found
        .iter()
        .map(|p| rel(root, p))
        .filter(|r| {
            let name = r.rsplit('/').next().unwrap_or(r);
            name.starts_with("corner") && name.ends_with(".lib")
        })
        .collect();
    v.sort();
    v
}

/// The files `k`'s entries reach, in read order: entries first, then breadth
/// first through each file's includes, `.lib` calls and Verilog-A sources.
/// Verilog-A sources, missing files and paths outside the root are manifest
/// lines, not inputs.
fn kit(k: &Kit, root: &Path, manifest: &mut Vec<String>) -> Vec<Input> {
    let entries = (k.entries)(root);
    for e in &entries {
        manifest.push(format!("kit {} entry {e}", k.set));
    }
    let mut todo: VecDeque<(PathBuf, bool)> = entries
        .iter()
        .map(|e| (normalize(&root.join(e)), false))
        .collect();
    let (mut seen, mut out) = (BTreeSet::new(), Vec::new());
    while let Some((p, verilog_a)) = todo.pop_front() {
        if !seen.insert(p.clone()) {
            continue;
        }
        let shown = rel(root, &p);
        let n = seen.len();
        if !p.starts_with(root) {
            manifest.push(format!("outside {} {shown}", k.set));
            continue;
        }
        if !p.is_file() {
            manifest.push(format!("missing {} {shown}", k.set));
            continue;
        }
        if verilog_a {
            manifest.push(format!(
                "order {} {n:04} {shown} verilog-a, not read",
                k.set
            ));
            continue;
        }
        manifest.push(format!("order {} {n:04} {shown}", k.set));
        let text = std::fs::read(&p).map(|b| decode(&b)).unwrap_or_default();
        let (src, name) = source(&text, Form::Library, &p);
        let dir = p.parent().unwrap_or(Path::new("")).to_path_buf();
        // A file `parse` refuses names nothing further; its refusal is recorded.
        if let Ok(Ok((ir, _))) = std::panic::catch_unwind(|| parse(&src, &name, k.dialect)) {
            for (f, va) in references(&ir) {
                todo.push_back((normalize(&dir.join(f)), va));
            }
        }
        out.push(Input {
            set: k.set,
            rel: shown,
            path: p,
            dialect: k.dialect,
            form: Form::Library,
        });
    }
    out
}

/// The files `ir` names, at the top and in every section: includes and
/// `.lib` calls (`false`), and Verilog-A sources (`true`). Interop's
/// `references`, verbatim in effect.
fn references(ir: &CircuitIR) -> Vec<(String, bool)> {
    let bodies = std::iter::once(&ir.top).chain(ir.sections.iter().map(|s| &s.body));
    bodies
        .flat_map(|b| {
            let libs = b.libs.iter().map(|(f, _)| (f.as_str(), false));
            let va = b.osdi_loads.iter().map(String::as_str);
            let va = va
                .filter(|f| spicerack::veriloga::is_source(f))
                .map(|f| (f, true));
            b.includes
                .iter()
                .map(|f| (f.as_str(), false))
                .chain(libs)
                .chain(va)
        })
        .map(|(f, v)| (f.trim_matches(['"', '\'']).to_string(), v))
        .collect()
}

/// `p` with `.` and `..` resolved lexically.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}
