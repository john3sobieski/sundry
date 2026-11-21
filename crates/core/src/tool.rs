//! The `Tool` trait: the single extension point of the app.

use crate::data::{Input, Output};
use crate::error::Result;
use crate::job::Job;
use crate::params::{ParamSpec, Params};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Category {
    Image,
    Vector,
    Convert,
    Text,
    Files,
    Dev,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Image,
        Category::Vector,
        Category::Convert,
        Category::Text,
        Category::Files,
        Category::Dev,
    ];

    /// Stable key, also used to build localization keys.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Image => "image",
            Category::Vector => "vector",
            Category::Convert => "convert",
            Category::Text => "text",
            Category::Files => "files",
            Category::Dev => "dev",
        }
    }
}

/// Static description of a tool. `name` and `description` are English
/// fallbacks; the UI localizes by `id` (keys like `tool.<id>.name`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Category,
}

impl ToolInfo {
    pub fn new(
        id: impl Into<String>,
        category: Category,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self { id: id.into(), name: name.into(), description: description.into(), category }
    }
}

pub trait Tool: Send + Sync {
    fn info(&self) -> ToolInfo;

    /// Parameter descriptions; a UI builds its form from these.
    fn params(&self) -> Vec<ParamSpec> {
        Vec::new()
    }

    /// `params` are already validated and complete (see `Registry::run`).
    /// Long work should call `job.check_cancelled()` and `job.set_progress()`.
    fn run(&self, input: &Input, params: &Params, job: &Job) -> Result<Output>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_keys_are_unique() {
        let mut keys: Vec<_> = Category::ALL.iter().map(|c| c.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Category::ALL.len());
    }
}
