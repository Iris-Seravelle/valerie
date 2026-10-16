//! Task target handlers for declarative build targets and verification pipelines.

pub mod fstar;
pub mod ocaml;

pub use fstar::FStarHandler;
pub use ocaml::{OcamlBinaryHandler, interpolate_variables};
