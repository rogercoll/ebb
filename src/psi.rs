use std::{
    fs::File,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::error::{Error, Result};

/// A PSI file (`memory.pressure`, `io.pressure`), kept open so each read is a single `pread`.
pub(crate) struct Psi {
    name: &'static str,
    path: PathBuf,
    file: File,
}

impl Psi {
    pub(crate) fn open(dir: &Path, name: &'static str) -> Result<Self> {
        let path = dir.join(name);
        let file = File::open(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Self { name, path, file })
    }

    /// Time at least one task in the cgroup was stalled on the resource, since it was created.
    // some avg10=0.00 avg60=0.00 avg300=0.00 total=106340
    // full avg10=0.00 avg60=0.00 avg300=0.00 total=0
    pub(crate) fn some_total(&self) -> Result<Duration> {
        let mut buf = [0; 256];
        // cgroup files regenerate their contents on every read at offset 0.
        let n = self.file.read_at(&mut buf, 0).map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        std::str::from_utf8(&buf[..n])
            .ok()
            .and_then(|data| data.lines().find_map(|line| line.strip_prefix("some ")))
            .and_then(|line| line.rsplit_once("total="))
            .and_then(|(_, us)| us.trim().parse().ok())
            .map(Duration::from_micros)
            .ok_or(Error::Parse {
                file: self.name,
                field: "some total=",
            })
    }
}
