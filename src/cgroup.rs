use std::path::Path;

use crate::{error::Result, io::CgroupIo, memory::CgroupMemory};

/// A cgroup v2 directory, e.g. `/sys/fs/cgroup/user.slice`.
pub struct Cgroup {
    pub(crate) memory: CgroupMemory,
    pub(crate) io: CgroupIo,
}

impl Cgroup {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        Ok(Self {
            memory: CgroupMemory::open(path)?,
            io: CgroupIo::open(path)?,
        })
    }
}
