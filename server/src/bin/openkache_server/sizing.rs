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
    #[arg(
        long,
        requires_all = ["memory_gib", "storage_gb"],
        conflicts_with = "config"
    )]
    cpus: Option<usize>,

    /// Maximum process memory in GiB.
    #[arg(
        long,
        requires_all = ["cpus", "storage_gb"],
        conflicts_with = "config"
    )]
    memory_gib: Option<u64>,

    /// Maximum SSD space in decimal GB.
    #[arg(
        long,
        requires_all = ["cpus", "memory_gib"],
        conflicts_with = "config"
    )]
    storage_gb: Option<u64>,

    /// Value-size model: light=100B, balanced=1KiB, heavy=2KiB.
    #[arg(long, value_enum, requires = "cpus", conflicts_with = "config")]
    profile: Option<Profile>,

    /// Storage directory used by the generated configuration.
    #[arg(long, requires = "cpus", conflicts_with = "config")]
    directory: Option<PathBuf>,

    /// Print the sizing result without opening storage or binding the server.
    #[arg(long, requires = "cpus", conflicts_with = "config")]
    plan: bool,
}

impl SizingArguments {
    pub(super) fn build_plan(&self) -> Result<Option<SizingPlan>, Box<dyn std::error::Error>> {
        let Some(cpu_count) = self.cpus else {
            return Ok(None);
        };
        let memory_bytes = checked_unit(
            self.memory_gib
                .expect("clap requires --memory-gib with --cpus"),
            GIB,
            "memory",
        )?;
        let storage_bytes = checked_unit(
            self.storage_gb
                .expect("clap requires --storage-gb with --cpus"),
            GB,
            "storage",
        )?;
        Ok(Some(
            SizingRequest {
                cpu_count,
                memory_bytes,
                storage_bytes,
                directory: self
                    .directory
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("target/kvkache-v1")),
                profile: self.profile.map(Into::into).unwrap_or_default(),
            }
            .plan()?,
        ))
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
        "Table: sg_index_bits={} capacity_per_worker={} memory={:.2}GiB budget={:.2}GiB",
        plan.sg_index_bits,
        plan.config.table.capacity_per_thread,
        plan.table_memory_bytes as f64 / GIB as f64,
        plan.table_memory_budget_bytes as f64 / GIB as f64,
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
