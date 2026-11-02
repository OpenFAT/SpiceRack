#![cfg(feature = "backend")]
//! End-to-end coverage of the analyses the ESPice backend exposes: the
//! shared suite in `common/mod.rs`, run on `Dialect::Espice`.
//!
//! Skipped automatically when espice is not on PATH. Not covered: XSPICE and
//! `.control` (not ESPice features; Laplace is in `test_laplace_ngspice.rs`), `.disto`/`.pz`/`.noise`
//! (their result layouts are not mapped by this backend yet), `.four`
//! (ESPice writes the table as a raw plot, not to stdout).

#[macro_use]
mod common;

use spicerack::backend::Dialect;

// ESPice names the `.tf` impedance columns input_resistance and
// output_resistance where ngspice writes v1#input_impedance and
// output_impedance_at_v(out).
analysis_suite!(Dialect::Espice, tf_columns: ("output_resistance", "input_resistance"));
