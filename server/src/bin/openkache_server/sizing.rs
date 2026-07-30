//! Resource-sizing arguments and diagnostics for the server entry point.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use openkache::{SizingPlan, SizingProfile, SizingRequest};

const GIB: u64 = 1024 * 1024 * 1024;
const GB: u64 = 1_000_000_000;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Profile {
    Light,
    #[value(alias = "medium", alias = "middle")]
    Balanced,
    Heavy,
}

impl From<Profile> for SizingProfile {
    fn from(value: Profile) -> Self {
        match value {
            Profile::Light => Self::Light,
            Profile::Balanced => Self::Balanced,
            Profile::Heavy => Self::Heavy,
        }
    }
}

#[derive(Args)]
pub(super) struct SizingArguments {
    /// CPUs assigned to cache workers.
    #[arg(long, conflicts_with = "config")]
    cpus: Option<usize>,

    /// Override the detected process memory budget in GiB.
    #[arg(long, conflicts_with = "config")]
    memory_gib: Option<u64>,

    /// Override the detected SSD space budget in decimal GB.
    #[arg(long, conflicts_with = "config")]
    storage_gb: Option<u64>,

    /// Value-size model: light=100B, balanced=1KiB, heavy=2KiB.
    #[arg(long, value_enum, conflicts_with = "config")]
    profile: Option<Profile>,

    /// Storage directory used by the generated configuration.
    #[arg(long, conflicts_with = "config")]
    directory: Option<PathBuf>,

    /// Print the sizing result without opening storage or binding the server.
    #[arg(long, conflicts_with = "config")]
    plan: bool,
}

impl SizingArguments {
    pub(super) fn build_plan(
        &self,
        use_config_file: bool,
    ) -> Result<Option<SizingPlan>, Box<dyn std::error::Error>> {
        if use_config_file {
            return Ok(None);
        }
        let directory = self
            .directory
            .clone()
            .unwrap_or_else(|| PathBuf::from("target/kvkache-v1"));
        let mut request =
            SizingRequest::detect(directory, self.profile.map(Into::into).unwrap_or_default())?;
        if let Some(cpu_count) = self.cpus {
            request.cpu_count = cpu_count;
        }
        if let Some(memory_gib) = self.memory_gib {
            request.memory_bytes = checked_unit(memory_gib, GIB, "memory")?;
        }
        if let Some(storage_gb) = self.storage_gb {
            request.storage_bytes = checked_unit(storage_gb, GB, "storage")?;
        }
        Ok(Some(request.plan()?))
    }

    pub(super) const fn plan_only(&self) -> bool {
        self.plan
    }
}

pub(super) fn print_plan(plan: &SizingPlan) {
    println!(
        "Sizing: profile={} value_bytes={} workers={}",
        plan.profile.as_str(),
        plan.value_bytes,
        plan.config.runtime.thread_count
    );
    println!(
        "Storage: segments_per_worker={} segment={}MiB blob_segment={}MiB files={:.3}TB budget={:.3}TB",
        plan.config.storage.segments_per_thread,
        plan.config.storage.segment_size_mib,
        plan.config.storage.blob_segment_size_mib,
        plan.storage_file_bytes as f64 / 1_000_000_000_000.0,
        plan.storage_budget_bytes as f64 / 1_000_000_000_000.0,
    );
    println!(
        "Memory: process_budget={:.2}GiB table={:.2}GiB table_budget={:.2}GiB",
        plan.process_memory_budget_bytes as f64 / GIB as f64,
        plan.table_memory_bytes as f64 / GIB as f64,
        plan.table_memory_budget_bytes as f64 / GIB as f64,
    );
    println!(
        "Table: sg_index_bits={} capacity_per_worker={}",
        plan.sg_index_bits, plan.config.table.capacity_per_thread,
    );
    println!(
        "Keys: raw_capacity={} planned_live_capacity={}",
        plan.raw_key_capacity, plan.planned_key_capacity
    );
}

fn checked_unit(value: u64, unit: u64, name: &str) -> Result<u64, Box<dyn std::error::Error>> {
    value.checked_mul(unit).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{name} budget is too large"),
        )
        .into()
    })
}
