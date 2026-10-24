//! Parser for nutmeg-format `.raw` files (ngspice, VACASK, LTspice, Spectre nutbin):
//! `key: value` header lines, a `Variables:` block, then `Binary:` or `Values:` and the data.
//! Only the first plot in a file is read.

use num_complex::Complex64;
use crate::result::{RawData, VarInfo};

#[derive(Debug, thiserror::Error)]
pub enum RawFileError {
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Unexpected format: {0}")]
    Format(String),
}

/// Parse a .raw file from bytes. Handles UTF-8 and LTspice's UTF-16-LE headers.
pub fn parse_raw(data: &[u8]) -> Result<RawData, RawFileError> {
    let utf16 = data.starts_with(&[0xFF, 0xFE])
        || (data.len() >= 10 && data[1] == 0 && data[3] == 0 && data[5] == 0);
    if utf16 {
        return parse_utf8(&decode_utf16_le(data));
    }
    parse_utf8(data)
}

/// LTspice header text is UTF-16-LE (ASCII in the low byte); binary data after
/// `Binary:` is copied through untouched.
fn decode_utf16_le(data: &[u8]) -> Vec<u8> {
    let body = data.strip_prefix(&[0xFF, 0xFE]).unwrap_or(data);
    let mut out = Vec::with_capacity(body.len() / 2);
    for (k, pair) in body.chunks_exact(2).enumerate() {
        out.push(pair[0]);
        if pair[0] == b'\n' && out.trim_ascii_end().ends_with(b"Binary:") {
            out.extend_from_slice(&body[2 * k + 2..]);
            break;
        }
    }
    out
}

fn bad<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> RawFileError + '_ {
    move |e| RawFileError::Parse(format!("Bad {what}: {e}"))
}

fn parse_utf8(data: &[u8]) -> Result<RawData, RawFileError> {
    let mut raw = RawData::empty();
    let (mut num_vars, mut num_points) = (0usize, 0usize);
    let (mut in_variables, mut fast_access) = (false, false);
    let mut pos = 0;

    let is_binary = loop {
        let nl = data[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| RawFileError::Format("Unexpected EOF in header".into()))?;
        let line = std::str::from_utf8(&data[pos..pos + nl]).map_err(bad("header text"))?.trim_end();
        pos += nl + 1;

        match line {
            "Binary:" => break true,
            "Values:" => break false,
            _ if line.starts_with("Variables:") => in_variables = true,
            _ if in_variables => {
                // "\tindex\tname\ttype [extra...]"
                let mut parts = line.split_whitespace();
                if let (Some(i), Some(name), Some(ty)) = (parts.next(), parts.next(), parts.next()) {
                    raw.variables.push(VarInfo {
                        index: i.parse().map_err(bad("variable index"))?,
                        name: name.to_string(),
                        var_type: ty.to_string(),
                    });
                }
            }
            _ => {
                if let Some((key, value)) = line.split_once(':') {
                    let value = value.trim();
                    match key.trim() {
                        "Title" => raw.title = value.to_string(),
                        "Plotname" => raw.plot_name = value.to_string(),
                        "Flags" => {
                            let flags = value.to_lowercase();
                            raw.is_complex = flags.contains("complex");
                            fast_access = flags.contains("fastaccess");
                        }
                        "No. Variables" => num_vars = value.parse().map_err(bad("num_vars"))?,
                        "No. Points" => num_points = value.parse().map_err(bad("num_points"))?,
                        _ => {}
                    }
                }
            }
        }
    };

    if num_vars == 0 || num_points == 0 {
        return Err(RawFileError::Format("No variables or points declared".into()));
    }
    if raw.variables.len() != num_vars {
        return Err(RawFileError::Format(format!(
            "Header declares {num_vars} variables but {} were listed",
            raw.variables.len()
        )));
    }
    if let Some(v) = raw.variables.iter().find(|v| v.index >= num_vars) {
        return Err(RawFileError::Format(format!(
            "Variable '{}' declares out-of-range index {} (num_vars = {num_vars})",
            v.name, v.index
        )));
    }

    let body = &data[pos..];
    if is_binary {
        parse_binary(body, &mut raw, num_vars, num_points, fast_access)?;
    } else {
        parse_ascii(body, &mut raw, num_vars, num_points)?;
    }
    Ok(raw)
}

/// Little-endian f64s, point-major (`p*nv + v`) or, with FastAccess, variable-major.
/// One length check, then each column is gathered straight from the byte block.
fn parse_binary(
    body: &[u8],
    raw: &mut RawData,
    nv: usize,
    np: usize,
    fast_access: bool,
) -> Result<(), RawFileError> {
    let width = if raw.is_complex { 2 } else { 1 };
    let need = nv * np * width * 8;
    if body.len() < need {
        return Err(RawFileError::Format(format!(
            "Binary block is {} bytes, expected {need}",
            body.len()
        )));
    }
    let f = |j: usize| f64::from_le_bytes(body[j * 8..j * 8 + 8].try_into().expect("8-byte slice"));
    let slot = |v: usize, p: usize| width * if fast_access { v * np + p } else { p * nv + v };
    if raw.is_complex {
        raw.complex_data = (0..nv)
            .map(|v| (0..np).map(|p| Complex64::new(f(slot(v, p)), f(slot(v, p) + 1))).collect())
            .collect();
    } else {
        raw.real_data = (0..nv).map(|v| (0..np).map(|p| f(slot(v, p))).collect()).collect();
    }
    Ok(())
}

