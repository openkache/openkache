//! CLI argument parsing using clap. Defines the [`Cli`] and [`CliCommand`] structs, the
//! higher-level [`Command`] enum, and [`AppConfig::parse()`] which merges CLI flags with
//! an optional TOML config file.

use std::path::PathBuf;

use clap::Parser;

use crate::config::AppConfig;
use crate::error::{KvError, Result};

#[derive(Parser)]
#[command(name = "kvkache", about = "A fast key-value cache server")]
pub struct Cli {
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,

    #[arg(long, value_name = "LIST")]
    pub cpu_ids: Option<String>,

    #[arg(long, value_name = "N")]
    pub io_uring_entries: Option<u32>,

    #[arg(long, value_name = "N")]
    pub max_inflight: Option<usize>,

    #[arg(long, value_name = "N")]
    pub batch_size: Option<usize>,

    #[arg(long, value_name = "N")]
    pub batch_wait_us: Option<u64>,

    #[arg(long, value_name = "PATH")]
    pub directory: Option<PathBuf>,

    #[arg(long, value_name = "N")]
    pub sg_per_thread: Option<usize>,

    #[arg(long, value_name = "N")]
    pub sg_mib: Option<usize>,

    #[arg(long, value_name = "N")]
    pub page_bytes: Option<usize>,

    #[arg(long, value_name = "N")]
    pub index_capacity_per_thread: Option<usize>,

    #[arg(long, value_name = "N")]
    pub index_load_percent: Option<usize>,

    #[arg(long, value_name = "N")]
    pub fingerprint_bits: Option<usize>,

    #[arg(long, value_name = "N")]
    pub mini_buckets: Option<usize>,

    #[arg(long, value_name = "N")]
    pub front_back_ratio: Option<usize>,

    #[arg(long, value_name = "N")]
    pub event_interval: Option<usize>,

    #[arg(long, value_name = "N")]
    pub input_max_us: Option<u64>,

    #[arg(long, value_name = "N")]
    pub output_max_us: Option<u64>,

    #[arg(long, value_name = "N")]
    pub read_max_us: Option<u64>,

    #[arg(long, value_name = "N")]
    pub write_max_us: Option<u64>,

    #[arg(long, value_name = "N")]
    pub request_max_us: Option<u64>,

    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(Parser)]
pub enum CliCommand {
    Get { key: String },
    Set { key: String, value: String },
    Delete { key: String },
    Sync,
    Stats,
}

fn parse_cpu_ids(value: &str) -> Result<Vec<usize>> {
    value
        .split(',')
        .map(|cpu| {
            cpu.trim()
                .parse()
                .map_err(|_| KvError::Usage("--cpu-ids: expected comma-separated integers".into()))
        })
        .collect()
}

pub enum Command {
    Get(Vec<u8>),
    Set(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
    Sync,
    Stats,
}

impl AppConfig {
    pub fn parse() -> Result<(Self, Command)> {
        let cli = Cli::parse();

        let mut config = if let Some(path) = &cli.config {
            let text = std::fs::read_to_string(path).map_err(|error| {
                KvError::InvalidConfig(format!("cannot read config {}: {error}", path.display()))
            })?;
            toml::from_str(&text).map_err(|error| {
                KvError::InvalidConfig(format!("cannot parse config {}: {error}", path.display()))
            })?
        } else {
            Self::default()
        };

        if let Some(v) = cli.threads {
            config.runtime.thread_count = v;
        }
        if let Some(v) = cli.cpu_ids {
            config.runtime.cpu_ids = parse_cpu_ids(&v)?;
        }
        if let Some(v) = cli.event_interval {
            config.runtime.event_interval = v;
        }
        if let Some(v) = cli.io_uring_entries {
            config.io_uring.entries_per_worker = v;
        }
        if let Some(v) = cli.max_inflight {
            config.io_uring.max_inflight_per_worker = v;
        }
        if let Some(v) = cli.batch_size {
            config.io_uring.batch_size = v;
        }
        if let Some(v) = cli.batch_wait_us {
            config.io_uring.batch_max_wait_us = v;
        }
        if let Some(v) = cli.input_max_us {
            config.timeouts.input_max_time_us = v;
        }
        if let Some(v) = cli.output_max_us {
            config.timeouts.output_max_time_us = v;
        }
        if let Some(v) = cli.read_max_us {
            config.timeouts.read_max_time_us = v;
        }
        if let Some(v) = cli.write_max_us {
            config.timeouts.write_max_time_us = v;
        }
        if let Some(v) = cli.request_max_us {
            config.timeouts.request_max_time_us = v;
        }
        if let Some(v) = cli.directory {
            config.storage.directory = v;
        }
        if let Some(v) = cli.sg_per_thread {
            config.storage.sg_per_thread = v;
        }
        if let Some(v) = cli.sg_mib {
            config.storage.sg_size_mib = v;
        }
        if let Some(v) = cli.page_bytes {
            config.storage.page_size_bytes = v;
        }
        if let Some(v) = cli.index_capacity_per_thread {
            config.index.capacity_per_thread = v;
        }
        if let Some(v) = cli.index_load_percent {
            config.index.target_load_percent = v;
        }
        if let Some(v) = cli.fingerprint_bits {
            config.index.fingerprint_bits = v;
        }
        if let Some(v) = cli.mini_buckets {
            config.index.mini_buckets = v;
        }
        if let Some(v) = cli.front_back_ratio {
            config.index.front_back_ratio = v;
        }

        config.validate()?;

        let command = match cli.command {
            Some(CliCommand::Get { key }) => Command::Get(key.into_bytes()),
            Some(CliCommand::Set { key, value }) => {
                Command::Set(key.into_bytes(), value.into_bytes())
            }
            Some(CliCommand::Delete { key }) => Command::Delete(key.into_bytes()),
            Some(CliCommand::Sync) => Command::Sync,
            Some(CliCommand::Stats) => Command::Stats,
            None => return Err(KvError::Usage(
                "usage: kvkache [options] <get KEY | set KEY VALUE | delete KEY | sync | stats>"
                    .into(),
            )),
        };

        Ok((config, command))
    }
}
