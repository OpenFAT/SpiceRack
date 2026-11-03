//! SpiceRack: a circuit as [`ir::CircuitIR`], read from any dialect
//! (`import`), written to any (`codegen`), run on a simulator (`backend`,
//! `simulation`) and read back (`rawfile`, `result`, `measure_parse`).
//! `python` is the PyO3 binding the `spicerack` package ships.
//!
//! Cites: none (each module cites its own manuals; `syntax::lexicon` and
//! `syntax::device` are the dialect readers' tables).
//!
//! Contents:
//! - `pub mod backend` (feature `backend`), `codegen`, `import`, `ir`, `lint`,
//!   `measure_parse`, `normalize`, `rawfile`, `result`,
//!   `simulation` (feature `backend`), `syntax` (the transpiler front end `import` reads with),
//!   `unit`, `veriloga`, `python` (feature `python`)

#![allow(clippy::too_many_arguments)]

#[cfg(feature = "backend")]
pub mod backend;
pub mod codegen;
pub mod import;
pub mod ir;
pub mod lint;
pub mod measure_parse;
pub mod normalize;
pub mod rawfile;
pub mod result;
#[cfg(feature = "backend")]
pub mod simulation;
pub mod syntax;
pub mod unit;
pub mod veriloga;

#[cfg(feature = "python")]
#[allow(non_snake_case)]
pub mod python;
