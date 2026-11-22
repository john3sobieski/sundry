//! Error type shared by the whole core.

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("unknown tool `{0}`")]
    ToolNotFound(String),
    #[error("tool `{0}` is already registered")]
    DuplicateTool(String),
    #[error("invalid tool definition `{id}`: {reason}")]
    InvalidTool { id: String, reason: String },
    #[error("missing parameter `{0}`")]
    MissingParam(String),
    #[error("invalid parameter `{name}`: {reason}")]
    InvalidParam { name: String, reason: String },
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Processing(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("no configuration directory found")]
    NoConfigDir,
    #[error("failed to read settings: {0}")]
    SettingsParse(#[from] toml::de::Error),
    #[error("failed to write settings: {0}")]
    SettingsWrite(#[from] toml::ser::Error),
    #[error("logging setup failed: {0}")]
    Logging(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
