// CPU discovery, command parsing, usage text, and low-level worker configuration.

fn allowed_cpu_ids() -> Result<HashSet<usize>> {
    let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
    let result = unsafe {
        libc::sched_getaffinity(
            0,
            std::mem::size_of::<libc::cpu_set_t>(),
            &mut set as *mut _,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok((0..libc::CPU_SETSIZE as usize)
        .filter(|cpu| unsafe { libc::CPU_ISSET(*cpu, &set) })
        .collect())
}

fn expand_thread_pattern(pattern: &str, thread_id: usize) -> String {
    pattern
        .replace("{thread_id:02}", &format!("{thread_id:02}"))
        .replace("{thread_id}", &thread_id.to_string())
}

fn bits_for_count(count: usize) -> usize {
    (usize::BITS as usize - count.saturating_sub(1).leading_zeros() as usize).max(1)
}

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub(crate) data_path: PathBuf,
    pub(crate) index_path: PathBuf,
    pub(crate) sg_size: usize,
    pub(crate) sg_count: usize,
    pub(crate) page_size: usize,
    pub(crate) index_capacity: usize,
    pub(crate) index_target_load_percent: usize,
    pub(crate) fingerprint_bits: usize,
    pub(crate) mini_buckets: usize,
    pub(crate) front_back_ratio: usize,
    pub(crate) region_bits: usize,
    pub(crate) checkpoint_on_sg_flush: bool,
    pub(crate) recovery_enabled: bool,
    pub(crate) fallback_to_sg_scan: bool,
    pub(crate) fingerprint_hash_offset_bits: usize,
    pub(crate) read_max_time_us: u64,
    pub(crate) write_max_time_us: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_path: PathBuf::from("target/kvkache-v1/kvkache.data"),
            index_path: PathBuf::from("target/kvkache-v1/kvkache.index"),
            sg_size: 16 * 1024 * 1024,
            sg_count: 64,
            page_size: 4096,
            index_capacity: 10_000_000,
            index_target_load_percent: 88,
            fingerprint_bits: 8,
            mini_buckets: 32,
            front_back_ratio: 8,
            region_bits: 6,
            checkpoint_on_sg_flush: true,
            recovery_enabled: true,
            fallback_to_sg_scan: true,
            fingerprint_hash_offset_bits: 0,
            read_max_time_us: 1_000,
            write_max_time_us: 5_000,
        }
    }
}

impl Config {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.sg_count == 0 || self.sg_count > (1usize << self.region_bits) {
            return Err(KvError::InvalidConfig(format!(
                "sg-count must be in 1..={} for {} region bits",
                1usize << self.region_bits,
                self.region_bits
            )));
        }
        if self.fingerprint_bits > 16 {
            return Err(KvError::InvalidConfig(
                "fingerprint-bits must be between 0 and 16".into(),
            ));
        }
        if self.region_bits == 0 || self.region_bits > 8 {
            return Err(KvError::InvalidConfig(
                "region-bits must be between 1 and 8".into(),
            ));
        }
        if self.page_size < 512 || self.page_size > u16::MAX as usize {
            return Err(KvError::InvalidConfig(
                "page-bytes must be between 512 and 65535".into(),
            ));
        }
        if self.sg_size == 0 || !self.sg_size.is_multiple_of(self.page_size) {
            return Err(KvError::InvalidConfig(
                "SG size must be a non-zero multiple of page size".into(),
            ));
        }
        if self.sg_size.checked_mul(self.sg_count).is_none() {
            return Err(KvError::InvalidConfig(
                "total SG data size is too large".into(),
            ));
        }
        if self.index_capacity == 0 {
            return Err(KvError::InvalidConfig(
                "index-capacity must be non-zero".into(),
            ));
        }
        if !(1..=95).contains(&self.index_target_load_percent) {
            return Err(KvError::InvalidConfig(
                "index-load-percent must be between 1 and 95".into(),
            ));
        }
        if self.mini_buckets < 2 || self.mini_buckets > 96 {
            return Err(KvError::InvalidConfig(
                "mini-buckets must be between 2 and 96".into(),
            ));
        }
        if self.front_back_ratio < 2
            || self.front_back_ratio > 16
            || !self.front_back_ratio.is_power_of_two()
        {
            return Err(KvError::InvalidConfig(
                "front-back-ratio must be a power of two between 2 and 16".into(),
            ));
        }
        if self.fingerprint_hash_offset_bits > 64 {
            return Err(KvError::InvalidConfig(
                "fingerprint hash offset must be at most 64 bits".into(),
            ));
        }
        if self.read_max_time_us == 0 || self.write_max_time_us == 0 {
            return Err(KvError::InvalidConfig(
                "read/write timeouts must be non-zero".into(),
            ));
        }
        Ok(())
    }

    fn page_count(&self) -> usize {
        self.sg_size / self.page_size
    }

    fn data_bytes(&self) -> u64 {
        (self.sg_size * self.sg_count) as u64
    }

    fn signature(&self) -> [u64; 10] {
        [
            self.sg_size as u64,
            self.sg_count as u64,
            self.page_size as u64,
            self.index_capacity as u64,
            self.index_target_load_percent as u64,
            self.fingerprint_bits as u64,
            self.mini_buckets as u64,
            self.front_back_ratio as u64,
            self.region_bits as u64,
            self.fingerprint_hash_offset_bits as u64,
        ]
    }
}

