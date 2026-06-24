//! SwiftSync hintsfile generation, built on the bitcoinkernel chain API.
//!
//! - [`elias_fano`] encodes a sorted index list and writes CompactSize values.
//! - [`hintsfile`] writes the hintsfile container.
//! - [`generate`] drives the two-pass walk that produces the file.

pub mod elias_fano;
pub mod generate;
pub mod hintsfile;
