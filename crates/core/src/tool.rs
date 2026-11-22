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

/// What input a tool reads. The UI shows only the matching input controls and
/// the CLI rejects input the tool would ignore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum InputKind {
    /// The tool works from its parameters alone.
    None,
    /// Typed or piped text.
    Text,
    /// Files only.
    Files,
    /// Text or files.
    #[default]
    Any,
}

impl InputKind {
    /// Stable key for the UI.
    pub fn as_str(self) -> &'static str {
        match self {
            InputKind::None => "none",
            InputKind::Text => "text",
            InputKind::Files => "files",
            InputKind::Any => "any",
        }
    }

    pub fn accepts_text(self) -> bool {
        matches!(self, InputKind::Text | InputKind::Any)
    }

    pub fn accepts_files(self) -> bool {
        matches!(self, InputKind::Files | InputKind::Any)
    }
}

/// What a tool's files are, so a frontend can offer a matching viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Media {
    /// No special viewer.
    #[default]
    None,
    /// Raster images in, raster images out (previews, before/after view).
    Image,
}

impl Media {
    /// Stable key for the UI.
    pub fn as_str(self) -> &'static str {
        match self {
            Media::None => "none",
            Media::Image => "image",
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
    pub input: InputKind,
    pub media: Media,
    /// Every input file is processed on its own with the same parameters, so a
    /// frontend may run files one by one or in parallel (batch mode).
    pub per_file: bool,
}

impl ToolInfo {
    pub fn new(
        id: impl Into<String>,
        category: Category,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            category,
            input: InputKind::Any,
            media: Media::None,
            per_file: false,
        }
    }

    /// Declares what input the tool reads (default: `InputKind::Any`).
    pub fn with_input(mut self, input: InputKind) -> Self {
        self.input = input;
        self
    }

    /// Declares the kind of files the tool works on (default: `Media::None`).
    pub fn with_media(mut self, media: Media) -> Self {
        self.media = media;
        self
    }

    /// Marks the tool as independent per input file (see `ToolInfo::per_file`).
    pub fn per_file(mut self) -> Self {
        self.per_file = true;
        self
    }
}

pub trait Tool: Send + Sync {
    fn info(&self) -> ToolInfo;

    /// Parameter descriptions; a UI builds its form from these.
    fn params(&self) -> Vec<ParamSpec> {
        Vec::new()
    }

    /// `params` are already validated and complete (see `Registry::run`).
    /// Long work should call `job.check_cancelled()` and `job.set_progress()`;
    /// a per-file tool leaves the progress alone when it gets a single item,
    /// because a batch runner reports the progress of the whole batch.
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

    #[test]
    fn input_kinds_say_what_they_accept() {
        assert!(!InputKind::None.accepts_text() && !InputKind::None.accepts_files());
        assert!(InputKind::Text.accepts_text() && !InputKind::Text.accepts_files());
        assert!(!InputKind::Files.accepts_text() && InputKind::Files.accepts_files());
        assert!(InputKind::Any.accepts_text() && InputKind::Any.accepts_files());
        assert_eq!(ToolInfo::new("a", Category::Dev, "A", "").input, InputKind::Any);
        let info = ToolInfo::new("a", Category::Dev, "A", "").with_input(InputKind::None);
        assert_eq!(info.input, InputKind::None);
    }

    #[test]
    fn media_and_batch_flags_default_to_off() {
        let info = ToolInfo::new("a", Category::Image, "A", "");
        assert_eq!(info.media, Media::None);
        assert!(!info.per_file);
        let info = info.with_media(Media::Image).per_file();
        assert_eq!(info.media.as_str(), "image");
        assert!(info.per_file);
    }
}
