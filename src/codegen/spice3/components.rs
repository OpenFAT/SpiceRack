//! One `Component` -> its SPICE3-family card (a Laplace source is rewritten
//! into a card the dialect parses).

use super::values::{dec, key, ngspice, num, value};
use super::waveform::{refuse_wave, waveform};
use crate::codegen::{CodeGenError, Dialect, laplace, params, verbatim};
use crate::ir::*;

/// `comp`'s card, written onto `out` with no line end (a Laplace source is
/// two lines). Nothing is written when it errs.
pub(crate) fn component(
    out: &mut String,
    d: Dialect,
    x: bool,
    comp: &Component,
) -> Result<(), CodeGenError> {
    let ps = |p| params(p, verbatim);
    match comp {
        Component::Resistor {
            name,
            n1,
            n2,
            value: v,
            model,
            params: p,
        }
        | Component::Capacitor {
            name,
            n1,
            n2,
            value: v,
            model,
            params: p,
        }
        | Component::Inductor {
            name,
            n1,
            n2,
            value: v,
            model,
            params: p,
        } => {
            let letter = match comp {
                Component::Resistor { .. } => 'R',
                Component::Capacitor { .. } => 'C',
                _ => 'L',
            };
            if model.is_some() && d == Dialect::Ltspice {
                return Err(CodeGenError::UnsupportedComponent {
                    backend: "ltspice".into(),
                    component: format!("{letter}{name}: model reference"),
                });
            }
            w!(out, "{letter}{} {} {}", name, n1, n2);
            rclvalue(out, d, x, v, model);
            w!(out, "{}", ps(p));
        }
        Component::MutualInductor {
            name,
            inductor1,
            inductor2,
            coupling,
        } => {
            w!(
                out,
                "K{} L{} L{} {}",
                name,
                inductor1,
                inductor2,
                dec(d, x, *coupling)
            )
        }
        Component::VoltageSource {
            name,
            np,
            nm,
            value: v,
            ac_magnitude,
            ac_phase,
            waveform: wf,
            ..
        }
        | Component::CurrentSource {
            name,
            np,
            nm,
            value: v,
            ac_magnitude,
            ac_phase,
            waveform: wf,
        } => {
            let letter = if matches!(comp, Component::VoltageSource { .. }) {
                'V'
            } else {
                'I'
            };
            if let Some(wf) = wf {
                refuse_wave(d, letter, name, wf)?;
            }
            let port = match comp {
                Component::VoltageSource { port, .. } => port.as_ref(),
                _ => None,
            };
            // ngspice and ESPice only (manual §4.1.11; ESPice reference,
            // `portnum n z0 Z`); HSPICE's is a `P` element, and dropping the
            // port would lose its series Z0.
            if port.is_some() && !ngspice(d) {
                return Err(CodeGenError::UnsupportedComponent {
                    backend: d.name().into(),
                    component: format!("V{name}: RF port"),
                });
            }
            w!(out, "{}{} {} {} {}", letter, name, np, nm, value(d, x, v));
            if let Some(mag) = ac_magnitude {
                w!(out, " AC {}", dec(d, x, *mag));
                if let Some(phase) = ac_phase {
                    w!(out, " {}", dec(d, x, *phase));
                }
            }
            if let Some(p) = port {
                w!(out, " portnum {} z0 {}", p.num, dec(d, x, p.z0));
            }
            if let Some(wf) = wf {
                out.push(' ');
                waveform(out, d, x, wf);
            }
        }
        Component::BehavioralVoltage {
            name,
            np,
            nm,
            expression,
        } => {
            if laplace::is_laplace(expression) {
                out.push_str(&laplace_source(d, x, name, np, nm, expression, false)?);
            } else if d == Dialect::Hspice {
                w!(out, "E{} {} {} vol='{}'", name, np, nm, expression)
            } else if d == Dialect::Xyce {
                // Xyce takes the B expression braced (RG p.238).
                w!(out, "B{name} {np} {nm} V={{{expression}}}")
            } else {
                w!(out, "B{} {} {} V={}", name, np, nm, expression)
            }
        }
        Component::BehavioralCurrent {
            name,
            np,
            nm,
            expression,
        } => {
            if laplace::is_laplace(expression) {
                out.push_str(&laplace_source(d, x, name, np, nm, expression, true)?);
            } else if d == Dialect::Hspice {
                w!(out, "G{} {} {} cur='{}'", name, np, nm, expression)
            } else if d == Dialect::Xyce {
                w!(out, "B{name} {np} {nm} I={{{expression}}}")
            } else {
                w!(out, "B{} {} {} I={}", name, np, nm, expression)
            }
        }
        Component::Vcvs {
            name,
            np,
            nm,
            ncp,
            ncm,
            gain,
        } => w!(
            out,
            "E{} {} {} {} {} {}",
            name,
            np,
            nm,
            ncp,
            ncm,
            dec(d, x, *gain)
        ),
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
                "G{} {} {} {} {} {}",
                name,
                np,
                nm,
                ncp,
                ncm,
                dec(d, x, *transconductance)
            )
        }
        Component::Cccs {
            name,
            np,
            nm,
            vsense,
            gain,
        } => w!(
            out,
            "F{} {} {} {} {}",
            name,
            np,
            nm,
            vsense,
            dec(d, x, *gain)
        ),
        Component::Poly {
            name,
            letter,
            np,
            nm,
            controls,
            coeffs,
        } => {
            let dims = if matches!(letter, 'E' | 'G') {
                controls.len() / 2
            } else {
                controls.len()
            };
            w!(
                out,
                "{letter}{name} {np} {nm} POLY({dims}) {} ",
                controls.join(" ")
            );
            for (i, c) in coeffs.iter().enumerate() {
                w!(out, "{}{}", if i == 0 { "" } else { " " }, num(d, x, *c));
            }
        }
        Component::Ccvs {
            name,
            np,
            nm,
            vsense,
            transresistance,
        } => {
            w!(
                out,
                "H{} {} {} {} {}",
                name,
                np,
                nm,
                vsense,
                dec(d, x, *transresistance)
            )
        }
        Component::Diode {
            name,
            np,
            nm,
            model,
            params: p,
        } => w!(out, "D{} {} {} {}{}", name, np, nm, model, ps(p)),
        Component::Bjt {
            name,
            nc,
            nb,
            ne,
            extra_nodes,
            model,
            params: p,
        } => {
            w!(out, "Q{} {} {} {}", name, nc, nb, ne);
            for n in extra_nodes {
                w!(out, " {n}");
            }
            w!(out, " {}{}", model, ps(p));
        }
        Component::Mosfet {
            name,
            nd,
            ng,
            ns,
            nb,
            extra_nodes,
            model,
            params: p,
        } => {
            w!(out, "M{} {} {} {}", name, nd, ng, ns);
            for n in nb.iter().chain(extra_nodes) {
                w!(out, " {n}");
            }
            w!(out, " {}{}", model, ps(p));
        }
        Component::Jfet {
            name,
            nd,
            ng,
            ns,
            model,
            params: p,
        } => w!(out, "J{} {} {} {} {}{}", name, nd, ng, ns, model, ps(p)),
        Component::Mesfet {
            name,
            nd,
            ng,
            ns,
            model,
            params: p,
        } => w!(out, "Z{} {} {} {} {}{}", name, nd, ng, ns, model, ps(p)),
        Component::VSwitch {
            name,
            np,
            nm,
            ncp,
            ncm,
            model,
        } => w!(out, "S{} {} {} {} {} {}", name, np, nm, ncp, ncm, model),
        Component::ISwitch {
            name,
            np,
            nm,
            vcontrol,
            model,
        } => w!(out, "W{} {} {} {} {}", name, np, nm, vcontrol, model),
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
                "T{} {} {} {} {} Z0={} TD={}",
                name,
                inp,
                inm,
                outp,
                outm,
                dec(d, x, *z0),
                dec(d, x, *td)
            )
        }
        Component::Xspice { name, .. } if matches!(d, Dialect::Hspice | Dialect::Xyce) => {
            return Err(CodeGenError::UnsupportedComponent {
                backend: key(d).into(),
                component: format!("XSPICE A{}", name),
            });
        }
        Component::Xspice {
            name,
            connections,
            model,
        } => {
            // LTspice has no XSPICE A-elements -- emit as comment
            let lead = if d == Dialect::Ltspice {
                "* XSPICE (unsupported): "
            } else {
                ""
            };
            w!(out, "{lead}A{}", name);
            for conn in connections {
                w!(out, " {conn}");
            }
            w!(out, " {model}");
        }
        Component::RawSpice { line } => out.push_str(line),
    }
    Ok(())
}

