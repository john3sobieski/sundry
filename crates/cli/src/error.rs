//! CLI errors and their exit codes.

use sundry_core::Error as CoreError;

pub const EXIT_FAILURE: u8 = 1;
pub const EXIT_USAGE: u8 = 2;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("{path}: {source}")]
    File { path: String, source: std::io::Error },
    #[error("{0}")]
    Output(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl CliError {
    /// 2 for mistakes in the command line, 1 for everything else.
    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::Usage(_) => EXIT_USAGE,
            CliError::Core(
                CoreError::ToolNotFound(_)
                | CoreError::MissingParam(_)
                | CoreError::InvalidParam { .. },
            ) => EXIT_USAGE,
            _ => EXIT_FAILURE,
        }
    }

    /// A short follow-up line telling the user where to look next.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            CliError::Usage(_) => Some("Try `sundry --help`."),
            CliError::Core(CoreError::ToolNotFound(_)) => {
                Some("Run `sundry list` to see available tools.")
            }
            CliError::Core(CoreError::InvalidParam { .. } | CoreError::MissingParam(_)) => {
                Some("Run `sundry info <tool>` to see its parameters.")
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes() {
        assert_eq!(CliError::Usage("x".into()).exit_code(), EXIT_USAGE);
        assert_eq!(CliError::Core(CoreError::ToolNotFound("x".into())).exit_code(), EXIT_USAGE);
        assert_eq!(CliError::Core(CoreError::Cancelled).exit_code(), EXIT_FAILURE);
        assert_eq!(CliError::Output("x".into()).exit_code(), EXIT_FAILURE);
    }
}
