//! ngspice: `ngspice -b` as a subprocess, or libngspice in process.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_short, c_void};
use std::process::Command;
use std::sync::Mutex;

use super::{exec, read_results, scratch, BackendError};
use crate::result::{RawData, VarInfo};

/// Run a deck through `ngspice -b` and read back its rawfile.
///
/// ngspice refuses `.measure` (and ignores `.four`) under `-b -r rawfile`, so
/// such a deck has its analysis moved into a `.control` block that writes the
/// rawfile itself; a deck that already owns a control block gets a `write`
/// appended, or it produces no rawfile at all. This is run mechanics (where
/// the rawfile goes), not dialect translation.
pub(super) fn run(netlist: &str) -> Result<RawData, BackendError> {
    let (dir, cir) = scratch(netlist, "deck.cir")?;
    let raw_path = dir.path().join("out.raw");
    let rewritten = control_block_deck(netlist, &raw_path).or_else(|| append_write_block(netlist, &raw_path));
    let mut cmd = Command::new("ngspice");
    cmd.arg("-b");
    match rewritten {
        Some(deck) => std::fs::write(&cir, deck)?,
        None => {
            cmd.arg("-r").arg(&raw_path);
        }
    }
    let stdout = exec(cmd.arg(&cir), "ngspice", None)?;
    read_results(&raw_path, stdout)
}

/// Analyses worth moving into a `.control` block.
///
/// Deliberately only the three that ngspice's `.measure` supports (it accepts
/// DC, AC, TRAN and SP). Rewriting anything else is pure downside: `noise`,
/// `disto` and `pz` emit *several* plots, and the `write` command captures
/// only the current one — for `.noise` that is "Integrated Noise", which would
/// silently drop the spectra that callers actually read.
const ANALYSIS_CARDS: [&str; 3] = ["ac", "tran", "dc"];

/// Control commands that mean an existing `.control` block owns the run.
///
/// The codegen emits its own `.control` block for `pre_osdi` loads, and that
/// block runs nothing — bailing on *any* `.control` would mean OSDI decks can
/// never report `.measure` results. ngspice accepts several `.control` blocks
/// per deck (verified on 44.2), so only a block that actually starts an
/// analysis is off limits.
const RUN_COMMANDS: [&str; 5] = ["run", "op", "ac", "tran", "dc"];

/// Lowercased first whitespace-separated token of a line.
fn first_word(line: &str) -> String {
    line.split_whitespace().next().unwrap_or("").to_lowercase()
}

/// True if some `.control`/`.endc` block in the deck issues a run command.
fn has_owning_control_block(netlist: &str) -> bool {
    let mut inside = false;
    for line in netlist.lines() {
        match first_word(line).as_str() {
            ".control" => inside = true,
            ".endc" => inside = false,
            word if inside && RUN_COMMANDS.contains(&word) => return true,
            _ => {}
        }
    }
    false
}

/// Split a `.four` card into `(fundamental, outputs)`.
///
/// SPICE3 syntax is `.four <freq> <ov1> [ov2 ...]`; there is no harmonic-count
/// field, and the codegen no longer emits one for this dialect.
fn parse_four_card(line: &str) -> Option<(String, Vec<String>)> {
    let mut parts = line.split_whitespace();
    if parts.next()?.to_lowercase() != ".four" {
        return None;
    }
    let fundamental = parts.next()?.to_string();
    let outputs: Vec<String> = parts.map(str::to_string).collect();
    if outputs.is_empty() {
        return None;
    }
    Some((fundamental, outputs))
}

/// Rewrite a deck so `.measure` and `.four` results reach stdout.
///
/// ngspice refuses both under `-b -r rawfile`:
///   - "No .measure possible in batch mode (-b) with -r rawfile set!"
///   - ".fourier line ignored since rawfile was produced."
///
/// The cure is the same for both: move the analysis card into a `.control`
/// block that writes the rawfile itself. Inside the block `.four` has to become
/// the `fourier` command — there is no `four` command ("four: no such command
/// available in ngspice").
///
/// Returns `None` when the deck needs no rewrite (nothing to recover, no
/// analysis card found, or a control block already runs the simulation).
fn control_block_deck(netlist: &str, raw_path: &std::path::Path) -> Option<String> {
    let has_measure = netlist.lines().any(|l| first_word(l).starts_with(".meas"));
    let four = netlist.lines().find_map(parse_four_card);
    if !has_measure && four.is_none() {
        return None;
    }
    if has_owning_control_block(netlist) {
        return None;
    }

    let analysis_idx = netlist.lines().position(|l| {
        let word = first_word(l);
        ANALYSIS_CARDS.iter().any(|c| word == format!(".{}", c))
    })?;

    let mut out = Vec::new();
    for (i, line) in netlist.lines().enumerate() {
        if i == analysis_idx {
            // `.ac dec 10 1 1k` -> bare `ac dec 10 1 1k` inside the block
            out.push(".control".to_string());
            out.push(line.trim_start().trim_start_matches('.').to_string());
            if let Some((fundamental, outputs)) = &four {
                out.push(format!("fourier {} {}", fundamental, outputs.join(" ")));
            }
            out.push(format!("write {}", raw_path.display()));
            out.push(".endc".to_string());
        } else if four.is_some() && first_word(line) == ".four" {
            // Leaving the dot-card in place makes ngspice run the analysis twice.
            continue;
        } else {
            out.push(line.to_string());
        }
    }
    out.push(String::new());
    Some(out.join("\n"))
}

