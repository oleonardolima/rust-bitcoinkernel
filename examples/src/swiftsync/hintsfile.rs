//! Writer for the SwiftSync hintsfile container.
//!
//! The container layout is:
//!
//! | Field   | Bytes            | Value                              |
//! | ------- | ---------------- | ---------------------------------- |
//! | Magic   | 4                | `0x55 0x54 0x58 0x4f` ("UTXO")     |
//! | Version | 1                | `0x00`                             |
//! | Height  | 4, little-endian | terminal height `n`                |
//! | Hints   | variable         | one Elias-Fano entry per block     |
//!
//! The hints are the standard Bitcoin `vector<EliasFano>` serialization: a
//! CompactSize element count followed by the entries themselves. The count
//! equals the number of blocks covered, which is `n + 1` (blocks `0..=n`,
//! including the genesis block whose entry is empty).

use std::io::{self, Write};

use super::elias_fano::{encode_elias_fano, write_compact_size};

/// Hintsfile magic bytes, spelling "UTXO" in ASCII.
pub const MAGIC: [u8; 4] = [0x55, 0x54, 0x58, 0x4f];

/// Hintsfile format version.
pub const VERSION: u8 = 0x00;

/// Streams a hintsfile to an underlying writer.
///
/// Construct with [`HintsFileWriter::new`], which writes the header and the
/// CompactSize count of blocks, then call [`append_block`](Self::append_block)
/// exactly `num_blocks` times, once per block from genesis to the terminal
/// height in order.
pub struct HintsFileWriter<W: Write> {
    inner: W,
}

impl<W: Write> HintsFileWriter<W> {
    /// Writes the header (magic, version, terminal height) and the CompactSize
    /// count of Elias-Fano entries that will follow.
    ///
    /// `num_blocks` must equal `terminal_height + 1`.
    pub fn new(mut inner: W, terminal_height: u32, num_blocks: u64) -> io::Result<Self> {
        inner.write_all(&MAGIC)?;
        inner.write_all(&[VERSION])?;
        inner.write_all(&terminal_height.to_le_bytes())?;

        let mut count = Vec::new();
        write_compact_size(&mut count, num_blocks);
        inner.write_all(&count)?;

        Ok(Self { inner })
    }

    /// Appends one block's unspent output indices as an Elias-Fano entry.
    ///
    /// `sorted_indices` must be sorted in strictly increasing order; an empty
    /// slice encodes to a single `0x00` byte.
    pub fn append_block(&mut self, sorted_indices: &[u64]) -> io::Result<()> {
        let mut entry = Vec::new();
        encode_elias_fano(sorted_indices, &mut entry);
        self.inner.write_all(&entry)
    }

    /// Flushes and returns the underlying writer.
    pub fn finish(mut self) -> io::Result<W> {
        self.inner.flush()?;
        Ok(self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_layout() {
        let mut buf = Vec::new();
        let writer = HintsFileWriter::new(&mut buf, 0x0102_0304, 1).unwrap();
        writer.finish().unwrap();
        // Magic, version, height little-endian, then CompactSize count = 1.
        assert_eq!(
            buf,
            vec![0x55, 0x54, 0x58, 0x4f, 0x00, 0x04, 0x03, 0x02, 0x01, 0x01]
        );
    }

    #[test]
    fn genesis_only_file() {
        let mut buf = Vec::new();
        let mut writer = HintsFileWriter::new(&mut buf, 0, 1).unwrap();
        // Genesis coinbase is unspendable, so its entry is empty.
        writer.append_block(&[]).unwrap();
        writer.finish().unwrap();
        assert_eq!(
            buf,
            vec![0x55, 0x54, 0x58, 0x4f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
    }
}
