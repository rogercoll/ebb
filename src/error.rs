use std::{io, path::PathBuf};

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
