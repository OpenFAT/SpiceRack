#![allow(clippy::too_many_arguments)]

#[cfg(feature = "backend")]
pub mod backend;
pub mod codegen;
pub mod import;
pub mod ir;
pub mod lint;
pub mod measure_parse;
pub mod normalize;
pub mod peg;
pub mod rawfile;
pub mod result;
#[cfg(feature = "backend")]
pub mod simulation;
pub mod unit;
pub mod veriloga;

#[cfg(feature = "python")]
#[allow(non_snake_case)]
pub mod python;
