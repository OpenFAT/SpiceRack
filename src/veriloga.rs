//! HDL sources on their way to a simulator: Verilog-A models and digital
//! Verilog blocks, plus the external tools that compile them.
//!
//! `veriloga()` records the `.va` source in a subcircuit's load list; each
//! backend's codegen decides what that becomes. ngspice and VACASK load OSDI,
//! so their codegen compiles the source with OpenVAF ([`compile_osdi`]);
//! Spectre includes it with `ahdl_include`; ESPice compiles it itself through
//! VerA (`.hdl`), so no OpenVAF run happens on that path.
//!
//! A digital Verilog block ([`lower`]) is either co-simulated (Icarus `.vvp`
//! under ngspice `d_cosim`) or synthesized by Yosys into standard cells.

#[cfg(feature = "backend")]
use std::ffi::OsStr;
#[cfg(feature = "backend")]
use std::path::{Path, PathBuf};
#[cfg(feature = "backend")]
use std::process::Command;

#[cfg(feature = "backend")]
use crate::ir::{VerilogBlock, VerilogConnection, VerilogMode};

/// True for a load-list entry that is Verilog-A source rather than a binary.
pub fn is_source(path: &str) -> bool {
    let p = std::path::Path::new(path);
    matches!(p.extension().and_then(|e| e.to_str()), Some(e) if e.eq_ignore_ascii_case("va") || e.eq_ignore_ascii_case("vams"))
}

#[cfg(feature = "backend")]
/// The `.va` file for a path or inline source.
pub fn resolve(source_or_path: &str) -> Result<PathBuf, String> {
    resolve_ext(source_or_path, "va")
}

#[cfg(feature = "backend")]
/// An existing `.{ext}` file (made absolute, so a deck written to a temp dir
/// still finds it), or inline source written to a temp file named by its
/// hash, so the same text always maps to the same file.
fn resolve_ext(source_or_path: &str, ext: &str) -> Result<PathBuf, String> {
    let trimmed = source_or_path.trim();
    if trimmed.ends_with(&format!(".{ext}")) && !trimmed.contains('\n') {
        let p = Path::new(trimmed);
        if !p.exists() {
            return Err(format!("{} file not found: {}", if ext == "va" { "Verilog-A" } else { "Verilog" }, trimmed));
        }
        return p.canonicalize().map_err(|e| format!("{}: {}", trimmed, e));
    }
    let dir = std::env::temp_dir().join("spicerack_hdl");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        trimmed.hash(&mut h);
        h.finish()
    };
    let file = dir.join(format!("inline_{:016x}.{ext}", hash));
    std::fs::write(&file, trimmed).map_err(|e| format!("Failed to write temp .{ext} file: {}", e))?;
    Ok(file)
}

