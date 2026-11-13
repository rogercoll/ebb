use std::path::Path;

use crate::{error::Result, memory::CgroupMemory};

/// A cgroup v2 directory, e.g. `/sys/fs/cgroup/user.slice`.
pub struct Cgroup {
    pub(crate) memory: CgroupMemory,
}

impl Cgroup {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            memory: CgroupMemory::open(path.as_ref())?,
        })
    }
}
