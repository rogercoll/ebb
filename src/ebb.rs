use std::{
    thread,
    time::{Duration, Instant},
};

use crate::{cgroup::Cgroup, error::Result, memory::CgroupMemory, MemConfig};

pub struct EbbD {
    cgroup: Cgroup,
}

impl EbbD {
    pub fn new(cgroup: Cgroup) -> Self {
        Self { cgroup }
    }

    /// Every interval, computes how much memory to reclaim from the cgroup and asks the
    /// kernel to reclaim it (TMO, §3.3).
    pub fn memory(&self, config: &MemConfig) -> Result<()> {
        let mut prev_stall = self.cgroup.memory_pressure()?;
        let mut prev_at = Instant::now();
        loop {
            thread::sleep(Duration::from_secs(config.interval));
            let current = self.cgroup.memory_current()?;
            let stall = self.cgroup.memory_pressure()?;
            let now = Instant::now();
            // Share of the period at least one task was stalled on memory.
            let pressure = stall.saturating_sub(prev_stall).as_secs_f64()
                / now.duration_since(prev_at).as_secs_f64();
            (prev_stall, prev_at) = (stall, now);

            let reclaim = CgroupMemory::reclaim_amount(
                current,
                pressure,
                config.reclaim_ratio,
                config.psi_threshold,
            );
            // Less than a page isn't worth a syscall.
            if reclaim >= 4096 {
                self.cgroup.memory_reclaim(reclaim)?;
            }
            println!(
                "current={current} pressure={:.4}% reclaim={reclaim}",
                pressure * 100.0
            );
        }
    }
}
