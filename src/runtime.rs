//! The async runtimes of the commands.

use tokio::runtime::{Builder, Runtime};

use crate::error::{Error, Result};

/// For the server and `clai run`: sessions, tools and signals at once.
pub fn multi_thread() -> Result<Runtime> {
    Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Error::io("async runtime"))
}

/// For a client command: one connection, little work.
pub fn current_thread() -> Result<Runtime> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Error::io("async runtime"))
}
