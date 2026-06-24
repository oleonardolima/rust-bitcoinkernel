//! Elias-Fano encoding and CompactSize serialization for the SwiftSync hintsfile.
//!
//! The SwiftSync hintsfile records, for every block, the indices of the outputs
//! that remain unspent at a terminal height. These indices form a sorted,
//! strictly increasing sequence, which Elias-Fano encodes close to the
//! information-theoretic optimum without any third party dependencies.
//!
//! The encoding splits each value into a fixed number of low bits and the
//! remaining high bits:
//!
//! - the low bits of every value are concatenated into a bitset `L`;
//! - the high bits are stored as the gaps between consecutive values, each gap
//!   written in unary, concatenated into a bitset `H`.
//!
//! Both bitsets are packed most-significant-bit first into bytes. The number of
//! low bits is `l = floor(log2((m + 1) / n))`, where `n` is the number of
//! values and `m` is the maximum (last) value.
//!
//! The on-disk form of one Elias-Fano structure is `N` (CompactSize) followed
//! by `M` (CompactSize), then the raw `L` bytes and the raw `H` bytes. As a
//! special case, an empty sequence (`N == 0`) serializes to a single `0x00`.
//!
//! This module is intentionally dependency free; the unit tests check the
//! encoder against the BIP test vectors.

/// Appends a Bitcoin CompactSize (variable length unsigned integer) to `out`.
///
/// Encoding:
/// - `n < 0xFD`: a single byte;
/// - `n <= 0xFFFF`: `0xFD` followed by a little-endian `u16`;
/// - `n <= 0xFFFF_FFFF`: `0xFE` followed by a little-endian `u32`;
/// - otherwise: `0xFF` followed by a little-endian `u64`.
pub fn write_compact_size(out: &mut Vec<u8>, n: u64) {
    if n < 0xFD {
        out.push(n as u8);
    } else if n <= 0xFFFF {
        out.push(0xFD);
        out.extend_from_slice(&(n as u16).to_le_bytes());
    } else if n <= 0xFFFF_FFFF {
        out.push(0xFE);
        out.extend_from_slice(&(n as u32).to_le_bytes());
    } else {
        out.push(0xFF);
        out.extend_from_slice(&n.to_le_bytes());
    }
}

