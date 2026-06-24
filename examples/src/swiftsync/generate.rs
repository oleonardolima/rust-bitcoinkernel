//! Two-pass hintsfile generation driven by the bitcoinkernel chain API.
//!
//! The kernel does not expose a direct UTXO set query, so spent-ness is
//! reconstructed by walking the chain twice:
//!
//! 1. Pass one records every spent outpoint, taken straight from transaction
//!    inputs.
//! 2. Pass two walks each block's outputs in order, skips unspendable outputs,
//!    and for each spendable output decides whether it survives to the terminal
//!    height (its outpoint never appears in the spent set). Surviving outputs
//!    are recorded in the hintsfile.
//!
//! An output created at or before the terminal height `n` is unspent at `n` if
//! and only if its outpoint is never spent within blocks `0..=n` (this treats
//! intra-block spends as spent, matching the BIP).

use std::collections::HashSet;
use std::io::Write;

use bitcoinkernel::{prelude::*, ChainType, ChainstateManager, KernelError};

use super::hintsfile::HintsFileWriter;

/// Summary statistics returned by [`generate`].
pub struct Stats {
    /// Terminal height the hintsfile was generated for.
    pub terminal_height: i32,
    /// Number of blocks covered (`terminal_height + 1`).
    pub num_blocks: u64,
    /// Number of distinct spent outpoints tracked in pass one.
    pub spent_outpoints: usize,
    /// Number of outputs recorded as unspent at the terminal height.
    pub total_unspent: u64,
}

/// Generates a hintsfile for blocks `0..=terminal_height` and writes it to `out`.
///
/// `terminal_height` is clamped to the active chain tip.
pub fn generate<W: Write>(
    chainman: &ChainstateManager,
    chain_type: ChainType,
    terminal_height: i32,
    mut out: W,
) -> Result<Stats, KernelError> {
    let chain = chainman.active_chain();
    let n = terminal_height.min(chain.height());
    let is_mainnet = chain_type == ChainType::Mainnet;
    let progress_interval = (n / 20).max(1);

    // Pass one: collect every spent outpoint.
    let mut spent: HashSet<([u8; 32], u32)> = HashSet::new();
    for h in 0..=n {
        let entry = chain.at_height(h as usize).expect("height within chain");
        let block = chainman.read_block_data(&entry)?;
        for tx in block.transactions() {
            for input in tx.inputs() {
                let outpoint = input.outpoint();
                if outpoint.is_null() {
                    continue; // coinbase input spends nothing
                }
                spent.insert((outpoint.txid().to_bytes(), outpoint.index()));
            }
        }
        if h % progress_interval == 0 {
            log::info!("pass 1/2 (spent set): block {h} / {n}");
        }
    }

    // Pass two: per block, record the outputs that survive to height n.
    let num_blocks = n as u64 + 1;
    let mut writer = HintsFileWriter::new(&mut out, n as u32, num_blocks)
        .map_err(|e| KernelError::Internal(format!("failed to write header: {e}")))?;
    let mut total_unspent = 0u64;
    for h in 0..=n {
        let entry = chain.at_height(h as usize).expect("height within chain");
        let block = chainman.read_block_data(&entry)?;

        let mut indices: Vec<u64> = Vec::new();
        let mut spendable_index: u64 = 0;
        for (tx_pos, tx) in block.transactions().enumerate() {
            let is_coinbase = tx_pos == 0;
            let coinbase_unspendable = is_coinbase && coinbase_is_unspendable(h, is_mainnet);
            let txid = tx.txid().to_bytes();
            for (vout, output) in tx.outputs().enumerate() {
                if coinbase_unspendable || script_is_unspendable(output.script_pubkey().as_bytes())
                {
                    continue; // unspendable outputs never consume an index
                }
                let index = spendable_index;
                spendable_index += 1;
                if !spent.contains(&(txid, vout as u32)) {
                    indices.push(index); // unspent at height n
                    total_unspent += 1;
                }
            }
        }
        writer
            .append_block(&indices)
            .map_err(|e| KernelError::Internal(format!("failed to write block {h}: {e}")))?;
        if h % progress_interval == 0 {
            log::info!("pass 2/2 (hints): block {h} / {n}");
        }
    }
    writer
        .finish()
        .map_err(|e| KernelError::Internal(format!("failed to flush hintsfile: {e}")))?;

    Ok(Stats {
        terminal_height: n,
        num_blocks,
        spent_outpoints: spent.len(),
        total_unspent,
    })
}

/// Returns true if a script can never be spent and so never enters the UTXO set.
///
/// An output is unspendable if its script is longer than 10,000 bytes or begins
/// with `OP_RETURN` (`0x6a`).
fn script_is_unspendable(script: &[u8]) -> bool {
    script.len() > 10_000 || script.first() == Some(&0x6a)
}

/// Returns true if a coinbase output at `height` is unspendable by consensus.
///
/// The genesis coinbase is unspendable on every network. On mainnet the two
/// BIP-30 duplicate coinbases overwrote earlier coinbases, leaving the earlier
/// outputs at heights 91722 and 91812 permanently unspendable.
fn coinbase_is_unspendable(height: i32, is_mainnet: bool) -> bool {
    height == 0 || (is_mainnet && (height == 91722 || height == 91812))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_return_is_unspendable() {
        assert!(script_is_unspendable(&[0x6a]));
        assert!(script_is_unspendable(&[0x6a, 0x01, 0x02]));
    }

    #[test]
    fn oversized_script_is_unspendable() {
        assert!(script_is_unspendable(&vec![0u8; 10_001]));
        assert!(!script_is_unspendable(&vec![0u8; 10_000]));
    }

    #[test]
    fn normal_script_is_spendable() {
        // A P2PKH script pubkey.
        let p2pkh = [
            0x76, 0xa9, 0x14, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
            0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x88, 0xac,
        ];
        assert!(!script_is_unspendable(&p2pkh));
    }

    #[test]
    fn genesis_coinbase_always_unspendable() {
        assert!(coinbase_is_unspendable(0, false));
        assert!(coinbase_is_unspendable(0, true));
    }

    #[test]
    fn bip30_only_on_mainnet() {
        assert!(coinbase_is_unspendable(91722, true));
        assert!(coinbase_is_unspendable(91812, true));
        assert!(!coinbase_is_unspendable(91722, false));
        assert!(!coinbase_is_unspendable(100, true));
    }
}