// ---- private ----

/// R/C/L value field, with an optional `.model` reference after it:
/// ` [value] model`. The import side leaves `value` an empty `Raw` when the
/// bare word was a model name, so an empty text is skipped rather than
/// written as a blank field.
fn rclvalue(out: &mut String, d: Dialect, x: bool, v: &IrValue, model: &Option<String>) {
    match v {
        IrValue::Raw { text } if text.is_empty() => {}
        v => w!(out, " {}", value(d, x, v)),
    }
    if let Some(m) = model {
        w!(out, " {m}");
    }
}

/// A `Laplace(...)` behavioural source, rewritten into something the
/// dialect actually parses.
///
/// The verbatim `B1 out 0 V=Laplace(v(in), 1/(1+s*1.59e-4))` this replaces
/// is not syntax any supported simulator has: ngspice 44.2 answers it with
/// `Undefined parameter [s]` and `exit(1)`. See `codegen::laplace` for the
/// evidence behind both replacements.
///
/// * ngspice: the XSPICE `s_xfer` code model, two lines (instance +
///   `.model`). Verified numerically.
/// * HSPICE and ESPice: `E/G … LAPLACE c+ c- num… / den…`. ESPice verified
///   numerically (`tests/test_laplace_ngspice.rs`).
/// * LTspice: `Laplace=` on an E/G source. **Unverified** — LTspice cannot
///   be run here. The expression is still parsed first, so what goes out is
///   a transfer function we understand rather than an opaque string.
fn laplace_source(
    d: Dialect,
    x: bool,
    name: &str,
    np: &str,
    nm: &str,
    expression: &str,
    current_output: bool,
) -> Result<String, CodeGenError> {
    let parsed =
        laplace::parse(expression).map_err(|why| CodeGenError::Other(format!("B{name}: {why}")))?;
    let (ip, im) = parsed.input().clone();
    let letter = if current_output { 'G' } else { 'E' };

    // A constant H(s) is a plain controlled source in every dialect. It is
    // also the only safe option on ngspice: `den_coeff=[1]` segfaults
    // ngspice 44.2 outright.
    if let laplace::Laplace::Gain { gain, .. } = parsed {
        return Ok(format!(
            "{letter}B{name} {np} {nm} {ip} {im} {}",
            num(d, x, gain)
        ));
    }
    let laplace::Laplace::Xfer { num, den, .. } = parsed else {
        unreachable!()
    };

    // Xyce has no LAPLACE E/G form (RG §6.1.5).
    if d == Dialect::Xyce {
        return Err(CodeGenError::UnsupportedComponent {
            backend: "xyce".into(),
            component: format!("B{name}: Laplace source"),
        });
    }
    if d == Dialect::Ltspice {
        return Ok(format!(
            "{letter}B{name} {np} {nm} {ip} {im} Laplace={}/{}",
            laplace::horner(&num),
            laplace::horner(&den),
        ));
    }
    // `E1 b 0 LAPLACE a 0 k0 k1 ... / d0 d1 ...`, ascending powers. ESPice
    // reads the same card in its ngspice tokenizer and has no XSPICE
    // `s_xfer` (ESPice `docs/using/reference.md`, controlled sources).
    if matches!(d, Dialect::Hspice | Dialect::Espice) {
        let asc = |c: &[f64]| laplace::coeff_array(&c.iter().rev().copied().collect::<Vec<_>>());
        return Ok(format!(
            "{letter}B{name} {np} {nm} LAPLACE {ip} {im} {} / {}",
            asc(&num),
            asc(&den)
        ));
    }
    let port = if current_output { "id" } else { "vd" };
    // One integrator stage per denominator order; `int_ic=[]` is
    // rejected with "Array parameter must have at least one value".
    let ic = vec!["0"; den.len() - 1].join(" ");
    Ok(format!(
        "AB{name} %vd({ip} {im}) %{port}({np} {nm}) B{name}_xfer\n\
         .model B{name}_xfer s_xfer(num_coeff=[{}] den_coeff=[{}] int_ic=[{ic}])",
        laplace::coeff_array(&num),
        laplace::coeff_array(&den),
    ))
}
