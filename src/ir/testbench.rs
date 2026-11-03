use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::*;

// ── Testbench ──

/// Testbench -- wraps a DUT with stimulus + analysis. The DUT is
/// [`CircuitIR::top`]; JSON written before 0.1.3 also carries a `dut` name,
/// which is ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Testbench {
    pub stimulus: Vec<Component>,
    pub analyses: Vec<Analysis>,
    pub options: SimOptions,
    pub saves: Vec<String>,
    pub measures: Vec<String>,
    pub temperature: Option<f64>,
    pub nominal_temperature: Option<f64>,
    pub initial_conditions: Vec<(String, f64)>,
    pub node_sets: Vec<(String, f64)>,
    pub step_params: Vec<StepParam>,
    pub extra_lines: Vec<String>,
    /// `.print`/`.plot` cards: what a SPICE run prints to stdout. A rawfile
    /// run ignores them (ngspice manual p.353), so dialects without the card
    /// (LTspice, Spectre, VACASK) omit them without changing a result.
    #[serde(default)]
    pub prints: Vec<PrintCard>,
    /// ngspice's compatibility mode (`hsa`, `ps`, `lt`, ...), which it reads
    /// only before parsing the deck. The ngspice backend passes it as
    /// `-D ngbehavior=<mode>`; every other dialect ignores it, since it says
    /// how ngspice reads text, not what the circuit is.
    #[serde(default)]
    pub ngbehavior: Option<String>,
}

/// One `.print` card: the analysis (`tran`, `dc`, ...; empty if none named)
/// and its output expressions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrintCard {
    pub analysis: String,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepParam {
    pub param: String,
    pub start: f64,
    pub stop: f64,
    pub step: f64,
    /// `lin`, `dec`, `oct` (or Spectre's `log`); `None` is linear.
    pub sweep_type: Option<Sweep>,
}

// ── Analysis types ──

