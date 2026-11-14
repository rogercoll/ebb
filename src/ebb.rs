use std::{
    collections::HashMap,
    hash::{BuildHasher, RandomState},
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::{
    cgroup::{Cgroup, CgroupTree, Leaf},
    error::Result,
    event::{Event, Poller, Token},
    memory::CgroupMemory,
    IoConfig, MemConfig,
};

pub enum Target {
    Cgroup(PathBuf),
    Leaves { root: PathBuf, rescan: Duration },
}

pub struct EbbD {
    mem: MemConfig,
    io: IoConfig,
    period: Duration,
    poller: Poller,
    // Only with --leaves. Errors then drop the cgroup instead of stopping the daemon.
    tree: Option<CgroupTree>,
    reclaimers: HashMap<Token, Reclaimer>,
}

impl EbbD {
    pub fn new(target: Target, mem: MemConfig, io: IoConfig) -> Result<Self> {
        let period = Duration::from_secs(mem.interval);
        let mut ebbd = Self {
            mem,
            io,
            period,
            poller: Poller::new()?,
            tree: None,
            reclaimers: HashMap::new(),
        };
        match target {
            Target::Cgroup(path) => ebbd.track(Reclaimer::new(path)?, period)?,
            Target::Leaves { root, rescan } => {
                ebbd.poller.add_rescan(rescan)?;
                ebbd.tree = Some(CgroupTree::new(root));
                ebbd.rescan()?;
            }
        }
        Ok(ebbd)
    }

    /// Every interval, computes how much memory to reclaim from each cgroup and asks the
    /// kernel to reclaim it (TMO, §3.3). Skips the period while IO pressure is at or above
    /// its threshold.
    pub fn memory(&mut self) -> Result<()> {
        loop {
            for event in self.poller.wait()? {
                match event {
                    Event::Tick(token) => self.tick(token)?,
                    Event::Rescan => self.rescan()?,
                }
            }
        }
    }

    fn tick(&mut self, token: Token) -> Result<()> {
        // Dropped by an earlier event in the same batch.
        let Some(reclaimer) = self.reclaimers.get_mut(&token) else {
            return Ok(());
        };
        let Err(err) = reclaimer.tick(&self.mem, &self.io) else {
            return Ok(());
        };
        let Some(tree) = &mut self.tree else {
            return Err(err);
        };
        eprintln!("ebbd: dropping {}: {err}", reclaimer.path.display());
        // So the next rescan adds it back if it's still, or again, a leaf.
        tree.forget(&reclaimer.path);
        self.untrack(token)
    }

    fn rescan(&mut self) -> Result<()> {
        let Some(tree) = &mut self.tree else {
            return Ok(());
        };
        for leaf in tree.rescan()? {
            match leaf {
                Leaf::Removed(path) => {
                    let token = self.reclaimers.iter().find(|(_, r)| r.path == path);
                    if let Some(&token) = token.map(|(token, _)| token) {
                        self.untrack(token)?;
                    }
                }
                Leaf::Added(path) => match Reclaimer::new(path) {
                    Ok(reclaimer) => {
                        // First tick somewhere in the next period, so the cgroups don't
                        // all reclaim at the same time.
                        let spread = RandomState::new().hash_one(&reclaimer.path);
                        let first =
                            self.period + self.period.mul_f64(spread as f64 / u64::MAX as f64);
                        self.track(reclaimer, first)?;
                    }
                    Err(err) => eprintln!("ebbd: skipping {err}"),
                },
            }
        }
        Ok(())
    }

    fn track(&mut self, reclaimer: Reclaimer, first: Duration) -> Result<()> {
        let token = self.poller.add_tick(self.period, first)?;
        self.reclaimers.insert(token, reclaimer);
        Ok(())
    }

    fn untrack(&mut self, token: Token) -> Result<()> {
        self.reclaimers.remove(&token);
        self.poller.remove(token)
    }
}

struct Reclaimer {
    path: PathBuf,
    cgroup: Cgroup,
    prev: Sample,
}

impl Reclaimer {
    fn new(path: PathBuf) -> Result<Self> {
        let cgroup = Cgroup::new(&path)?;
        let prev = Sample::read(&cgroup)?;
        Ok(Self { path, cgroup, prev })
    }

    fn tick(&mut self, config: &MemConfig, io: &IoConfig) -> Result<()> {
        let current = self.cgroup.memory.current()?;
        let now = Sample::read(&self.cgroup)?;
        // Share of the period at least one task was stalled on memory / IO.
        let elapsed = now.at.duration_since(self.prev.at).as_secs_f64();
        let pressure = now.memory.saturating_sub(self.prev.memory).as_secs_f64() / elapsed;
        let io_pressure = now.io.saturating_sub(self.prev.io).as_secs_f64() / elapsed;
        self.prev = now;

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
            "{} current={current} pressure={:.4}% io_pressure={:.4}% reclaim={reclaim}",
            self.path.display(),
            pressure * 100.0,
            io_pressure * 100.0
        );
        Ok(())
    }
}

struct Sample {
    memory: Duration,
    io: Duration,
    at: Instant,
}

impl Sample {
    fn read(cgroup: &Cgroup) -> Result<Self> {
        Ok(Self {
            memory: cgroup.memory.some_total()?,
            io: cgroup.io.some_total()?,
            at: Instant::now(),
        })
    }
}
