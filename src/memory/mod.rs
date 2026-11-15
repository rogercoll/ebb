use std::{io, path::Path, time::Duration};

use crate::{
    error::{Error, Result},
    file::CgroupFile,
    psi,
};

/// The cgroup's memory files, kept open.
pub struct CgroupMemory {
    pressure: CgroupFile,
    current: CgroupFile,
    reclaim: CgroupFile,
}

impl CgroupMemory {
    pub(crate) fn open(dir: &Path) -> Result<Self> {
        Ok(Self {
            pressure: CgroupFile::open(dir, "memory.pressure")?,
            current: CgroupFile::open(dir, "memory.current")?,
            reclaim: CgroupFile::open_write(dir, "memory.reclaim")?,
        })
    }

    /// Time at least one task in the cgroup was stalled on memory, since it was created.
    pub(crate) fn some_total(&self) -> Result<Duration> {
        self.pressure.read("some total=", psi::some_total)
    }

    /// Memory charged to the cgroup, in bytes.
    // 263462912
    pub(crate) fn current(&self) -> Result<u64> {
        self.current.read("bytes", |data| data.trim().parse().ok())
    }

    /// Asks the kernel to reclaim `bytes` from the cgroup, once, without setting a limit.
    pub(crate) fn reclaim(&self, bytes: u64) -> Result<()> {
        match self.reclaim.write(bytes.to_string().as_bytes()) {
            // EAGAIN: the kernel couldn't reclaim all of it (e.g. nothing reclaimable is
            // left). That's a normal outcome, not an error.
            Err(Error::Io { source, .. }) if source.kind() == io::ErrorKind::WouldBlock => Ok(()),
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes fake cgroup memory files into a temp dir and opens them.
    fn fake_cgroup(name: &str, pressure: &str, current: &str) -> CgroupMemory {
        let dir = std::env::temp_dir().join(format!("releafd-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("memory.pressure"), pressure).unwrap();
        std::fs::write(dir.join("memory.current"), current).unwrap();
        std::fs::write(dir.join("memory.reclaim"), "").unwrap();
        let memory = CgroupMemory::open(&dir).unwrap();
        // The open files stay readable after the directory is gone.
        std::fs::remove_dir_all(&dir).unwrap();
        memory
    }

    #[test]
    fn parses_memory_files() {
        let memory = fake_cgroup(
            "ok",
            "some avg10=0.00 avg60=0.00 avg300=0.00 total=106340\n\
             full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
            "263462912\n",
        );
        assert_eq!(memory.some_total().unwrap(), Duration::from_micros(106340));
        assert_eq!(memory.current().unwrap(), 263462912);

        let memory = fake_cgroup("garbage", "garbage", "garbage");
        assert!(memory.some_total().is_err());
        assert!(memory.current().is_err());
    }
}
