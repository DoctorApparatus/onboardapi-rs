use thiserror::Error;

use crate::wire::WireError;

#[derive(Debug, Error)]
pub enum Error {
    /// The C++ library threw while executing `context`.
    #[error("{context}: {message}")]
    Native {
        context: String,
        code: i32,
        message: String,
    },

    #[error(transparent)]
    Wire(#[from] WireError),

    #[error("service name contains a NUL byte: {0:?}")]
    InvalidName(String),

    #[error("background task failed")]
    Join(#[from] tokio::task::JoinError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Builds an error from a non-zero shim status, reading the thread-local message.
    pub(crate) fn native(code: i32, context: impl FnOnce() -> String) -> Self {
        Error::Native {
            context: context(),
            code,
            message: onboardapi_sys::last_error(),
        }
    }
}

/// Turns a shim status code into a `Result`.
pub(crate) fn check(code: i32, context: impl FnOnce() -> String) -> Result<()> {
    if code == onboardapi_sys::OBAPI_OK {
        Ok(())
    } else {
        Err(Error::native(code, context))
    }
}
