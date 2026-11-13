use std::{
    fs::File,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::error::{Error, Result};

/// The cgroup's memory files, kept open so each tick is a single `pread`.
pub struct CgroupMemory {
    dir: PathBuf,
    pressure: File,
    current: File,
}

impl CgroupMemory {
    pub(crate) fn open(dir: &Path) -> Result<Self> {
        let open = |file: &str| {
            let path = dir.join(file);
            File::open(&path).map_err(|source| Error::Io { path, source })
        };
        Ok(Self {
            dir: dir.to_owned(),
            pressure: open("memory.pressure")?,
            current: open("memory.current")?,
        })
    }

    /// Time at least one task in the cgroup was stalled on memory, since it was created.
    // some avg10=0.00 avg60=0.00 avg300=0.00 total=106340
    // full avg10=0.00 avg60=0.00 avg300=0.00 total=0
    pub(crate) fn some_total(&self) -> Result<Duration> {
        let mut buf = [0; 256];
        self.read(&self.pressure, "memory.pressure", &mut buf)?
            .lines()
            .find_map(|line| line.strip_prefix("some "))
            .and_then(|line| line.rsplit_once("total="))
            .and_then(|(_, us)| us.trim().parse().ok())
            .map(Duration::from_micros)
            .ok_or(Error::Parse {
                file: "memory.pressure",
                field: "some total=",
            })
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