fn parse_usize(value: &str, flag: &str) -> Result<usize> {
    value
        .parse()
        .map_err(|_| KvError::Usage(format!("{flag} expects a non-negative integer")))
}

fn parse_u64(value: &str, flag: &str) -> Result<u64> {
    value
        .parse()
        .map_err(|_| KvError::Usage(format!("{flag} expects a non-negative integer")))
}

fn required_arg<I: Iterator<Item = String>>(args: &mut I, usage: &str) -> Result<String> {
    args.next()
        .ok_or_else(|| KvError::Usage(format!("usage: {usage}")))
}

fn parse_command<I: Iterator<Item = String>>(args: &mut I) -> Result<Command> {
    let command = match args.next().as_deref() {
        Some("get") => Command::Get(required_arg(args, "get KEY")?.into_bytes()),
        Some("set") => Command::Set(
            required_arg(args, "set KEY VALUE")?.into_bytes(),
            required_arg(args, "set KEY VALUE")?.into_bytes(),
        ),
        Some("delete") => Command::Delete(required_arg(args, "delete KEY")?.into_bytes()),
        Some("sync") => Command::Sync,
        Some("stats") => Command::Stats,
        Some(command) => {
            return Err(KvError::Usage(format!(
                "unknown command {command}\n{}",
                usage()
            )));
        }
        None => return Err(KvError::Usage(usage())),
    };
    if args.next().is_some() {
        return Err(KvError::Usage("too many command arguments".into()));
    }
    Ok(command)
}

fn usage() -> String {
    "KVKACHE-v1 [options] <get KEY | set KEY VALUE | delete KEY | sync | stats>\n\
     Options:\n\
       --config PATH                TOML configuration file\n\
       --threads N                  worker thread count\n\
       --cpu-ids LIST               zero-based CPU IDs, comma separated\n\
       --io-uring-entries N         io_uring entries per worker\n\
       --max-inflight N             maximum in-flight I/O per worker\n\
       --batch-size N               requests processed per worker batch\n\
       --batch-wait-us N            partial batch wait in microseconds\n\
       --directory PATH             worker data/index directory\n\
       --sg-per-thread N            circular SGs owned by each worker\n\
       --sg-mib N                   SG size in MiB [16]\n\
       --page-bytes N               page size [4096]\n\
       --index-capacity-per-thread N expected live keys per worker\n\
       --index-load-percent N       planned filter load [88]\n\
       --fingerprint-bits N         R bits; zero removes R [8]\n\
       --mini-buckets N             M values per bucket [32]\n\
       --front-back-ratio N         front/back bucket ratio [8]"
        .into()
}

enum Command {
    Get(Vec<u8>),
    Set(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
    Sync,
    Stats,
}
