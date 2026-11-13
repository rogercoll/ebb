use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    error::{Error, Result},
    psi::Psi,
};

/// The cgroup's memory files, kept open so each tick is a single `pread`/`pwrite` per file.
pub struct CgroupMemory {
    dir: PathBuf,
    pressure: Psi,
    current: File,
    reclaim: File,
}

impl CgroupMemory {
    pub(crate) fn open(dir: &Path) -> Result<Self> {
        let open = |file: &str, write: bool| {
            let path = dir.join(file);
            OpenOptions::new()
                .read(!write)
                .write(write)
                .open(&path)
                .map_err(|source| Error::Io { path, source })
        };
        Ok(Self {
            dir: dir.to_owned(),
            pressure: Psi::open(dir, "memory.pressure")?,
            current: open("memory.current", false)?,
            reclaim: open("memory.reclaim", true)?,
        })
    }

    /// Time at least one task in the cgroup was stalled on memory, since it was created.
    pub(crate) fn some_total(&self) -> Result<Duration> {
        self.pressure.some_total()
    }

    /// Memory charged to the cgroup, in bytes.
    // 263462912
    pub(crate) fn current(&self) -> Result<u64> {
        let mut buf = [0; 32];
        self.read(&self.current, "memory.current", &mut buf)?
            .trim()
            .parse()
            .map_err(|_| Error::Parse {
                file: "memory.current",
                field: "bytes",
            })
    }

    /// Asks the kernel to reclaim `bytes` from the cgroup, once, without setting a limit.
    pub(crate) fn reclaim(&self, bytes: u64) -> Result<()> {
        match self.reclaim.write_all_at(bytes.to_string().as_bytes(), 0) {
            // EAGAIN: the kernel couldn't reclaim all of it (e.g. nothing reclaimable is
            // left). That's a normal outcome, not an error.
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(()),
            result => result.map_err(|source| Error::Io {
                path: self.dir.join("memory.reclaim"),
                source,
            }),
        }
    }

    /// Reads the whole file from the start. cgroup files regenerate their contents on
    /// every read at offset 0.
    fn read<'a>(&self, file: &File, name: &'static str, buf: &'a mut [u8]) -> Result<&'a str> {
        let n = file.read_at(buf, 0).map_err(|source| Error::Io {
            path: self.dir.join(name),
            source,
        })?;
        std::str::from_utf8(&buf[..n]).map_err(|_| Error::Parse {
            file: name,
            field: "utf-8",
        })
    }

    /// Bytes to reclaim this period (TMO, §3.3):
    /// `current × ratio × max(0, 1 − pressure / threshold)`.
    ///
    /// `pressure` and `threshold` are fractions of wall time stalled (0.001 = 0.1%).
    pub(crate) fn reclaim_amount(current: u64, pressure: f64, ratio: f64, threshold: f64) -> u64 {
        (current as f64 * ratio * (1.0 - pressure / threshold).max(0.0)) as u64
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

    #[test]
    fn reclaim_amount() {
        let gib = 1 << 30;
        // No pressure: the full ratio.
        assert_eq!(
            CgroupMemory::reclaim_amount(gib, 0.0, 0.01, 0.001),
            gib / 100
        );
        // Halfway to the threshold: half of it.
        assert_eq!(
            CgroupMemory::reclaim_amount(gib, 0.0005, 0.01, 0.001),
            gib / 200
        );
        // At or above the threshold: nothing.
        assert_eq!(CgroupMemory::reclaim_amount(gib, 0.001, 0.01, 0.001), 0);
        assert_eq!(CgroupMemory::reclaim_amount(gib, 0.5, 0.01, 0.001), 0);
    }
}
