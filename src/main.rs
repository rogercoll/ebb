mod cgroup;
mod ebb;
mod error;
mod event;
mod io;
mod memory;
mod psi;

use std::{path::PathBuf, process::ExitCode, time::Duration};

use clap::Parser;
use rustix::process::{getrlimit, setrlimit, Resource, Rlimit};

use crate::ebb::{EbbD, Target};

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// cgroup v2 directory, e.g. /sys/fs/cgroup/user.slice
    cgroup: PathBuf,

    /// Reclaim from every leaf cgroup under CGROUP instead of CGROUP itself
    #[arg(long)]
    leaves: bool,

    /// Seconds between scans for new and removed leaves (with --leaves)
    #[arg(long, env = "EBBD_RESCAN_INTERVAL", default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
    rescan_interval: u64,

    #[command(flatten)]
    mem: MemConfig,

    #[command(flatten)]
    io: IoConfig,
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

/// IO pressure gate: no reclaim while IO pressure is above the threshold, since refaults
/// can slow the device without showing up as memory stalls.
#[derive(clap::Args)]
pub struct IoConfig {
    /// IO "some" pressure at which reclaim stops, as a fraction of time stalled (0.001 = 0.1%)
    #[arg(id = "io_psi_threshold", long = "io-psi-threshold", value_name = "IO_PSI_THRESHOLD", env = "EBBD_IO_PSI_THRESHOLD", default_value_t = 0.001, value_parser = positive)]
    pub psi_threshold: f64,
}

fn positive(s: &str) -> std::result::Result<f64, String> {
    match s.parse::<f64>() {
        Ok(v) if v > 0.0 && v.is_finite() => Ok(v),
        _ => Err(format!("{s:?} is not a positive number")),
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    // Each cgroup keeps 5 fds open, so the usual soft limit of 1024 runs out at ~200.
    let nofile = getrlimit(Resource::Nofile);
    let raised = Rlimit {
        current: nofile.maximum,
        ..nofile
    };
    if let Err(err) = setrlimit(Resource::Nofile, raised) {
        eprintln!("ebbd: raising the open file limit: {err}");
    }

    let target = if args.leaves {
        Target::Leaves {
            root: args.cgroup,
            rescan: Duration::from_secs(args.rescan_interval),
        }
    } else {
        Target::Cgroup(args.cgroup)
    };
    match EbbD::new(target, args.mem, args.io).and_then(|mut ebbd| ebbd.memory()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ebbd: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    #[test]
    fn cli_is_valid() {
        super::Args::command().debug_assert();
    }
}
