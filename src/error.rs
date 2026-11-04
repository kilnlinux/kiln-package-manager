//! One error type for the whole CLI. Every error is a message for a human;
//! the variant only decides the exit code.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// Bad command line. Exit code 2.
    Usage(String),
    /// Anything else. Exit code 1.
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) => 2,
            Error::Failed(_) => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) | Error::Failed(m) => f.write_str(m),
        }
    }
}

/// Shorthand for `Err(Error::Failed(..))` built from a format string.
#[macro_export]
macro_rules! fail {
    ($($arg:tt)*) => { Err($crate::error::Error::Failed(format!($($arg)*))) };
}

/// Attach context to an I/O error: `fs::read(p).map_err(io_err("reading", p))`.
pub fn io_err(action: &str, path: &std::path::Path) -> impl FnOnce(std::io::Error) -> Error {
    let msg = format!("{action} {}", path.display());
    move |e| Error::Failed(format!("{msg}: {e}"))
}
