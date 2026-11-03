use serde::{Deserialize, Serialize};

// ── Value and waveform types ──

/// Component value in the IR -- numeric, expression, or raw
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IrValue {
    Numeric { value: f64 },
    Expression { expr: String },
    Raw { text: String },
}

/// One waveform parameter: a number, or an expression over `.param`s
/// (`PULSE(0 {vdd} ...)`). JSON keeps a number a number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IrNum {
    Num(f64),
    Expr(String),
}

impl IrNum {
    pub fn num(&self) -> Option<f64> {
        match self {
            IrNum::Num(v) => Some(*v),
            IrNum::Expr(_) => None,
        }
    }
}

impl From<f64> for IrNum {
    fn from(v: f64) -> Self {
        IrNum::Num(v)
    }
}

impl From<&IrNum> for IrValue {
    fn from(n: &IrNum) -> Self {
        match n {
            IrNum::Num(value) => IrValue::Numeric { value: *value },
            IrNum::Expr(expr) => IrValue::Expression { expr: expr.clone() },
        }
    }
}

/// `x == 0.0` reads as before; an expression equals no number.
impl PartialEq<f64> for IrNum {
    fn eq(&self, other: &f64) -> bool {
        self.num() == Some(*other)
    }
}

/// Waveform types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IrWaveform {
    Sin {
        offset: IrNum,
        amplitude: IrNum,
        frequency: IrNum,
        delay: IrNum,
        damping: IrNum,
        phase: IrNum,
    },
    Pulse {
        initial: IrNum,
        pulsed: IrNum,
        delay: IrNum,
        rise_time: IrNum,
        fall_time: IrNum,
        pulse_width: IrNum,
        period: IrNum,
    },
    /// `repeat` is ngspice's `r=`: the time point the sequence repeats from,
    /// forever (`0` repeats all of it; one of the times before the last).
    /// `delay` is `td=`: the whole sequence shifted later. `None` is the
    /// card without them (once, no delay), and JSON leaves it out.
    Pwl {
        values: Vec<(IrNum, IrNum)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repeat: Option<IrNum>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delay: Option<IrNum>,
    },
    Exp {
        initial: IrNum,
        pulsed: IrNum,
        rise_delay: IrNum,
        rise_tau: IrNum,
        fall_delay: IrNum,
        fall_tau: IrNum,
    },
    Sffm {
        offset: IrNum,
        amplitude: IrNum,
        carrier_freq: IrNum,
        modulation_index: IrNum,
        signal_freq: IrNum,
    },
    Am {
        amplitude: IrNum,
        offset: IrNum,
        modulating_freq: IrNum,
        carrier_freq: IrNum,
        delay: IrNum,
    },
    /// ngspice transient noise (manual §4.1.7): Gaussian white noise of rms
    /// `na`, a new sample every `nt` seconds; 1/f noise of exponent `nalpha`
    /// and rms `namp`; a random telegraph signal of amplitude `rtsam` with
    /// mean capture and emission times `rtscapt`, `rtsemt` (0 for none),
    /// added to the source's DC value. ngspice only: no other dialect has a source form.
    TrNoise {
        na: IrNum,
        nt: IrNum,
        nalpha: IrNum,
        namp: IrNum,
        rtsam: IrNum,
        rtscapt: IrNum,
        rtsemt: IrNum,
    },
    /// ngspice random source (manual §4.1.8): a `kind` draw held for `ts`
    /// seconds, starting after `td` (the output is `p2` until then), added
    /// to the source's DC value as TRNOISE is. `p1`,
    /// `p2`: range and offset (uniform, values within `p2 ± p1`), standard
    /// deviation and mean (Gaussian), mean and offset (exponential), lambda
    /// and offset (Poisson). ngspice only.
    TrRandom {
        kind: RandomKind,
        ts: IrNum,
        td: IrNum,
        p1: IrNum,
        p2: IrNum,
    },
}

/// TRRANDOM's distribution, its `TYPE` number in the deck and in JSON;
/// any other number fails to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
#[repr(u8)]
pub enum RandomKind {
    Uniform = 1,
    Gaussian = 2,
    Exponential = 3,
    Poisson = 4,
}

impl RandomKind {
    /// The four kinds, in `TYPE` order.
    pub const ALL: [RandomKind; 4] = [
        RandomKind::Uniform,
        RandomKind::Gaussian,
        RandomKind::Exponential,
        RandomKind::Poisson,
    ];

    /// The lowercase name (`uniform`, ...), as Python spells it.
    pub fn name(self) -> &'static str {
        match self {
            RandomKind::Uniform => "uniform",
            RandomKind::Gaussian => "gaussian",
            RandomKind::Exponential => "exponential",
            RandomKind::Poisson => "poisson",
        }
    }
}

impl TryFrom<u8> for RandomKind {
    type Error = String;

    fn try_from(t: u8) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|k| *k as u8 == t)
            .ok_or_else(|| format!("unknown TRRANDOM type {t} (expected 1 to 4)"))
    }
}

impl From<RandomKind> for u8 {
    fn from(k: RandomKind) -> u8 {
        k as u8
    }
}

impl std::str::FromStr for RandomKind {
    type Err = String;

    /// Reads a [`RandomKind::name`] in any case.
    fn from_str(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|k| k.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| {
                format!(
                    "unknown random type '{s}' (expected uniform, gaussian, exponential or poisson)"
                )
            })
    }
}

