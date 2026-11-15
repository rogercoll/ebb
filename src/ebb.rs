use std::{
    collections::HashMap,
    hash::{BuildHasher, RandomState},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    cgroup::{Cgroup, CgroupTree, Leaf},
    error::Result,
    event::{Event, Poller, Token},
    policy::Pressure,
};

pub struct EbbD<F, P> {
    period: Duration,
    poller: Poller,
    tree: CgroupTree<F>,
    policy: P,
    reclaimers: HashMap<Token, Reclaimer>,
}

impl<F: Fn(&Path) -> bool, P: Fn(Pressure, u64) -> u64> EbbD<F, P> {
    /// `policy` gets each period's pressure and `memory.current`, and returns the bytes to
    /// reclaim.
    pub(crate) fn new(
        tree: CgroupTree<F>,
        policy: P,
        period: Duration,
        rescan: Duration,
    ) -> Result<Self> {
        let mut ebbd = Self {
            period,
            poller: Poller::new()?,
            tree,
            policy,
            reclaimers: HashMap::new(),
        };
        ebbd.poller.add_rescan(rescan)?;
        ebbd.rescan()?;
        if ebbd.reclaimers.is_empty() {
            eprintln!("ebbd: no leaf cgroups to reclaim yet, rescanning every {rescan:?}");
        }
        Ok(ebbd)
    }

    /// Every period, asks the policy how much memory to reclaim from each cgroup and asks the
    /// kernel to reclaim it.
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
        let Err(err) = reclaimer.tick(&self.policy) else {
            return Ok(());
        };
        eprintln!("ebbd: dropping {}: {err}", reclaimer.path.display());
        // So the next rescan adds it back if it's still, or again, a leaf.
        self.tree.forget(&reclaimer.path);
        self.untrack(token)
    }

    fn rescan(&mut self) -> Result<()> {
        for leaf in self.tree.rescan()? {
            match leaf {
                Leaf::Removed(path) => {
                    let token = self
                        .reclaimers
                        .iter()
                        .find_map(|(&token, r)| (r.path == path).then_some(token));
                    if let Some(token) = token {
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

    fn tick(&mut self, policy: &impl Fn(Pressure, u64) -> u64) -> Result<()> {
        let current = self.cgroup.memory.current()?;
        let now = Sample::read(&self.cgroup)?;
        let pressure = now.pressure_since(&self.prev);
        self.prev = now;

        let reclaim = policy(pressure, current);
        // Less than a page isn't worth a syscall.
        if reclaim >= 4096 {
            self.cgroup.memory.reclaim(reclaim)?;
        }
        println!(
            "{} current={current} pressure={:.4}% io_pressure={:.4}% reclaim={reclaim}",
            self.path.display(),
            pressure.memory * 100.0,
            pressure.io * 100.0
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

    /// Share of the time since `prev` at least one task was stalled on memory / IO.
    fn pressure_since(&self, prev: &Sample) -> Pressure {
        let elapsed = self.at.duration_since(prev.at).as_secs_f64();
        let share =
            |now: Duration, prev: Duration| now.saturating_sub(prev).as_secs_f64() / elapsed;
        Pressure {
            memory: share(self.memory, prev.memory),
            io: share(self.io, prev.io),
        }
    }
}