/// Give a hand-written `.control` deck somewhere to put its results.
///
/// A deck whose analysis lives in a `.control` block writes nothing under
/// `-b -r rawfile`: ngspice reserves the file for dot-card analyses, the
/// control block runs, and the rawfile is never created — the backend then
/// fails with "Failed to read raw file". Appending a second control block that
/// just does `write` captures the current plot. Returns `None` when the deck
/// has no control block of its own or already writes one.
fn append_write_block(netlist: &str, raw_path: &std::path::Path) -> Option<String> {
    if !has_owning_control_block(netlist) {
        return None;
    }
    if netlist.lines().any(|l| first_word(l) == "write") {
        return None;
    }

    let block = format!(".control\nwrite {}\n.endc", raw_path.display());
    let mut out: Vec<String> = Vec::new();
    let mut inserted = false;
    for line in netlist.lines() {
        if !inserted && first_word(line) == ".end" {
            out.push(block.clone());
            inserted = true;
        }
        out.push(line.to_string());
    }
    if !inserted {
        out.push(block);
    }
    out.push(String::new());
    Some(out.join("\n"))
}

#[cfg(test)]
mod deck_tests {
    use super::control_block_deck;
    use std::path::Path;

    const RAW: &str = "/tmp/out.raw";

    #[test]
    fn rewrites_ac_deck_when_measures_present() {
        let deck = "* t\nR1 a b 1k\n.meas ac g find vdb(b) at=100\n.ac dec 10 1 1k\n.end\n";
        let out = control_block_deck(deck, Path::new(RAW)).expect("should rewrite");
        assert!(out.contains(".control\nac dec 10 1 1k\nwrite /tmp/out.raw\n.endc"));
        // The dot-card must be gone, or ngspice runs the analysis twice.
        assert!(!out.contains(".ac dec"));
    }

