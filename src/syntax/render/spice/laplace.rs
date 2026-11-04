//! A behavioural source's `Laplace(V(a[,b]), H(s))` expression tree → the
//! rational transfer function it names, for the `Lift(Laplace*)` rules:
//! the call found by walking the value's tree, its input probe read off
//! the first argument, and `H(s)` evaluated over the second argument's
//! tree (numbers, `s`, `+ - * /`, whole powers, parentheses) into a
//! numerator and denominator in descending powers of `s`; and the
//! coefficient spellings the rules write. No text is scanned.
//!
//! A tree the rule cannot read (a symbolic coefficient, a fractional
//! power, an improper or constant-denominator ratio with a pole term,
//! arithmetic around the call, a current input) is `W0010`'s.
//!
//! Cites: what ngspice 44.2 accepts, as the old writer's Laplace reader
//! recorded it: descending coefficients, `deg num <= deg den`, no
//! degree-0 denominator (a constant `H(s)` written as `den_coeff=[1]`
//! segfaults it).
//!
//! Contents:
//! - `pub(super) enum Laplace`
//! - `fn has_laplace(rd: &Rd, v: u32) -> bool`, `fn read(rd: &Rd, v: u32) -> Option<Laplace>`
//! - `fn coeff_array(c: &[f64]) -> String`, `fn horner(c: &[f64]) -> String`

use crate::syntax::ast::{NONE, Name, Node, Tag};
use crate::syntax::render::read::Rd;

/// What a `Laplace(...)` source is.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Laplace {
    /// `H(s)` is a constant: a plain controlled source of this gain.
    Gain { input: (String, String), gain: f64 },
    /// A transfer function, `num`/`den` in descending powers of `s`, with
    /// `den.len() >= 2` and `num.len() <= den.len()`.
    Xfer {
        input: (String, String),
        num: Vec<f64>,
        den: Vec<f64>,
    },
}

/// `num/den`, ascending in powers of `s` (index is the power).
#[derive(Clone, Debug, PartialEq)]
struct Rational {
    num: Vec<f64>,
    den: Vec<f64>,
}

/// One step of [`rational`]'s walk: a node to open, or an operator whose
/// operands are on the value stack.
enum Step {
    Open(u32),
    Binary(u32),
    Neg,
}

/// The deepest power `H(s)` may raise to (`s^16`).
const POWER_MAX: f64 = 16.0;

/// Whether the tree of value `v` calls `laplace` (any case).
pub(super) fn has_laplace(rd: &Rd, v: u32) -> bool {
    let a = rd.ast;
    let mut work = vec![v];
    while let Some(n) = work.pop() {
        if n == NONE {
            continue;
        }
        let node = Node(n);
        match a.tag(node) {
            Tag::Call | Tag::Probe => {
                if a.name(Name(a.lhs(node))).eq_ignore_ascii_case("laplace") {
                    return true;
                }
                work.extend(a.list(a.rhs(node) + 1));
            }
            Tag::Unary | Tag::Paren | Tag::Delimited => work.push(a.lhs(node)),
            Tag::Binary => work.extend([a.lhs(node), a.rhs(node)]),
            Tag::Ternary => {
                let at = a.rhs(node);
                work.extend([a.lhs(node), a.extra(at), a.extra(at + 1)]);
            }
            // else: leaves and lists hold no call a behavioural source reads.
            _ => {}
        }
    }
    false
}

/// The transfer function value `v` names, when `v` is one `Laplace(V(…),
/// H(s))` call `H` of which reads; `None` otherwise (`W0010`).
pub(super) fn read(rd: &Rd, v: u32) -> Option<Laplace> {
    let a = rd.ast;
    let mut call = v;
    if call != NONE && a.tag(Node(call)) == Tag::Delimited {
        call = a.lhs(Node(call));
    }
    if call == NONE
        || a.tag(Node(call)) != Tag::Call
        || !a
            .name(Name(a.lhs(Node(call))))
            .eq_ignore_ascii_case("laplace")
    {
        return None;
    }
    let [input, h] = a.list(a.rhs(Node(call)) + 1) else {
        return None;
    };
    let input = probe(rd, *input)?;
    let (num, den) = rational(rd, *h)?.descending();
    if den.len() < 2 {
        // A constant denominator: `s_xfer` segfaults on it, a gain is the
        // card anyone would write.
        return (num.len() == 1).then(|| Laplace::Gain {
            input,
            gain: num[0] / den[0],
        });
    }
    (num.len() <= den.len()).then_some(Laplace::Xfer { input, num, den })
}

