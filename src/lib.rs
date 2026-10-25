#![allow(clippy::too_many_arguments)]

pub mod unit;
pub mod rawfile;
pub mod normalize;
pub mod result;
#[cfg(feature = "backend")]
pub mod backend;
#[cfg(feature = "backend")]
pub mod simulation;
pub mod measure_parse;
pub mod lint;
pub mod import;
pub mod peg;
pub mod ir;
pub mod codegen;
pub mod veriloga;

#[cfg(feature = "python")]
#[allow(non_snake_case)]
pub mod python;
