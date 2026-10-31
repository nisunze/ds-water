//! Water network hydraulics.
//!
//! [`Hydraulics`] compiles a [`ds_water_model::WaterNetwork`] once (index,
//! curves, sparse ordering) and then solves snapshots or an extended period.
//! The same code runs native, headless and in the browser's WASM unit.

// Numeric kernels index several parallel arrays by one position; iterator
// chains would hide the algorithm the comments describe.
#![allow(clippy::needless_range_loop)]

mod gga;
pub mod sparse;

pub use gga::{Hydraulics, PrepareError, RunStatus, Snapshot};

#[cfg(test)]
mod tests;
