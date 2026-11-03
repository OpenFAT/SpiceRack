//! One `Component` -> its Spectre instance statement. Sources as in
//! `spectre/mod.rs`.
//!
//! Contents:
//! - `pub(crate) fn emit_component(out: &mut String, comp: &Component, n: Names) -> Result<(), CodeGenError>`

use crate::ir::*;

use super::names::Names;
use super::values::{
    emit_value, emit_waveform_params, spectre_num, spectre_value, unsupported_component,
};
use crate::codegen::{CodeGenError, model_refuse};

/// `comp`'s instance statement, written onto `out` with no line end; its
/// identifiers spelled by `n`. Nothing is written when it errs.
pub(crate) fn emit_component(
    out: &mut String,
    comp: &Component,
    n: Names,
) -> Result<(), CodeGenError> {
    let ps = |p| crate::codegen::params(p, spectre_value);
    match comp {
        // `[CMP]` p.628 `r1 (1 2) resistor r=1.2K m=2`
        Component::Resistor {
            name,
            n1,
            n2,
            value,
            model,
            params,
        } => {
            if model.is_some() {
                model_refuse("spectre", &n.d(name).to_string(), model)?;
            }
            w!(
                out,
                "r{} ({} {}) resistor r={}{}",
                n.d(name),
                n.n(n1),
                n.n(n2),
                emit_value(value),
                ps(params)
            );
        }
        // `[CMP]` p.283 `c2 (1 0) capacitor c=2.5u tc1=1e-8`
        Component::Capacitor {
            name,
            n1,
            n2,
            value,
            model,
            params,
        } => {
            if model.is_some() {
                model_refuse("spectre", &n.d(name).to_string(), model)?;
            }
            w!(
                out,
                "c{} ({} {}) capacitor c={}{}",
                n.d(name),
                n.n(n1),
                n.n(n2),
                emit_value(value),
                ps(params)
            );
        }
        // `[CMP]` p.372 `l33 (0 net29) inductor l=10e-9 r=1 m=1`
        Component::Inductor {
            name,
            n1,
            n2,
            value,
            model,
            params,
        } => {
            if model.is_some() {
                model_refuse("spectre", &n.d(name).to_string(), model)?;
            }
            w!(
                out,
                "l{} ({} {}) inductor l={}{}",
                n.d(name),
                n.n(n1),
                n.n(n2),
                emit_value(value),
                ps(params)
            );
        }
        // `[CMP]` p.577 `ml1 mutual_inductor coupling=1 ind1=l1 ind2=l2`
        Component::MutualInductor {
            name,
            inductor1,
            inductor2,
            coupling,
        } => {
            w!(
                out,
                "k{} mutual_inductor coupling={} ind1=l{} ind2=l{}",
                n.d(name),
                spectre_num(*coupling),
                n.d(inductor1),
                n.d(inductor2)
            );
        }
        // `[CMP]` p.683 `Name p n vsource parameter=value ...`; `dc` is
        // instance parameter #1, `mag`/`phase` are the small-signal
        // parameters #39/#40 (p.685).
        // `[CMP]` p.380 `i1 (in 0) isource dc=0 type=pulse delay=10n ...`
        // An RF port is Spectre's `port` instance, whose mapping from
        // ngspice's `portnum`/`z0` is unverified: refused, not dropped.
        Component::VoltageSource {
            name,
            port: Some(_),
            ..
        } => return Err(unsupported_component(&format!("V{name}: RF port"))),
        Component::VoltageSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform,
            port: None,
        }
        | Component::CurrentSource {
            name,
            np,
            nm,
            value,
            ac_magnitude,
            ac_phase,
            waveform,
        } => {
            let (l, master) = if matches!(comp, Component::VoltageSource { .. }) {
                ('v', "vsource")
            } else {
                ('i', "isource")
            };
            let wave = waveform.as_ref().map(emit_waveform_params).transpose()?;
            w!(
                out,
                "{l}{} ({} {}) {master} dc={}",
                n.d(name),
                n.n(np),
                n.n(nm),
                emit_value(value)
            );
            if let Some(mag) = ac_magnitude {
                w!(out, " mag={}", spectre_num(*mag));
                if let Some(phase) = ac_phase {
                    w!(out, " phase={}", spectre_num(*phase));
                }
            }
            if let Some(wave) = wave {
                w!(out, " {wave}");
            }
        }
        // Spectre's bsource does exist (`[CMP]` p.856:
        // `name (node1 node2) bsource v=generic_expr`) but its expression
        // grammar is not SPICE's: node voltages are `v(a,b)` (lowercase —
        // the language is case sensitive, `[KUN]` B.2), branch currents are
        // `i("inst_id:index")` not `I(Vx)`, and time is `$time`. The IR
        // holds an opaque SPICE expression string we cannot faithfully
        // rewrite, so refuse rather than emit something that looks right.
        Component::BehavioralVoltage { .. } | Component::BehavioralCurrent { .. } => {
            return Err(unsupported_component(
                "behavioural source (Spectre bsource uses v(a,b) / i(\"inst:idx\") / $time, \
                 not SPICE V()/I()/time; the expression cannot be translated safely)",
            ));
        }
        // `[CMP]` p.680 `e1 (out1 0 pos neg) vcvs gain=10`
        Component::Vcvs {
            name,
            np,
            nm,
            ncp,
            ncm,
            gain,
        } => {
            w!(
                out,
                "e{} ({} {} {} {}) vcvs gain={}",
                n.d(name),
                n.n(np),
                n.n(nm),
                n.n(ncp),
                n.n(ncm),
                spectre_num(*gain)
            )
        }
        // `[CMP]` p.678 `Name sink src ps ns ... vccs`, parameter `gm`
        Component::Vccs {
            name,
            np,
            nm,
            ncp,
            ncm,
            transconductance,
        } => {
            w!(
                out,
                "g{} ({} {} {} {}) vccs gm={}",
                n.d(name),
                n.n(np),
                n.n(nm),
                n.n(ncp),
                n.n(ncm),
                spectre_num(*transconductance)
            )
        }
        // `[CMP]` p.286 `vcs (pos gnd) cccs gain=2.5 probe=v1`
        Component::Cccs {
            name,
            np,
            nm,
            vsense,
            gain,
        } => {
            w!(
                out,
                "f{} ({} {}) cccs probe={} gain={}",
                n.d(name),
                n.n(np),
                n.n(nm),
                n.d(vsense),
                spectre_num(*gain)
            )
        }
        // R5 pp.590-616: `pvcvs`/`pvccs` take the controlling pairs as
        // terminals, `pcccs`/`pccvs` the sensing sources as `probes`; both
        // read `coeffs` in SPICE's term order. (Not run here: no Spectre.)
        Component::Poly {
            name,
            letter,
            np,
            nm,
            controls,
            coeffs,
        } => {
            let master = match letter {
                'E' => "pvcvs",
                'G' => "pvccs",
                'F' => "pcccs",
                _ => "pccvs",
            };
            // Controlling nodes are terminals; sensing sources are probes.
            let nodes = matches!(letter, 'E' | 'G');
            let l = letter.to_ascii_lowercase();
            if nodes {
                w!(out, "{l}{} ({} {} ", n.d(name), n.n(np), n.n(nm));
            } else {
                w!(
                    out,
                    "{l}{} ({} {}) {master} probes=[",
                    n.d(name),
                    n.n(np),
                    n.n(nm)
                );
            }
            for (i, c) in controls.iter().enumerate() {
                let c = if nodes { n.n(c) } else { n.d(c) };
                w!(out, "{}{c}", if i == 0 { "" } else { " " });
            }
            if nodes {
                w!(out, ") {master} coeffs=[");
            } else {
                out.push_str("] coeffs=[");
            }
            for (i, c) in coeffs.iter().enumerate() {
                w!(out, "{}{}", if i == 0 { "" } else { " " }, spectre_num(*c));
            }
            out.push(']');
        }
        // `[CMP]` p.288 `vvs (pos gnd) ccvs rm=1 probe=v1`
        Component::Ccvs {
            name,
            np,
            nm,
            vsense,
            transresistance,
        } => {
            w!(
                out,
                "h{} ({} {}) ccvs probe={} rm={}",
                n.d(name),
                n.n(np),
                n.n(nm),
                n.d(vsense),
                spectre_num(*transresistance)
            )
        }
        // `[CMP]` p.302 `Name a c ModelName parameter=value ...`,
        // `d0 (dp dn) pdiode l=3e-4 w=2.5e-4 area=1`
        Component::Diode {
            name,
            np,
            nm,
            model,
            params,
        } => {
            w!(
                out,
                "d{} ({} {}) {}{}",
                n.d(name),
                n.n(np),
                n.n(nm),
                n.n(model),
                ps(params)
            )
        }
        // `[CMP]` bjt models: `Name c b e [s] ModelName parameter=value ...`
        Component::Bjt {
            name,
            nc,
            nb,
            ne,
            extra_nodes,
            model,
            params,
        } => {
            w!(out, "q{} ({} {} {}", n.d(name), n.n(nc), n.n(nb), n.n(ne));
            for x in extra_nodes {
                w!(out, " {}", n.n(x));
            }
            w!(out, ") {}{}", n.n(model), ps(params));
        }
        // `[CMP]` mos models: `Name d g s b ModelName parameter=value ...`
        Component::Mosfet {
            name,
            nd,
            ng,
            ns,
            nb: Some(nb),
            extra_nodes,
            model,
            params,
        } if extra_nodes.is_empty() => {
            w!(
                out,
                "m{} ({} {} {} {}) {}{}",
                n.d(name),
                n.n(nd),
                n.n(ng),
                n.n(ns),
                n.n(nb),
                n.n(model),
                ps(params)
            )
        }
        Component::Mosfet { .. } => {
            return Err(unsupported_component(
                "MOSFET without a bulk node or with extra nodes (VDMOS, SOI): no mapped Spectre terminal list",
            ));
        }
        // `[CMP]` jfet: `Name d g s ModelName parameter=value ...`
        Component::Jfet {
            name,
            nd,
            ng,
            ns,
            model,
            params,
        } => {
            w!(
                out,
                "j{} ({} {} {}) {}{}",
                n.d(name),
                n.n(nd),
                n.n(ng),
                n.n(ns),
                n.n(model),
                ps(params)
            )
        }
        Component::Mesfet {
            name,
            nd,
            ng,
            ns,
            model,
            params,
        } => {
            w!(
                out,
                "z{} ({} {} {}) {}{}",
                n.d(name),
                n.n(nd),
                n.n(ng),
                n.n(ns),
                n.n(model),
                ps(params)
            )
        }
        // Spectre's voltage-controlled switch is `relay`, `[CMP]` p.625:
        // `Name 1 2 ps ns ModelName parameter=value ...` /
        // `rel1 (1 2 ps ns) my_relay ropen=1G rclosed=2` — the same node
        // order as SPICE `S`. NOTE: the referenced model must be declared
        // `model <name> relay ...`; a SPICE `.model <name> sw` carried
        // through the IR names a primitive Spectre does not have.
        Component::VSwitch {
            name,
            np,
            nm,
            ncp,
            ncm,
            model,
        } => {
            w!(
                out,
                "s{} ({} {} {} {}) {}",
                n.d(name),
                n.n(np),
                n.n(nm),
                n.n(ncp),
                n.n(ncm),
                n.n(model)
            )
        }
        // `relay` is voltage controlled only; `switch` (`[CMP]` p.643) is a
        // multi-throw switch whose position only changes between analyses.
        // Neither is a current-controlled switch.
        Component::ISwitch { .. } => {
            return Err(unsupported_component(
                "current-controlled switch (Spectre has relay (voltage controlled) and \
                 switch (position set between analyses); neither is SPICE's W element)",
            ));
        }
        // `[CMP]` p.644-645 `t1 (1 0 2 0) tline z0=100`, instance params
        // `z0` (#1) and `td` (#2).
        Component::TLine {
            name,
            inp,
            inm,
            outp,
            outm,
            z0,
            td,
        } => {
            w!(
                out,
                "t{} ({} {} {} {}) tline z0={} td={}",
                n.d(name),
                n.n(inp),
                n.n(inm),
                n.n(outp),
                n.n(outm),
                spectre_num(*z0),
                spectre_num(*td)
            )
        }
        Component::Xspice { .. } => {
            return Err(unsupported_component(
                "XSPICE A-device (ngspice-specific; Spectre's behavioural path is Verilog-A \
                 via ahdl_include)",
            ));
        }
        Component::RawSpice { line } => {
            return Err(unsupported_component(&format!(
                "raw SPICE line '{}' (Spectre reads SPICE only inside a simulator lang=spice \
                 region, which cannot be placed automatically)",
                line.trim()
            )));
        }
    }
    Ok(())
}
