use std::{
    thread,
    time::{Duration, Instant},
};

use crate::{cgroup::Cgroup, error::Result, memory::CgroupMemory, IoConfig, MemConfig};

pub struct EbbD {
    cgroup: Cgroup,
}

impl EbbD {
    pub fn new(cgroup: Cgroup) -> Self {
        Self { cgroup }
    }

    /// Every interval, computes how much memory to reclaim from the cgroup and asks the
    /// kernel to reclaim it (TMO, §3.3). Skips the period while IO pressure is at or above
    /// its threshold.
    pub fn memory(&self, config: &MemConfig, io: &IoConfig) -> Result<()> {
        let mut prev_stall = self.cgroup.memory.some_total()?;
        let mut prev_io_stall = self.cgroup.io.some_total()?;
        let mut prev_at = Instant::now();
        loop {
            thread::sleep(Duration::from_secs(config.interval));
            let current = self.cgroup.memory.current()?;
            let stall = self.cgroup.memory.some_total()?;
            let io_stall = self.cgroup.io.some_total()?;
            let now = Instant::now();
            // Share of the period at least one task was stalled on memory / IO.
            let elapsed = now.duration_since(prev_at).as_secs_f64();
            let pressure = stall.saturating_sub(prev_stall).as_secs_f64() / elapsed;
            let io_pressure = io_stall.saturating_sub(prev_io_stall).as_secs_f64() / elapsed;
            (prev_stall, prev_io_stall, prev_at) = (stall, io_stall, now);

            let reclaim = if io_pressure >= io.psi_threshold {
                0
            } else {
                CgroupMemory::reclaim_amount(
                    current,
                    pressure,
                    config.reclaim_ratio,
                    config.psi_threshold,
                )
            };
            // Less than a page isn't worth a syscall.
            if reclaim >= 4096 {
                self.cgroup.memory.reclaim(reclaim)?;
            }
            println!(
                "current={current} pressure={:.4}% io_pressure={:.4}% reclaim={reclaim}",
                pressure * 100.0,
                io_pressure * 100.0
            );
        }
    }
}
