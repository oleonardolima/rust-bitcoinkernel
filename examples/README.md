# Examples

## silentpaymentscanner

Scans a chain for silent payments outputs. To run it:

    cargo run --bin silentpaymentscanner -- /path/to/.bitcoin/regtest

## swiftsync_hints

Generates a [SwiftSync](https://github.com/bitcoin/bips/pull/2152) hintsfile: a
compact, Elias-Fano encoded record of which transaction outputs remain unspent
at a terminal height. SwiftSync clients use such a file to accelerate initial
block download.

The example walks the chain with the bitcoinkernel API, determines which
outputs survive to the terminal height, and writes the BIP hintsfile format.

To run it against a synced signet data directory:

    cargo run --bin swiftsync_hints -- /path/to/.bitcoin/signet --chain signet --out signet-hints.bin

For a custom signet, pass the hex encoded challenge script (this selects the
signet chain automatically):

    cargo run --bin swiftsync_hints -- /path/to/datadir --signet-challenge 51 --out hints.bin

Flags:

- `--chain <type>`: `mainnet`, `testnet`, `testnet4`, `signet`, or `regtest`
  (default `signet`).
- `--signet-challenge <hex>`: hex encoded signet challenge for a custom signet
  (for example `51` for OP_TRUE).
- `--height <n>`: terminal height (default and maximum: the chain tip). Useful
  to bound time and memory on large chains.
- `--out <path>`: output path (default `hints.bin`).

The spent-outpoint set is held in memory, so the peak memory is proportional to
the number of outputs spent up to the terminal height. This is modest on
regtest, signet, and testnet; for mainnet at full height, scope the run with
`--height`.
