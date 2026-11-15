use std::{
    fs::{File, OpenOptions},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

use crate::error::{Error, Result};

/// A cgroup file kept open, so each read or write is a single `pread`/`pwrite`.
pub(crate) struct CgroupFile {
    name: &'static str,
    path: PathBuf,
    file: File,
}

impl CgroupFile {
    pub(crate) fn open(dir: &Path, name: &'static str) -> Result<Self> {
        Self::with(dir, name, OpenOptions::new().read(true))
    }

    pub(crate) fn open_write(dir: &Path, name: &'static str) -> Result<Self> {
        Self::with(dir, name, OpenOptions::new().write(true))
    }

    /// Reads the whole file and parses it. cgroup files regenerate their contents on every
    /// read at offset 0.
    pub(crate) fn read<T>(
        &self,
        field: &'static str,
        parse: impl FnOnce(&str) -> Option<T>,
    ) -> Result<T> {
        let mut buf = [0; 256];
        let n = self
            .file
            .read_at(&mut buf, 0)
            .map_err(Error::io(&self.path))?;
        std::str::from_utf8(&buf[..n])
            .ok()
            .and_then(parse)
            .ok_or(Error::Parse {
                file: self.name,
                field,
            })
    }

    pub(crate) fn write(&self, data: &[u8]) -> Result<()> {
        self.file
            .write_all_at(data, 0)
            .map_err(Error::io(&self.path))
    }

    fn with(dir: &Path, name: &'static str, options: &OpenOptions) -> Result<Self> {
        let path = dir.join(name);
        let file = options.open(&path).map_err(Error::io(&path))?;
        Ok(Self { name, path, file })
    }
}
