use std::{
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// A cgroup file didn't have the expected format.
    #[error("{file}: no \"{field}\"")]
    Parse {
        file: &'static str,
        field: &'static str,
    },

    /// Reading a cgroup file failed.
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// A syscall failed (epoll, timerfd).
    #[error("{op}: {source}")]
    Os {
        op: &'static str,
        source: rustix::io::Errno,
    },
}

impl Error {
    /// For `map_err`. Copies `path` only if there is an error.
    pub(crate) fn io(path: &Path) -> impl FnOnce(io::Error) -> Self + '_ {
        move |source| Self::Io {
            path: path.to_owned(),
            source,
        }
    }

    pub(crate) fn os(op: &'static str) -> impl FnOnce(rustix::io::Errno) -> Self {
        move |source| Self::Os { op, source }
    }
}
