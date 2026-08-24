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

    /// Enables storage io_uring SQPOLL; optionally combine with --sqpoll-cpu-ids.
    #[arg(long)]
    pub sqpoll: bool,

    /// Worker-indexed SQPOLL kernel-thread CPU affinities.
    #[arg(long, value_name = "LIST")]
    pub sqpoll_cpu_ids: Option<String>,

    #[arg(long, value_name = "N")]
    pub max_inflight: Option<usize>,

    #[arg(long, value_name = "MIB")]
    pub max_inflight_value_mib: Option<usize>,

    #[arg(long, value_name = "N")]
    pub batch_size: Option<usize>,

    #[arg(long, value_name = "N")]
    pub batch_wait_us: Option<u64>,

    #[arg(long, value_name = "PATH")]
    pub directory: Option<PathBuf>,

    #[arg(long, value_name = "N")]
    pub segments_per_thread: Option<usize>,

    #[arg(long, value_name = "N")]
    pub segment_mib: Option<usize>,

    #[arg(long, value_name = "N")]
    pub blob_segment_mib: Option<usize>,

    #[arg(long, value_name = "MIB")]
    pub max_item_mib: Option<usize>,

    #[arg(long, value_name = "N")]
    pub table_capacity_per_thread: Option<usize>,

    #[arg(long, value_name = "N")]
    pub table_load_percent: Option<usize>,

    #[arg(long, value_name = "N")]
    pub fingerprint_bits: Option<usize>,

    #[arg(long, value_name = "N")]
    pub unary_count: Option<usize>,

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
    #[command(name = "experimental_sync", visible_alias = "experimental-sync")]
    ExperimentalSync,
    #[command(name = "experimental_stats", visible_alias = "experimental-stats")]
    ExperimentalStats,
}

pub(crate) fn parse_cpu_ids(value: &str) -> Result<Vec<usize>> {
    parse_cpu_list(value, "--cpu-ids")
}

fn parse_cpu_list(value: &str, option: &str) -> Result<Vec<usize>> {
    value
        .split(',')
        .map(|cpu| {
            cpu.trim()
                .parse()
                .map_err(|_| KvError::Usage(format!("{option}: expected comma-separated integers")))
        })
        .collect()
}

pub enum Command {
    Get(Vec<u8>),
    Set(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
    ExperimentalSync,
    ExperimentalStats,
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
        if cli.sqpoll {
            config.io_uring.sqpoll = true;
        }
        if let Some(v) = cli.sqpoll_cpu_ids {
            config.io_uring.sqpoll_cpu_ids = parse_cpu_list(&v, "--sqpoll-cpu-ids")?;
        }
        if let Some(v) = cli.max_inflight {
            config.io_uring.max_inflight_per_worker = v;
        }
        if let Some(v) = cli.max_inflight_value_mib {
            config.network.max_inflight_value_mib = v;
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
        if let Some(v) = cli.segments_per_thread {
            config.storage.segments_per_thread = v;
        }
        if let Some(v) = cli.segment_mib {
            config.storage.segment_size_mib = v;
        }
        if let Some(v) = cli.blob_segment_mib {
            config.storage.blob_segment_size_mib = v;
        }
        if let Some(v) = cli.max_item_mib {
            config.storage.max_item_size_mib = v;
        }
        if let Some(v) = cli.table_capacity_per_thread {
            config.table.capacity_per_thread = v;
        }
        if let Some(v) = cli.table_load_percent {
            config.table.target_load_percent = v;
        }
        if let Some(v) = cli.fingerprint_bits {
            config.table.fingerprint_bits = v;
        }
        if let Some(v) = cli.unary_count {
            config.table.unary_count = v;
        }
        if let Some(v) = cli.front_back_ratio {
            config.table.front_back_ratio = v;
        }

        config.validate()?;

        let command = match cli.command {
            Some(CliCommand::Get { key }) => Command::Get(key.into_bytes()),
            Some(CliCommand::Set { key, value }) => {
                Command::Set(key.into_bytes(), value.into_bytes())
            }
            Some(CliCommand::Delete { key }) => Command::Delete(key.into_bytes()),
            Some(CliCommand::ExperimentalSync) => Command::ExperimentalSync,
            Some(CliCommand::ExperimentalStats) => Command::ExperimentalStats,
            None => return Err(KvError::Usage(
                "usage: kvkache [options] <get KEY | set KEY VALUE | delete KEY | experimental_sync | experimental_stats>"
                    .into(),
            )),
        };

        Ok((config, command))
    }
}
