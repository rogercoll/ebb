use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

use crate::{
    error::{Error, Result},
    memory::CgroupMemory,
};

/// A cgroup v2 directory, e.g. `/sys/fs/cgroup/user.slice`.
pub struct Cgroup {
    path: PathBuf,
    memory: CgroupMemory,
}

impl Cgroup {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let memory = CgroupMemory::open(&path)?;
        Ok(Self { path, memory })
    }

    /// Time at least one task in the cgroup was stalled on memory, since it was created.
    pub(crate) fn memory_pressure(&self) -> Result<Duration> {
        self.memory.some_total()
    }

    /// Memory charged to the cgroup, in bytes.
    pub(crate) fn memory_current(&self) -> Result<u64> {
        self.memory.current()
    }

    /// Asks the kernel to reclaim `bytes` from the cgroup, once, without setting a limit.
    pub(crate) fn memory_reclaim(&self, bytes: u64) -> Result<()> {
        let path = self.path.join("memory.reclaim");
        match OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|mut file| file.write_all(bytes.to_string().as_bytes()))
        {
            // EAGAIN: the kernel couldn't reclaim all of it (e.g. nothing reclaimable is
            // left). That's a normal outcome, not an error.
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(()),
            result => result.map_err(|source| Error::Io { path, source }),
        }
    }
}
