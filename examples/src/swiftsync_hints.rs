//! SwiftSync hintsfile generator.
//!
//! Walks a Bitcoin data directory with the bitcoinkernel API and writes a
//! SwiftSync hintsfile: a compact, Elias-Fano encoded record of which outputs
//! remain unspent at a terminal height.
//!
//! Usage:
//!
//!     swiftsync_hints <data_dir> [--chain <type>] [--signet-challenge <hex>]
//!                     [--height <n>] [--out <path>]
//!
//! - `<data_dir>` is a Bitcoin Core data directory containing a `blocks/`
//!   subdirectory (for example `~/.bitcoin/signet`).
//! - `--chain` is one of `mainnet`, `testnet`, `testnet4`, `signet`, `regtest`
//!   (default `signet`).
//! - `--signet-challenge` is the hex encoded signet challenge script for a
//!   custom signet (for example `51` for OP_TRUE). Supplying it selects the
//!   signet chain with that challenge.
//! - `--height` is the terminal height; it defaults to (and is clamped to) the
//!   chain tip.
//! - `--out` is the output path (default `hints.bin`).

mod swiftsync;

use std::env;
use std::fs::File;
use std::io::BufWriter;
use std::process;
use std::sync::Arc;

use bitcoinkernel::{
    ChainType, ChainstateManagerBuilder, Context, ContextBuilder, KernelError, Log, Logger,
};
use env_logger::Builder;
use log::LevelFilter;

use swiftsync::generate;

const USAGE: &str =
    "Usage: swiftsync_hints <data_dir> [--chain mainnet|testnet|testnet4|signet|regtest] \
     [--signet-challenge <hex>] [--height <n>] [--out <path>]";

struct Config {
    data_dir: String,
    chain: ChainType,
    signet_challenge: Option<Vec<u8>>,
    height: Option<i32>,
    out: String,
}

fn parse_chain(value: &str) -> Result<ChainType, String> {
    match value.to_lowercase().as_str() {
        "mainnet" | "main" => Ok(ChainType::Mainnet),
        "testnet" | "testnet3" | "test" => Ok(ChainType::Testnet),
        "testnet4" => Ok(ChainType::Testnet4),
        "signet" => Ok(ChainType::Signet),
        "regtest" => Ok(ChainType::Regtest),
        other => Err(format!("unknown chain type: {other}")),
    }
}

fn parse_args() -> Result<Config, String> {
    let mut args = env::args().skip(1);
    let mut data_dir: Option<String> = None;
    let mut chain = ChainType::Signet;
    let mut signet_challenge: Option<Vec<u8>> = None;
    let mut height: Option<i32> = None;
    let mut out = "hints.bin".to_string();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--chain" => {
                let value = args.next().ok_or("--chain requires a value")?;
                chain = parse_chain(&value)?;
            }
            "--signet-challenge" => {
                let value = args.next().ok_or("--signet-challenge requires a value")?;
                let bytes =
                    hex::decode(&value).map_err(|_| format!("invalid hex challenge: {value}"))?;
                signet_challenge = Some(bytes);
            }
            "--height" => {
                let value = args.next().ok_or("--height requires a value")?;
                let parsed: i32 = value
                    .parse()
                    .map_err(|_| format!("invalid height: {value}"))?;
                if parsed < 0 {
                    return Err("--height must not be negative".to_string());
                }
                height = Some(parsed);
            }
            "--out" => {
                out = args.next().ok_or("--out requires a value")?;
            }
            "-h" | "--help" => return Err("help".to_string()),
            other if other.starts_with("--") => return Err(format!("unknown flag: {other}")),
            other => {
                if data_dir.is_some() {
                    return Err(format!("unexpected argument: {other}"));
                }
                data_dir = Some(other.to_string());
            }
        }
    }

    // A custom signet challenge always selects the signet chain.
    if signet_challenge.is_some() {
        chain = ChainType::Signet;
    }

    Ok(Config {
        data_dir: data_dir.ok_or("missing <data_dir>")?,
        chain,
        signet_challenge,
        height,
        out,
    })
}

struct MainLog {}

impl Log for MainLog {
    fn log(&self, message: &str) {
        log::info!(
            target: "libbitcoinkernel",
            "{}",
            message
                .strip_suffix("\r\n")
                .or_else(|| message.strip_suffix('\n'))
                .unwrap_or(message)
        );
    }
}

fn setup_logging() -> Result<Logger, KernelError> {
    let mut builder = Builder::from_default_env();
    builder.filter(None, LevelFilter::Info).init();
    Logger::new(MainLog {})
}

fn create_context(
    chain: ChainType,
    signet_challenge: Option<&[u8]>,
) -> Result<Arc<Context>, KernelError> {
    let builder = match signet_challenge {
        Some(challenge) => ContextBuilder::new().signet(challenge),
        None => ContextBuilder::new().chain_type(chain),
    };
    Ok(Arc::new(builder.build()?))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _logger = setup_logging()?;

    let config = match parse_args() {
        Ok(config) => config,
        Err(message) => {
            if message != "help" {
                eprintln!("error: {message}");
            }
            eprintln!("{USAGE}");
            process::exit(if message == "help" { 0 } else { 2 });
        }
    };

    let context = create_context(config.chain, config.signet_challenge.as_deref())?;
    let blocks_dir = format!("{}/blocks", config.data_dir);
    let chainman =
        ChainstateManagerBuilder::new(&context, &config.data_dir, &blocks_dir)?.build()?;
    chainman.import_blocks()?;

    let tip = chainman.active_chain().height();
    let terminal_height = config.height.unwrap_or(tip).min(tip);
    log::info!(
        "generating hintsfile for {:?} up to height {terminal_height} (tip {tip})",
        config.chain
    );

    let file = BufWriter::new(File::create(&config.out)?);
    let stats = generate::generate(&chainman, config.chain, terminal_height, file)?;

    log::info!(
        "wrote {} for heights 0..={} ({} blocks, {} unspent outputs, {} spent outpoints tracked)",
        config.out,
        stats.terminal_height,
        stats.num_blocks,
        stats.total_unspent,
        stats.spent_outpoints
    );

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
