//! Registry of tools: registration, lookup, search and validated execution.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::data::{Input, Output};
use crate::error::{Error, Result};
use crate::job::Job;
use crate::params::{ParamSpec, Params, resolve};
use crate::tool::{Category, Tool, ToolInfo};

pub struct ToolEntry {
    info: ToolInfo,
    params: Vec<ParamSpec>,
    tool: Arc<dyn Tool>,
}

impl ToolEntry {
    pub fn info(&self) -> &ToolInfo {
        &self.info
    }

    pub fn params(&self) -> &[ParamSpec] {
        &self.params
    }

    /// Shared handle, e.g. to move into a background thread.
    pub fn tool(&self) -> Arc<dyn Tool> {
        Arc::clone(&self.tool)
    }
}

#[derive(Default)]
pub struct Registry {
    tools: BTreeMap<String, ToolEntry>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a tool after checking its id and parameter specs.
    pub fn register(&mut self, tool: impl Tool + 'static) -> Result<()> {
        let info = tool.info();
        let params = tool.params();
        validate_definition(&info, &params)?;
        if self.tools.contains_key(&info.id) {
            return Err(Error::DuplicateTool(info.id));
        }
        tracing::debug!(tool = info.id.as_str(), "registered tool");
        let id = info.id.clone();
        let entry = ToolEntry { info, params, tool: Arc::new(tool) };
        self.tools.insert(id, entry);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<&ToolEntry> {
        self.tools.get(id).ok_or_else(|| Error::ToolNotFound(id.to_owned()))
    }

    /// All tools, ordered by id.
    pub fn iter(&self) -> impl Iterator<Item = &ToolEntry> {
        self.tools.values()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn by_category(&self, category: Category) -> Vec<&ToolEntry> {
        self.iter().filter(|e| e.info.category == category).collect()
    }

    /// Case-insensitive substring search over id, name and description.
    pub fn search(&self, query: &str) -> Vec<&ToolEntry> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.iter().collect();
        }
        self.iter()
            .filter(|e| {
                let i = &e.info;
                i.id.to_lowercase().contains(&q)
                    || i.name.to_lowercase().contains(&q)
                    || i.description.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Validates `params` against the tool's specs (defaults are filled in),
    /// then runs the tool. A job cancelled before the start never runs it.
    pub fn run(&self, id: &str, input: &Input, params: &Params, job: &Job) -> Result<Output> {
        let entry = self.get(id)?;
        let resolved = resolve(&entry.params, params)?;
        job.check_cancelled()?;
        tracing::debug!(tool = id, "running tool");
        let output = entry.tool.run(input, &resolved, job)?;
        job.set_progress(1.0);
        Ok(output)
    }
}

fn is_id_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '.' | '-' | '_')
}

fn validate_definition(info: &ToolInfo, params: &[ParamSpec]) -> Result<()> {
    let invalid = |reason: String| Error::InvalidTool { id: info.id.clone(), reason };
    let id_ok = !info.id.is_empty() && info.id.chars().all(is_id_char);
    if !id_ok {
        return Err(invalid("id must use only a-z, 0-9, '.', '-', '_'".to_owned()));
    }
    for (i, spec) in params.iter().enumerate() {
        if params[..i].iter().any(|p| p.id == spec.id) {
            return Err(invalid(format!("duplicate parameter `{}`", spec.id)));
        }
        spec.validate().map_err(|e| invalid(e.to_string()))?;
    }
    Ok(())
}
