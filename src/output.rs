//! Writing command results to standard output.

use std::io::{self, Write};

use crate::error::{Error, Result};

/// Writes `text` to standard output. A closed pipe (`clai models | head -1`)
/// is not an error: the reader got what it wanted.
pub fn write_stdout(text: &str) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(source) => Err(Error::Io {
            context: "standard output".to_owned(),
            source,
        }),
    }
}
