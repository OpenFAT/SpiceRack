//! Spectre `statistics { }` blocks as data: the varied parameters of each
//! scope, their correlations and truncation (Spectre Circuit Simulator
//! Reference 19.1 pp.175-179). Filled by the Spectre importer; SPICE-origin
//! statistical calls stay text in their expressions, so SPICE -> IR -> SPICE
//! is the identity.
//!
//! Cites: R19 pp.175-179.
//!
//! Contents:
//! - `pub struct Statistics`, `pub struct Vary`, `pub enum Dist`, `pub struct Correlate`

use serde::{Deserialize, Serialize};

/// One `statistics { }` block (R19 pp.175-179).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Statistics {
    /// `process { vary ... }`: sampled once per Monte Carlo iteration.
    #[serde(default)]
    pub process: Vec<Vary>,
    /// `mismatch { vary ... }`: sampled per subcircuit instance.
    #[serde(default)]
    pub mismatch: Vec<Vary>,
    #[serde(default)]
    pub correlations: Vec<Correlate>,
    /// `truncate tr=` per scope: the block's, `process`'s, `mismatch`'s;
    /// `None` inherits (R19 p.179).
    #[serde(default)]
    pub truncate: [Option<String>; 3],
}

/// `vary <param> dist=<dist> std=<spread> | N=<spread> [percent=yes]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vary {
    pub param: String,
    pub dist: Dist,
    /// `std=` for `gauss` and `lnorm`, the half-range `N=` for `unif`; an
    /// expression as written.
    pub spread: String,
    #[serde(default)]
    pub percent: bool,
}

/// A `vary`'s distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dist {
    Gauss,
    Lnorm,
    Unif,
}

/// `correlate param=[a b] cc=c` (process parameters) or `correlate
/// dev=[i1 i2] [param=[...]] cc=c` (matched instances).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Correlate {
    #[serde(default)]
    pub params: Vec<String>,
    #[serde(default)]
    pub devices: Vec<String>,
    pub cc: String,
}