/// Accumulates bits and packs them most-significant-bit first into bytes.
///
/// The first bit pushed becomes the most significant bit of the first byte. The
/// final byte is zero padded on the right when the bit count is not a multiple
/// of eight.
struct BitWriter {
    bytes: Vec<u8>,
    nbits: usize,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            nbits: 0,
        }
    }

    fn push_bit(&mut self, bit: bool) {
        let byte_idx = self.nbits / 8;
        if byte_idx == self.bytes.len() {
            self.bytes.push(0);
        }
        if bit {
            let shift = 7 - (self.nbits % 8);
            self.bytes[byte_idx] |= 1 << shift;
        }
        self.nbits += 1;
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Returns the number of low bits to use for `n` values with maximum `m`.
///
/// This is `l = floor(log2((m + 1) / n))`, computed without floating point as
/// the largest `l >= 0` such that `n * 2^l <= m + 1`.
fn low_bit_count(n: u64, m: u64) -> u32 {
    let limit = m as u128 + 1;
    let mut l = 0u32;
    while l < 63 && (n as u128) << (l + 1) <= limit {
        l += 1;
    }
    l
}

/// Encodes `indices` as an Elias-Fano structure and appends it to `out`.
///
/// `indices` must be sorted in strictly increasing order. An empty slice
/// encodes to the single byte `0x00`.
pub fn encode_elias_fano(indices: &[u64], out: &mut Vec<u8>) {
    let n = indices.len() as u64;
    if n == 0 {
        out.push(0x00);
        return;
    }

    debug_assert!(
        indices.windows(2).all(|w| w[0] < w[1]),
        "Elias-Fano input must be strictly increasing"
    );

    let m = *indices.last().unwrap();
    write_compact_size(out, n);
    write_compact_size(out, m);

    let l = low_bit_count(n, m);

    // Low bits: the l least significant bits of every value, emitted least
    // significant bit first, concatenated into a single bitset.
    let mut low = BitWriter::new();
    for &value in indices {
        for j in 0..l {
            low.push_bit((value >> j) & 1 == 1);
        }
    }
    out.extend_from_slice(&low.into_bytes());

    // High bits: the gap between successive high parts (value >> l), each gap
    // written in unary as that many zeros followed by a single one.
    let mut high = BitWriter::new();
    let mut prev_upper = 0u64;
    for &value in indices {
        let upper = value >> l;
        let gap = upper - prev_upper;
        for _ in 0..gap {
            high.push_bit(false);
        }
        high.push_bit(true);
        prev_upper = upper;
    }
    out.extend_from_slice(&high.into_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(indices: &[u64]) -> Vec<u8> {
        let mut out = Vec::new();
        encode_elias_fano(indices, &mut out);
        out
    }

    #[test]
    fn empty_sequence_is_single_zero() {
        assert_eq!(encode(&[]), vec![0x00]);
    }

    #[test]
    fn single_zero_value() {
        // n = 1, m = 0 -> l = 0, no low bits, high bits = unary(0) = "1".
        // N = 0x01, M = 0x00, L = (empty), H = 0b1000_0000 = 0x80.
        assert_eq!(encode(&[0]), vec![0x01, 0x00, 0x80]);
    }

    // The following expected outputs are the BIP hintsfile Elias-Fano test
    // vectors (bip-xxxx-hintsfile/test_vectors/elias_fano.json).
    #[test]
    fn bip_vector_1() {
        let indices = [13, 16, 19, 22, 25, 28, 31, 34, 37, 40];
        assert_eq!(encode(&indices), hex::decode("0a288d8d8016ad50").unwrap());
    }

    #[test]
    fn bip_vector_2() {
        let indices = [
            5, 12, 19, 26, 33, 40, 47, 54, 61, 68, 75, 82, 89, 96, 103, 110, 117,
        ];
        assert_eq!(
            encode(&indices),
            hex::decode("11758d8d8d8d804a4949292524").unwrap()
        );
    }

    #[test]
    fn bip_vector_3() {
        let indices: Vec<u64> = (0..50).map(|i| 17 + i * 3).collect();
        assert_eq!(
            encode(&indices),
            hex::decode("32a4aaaaaaaaaaaa800094a5294a5294a5294a5294a5294a5290").unwrap()
        );
    }

    #[test]
    fn compact_size_boundaries() {
        let cases: &[(u64, &[u8])] = &[
            (0x00, &[0x00]),
            (0xFC, &[0xFC]),
            (0xFD, &[0xFD, 0xFD, 0x00]),
            (0xFFFF, &[0xFD, 0xFF, 0xFF]),
            (0x0001_0000, &[0xFE, 0x00, 0x00, 0x01, 0x00]),
            (0xFFFF_FFFF, &[0xFE, 0xFF, 0xFF, 0xFF, 0xFF]),
            (
                0x0001_0000_0000,
                &[0xFF, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00],
            ),
        ];
        for (value, expected) in cases {
            let mut out = Vec::new();
            write_compact_size(&mut out, *value);
            assert_eq!(&out, expected, "compact size mismatch for {value:#x}");
        }
    }

    #[test]
    fn low_bit_count_matches_spec() {
        // l = floor(log2((m + 1) / n)).
        assert_eq!(low_bit_count(3, 12), 2);
        assert_eq!(low_bit_count(10, 40), 2);
        assert_eq!(low_bit_count(17, 117), 2);
        assert_eq!(low_bit_count(50, 164), 1);
        assert_eq!(low_bit_count(1, 0), 0);
    }
}
