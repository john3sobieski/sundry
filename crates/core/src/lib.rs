//! Core of Sundry: the `Tool` trait, registry, parameters, jobs and settings.
//! No UI and no file-path handling in tools: they work on in-memory data.

pub mod data;
pub mod error;
pub mod job;
pub mod logging;
pub mod params;
pub mod registry;
pub mod settings;
pub mod tool;

pub use data::{Data, Input, Item, Output};
pub use error::{Error, Result};
pub use job::Job;
pub use params::{ParamKind, ParamSpec, Params, Value};
pub use registry::{Registry, ToolEntry};
pub use settings::{Language, LogLevel, Settings, Theme};
pub use tool::{Category, Tool, ToolInfo};