#[cfg(feature = "backend")]
/// Run `tool args` to make `out` from `src`, skipped when `out` is newer.
fn run_tool(src: &Path, out: &Path, tool: &str, install: &str, args: &[&OsStr]) -> Result<String, String> {
    let mtime = |p: &Path| p.metadata().and_then(|m| m.modified());
    let done = out.to_string_lossy().into_owned();
    if let (Ok(s), Ok(o)) = (mtime(src), mtime(out))
        && o > s
    {
        return Ok(done);
    }
    let output = Command::new(tool).args(args).output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{tool} not found on $PATH. {install}"),
        _ => format!("Failed to run {tool}: {e}"),
    })?;
    if !output.status.success() {
        return Err(format!(
            "{tool} failed (exit {}):\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }
    Ok(done)
}

#[cfg(feature = "backend")]
/// Compile `va` to an `.osdi` beside it with OpenVAF.
pub fn compile_osdi(va: &Path) -> Result<String, String> {
    let osdi = va.with_extension("osdi");
    let install = "Install OpenVAF to compile Verilog-A models.\nSee: https://openvaf.semimod.de";
    run_tool(va, &osdi, "openvaf", install, &[va.as_os_str(), "-o".as_ref(), osdi.as_os_str()])
}

#[cfg(feature = "backend")]
/// A load list with every Verilog-A source replaced by its compiled `.osdi`,
/// for the backends that load OSDI.
pub fn osdi_loads(loads: &[String]) -> Result<Vec<String>, String> {
    loads.iter().map(|l| if is_source(l) { compile_osdi(Path::new(l)) } else { Ok(l.clone()) }).collect()
}

#[cfg(feature = "backend")]
/// What a digital Verilog block becomes in a netlist.
#[derive(Debug, PartialEq)]
pub enum Lowered {
    /// ngspice XSPICE `d_cosim` running the compiled `vvp`; one `[net ...]`
    /// group per port.
    Cosim { vvp: String, ports: Vec<String> },
    /// Standard-cell subcircuit calls; `spice_models` holds the cell subckts.
    Gates { spice_models: String, cells: Vec<Cell> },
}

#[cfg(feature = "backend")]
/// One synthesized standard-cell instance.
#[derive(Debug, PartialEq)]
pub struct Cell {
    pub name: String,
    pub cell: String,
    pub nodes: Vec<String>,
}

#[cfg(feature = "backend")]
fn nets(c: &VerilogConnection) -> &[String] {
    match c {
        VerilogConnection::Single(n) => std::slice::from_ref(n),
        VerilogConnection::Bus(v) => v,
    }
}

#[cfg(feature = "backend")]
/// Compile (Icarus) or synthesize (Yosys) a Verilog block.
pub fn lower(vb: &VerilogBlock) -> Result<Lowered, String> {
    let v = resolve_ext(&vb.source, "v")?;
    if matches!(vb.mode, VerilogMode::Simulate) {
        let vvp = v.with_extension("vvp");
        let install = "Install Icarus Verilog for co-simulation.\n\
             Install: sudo apt install iverilog (Debian/Ubuntu) or brew install icarus-verilog (macOS)";
        let vvp = run_tool(&v, &vvp, "iverilog", install, &["-o".as_ref(), vvp.as_os_str(), v.as_os_str()])?;
        let ports = vb.connections.values().map(|c| format!("[{}]", nets(c).join(" "))).collect();
        return Ok(Lowered::Cosim { vvp, ports });
    }

    let liberty = match (&vb.liberty, &vb.pdk) {
        (Some(l), _) => l.clone(),
        (None, Some(p)) => pdk_paths(p)?.0,
        (None, None) => return Err("mode='synthesize' requires either pdk or liberty".into()),
    };
    let spice_models = match (&vb.spice_models, &vb.pdk) {
        (Some(s), _) => s.clone(),
        (None, Some(p)) => pdk_paths(p)?.1,
        (None, None) => return Err("mode='synthesize' requires either pdk or both liberty and spice_models".into()),
    };

    let text = std::fs::read_to_string(&v).map_err(|e| format!("Failed to read {}: {}", v.display(), e))?;
    let top = regex::Regex::new(r"(?m)^\s*module\s+(\w+)")
        .expect("static regex")
        .captures(&text)
        .map(|c| c[1].to_string())
        .ok_or("Could not extract module name from Verilog source. Ensure source contains `module <name>`.")?;
    let synth = v.with_extension("synth.v");
    let script = format!(
        "read_verilog {}; synth -top {top}; dfflibmap -liberty {liberty}; abc -liberty {liberty}; write_verilog {}",
        v.display(),
        synth.display()
    );
    let install = "Install Yosys for Verilog synthesis.\n\
         Install: sudo apt install yosys (Debian/Ubuntu) or brew install yosys (macOS)";
    let synth = run_tool(&v, &synth, "yosys", install, &["-p".as_ref(), script.as_ref()])?;
    let netlist = std::fs::read_to_string(&synth).map_err(|e| format!("Failed to read synthesized netlist: {}", e))?;

    // A synthesized net named like a user port lands on that port's first net;
    // every other net is prefixed with the instance name.
    let inst = &vb.instance_name;
    let cells = gate_instances(&netlist)
        .into_iter()
        .map(|(gate, cell, ports)| Cell {
            name: format!("{inst}_{gate}"),
            cell,
            nodes: ports
                .into_iter()
                .map(|net| match vb.connections.get(&net).and_then(|c| nets(c).first()) {
                    Some(n) => n.clone(),
                    None => format!("{inst}_{net}"),
                })
                .collect(),
        })
        .collect();
    Ok(Lowered::Gates { spice_models, cells })
}

#[cfg(feature = "backend")]
/// Cell instances of a Yosys `write_verilog` netlist as (instance, cell, nets
/// in port order), e.g. `sky130_fd_sc_hd__and2_1 _0_ ( .A(\a ), .X(\out ) );`.
fn gate_instances(netlist: &str) -> Vec<(String, String, Vec<String>)> {
    let inst = regex::Regex::new(r"(?m)^\s+(\w+)\s+(\w+)\s*\(([^;]+)\);").expect("static regex");
    let port = regex::Regex::new(r"\.\s*\w+\s*\(\s*\\?([^\s)]+)\s*\)").expect("static regex");
    inst.captures_iter(netlist)
        .filter(|c| !matches!(&c[1], "wire" | "assign" | "input" | "output" | "module" | "endmodule" | "reg"))
        .map(|c| (c[2].to_string(), c[1].to_string(), port.captures_iter(&c[3]).map(|p| p[1].to_string()).collect()))
        .collect()
}

#[cfg(feature = "backend")]
/// (liberty, SPICE cell models) of `$PDK_ROOT/<pdk>[A]/libs.ref/<pdk>/`;
/// the liberty file prefers the `tt` corner.
fn pdk_paths(pdk: &str) -> Result<(String, String), String> {
    let root = std::env::var("PDK_ROOT").map_err(|_| {
        "PDK_ROOT environment variable not set. Set it to your PDK installation root, \
         or provide explicit liberty= and spice_models= paths."
            .to_string()
    })?;
    let find = |sub: &str, ext: &str, prefer: &str| {
        ["A", ""].into_iter().find_map(|sfx| {
            let dir = std::fs::read_dir(format!("{root}/{pdk}{sfx}/libs.ref/{pdk}/{sub}")).ok()?;
            let mut files: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == ext)).collect();
            files.sort();
            let i = files.iter().position(|p| p.file_name().is_some_and(|n| n.to_string_lossy().contains(prefer))).unwrap_or(0);
            files.get(i).map(|p| p.to_string_lossy().into_owned())
        })
        .ok_or_else(|| {
            format!(
                "Could not find .{ext} file for PDK '{pdk}' under $PDK_ROOT={root}. \
                 Searched: {root}/{pdk}A/libs.ref/{pdk}/{sub}/ and {root}/{pdk}/libs.ref/{pdk}/{sub}/"
            )
        })
    };
    Ok((find("lib", "lib", "tt")?, find("spice", "spice", "")?))
}