    #[test]
    fn leaves_deck_alone_without_measures() {
        let deck = "* t\nR1 a b 1k\n.ac dec 10 1 1k\n.end\n";
        assert!(control_block_deck(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn never_rewrites_noise_which_emits_multiple_plots() {
        // `write` would capture only "Integrated Noise" and drop the spectra.
        let deck = "* t\nR1 a b 1k\n.meas tran x find v(b) at=1\n.noise v(b) V1 dec 10 1 1k\n.end\n";
        let out = control_block_deck(deck, Path::new(RAW));
        assert!(out.is_none(), "noise must keep the -r path, got: {:?}", out);
    }

    #[test]
    fn defers_to_a_hand_written_control_block() {
        let deck = "* t\n.meas ac g find vdb(b) at=100\n.control\nrun\n.endc\n.ac dec 10 1 1k\n.end\n";
        assert!(control_block_deck(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn rewrites_around_an_osdi_only_control_block() {
        // The codegen's own pre_osdi block runs nothing, so it must not veto the
        // rewrite — otherwise OSDI decks can never report measures.
        let deck = "* t\n.control\npre_osdi /m.osdi\n.endc\nR1 a b 1k\n\
                    .meas tran x find v(b) at=1\n.tran 1n 1u\n.end\n";
        let out = control_block_deck(deck, Path::new(RAW)).expect("should rewrite");
        assert!(out.contains("pre_osdi /m.osdi"), "osdi block kept: {}", out);
        assert!(out.contains("tran 1n 1u\nwrite /tmp/out.raw"), "rewritten: {}", out);
    }

    #[test]
    fn routes_four_through_the_fourier_command() {
        // `.four` is ignored under `-b -r`, and there is no `four` command
        // inside a control block — it has to become `fourier`.
        let deck = "* t\nV1 a 0 SIN(0 1 1k)\n.tran 1u 10m\n.four 1k v(a)\n.end\n";
        let out = control_block_deck(deck, Path::new(RAW)).expect("should rewrite");
        assert!(out.contains("tran 1u 10m\nfourier 1k v(a)\n"), "order: {}", out);
        assert!(!out.contains(".four "), "dot-card removed: {}", out);
    }

    #[test]
    fn four_without_a_transient_card_is_left_alone() {
        let deck = "* t\n.four 1k v(a)\n.end\n";
        assert!(control_block_deck(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn appends_a_write_block_to_a_hand_written_control_deck() {
        let deck = "* t\nV1 a 0 3\n.control\nop\n.endc\n.end\n";
        let out = super::append_write_block(deck, Path::new(RAW)).expect("should append");
        assert!(out.contains(".control\nwrite /tmp/out.raw\n.endc\n.end"), "{}", out);
    }

    #[test]
    fn leaves_a_control_deck_that_already_writes_alone() {
        let deck = "* t\n.control\nop\nwrite mine.raw\n.endc\n.end\n";
        assert!(super::append_write_block(deck, Path::new(RAW)).is_none());
    }

    #[test]
    fn does_not_append_to_a_plain_dot_card_deck() {
        let deck = "* t\nV1 a 0 3\n.op\n.end\n";
        assert!(super::append_write_block(deck, Path::new(RAW)).is_none());
    }
}
// ── libngspice (dlopen) ──
//
// ponytail: same decks and results as the subprocess path, minus the temp
// files (and minus its `.control` rewrite, so `.four` is lost here). Selected
// only when libngspice.so is found; its test is #[ignore]d without the library.

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

const RTLD_LAZY: c_int = 1;

type SendCharFn = extern "C" fn(*const c_char, c_int, *mut c_void) -> c_int;
type SendStatFn = extern "C" fn(*const c_char, c_int, *mut c_void) -> c_int;
type ControlledExitFn = extern "C" fn(c_int, bool, bool, c_int, *mut c_void) -> c_int;
type SendDataFn = extern "C" fn(*mut c_void, c_int, c_int, *mut c_void) -> c_int;
type SendInitDataFn = extern "C" fn(*mut c_void, c_int, *mut c_void) -> c_int;
type BGThreadRunningFn = extern "C" fn(bool, c_int, *mut c_void) -> c_int;
type NgSpiceInitFn = unsafe extern "C" fn(
    SendCharFn, SendStatFn, ControlledExitFn, SendDataFn, SendInitDataFn, BGThreadRunningFn, *mut c_void,
) -> c_int;
type NgSpiceCmdFn = unsafe extern "C" fn(*const c_char) -> c_int;
type NgSpiceCircFn = unsafe extern "C" fn(*mut *mut c_char) -> c_int;
type NgGetVecInfoFn = unsafe extern "C" fn(*const c_char) -> *mut VecInfo;
type NgListFn = unsafe extern "C" fn(*const c_char) -> *mut *mut c_char;
type NgAllPlotsFn = unsafe extern "C" fn() -> *mut *mut c_char;

/// `vector_info` in sharedspice.h.
#[repr(C)]
struct VecInfo {
    v_name: *const c_char,
    v_type: c_int,
    v_flags: c_short,
    v_realdata: *mut f64,
    v_compdata: *mut NgComplex,
    v_length: c_int,
}

#[repr(C)]
struct NgComplex {
    re: f64,
    im: f64,
}

const LIB_SEARCH_PATHS: &[&str] = &[
    "libngspice.so",
    "libngspice.so.0",
    "/usr/lib/libngspice.so",
    "/usr/lib/libngspice.so.0",
    "/usr/local/lib/libngspice.so",
    "/usr/lib/x86_64-linux-gnu/libngspice.so",
    "/usr/lib/x86_64-linux-gnu/libngspice.so.0",
];

/// A loaded, initialised libngspice. ngspice is not thread-safe: one
/// instance per run.
pub struct Shared {
    lib: *mut c_void,
    circ: NgSpiceCircFn,
    command: NgSpiceCmdFn,
    get_vec_info: NgGetVecInfoFn,
    all_vecs: NgListFn,
    all_plots: NgAllPlotsFn,
    /// Text ngspice printed (`.meas` results). Boxed so its address, handed
    /// to ngspice as callback userdata, stays fixed.
    output: Box<Mutex<Vec<String>>>,
}

impl Shared {
    pub fn is_available() -> bool {
        // SAFETY: a handle dlopen just returned.
        open_library().map(|h| unsafe { dlclose(h) }).is_ok()
    }

    /// Load libngspice, resolve its symbols and call `ngSpice_Init`.
    pub fn new() -> Result<Self, BackendError> {
        let lib = open_library()?;
        // SAFETY: each symbol is cast to the signature sharedspice.h declares.
        unsafe {
            let init: NgSpiceInitFn = symbol(lib, "ngSpice_Init")?;
            let s = Self {
                lib,
                circ: symbol(lib, "ngSpice_Circ")?,
                command: symbol(lib, "ngSpice_Command")?,
                get_vec_info: symbol(lib, "ngGet_Vec_Info")?,
                all_vecs: symbol(lib, "ngSpice_AllVecs")?,
                all_plots: symbol(lib, "ngSpice_AllPlots")?,
                output: Box::new(Mutex::new(Vec::new())),
            };
            let user = &*s.output as *const Mutex<Vec<String>> as *mut c_void;
            let ret = init(cb_send_char, cb_ignore_text, cb_controlled_exit, cb_ignore_data, cb_ignore_init, cb_ignore_bg, user);
            if ret != 0 {
                return Err(sim_err(format!("ngSpice_Init returned error code {ret}")));
            }
            Ok(s)
        }
    }

    pub fn run(&self, netlist: &str) -> Result<RawData, BackendError> {
        let lines = netlist.lines().map(CString::new).collect::<Result<Vec<_>, _>>()
            .map_err(|_| sim_err("Netlist contains null bytes".into()))?;
        let mut ptrs: Vec<*mut c_char> = lines.iter().map(|c| c.as_ptr() as *mut c_char).collect();
        ptrs.push(std::ptr::null_mut());
        // SAFETY: a null-terminated array of C strings that outlives the call.
        let ret = unsafe { (self.circ)(ptrs.as_mut_ptr()) };
        if ret != 0 {
            return Err(sim_err(format!("ngSpice_Circ returned error code {ret}")));
        }
        let run = CString::new("run").expect("literal has no nul");
        // SAFETY: a valid C string.
        let ret = unsafe { (self.command)(run.as_ptr()) };
        if ret != 0 {
            return Err(sim_err(format!("ngSpice_Command('run') returned error code {ret}")));
        }
        self.results()
    }

    /// The most recent plot: every vector ngspice can hand back.
    fn results(&self) -> Result<RawData, BackendError> {
        // SAFETY: ngspice returns a null-terminated list it owns.
        let plot = unsafe { c_list((self.all_plots)()) }.into_iter().next()
            .ok_or_else(|| sim_err("No plots available after simulation".into()))?;
        let c_plot = CString::new(plot.clone()).map_err(|_| sim_err("bad plot name".into()))?;
        // SAFETY: as above.
        let names = unsafe { c_list((self.all_vecs)(c_plot.as_ptr())) };
        let mut raw = RawData::empty();
        raw.plot_name = plot.clone();
        raw.stdout = self.output.lock().map(|o| o.join("\n")).unwrap_or_default();
        for name in names {
            // ngspice 45 lists `#branch` currents it cannot provide unless saved.
            let Some((re, cx)) = self.vector(&format!("{plot}.{name}")).or_else(|| self.vector(&name)) else { continue };
            raw.variables.push(VarInfo { index: raw.variables.len(), var_type: infer_var_type(&name), name });
            raw.real_data.push(re);
            raw.is_complex |= !cx.is_empty();
            raw.complex_data.push(cx);
        }
        if raw.variables.is_empty() {
            return Err(sim_err(format!("No vectors in plot '{plot}'")));
        }
        if !raw.is_complex {
            raw.complex_data.clear();
        }
        Ok(raw)
    }

    /// (real parts, complex values — empty for a real vector).
    fn vector(&self, name: &str) -> Option<(Vec<f64>, Vec<num_complex::Complex64>)> {
        let c_name = CString::new(name).ok()?;
        // SAFETY: ngspice owns the returned info; it is copied out before the next call.
        unsafe {
            let vi = (self.get_vec_info)(c_name.as_ptr()).as_ref()?;
            let len = usize::try_from(vi.v_length).ok().filter(|&n| n > 0)?;
            if !vi.v_realdata.is_null() {
                return Some((std::slice::from_raw_parts(vi.v_realdata, len).to_vec(), Vec::new()));
            }
            if vi.v_compdata.is_null() {
                return None;
            }
            let c = std::slice::from_raw_parts(vi.v_compdata, len);
            Some((c.iter().map(|z| z.re).collect(), c.iter().map(|z| num_complex::Complex64::new(z.re, z.im)).collect()))
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let quit = CString::new("quit").expect("literal has no nul");
        // SAFETY: the library handle is live until dlclose.
        unsafe {
            (self.command)(quit.as_ptr());
            dlclose(self.lib);
        }
    }
}

fn sim_err(msg: String) -> BackendError {
    BackendError::SimulationError(msg)
}

fn open_library() -> Result<*mut c_void, BackendError> {
    for path in LIB_SEARCH_PATHS {
        let c = CString::new(*path).expect("paths have no nul");
        // SAFETY: dlopen with a valid C string.
        let h = unsafe { dlopen(c.as_ptr(), RTLD_LAZY) };
        if !h.is_null() {
            return Ok(h);
        }
    }
    // SAFETY: dlerror returns null or a C string.
    let why = unsafe { dlerror().as_ref().map(|p| CStr::from_ptr(p).to_string_lossy().into_owned()) };
    Err(sim_err(format!(
        "Failed to load libngspice.so: {}. Install with: sudo apt install libngspice0-dev",
        why.unwrap_or_else(|| "not found".into())
    )))
}

/// SAFETY: the caller names a symbol whose C type is `T` (a fn pointer).
unsafe fn symbol<T>(lib: *mut c_void, name: &str) -> Result<T, BackendError> {
    let c = CString::new(name).expect("symbol names have no nul");
    let sym = unsafe { dlsym(lib, c.as_ptr()) };
    if sym.is_null() {
        return Err(sim_err(format!("Failed to resolve symbol '{name}'")));
    }
    Ok(unsafe { std::mem::transmute_copy(&sym) })
}

/// SAFETY: `p` is null or a null-terminated array of C strings.
unsafe fn c_list(p: *mut *mut c_char) -> Vec<String> {
    let mut out = Vec::new();
    if p.is_null() {
        return out;
    }
    loop {
        let e = unsafe { *p.add(out.len()) };
        if e.is_null() {
            return out;
        }
        out.push(unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned());
    }
}

extern "C" fn cb_send_char(msg: *const c_char, _id: c_int, user: *mut c_void) -> c_int {
    if !msg.is_null() && !user.is_null() {
        // SAFETY: `user` is `Shared::output`, alive while ngspice is loaded.
        let out = unsafe { &*(user as *const Mutex<Vec<String>>) };
        let text = unsafe { CStr::from_ptr(msg) }.to_string_lossy().into_owned();
        if let Ok(mut o) = out.lock() {
            o.push(text);
        }
    }
    0
}

extern "C" fn cb_controlled_exit(status: c_int, _: bool, _: bool, id: c_int, user: *mut c_void) -> c_int {
    if status != 0 {
        let msg = CString::new(format!("ngspice controlled exit with status {status}")).expect("no nul");
        cb_send_char(msg.as_ptr(), id, user);
    }
    0
}

extern "C" fn cb_ignore_text(_: *const c_char, _: c_int, _: *mut c_void) -> c_int { 0 }
extern "C" fn cb_ignore_data(_: *mut c_void, _: c_int, _: c_int, _: *mut c_void) -> c_int { 0 }
extern "C" fn cb_ignore_init(_: *mut c_void, _: c_int, _: *mut c_void) -> c_int { 0 }
extern "C" fn cb_ignore_bg(_: bool, _: c_int, _: *mut c_void) -> c_int { 0 }

fn infer_var_type(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.as_str() {
        "time" | "frequency" => lower,
        _ if lower.starts_with("i(") || lower.starts_with('@') || lower.contains("#branch") => "current".into(),
        _ => "voltage".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_var_type() {
        assert_eq!(infer_var_type("time"), "time");
        assert_eq!(infer_var_type("frequency"), "frequency");
        assert_eq!(infer_var_type("v(out)"), "voltage");
        assert_eq!(infer_var_type("i(Vin)"), "current");
        assert_eq!(infer_var_type("@m1[id]"), "current");
        assert_eq!(infer_var_type("V(net1)"), "voltage");
    }

    #[test]
    #[ignore] // Requires libngspice.so
    fn shared_runs_op_and_tran() {
        let s = Shared::new().expect("libngspice");
        let op = s.run("test op\nV1 vdd 0 3.3\nR1 vdd out 1k\nR2 out 0 2k\n.op\n.end\n").unwrap();
        assert!(!op.variables.is_empty());
        let tr = s.run("t\nV1 in 0 PULSE(0 1 0 1n 1n 5u 10u)\nR1 in out 1k\nC1 out 0 1n\n.tran 10n 20u\n.end\n").unwrap();
        assert!(tr.real_data[0].len() > 1);
    }
}
