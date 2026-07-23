// Deserializable process/worker settings and validated per-worker storage geometry.

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub(crate) struct AppConfig {
    version: u32,
    runtime: RuntimeConfig,
    io_uring: IoUringConfig,
    timeouts: TimeoutConfig,
    storage: StorageConfig,
    index: IndexConfig,
    durability: DurabilityConfig,
    recovery: RecoveryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            runtime: RuntimeConfig::default(),
            io_uring: IoUringConfig::default(),
            timeouts: TimeoutConfig::default(),
            storage: StorageConfig::default(),
            index: IndexConfig::default(),
            durability: DurabilityConfig::default(),
            recovery: RecoveryConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct RuntimeConfig {
    thread_count: usize,
    cpu_ids: Vec<usize>,
    event_interval: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let thread_count = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        Self {
            thread_count,
            cpu_ids: (0..thread_count).collect(),
            event_interval: 31,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct IoUringConfig {
    sqpoll: bool,
    entries_per_worker: u32,
    max_inflight_per_worker: usize,
    batch_size: usize,
    batch_max_wait_us: u64,
}

impl Default for IoUringConfig {
    fn default() -> Self {
        Self {
            sqpoll: false,
            entries_per_worker: 256,
            max_inflight_per_worker: 8,
            batch_size: 16,
            batch_max_wait_us: 10,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct TimeoutConfig {
    input_max_time_us: u64,
    output_max_time_us: u64,
    read_max_time_us: u64,
    write_max_time_us: u64,
    request_max_time_us: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            input_max_time_us: 10_000,
            output_max_time_us: 1_000_000,
            read_max_time_us: 100_000,
            write_max_time_us: 1_000_000,
            request_max_time_us: 2_000_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct StorageConfig {
    directory: PathBuf,
    data_file_pattern: String,
    index_file_pattern: String,
    sg_per_thread: usize,
    sg_size_mib: usize,
    page_size_bytes: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("target/kvkache-v1"),
            data_file_pattern: "data-{thread_id:02}.sg".into(),
            index_file_pattern: "index-{thread_id:02}.chk".into(),
            sg_per_thread: 4,
            sg_size_mib: 16,
            page_size_bytes: 4096,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct IndexConfig {
    capacity_per_thread: usize,
    target_load_percent: usize,
    fingerprint_bits: usize,
    mini_buckets: usize,
    front_back_ratio: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            capacity_per_thread: 625_000,
            target_load_percent: 88,
            fingerprint_bits: 8,
            mini_buckets: 32,
            front_back_ratio: 8,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct DurabilityConfig {
    checkpoint_on_sg_flush: bool,
}

impl Default for DurabilityConfig {
    fn default() -> Self {
        Self {
            checkpoint_on_sg_flush: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct RecoveryConfig {
    enabled: bool,
    fallback_to_sg_scan: bool,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fallback_to_sg_scan: true,
        }
    }
}

impl AppConfig {
    fn parse() -> Result<(Self, Command)> {
        let arguments = env::args().skip(1).collect::<Vec<_>>();
        let config_path = arguments
            .windows(2)
            .find(|pair| pair[0] == "--config")
            .map(|pair| PathBuf::from(&pair[1]));
        let mut config = if let Some(path) = &config_path {
            let text = fs::read_to_string(path).map_err(|error| {
                KvError::InvalidConfig(format!("cannot read config {}: {error}", path.display()))
            })?;
            toml::from_str(&text).map_err(|error| {
                KvError::InvalidConfig(format!("cannot parse config {}: {error}", path.display()))
            })?
        } else {
            Self::default()
        };

        let mut args = arguments.into_iter().peekable();
        while let Some(argument) = args.peek() {
            if !argument.starts_with("--") {
                break;
            }
            let flag = args.next().unwrap();
            let value = |args: &mut std::iter::Peekable<_>, flag: &str| -> Result<String> {
                args.next()
                    .ok_or_else(|| KvError::Usage(format!("missing value for {flag}")))
            };
            match flag.as_str() {
                "--config" => {
                    let _ = value(&mut args, &flag)?;
                }
                "--threads" => {
                    config.runtime.thread_count = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--cpu-ids" => {
                    config.runtime.cpu_ids = value(&mut args, &flag)?
                        .split(',')
                        .map(|cpu| parse_usize(cpu.trim(), &flag))
                        .collect::<Result<Vec<_>>>()?
                }
                "--event-interval" => {
                    config.runtime.event_interval = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--io-uring-entries" => {
                    config.io_uring.entries_per_worker = value(&mut args, &flag)?
                        .parse()
                        .map_err(|_| KvError::Usage(format!("{flag} expects a u32")))?
                }
                "--max-inflight" => {
                    config.io_uring.max_inflight_per_worker =
                        parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--batch-size" => {
                    config.io_uring.batch_size = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--batch-wait-us" => {
                    config.io_uring.batch_max_wait_us = parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--input-max-us" => {
                    config.timeouts.input_max_time_us = parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--output-max-us" => {
                    config.timeouts.output_max_time_us =
                        parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--read-max-us" => {
                    config.timeouts.read_max_time_us = parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--write-max-us" => {
                    config.timeouts.write_max_time_us = parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--request-max-us" => {
                    config.timeouts.request_max_time_us =
                        parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--directory" => config.storage.directory = value(&mut args, &flag)?.into(),
                "--sg-per-thread" => {
                    config.storage.sg_per_thread = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--sg-mib" => {
                    config.storage.sg_size_mib = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--page-bytes" => {
                    config.storage.page_size_bytes = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--index-capacity-per-thread" => {
                    config.index.capacity_per_thread =
                        parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--index-load-percent" => {
                    config.index.target_load_percent =
                        parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--fingerprint-bits" => {
                    config.index.fingerprint_bits = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--mini-buckets" => {
                    config.index.mini_buckets = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--front-back-ratio" => {
                    config.index.front_back_ratio = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--help" => return Err(KvError::Usage(usage())),
                _ => {
                    return Err(KvError::Usage(format!(
                        "unknown option {flag}\n{}",
                        usage()
                    )));
                }
            }
        }

        let command = parse_command(&mut args)?;
        config.validate()?;
        Ok((config, command))
    }

    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(KvError::InvalidConfig(format!(
                "unsupported config version {}",
                self.version
            )));
        }
        if self.runtime.thread_count == 0 {
            return Err(KvError::InvalidConfig(
                "runtime.thread_count must be non-zero".into(),
            ));
        }
        if self.runtime.cpu_ids.len() != self.runtime.thread_count {
            return Err(KvError::InvalidConfig(
                "runtime.cpu_ids length must equal runtime.thread_count".into(),
            ));
        }
        let unique = self.runtime.cpu_ids.iter().copied().collect::<HashSet<_>>();
        if unique.len() != self.runtime.cpu_ids.len() {
            return Err(KvError::InvalidConfig(
                "runtime.cpu_ids must not contain duplicates".into(),
            ));
        }
        let allowed = allowed_cpu_ids()?;
        if let Some(cpu) = self
            .runtime
            .cpu_ids
            .iter()
            .find(|cpu| !allowed.contains(cpu))
        {
            return Err(KvError::InvalidConfig(format!(
                "CPU {cpu} is not in the process affinity set {allowed:?}"
            )));
        }
        if self.runtime.event_interval == 0 {
            return Err(KvError::InvalidConfig(
                "runtime.event_interval must be non-zero".into(),
            ));
        }
        if self.io_uring.sqpoll {
            return Err(KvError::InvalidConfig(
                "io_uring.sqpoll must remain false".into(),
            ));
        }
        if self.io_uring.entries_per_worker == 0
            || self.io_uring.max_inflight_per_worker == 0
            || self.io_uring.max_inflight_per_worker > self.io_uring.entries_per_worker as usize
        {
            return Err(KvError::InvalidConfig(
                "io_uring max_inflight must be in 1..=entries_per_worker".into(),
            ));
        }
        if self.io_uring.batch_size == 0 {
            return Err(KvError::InvalidConfig(
                "io_uring.batch_size must be non-zero".into(),
            ));
        }
        if self.timeouts.input_max_time_us == 0
            || self.timeouts.output_max_time_us == 0
            || self.timeouts.read_max_time_us == 0
            || self.timeouts.write_max_time_us == 0
            || self.timeouts.request_max_time_us == 0
        {
            return Err(KvError::InvalidConfig(
                "all timeout values must be non-zero".into(),
            ));
        }
        if self.storage.sg_per_thread == 0 || self.storage.sg_per_thread > 256 {
            return Err(KvError::InvalidConfig(
                "storage.sg_per_thread must be between 1 and 256".into(),
            ));
        }
        if self.storage.sg_size_mib.checked_mul(1024 * 1024).is_none() {
            return Err(KvError::InvalidConfig(
                "storage.sg_size_mib is too large".into(),
            ));
        }
        let data_names = (0..self.runtime.thread_count)
            .map(|thread_id| expand_thread_pattern(&self.storage.data_file_pattern, thread_id))
            .collect::<HashSet<_>>();
        let index_names = (0..self.runtime.thread_count)
            .map(|thread_id| expand_thread_pattern(&self.storage.index_file_pattern, thread_id))
            .collect::<HashSet<_>>();
        if data_names.len() != self.runtime.thread_count
            || index_names.len() != self.runtime.thread_count
        {
            return Err(KvError::InvalidConfig(
                "storage file patterns must expand to a unique file for every thread".into(),
            ));
        }
        if (0..self.runtime.thread_count).any(|thread_id| {
            expand_thread_pattern(&self.storage.data_file_pattern, thread_id)
                == expand_thread_pattern(&self.storage.index_file_pattern, thread_id)
        }) {
            return Err(KvError::InvalidConfig(
                "data and index file patterns must not resolve to the same file".into(),
            ));
        }
        self.worker_config(0).validate()
    }

    fn worker_config(&self, thread_id: usize) -> Config {
        let data_name = expand_thread_pattern(&self.storage.data_file_pattern, thread_id);
        let index_name = expand_thread_pattern(&self.storage.index_file_pattern, thread_id);
        Config {
            data_path: self.storage.directory.join(data_name),
            index_path: self.storage.directory.join(index_name),
            sg_size: self.storage.sg_size_mib * 1024 * 1024,
            sg_count: self.storage.sg_per_thread,
            page_size: self.storage.page_size_bytes,
            index_capacity: self.index.capacity_per_thread,
            index_target_load_percent: self.index.target_load_percent,
            fingerprint_bits: self.index.fingerprint_bits,
            mini_buckets: self.index.mini_buckets,
            front_back_ratio: self.index.front_back_ratio,
            region_bits: bits_for_count(self.storage.sg_per_thread),
            checkpoint_on_sg_flush: self.durability.checkpoint_on_sg_flush,
            recovery_enabled: self.recovery.enabled,
            fallback_to_sg_scan: self.recovery.fallback_to_sg_scan,
            // Routing consumes hash[0..8]. The filter uses hash[8..16], so
            // arbitrary (including non-power-of-two) worker counts do not
            // correlate routing with the packed Breadcrumb fingerprint.
            fingerprint_hash_offset_bits: 64,
            read_max_time_us: self.timeouts.read_max_time_us,
            write_max_time_us: self.timeouts.write_max_time_us,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn for_trace_benchmark(
        directory: PathBuf,
        cpu_ids: Vec<usize>,
        total_sg_count: usize,
        total_index_capacity: usize,
    ) -> Result<Self> {
        if cpu_ids.is_empty() || !total_sg_count.is_multiple_of(cpu_ids.len()) {
            return Err(KvError::InvalidConfig(
                "trace benchmark total SG count must divide evenly across workers".into(),
            ));
        }
        let mut config = Self::default();
        config.runtime.thread_count = cpu_ids.len();
        config.runtime.cpu_ids = cpu_ids;
        config.io_uring.entries_per_worker = 256;
        config.io_uring.max_inflight_per_worker = 8;
        config.io_uring.batch_size = 16;
        config.timeouts = TimeoutConfig {
            input_max_time_us: 30_000_000,
            output_max_time_us: 30_000_000,
            read_max_time_us: 5_000_000,
            write_max_time_us: 30_000_000,
            request_max_time_us: 30_000_000,
        };
        config.storage.directory = directory;
        config.storage.sg_per_thread = total_sg_count / config.runtime.thread_count;
        config.index.capacity_per_thread =
            total_index_capacity.div_ceil(config.runtime.thread_count);
        config.validate()?;
        Ok(config)
    }
}
