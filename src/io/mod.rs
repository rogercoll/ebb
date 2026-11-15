use std::{path::Path, time::Duration};

use crate::{error::Result, file::CgroupFile, psi};

/// The cgroup's IO files, kept open. Reclaim watches IO pressure too, because the refaults
/// it causes can slow the storage device without showing up as memory stalls (TMO, §3.3).
pub struct CgroupIo {
    pressure: CgroupFile,
}

impl CgroupIo {
    pub(crate) fn open(dir: &Path) -> Result<Self> {
        Ok(Self {
            pressure: CgroupFile::open(dir, "io.pressure")?,
        })
    }

    /// Time at least one task in the cgroup was stalled on IO, since it was created.
    pub(crate) fn some_total(&self) -> Result<Duration> {
        self.pressure.read("some total=", psi::some_total)
    }
}
