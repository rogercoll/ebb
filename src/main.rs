mod cgroup;
mod ebb;
mod error;
mod memory;

use std::{path::PathBuf, process::ExitCode};

use clap::Parser;

use crate::{cgroup::Cgroup, ebb::EbbD};

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// cgroup v2 directory, e.g. /sys/fs/cgroup/user.slice
    cgroup: PathBuf,

    #[command(flatten)]
    mem: MemConfig,
}

/// Memory reclaim settings. Defaults are the production values from the TMO paper (§3.3).
#[derive(clap::Args)]
pub struct MemConfig {
    /// Fraction of memory.current to reclaim per period when there is no pressure
    #[arg(long, env = "EBBD_MEM_RECLAIM_RATIO", default_value_t = 0.0005, value_parser = positive)]
    pub reclaim_ratio: f64,

    /// Memory "some" pressure at which reclaim stops, as a fraction of time stalled (0.001 = 0.1%)
    #[arg(long, env = "EBBD_MEM_PSI_THRESHOLD", default_value_t = 0.001, value_parser = positive)]
    pub psi_threshold: f64,

    /// Seconds between reclaim decisions
    #[arg(long, env = "EBBD_MEM_INTERVAL", default_value_t = 6, value_parser = clap::value_parser!(u64).range(1..))]
    pub interval: u64,
}

fn positive(s: &str) -> std::result::Result<f64, String> {
    match s.parse::<f64>() {
        Ok(v) if v > 0.0 && v.is_finite() => Ok(v),
        _ => Err(format!("{s:?} is not a positive number")),
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    match Cgroup::new(args.cgroup).and_then(|cgroup| EbbD::new(cgroup).memory(&args.mem)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ebbd: {err}");
            ExitCode::FAILURE
        }
    }
}
