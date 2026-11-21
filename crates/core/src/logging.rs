//! Logging setup (stderr for now; a log file comes with the UI).

use crate::error::{Error, Result};
use crate::settings::LogLevel;

/// Installs the global subscriber. Fails if one is already installed.
pub fn init(level: LogLevel) -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(level.to_tracing())
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|e| Error::Logging(e.to_string()))
}