/// Coefficients as an `s_xfer` array's text, in the order given.
pub(super) fn coeff_array(c: &[f64]) -> String {
    c.iter().map(|&v| coeff(v)).collect::<Vec<_>>().join(" ")
}

/// Descending coefficients as a Horner-nested polynomial in `s`: `[1e-3,
/// 1]` is `(1e-3*s+1e0)`, with no `^` or `**`, which dialects spell apart.
///
/// # Panics
/// When `c` is empty.
pub(super) fn horner(c: &[f64]) -> String {
    assert!(!c.is_empty(), "a polynomial has a coefficient");
    let mut out = coeff(c[0]);
    for &v in &c[1..] {
        out = format!("({out}*s+{})", coeff(v));
    }
    out
}

// ---- private ----

/// The probe `V(a)` / `V(a, b)` (any case) as its nodes, lowercased as
/// the old reading of the text left them; `b` is `0` for `V(a)`.
fn probe(rd: &Rd, n: u32) -> Option<(String, String)> {
    let a = rd.ast;
    let node = Node(n);
    let probe = matches!(a.tag(node), Tag::Probe | Tag::Call)
        && a.name(Name(a.lhs(node))).eq_ignore_ascii_case("v");
    if !probe {
        return None;
    }
    let args = a.list(a.rhs(node) + 1);
    let word = |&k: &u32| match a.tag(Node(k)) {
        Tag::Word | Tag::Ident | Tag::Num => Some(rd.text(k).trim().to_ascii_lowercase()),
        _ => None,
    };
    match args {
        [x] => Some((word(x)?, "0".into())),
        [x, y] => Some((word(x)?, word(y)?)),
        _ => None,
    }
}

/// `H(s)`'s tree `h` as a rational function of `s`, walked post-order on a
/// heap stack; `None` for anything but numbers, `s` and arithmetic.
fn rational(rd: &Rd, h: u32) -> Option<Rational> {
    let a = rd.ast;
    let mut work = vec![Step::Open(h)];
    let mut vals: Vec<Rational> = Vec::new();
    while let Some(step) = work.pop() {
        match step {
            Step::Open(n) => {
                let node = Node(n);
                match a.tag(node) {
                    Tag::Num => vals.push(Rational::constant(rd.t.value(a.main_token(node)))),
                    Tag::Ident if a.name(Name(a.lhs(node))).eq_ignore_ascii_case("s") => {
                        vals.push(Rational::s())
                    }
                    Tag::Paren => work.push(Step::Open(a.lhs(node))),
                    Tag::Unary => match rd.t.text(a.main_token(node)) {
                        "-" => work.extend([Step::Neg, Step::Open(a.lhs(node))]),
                        "+" => work.push(Step::Open(a.lhs(node))),
                        _ => return None,
                    },
                    Tag::Binary => work.extend([
                        Step::Binary(a.main_token(node)),
                        Step::Open(a.rhs(node)),
                        Step::Open(a.lhs(node)),
                    ]),
                    // else: a name, a call, a string: no coefficient.
                    _ => return None,
                }
            }
            Step::Neg => {
                let x = vals.pop().expect("an operand");
                vals.push(x.neg());
            }
            Step::Binary(op) => {
                let y = vals.pop().expect("a right operand");
                let x = vals.pop().expect("a left operand");
                vals.push(match rd.t.text(op) {
                    "+" => x.add(y),
                    "-" => x.add(y.neg()),
                    "*" => x.mul(y),
                    "/" => x.div(y)?,
                    "^" | "**" => x.powi(y.as_constant()?)?,
                    _ => return None,
                });
            }
        }
    }
    let r = vals.pop().expect("the tree's value");
    assert!(vals.is_empty(), "every operand consumed");
    (!is_zero(&r.den)).then_some(r)
}