/// Whitespace-separated: per point an index token, then one value per variable
/// (`re,im` for a complex plot).
fn parse_ascii(body: &[u8], raw: &mut RawData, nv: usize, np: usize) -> Result<(), RawFileError> {
    let text = std::str::from_utf8(body).map_err(bad("values text"))?;
    let mut tokens = text.split_ascii_whitespace();
    let mut next = || tokens.next().ok_or_else(|| RawFileError::Format("Values block ends early".into()));
    let num = |s: &str| s.parse::<f64>().map_err(|e| RawFileError::Parse(format!("Bad value '{s}': {e}")));

    if raw.is_complex {
        raw.complex_data = vec![Vec::with_capacity(np); nv];
    } else {
        raw.real_data = vec![Vec::with_capacity(np); nv];
    }
    for _ in 0..np {
        next()?; // point index
        for v in 0..nv {
            let tok = next()?;
            if raw.is_complex {
                let (re, im) = tok.split_once(',').unwrap_or((tok, "0"));
                raw.complex_data[v].push(Complex64::new(num(re)?, num(im)?));
            } else {
                raw.real_data[v].push(num(tok)?);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(flags: &str, vars: &[&str], np: usize, marker: &str) -> Vec<u8> {
        let mut h = format!(
            "Title: t\nPlotname: p\nFlags: {flags}\nNo. Variables: {}\nNo. Points: {np}\nVariables:\n",
            vars.len()
        );
        for (i, v) in vars.iter().enumerate() {
            h += &format!("\t{i}\t{v}\tvoltage\n");
        }
        h += marker;
        h.into_bytes()
    }

    #[test]
    fn ascii_real_and_complex() {
        let mut real = header("real", &["v(out)", "v(in)"], 2, "Values:\n");
        real.extend_from_slice(b" 0\t3.3\n\t1.0\n\n 1\t3.4\n\t1.1\n");
        let r = parse_raw(&real).unwrap();
        assert_eq!(r.title, "t");
        assert_eq!(r.real_data, vec![vec![3.3, 3.4], vec![1.0, 1.1]]);

        let mut cplx = header("complex", &["frequency", "v(out)"], 1, "Values:\n");
        cplx.extend_from_slice(b" 0\t1e3,0\n\t3,4\n");
        let c = parse_raw(&cplx).unwrap();
        assert!(c.is_complex && c.real_data.is_empty());
        assert_eq!(c.complex(1).unwrap()[0], Complex64::new(3.0, 4.0));
        assert_eq!(c.re(0), vec![1e3]);
    }

    /// Same data written interleaved and FastAccess must parse identically.
    #[test]
    fn binary_layouts_agree() {
        let cols: [[f64; 3]; 2] = [[0.0, 1e-6, 2e-6], [1.1, 2.2, 3.3]];
        let mut normal = header("real", &["time", "v(out)"], 3, "Binary:\n");
        for p in 0..3 {
            for col in &cols {
                normal.extend_from_slice(&col[p].to_le_bytes());
            }
        }
        let mut fast = header("real fastaccess", &["time", "v(out)"], 3, "Binary:\n");
        for col in &cols {
            for x in col {
                fast.extend_from_slice(&x.to_le_bytes());
            }
        }
        let want: Vec<Vec<f64>> = cols.iter().map(|c| c.to_vec()).collect();
        assert_eq!(parse_raw(&normal).unwrap().real_data, want);
        assert_eq!(parse_raw(&fast).unwrap().real_data, want);

        normal.truncate(normal.len() - 1);
        assert!(parse_raw(&normal).is_err(), "short binary block must error");
    }

    #[test]
    fn binary_complex_interleaved() {
        let mut buf = header("complex", &["frequency", "v(out)"], 2, "Binary:\n");
        for x in [1e3, 0.0, 0.99, -0.01, 1e4, 0.0, 0.8, -0.1] {
            buf.extend_from_slice(&f64::to_le_bytes(x));
        }
        let r = parse_raw(&buf).unwrap();
        assert_eq!(r.re(0), vec![1e3, 1e4]);
        assert_eq!(r.complex(1).unwrap()[1], Complex64::new(0.8, -0.1));
    }

    #[test]
    fn utf16_header_with_binary_body() {
        let mut buf = vec![0xFF, 0xFE];
        for b in header("real", &["time"], 1, "Binary:\n") {
            buf.extend_from_slice(&[b, 0]);
        }
        buf.extend_from_slice(&2.5f64.to_le_bytes());
        assert_eq!(parse_raw(&buf).unwrap().real_data, vec![vec![2.5]]);
    }
}