impl IrWaveform {
    /// Every parameter, in field order.
    pub fn nums(&self) -> Vec<&IrNum> {
        use IrWaveform::*;
        match self {
            Sin {
                offset,
                amplitude,
                frequency,
                delay,
                damping,
                phase,
            } => vec![offset, amplitude, frequency, delay, damping, phase],
            Pulse {
                initial,
                pulsed,
                delay,
                rise_time,
                fall_time,
                pulse_width,
                period,
            } => vec![
                initial,
                pulsed,
                delay,
                rise_time,
                fall_time,
                pulse_width,
                period,
            ],
            Pwl {
                values,
                repeat,
                delay,
            } => values
                .iter()
                .flat_map(|(t, v)| [t, v])
                .chain(repeat)
                .chain(delay)
                .collect(),
            Exp {
                initial,
                pulsed,
                rise_delay,
                rise_tau,
                fall_delay,
                fall_tau,
            } => vec![initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau],
            Sffm {
                offset,
                amplitude,
                carrier_freq,
                modulation_index,
                signal_freq,
            } => vec![
                offset,
                amplitude,
                carrier_freq,
                modulation_index,
                signal_freq,
            ],
            Am {
                amplitude,
                offset,
                modulating_freq,
                carrier_freq,
                delay,
            } => vec![amplitude, offset, modulating_freq, carrier_freq, delay],
            TrNoise {
                na,
                nt,
                nalpha,
                namp,
                rtsam,
                rtscapt,
                rtsemt,
            } => vec![na, nt, nalpha, namp, rtsam, rtscapt, rtsemt],
            TrRandom { ts, td, p1, p2, .. } => vec![ts, td, p1, p2],
        }
    }

    /// [`Self::nums`], mutable: for passes over the expressions (renaming,
    /// case folding).
    pub fn nums_mut(&mut self) -> Vec<&mut IrNum> {
        use IrWaveform::*;
        match self {
            Sin {
                offset,
                amplitude,
                frequency,
                delay,
                damping,
                phase,
            } => vec![offset, amplitude, frequency, delay, damping, phase],
            Pulse {
                initial,
                pulsed,
                delay,
                rise_time,
                fall_time,
                pulse_width,
                period,
            } => vec![
                initial,
                pulsed,
                delay,
                rise_time,
                fall_time,
                pulse_width,
                period,
            ],
            Pwl {
                values,
                repeat,
                delay,
            } => values
                .iter_mut()
                .flat_map(|(t, v)| [t, v])
                .chain(repeat)
                .chain(delay)
                .collect(),
            Exp {
                initial,
                pulsed,
                rise_delay,
                rise_tau,
                fall_delay,
                fall_tau,
            } => vec![initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau],
            Sffm {
                offset,
                amplitude,
                carrier_freq,
                modulation_index,
                signal_freq,
            } => vec![
                offset,
                amplitude,
                carrier_freq,
                modulation_index,
                signal_freq,
            ],
            Am {
                amplitude,
                offset,
                modulating_freq,
                carrier_freq,
                delay,
            } => vec![amplitude, offset, modulating_freq, carrier_freq, delay],
            TrNoise {
                na,
                nt,
                nalpha,
                namp,
                rtsam,
                rtscapt,
                rtsemt,
            } => vec![na, nt, nalpha, namp, rtsam, rtscapt, rtsemt],
            TrRandom { ts, td, p1, p2, .. } => vec![ts, td, p1, p2],
        }
    }

    /// The expression parameters.
    pub fn exprs_ref(&self) -> Vec<&str> {
        self.nums()
            .into_iter()
            .filter_map(|n| match n {
                IrNum::Expr(e) => Some(e.as_str()),
                IrNum::Num(_) => None,
            })
            .collect()
    }

    /// The Spectre `pwlperiod` that repeats PWL `values` exactly as `repeat`
    /// 0 (ngspice `r=0`) does, when one does: numeric points from t = 0
    /// whose last value equals the first, the last segment at least the
    /// default `twidth` (period/1000) long. Spectre ramps over `twidth` from
    /// the value at period - twidth to the first value before each repeat
    /// (Spectre Reference 5.0, `vsource` #17, #18, p.686); under these
    /// conditions that ramp is the last segment itself. `None` otherwise.
    pub fn spectre_pwl_period(values: &[(IrNum, IrNum)]) -> Option<f64> {
        let ([(t0, v0), ..], [.., (tp, _), (tn, vn)]) = (values, values) else {
            return None;
        };
        let (t0, v0, tp, tn, vn) = (t0.num()?, v0.num()?, tp.num()?, tn.num()?, vn.num()?);
        (t0 == 0.0 && vn == v0 && tn > 0.0 && tn - tp >= tn / 1000.0).then_some(tn)
    }

    /// The DC value SPICE takes for a card that gives none: the value at
    /// t=0 (AM starts at 0), and 0 for TRNOISE and TRRANDOM, which add to it.
    pub fn at_zero(&self) -> IrValue {
        use IrWaveform::*;
        match self {
            Sin { offset, .. } | Sffm { offset, .. } => offset.into(),
            Pulse { initial, .. } | Exp { initial, .. } => initial.into(),
            Pwl { values, .. } => values
                .first()
                .map_or(IrValue::Numeric { value: 0.0 }, |p| (&p.1).into()),
            // TRNOISE and TRRANDOM add to the card's DC value instead of
            // replacing it (ngspice 45: `V1 a 0 3 TRRANDOM(1 1u 10u 2 3)`
            // holds 6 V before TD), so a card with none has DC 0.
            Am { .. } | TrNoise { .. } | TrRandom { .. } => IrValue::Numeric { value: 0.0 },
        }
    }
}