impl Rational {
    fn constant(v: f64) -> Rational {
        Rational {
            num: vec![v],
            den: vec![1.0],
        }
    }

    fn s() -> Rational {
        Rational {
            num: vec![0.0, 1.0],
            den: vec![1.0],
        }
    }

    fn as_constant(&self) -> Option<f64> {
        (self.num.len() == 1 && self.den.len() == 1).then(|| self.num[0] / self.den[0])
    }

    fn mul(self, o: Rational) -> Rational {
        Rational {
            num: trim(poly_mul(&self.num, &o.num)),
            den: trim(poly_mul(&self.den, &o.den)),
        }
    }

    fn div(self, o: Rational) -> Option<Rational> {
        if is_zero(&o.num) {
            return None;
        }
        Some(Rational {
            num: trim(poly_mul(&self.num, &o.den)),
            den: trim(poly_mul(&self.den, &o.num)),
        })
    }

    fn add(self, o: Rational) -> Rational {
        let num = poly_add(&poly_mul(&self.num, &o.den), &poly_mul(&o.num, &self.den));
        Rational {
            num: trim(num),
            den: trim(poly_mul(&self.den, &o.den)),
        }
    }

    fn neg(self) -> Rational {
        Rational {
            num: self.num.iter().map(|c| -c).collect(),
            den: self.den,
        }
    }

    /// `self^n` for a whole `n` in `-16..=16`.
    fn powi(self, n: f64) -> Option<Rational> {
        if n.fract() != 0.0 || n.abs() > POWER_MAX {
            return None;
        }
        let base = if n < 0.0 {
            Rational::constant(1.0).div(self)?
        } else {
            self
        };
        let mut acc = Rational::constant(1.0);
        for _ in 0..n.abs() as u32 {
            acc = acc.mul(base.clone());
        }
        Some(acc)
    }

    /// `(num, den)` in descending powers.
    fn descending(&self) -> (Vec<f64>, Vec<f64>) {
        let flip = |p: &[f64]| {
            let mut v = trim(p.to_vec());
            v.reverse();
            v
        };
        (flip(&self.num), flip(&self.den))
    }
}

/// Drops exactly-zero high-order terms (a coefficient of 2.5e-8 is one).
fn trim(mut v: Vec<f64>) -> Vec<f64> {
    while v.len() > 1 && v[v.len() - 1] == 0.0 {
        v.pop();
    }
    v
}

fn poly_mul(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}

fn poly_add(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len().max(b.len())];
    for (i, x) in a.iter().enumerate() {
        out[i] += x;
    }
    for (i, y) in b.iter().enumerate() {
        out[i] += y;
    }
    out
}

fn is_zero(p: &[f64]) -> bool {
    p.iter().all(|c| *c == 0.0)
}

/// `{:e}`: an SI suffix (`1meg`) inside a coefficient array is noise.
fn coeff(v: f64) -> String {
    format!("{v:e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coefficients_are_spelled_as_exponents() {
        assert_eq!(coeff_array(&[1.59e-4, 1.0]), "1.59e-4 1e0");
        assert_eq!(horner(&[1.0]), "1e0");
        assert_eq!(horner(&[3.0, 2.0, 1.0]), "((3e0*s+2e0)*s+1e0)");
    }

    #[test]
    fn powers_must_be_whole_and_bounded() {
        assert_eq!(
            Rational::s().powi(2.0).unwrap().descending().0,
            [1.0, 0.0, 0.0]
        );
        assert!(Rational::s().powi(0.5).is_none());
        assert!(Rational::s().powi(17.0).is_none());
        let inv = Rational::s().powi(-1.0).unwrap();
        assert_eq!(inv.descending(), (vec![1.0], vec![1.0, 0.0]));
    }
}