/// Without the `backend` feature nothing runs OpenVAF: a Verilog-A source in
/// the load list is an error, a compiled `.osdi` passes through.
#[cfg(not(feature = "backend"))]
pub fn osdi_loads(loads: &[String]) -> Result<Vec<String>, String> {
    loads
        .iter()
        .map(|l| {
            if is_source(l) {
                Err(format!("{l}: compiling Verilog-A needs spicerack's `backend` feature"))
            } else {
                Ok(l.clone())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_vs_binary() {
        assert!(is_source("/m/a.va"));
        assert!(is_source("A.VA"));
        assert!(!is_source("/m/a.osdi"));
        assert!(!is_source("/m/va"));
    }

    #[test]
    fn binaries_pass_through_without_openvaf() {
        assert_eq!(osdi_loads(&["/m/a.osdi".into()]).unwrap(), vec!["/m/a.osdi".to_string()]);
    }

    #[cfg(feature = "backend")]
    #[test]
    fn inline_source_resolves_to_a_stable_file() {
        let src = "module t(a); inout a; electrical a; endmodule";
        let a = resolve(src).unwrap();
        assert_eq!(a, resolve(src).unwrap());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), src);
    }

    #[cfg(feature = "backend")]
    #[test]
    fn yosys_netlist_cells() {
        let n = "module top(a, b, out);\n  input a;\n  sky130_fd_sc_hd__and2_1 _0_ ( .A(\\a ), .B(b), .X(\\out ) );\nendmodule\n";
        assert_eq!(
            gate_instances(n),
            vec![("_0_".into(), "sky130_fd_sc_hd__and2_1".into(), vec!["a".into(), "b".into(), "out".into()])]
        );
    }
}