/// Analysis types -- all supported analyses as a tagged union
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Analysis {
    Op,
    Dc {
        sweeps: Vec<DcSweep>,
    },
    Ac {
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    Transient {
        step: f64,
        stop: f64,
        start: Option<f64>,
        max_step: Option<f64>,
        uic: bool,
    },
    Noise {
        output: String,
        reference: String,
        source: String,
        #[serde(flatten)]
        sweep: FreqSweep,
        points_per_summary: Option<u32>,
    },
    Tf {
        output: String,
        source: String,
    },
    Sensitivity {
        output: String,
        ac: Option<FreqSweep>,
    },
    PoleZero {
        node1: String,
        node2: String,
        node3: String,
        node4: String,
        tf_type: PzTransfer,
        pz_type: PzKind,
    },
    Distortion {
        #[serde(flatten)]
        sweep: FreqSweep,
        f2overf1: Option<f64>,
    },
    /// `sc_iter` (shooting iterations, ngspice's default 50), `steady_coeff`
    /// (the convergence weight, default 1e-3) and `uic` are ngspice's
    /// `.pss … sciter steadycoeff <uic>` (manual §11.3.12); JSON leaves them
    /// out when unset.
    Pss {
        fundamental: f64,
        stabilization: f64,
        observe_node: String,
        points_per_period: u32,
        harmonics: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sc_iter: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steady_coeff: Option<f64>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        uic: bool,
    },
    HarmonicBalance {
        frequencies: Vec<f64>,
        harmonics: Vec<u32>,
    },
    SPar {
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    Stability {
        probe: String,
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    TransientNoise {
        step: f64,
        stop: f64,
    },
    Fourier {
        fundamental: f64,
        outputs: Vec<String>,
        num_harmonics: Option<u32>,
    },
    // Vendor-specific
    SpectreSweep {
        param: String,
        start: f64,
        stop: f64,
        step: f64,
        inner: String,
        inner_type: SpectreAnalysis,
    },
    SpectreMonteCarlo {
        iterations: u32,
        inner: String,
        inner_type: SpectreAnalysis,
        seed: Option<u64>,
    },
    SpectrePac {
        #[serde(flatten)]
        pss: PssBase,
        #[serde(flatten)]
        sweep: FreqSweep,
        sweep_type: String,
    },
    SpectrePnoise {
        #[serde(flatten)]
        pss: PssBase,
        output: String,
        reference: String,
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    SpectrePxf {
        #[serde(flatten)]
        pss: PssBase,
        output: String,
        source: String,
        #[serde(flatten)]
        sweep: FreqSweep,
    },
    SpectrePstb {
        #[serde(flatten)]
        pss: PssBase,
        probe: String,
        #[serde(flatten)]
        sweep: FreqSweep,
    },
}

impl Analysis {
    pub fn kind_str(&self) -> &str {
        match self {
            Analysis::Op => "op",
            Analysis::Dc { .. } => "dc",
            Analysis::Ac { .. } => "ac",
            Analysis::Transient { .. } => "tran",
            Analysis::Noise { .. } => "noise",
            Analysis::Tf { .. } => "tf",
            Analysis::Sensitivity { ac: Some(_), .. } => "sens_ac",
            Analysis::Sensitivity { .. } => "sens",
            Analysis::PoleZero { .. } => "pz",
            Analysis::Distortion { .. } => "disto",
            Analysis::Pss { .. } => "pss",
            Analysis::HarmonicBalance { .. } => "hb",
            Analysis::SPar { .. } => "sp",
            Analysis::Stability { .. } => "stb",
            Analysis::TransientNoise { .. } => "trannoise",
            Analysis::Fourier { .. } => "tran",
            Analysis::SpectreSweep { .. } => "tran",
            Analysis::SpectreMonteCarlo { .. } => "tran",
            Analysis::SpectrePac { .. } => "pac",
            Analysis::SpectrePnoise { .. } => "pnoise",
            Analysis::SpectrePxf { .. } => "pxf",
            Analysis::SpectrePstb { .. } => "pstb",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DcSweep {
    pub source: String,
    pub start: f64,
    pub stop: f64,
    pub step: f64,
}

/// A frequency sweep. Flattened into each analysis that carries one, so the
/// JSON keeps the four fields `variation`, `points`, `start`, `stop` inline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FreqSweep {
    pub variation: Sweep,
    /// Per decade or octave, or in total for [`Sweep::Lin`].
    pub points: u32,
    pub start: f64,
    pub stop: f64,
}

/// A closed set of deck keywords: `name()` is the keyword, `FromStr` and
/// JSON read it in any case (anything else fails, naming the choices), and
/// JSON writes it lowercase, so the text is parsed once, where it enters.
macro_rules! keywords {
    ($(#[$m:meta])* $name:ident, $what:literal { $($(#[$vm:meta])* $v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "&'static str")]
        pub enum $name { $($(#[$vm])* $v),+ }

        impl $name {
            /// Every keyword, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$v),+];

            /// The keyword as a deck and JSON spell it.
            pub fn name(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            /// Reads a [`Self::name`] in any case.
            fn from_str(s: &str) -> Result<Self, String> {
                Self::ALL.iter().copied().find(|k| k.name().eq_ignore_ascii_case(s)).ok_or_else(|| {
                    let names: Vec<&str> = Self::ALL.iter().map(|k| k.name()).collect();
                    let (last, rest) = names.split_last().expect("a keyword set is never empty");
                    format!("unknown {} '{s}' (expected {} or {last})", $what, rest.join(", "))
                })
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(s: String) -> Result<Self, String> {
                s.parse()
            }
        }

        impl From<$name> for &'static str {
            fn from(k: $name) -> Self {
                k.name()
            }
        }
    };
}

keywords! {
    /// How a sweep spaces its points: a frequency sweep, or a `.step`.
    Sweep, "sweep" {
        Dec = "dec",
        Oct = "oct",
        Lin = "lin",
        /// Spectre's `log=N`: N points in total, log-spaced.
        Log = "log",
    }
}

keywords! {
    /// What a `.pz` transfer function relates (ngspice manual §11.3.6):
    /// output over input voltage, or output voltage over input current.
    PzTransfer, "transfer type" {
        Vol = "vol",
        Cur = "cur",
    }
}

keywords! {
    /// What a `.pz` finds: poles, zeros, or both.
    PzKind, "pole-zero type" {
        Pol = "pol",
        Zer = "zer",
        Pz = "pz",
    }
}

keywords! {
    /// The analysis a Spectre `sweep` or `montecarlo` runs inside it, as
    /// its statement keyword (`[REF]` analysis statements).
    SpectreAnalysis, "Spectre analysis" {
        Dc = "dc",
        Ac = "ac",
        Tran = "tran",
        Noise = "noise",
        Xf = "xf",
        Sp = "sp",
        Stb = "stb",
        Pss = "pss",
        Pac = "pac",
        Pnoise = "pnoise",
        Pxf = "pxf",
        Pstb = "pstb",
        Hb = "hb",
    }
}

/// The periodic steady state a SpectreRF small-signal analysis linearizes
/// around (`pac`, `pnoise`, `pxf`, `pstb`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PssBase {
    pub pss_fundamental: f64,
    pub pss_stabilization: f64,
    pub pss_harmonics: u32,
}

// ── Simulation options ──

/// Simulation options -- portable + per-backend
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SimOptions {
    pub portable: Vec<(String, String)>,
    pub backend_specific: BTreeMap<String, Vec<(String, String)>>,
}
