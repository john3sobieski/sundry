//! `list` and `info`: browsing the registry.

use std::fmt::Display;
use std::io::Write;

use sundry_core::{Category, ParamKind, ParamSpec, Registry, ToolEntry};

use crate::error::CliError;

pub fn list(
    registry: &Registry,
    category: Option<Category>,
    query: Option<&str>,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let found: Vec<&ToolEntry> = registry
        .search(query.unwrap_or(""))
        .into_iter()
        .filter(|e| category.is_none_or(|c| e.info().category == c))
        .collect();
    if found.is_empty() {
        let message =
            if registry.is_empty() { "No tools are built in yet." } else { "No tools match." };
        writeln!(out, "{message}")?;
        return Ok(());
    }
    let rows: Vec<(&str, &str, &str)> = found
        .iter()
        .map(|e| {
            let info = e.info();
            (info.id.as_str(), info.category.as_str(), info.name.as_str())
        })
        .collect();
    let w0 = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0).max("ID".len());
    let w1 = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0).max("CATEGORY".len());
    writeln!(out, "{:<w0$}  {:<w1$}  NAME", "ID", "CATEGORY")?;
    for (id, cat, name) in rows {
        writeln!(out, "{id:<w0$}  {cat:<w1$}  {name}")?;
    }
    Ok(())
}

pub fn info(registry: &Registry, tool: &str, out: &mut dyn Write) -> Result<(), CliError> {
    let entry = registry.get(tool)?;
    let info = entry.info();
    writeln!(out, "{} [{}]", info.id, info.category.as_str())?;
    writeln!(out, "{}", info.name)?;
    if !info.description.is_empty() {
        writeln!(out, "{}", info.description)?;
    }
    writeln!(out)?;
    writeln!(out, "Usage: sundry {} [-i FILE | -t TEXT]... [-p NAME=VALUE]... [-o PATH]", info.id)?;
    writeln!(out)?;
    let params = entry.params();
    if params.is_empty() {
        writeln!(out, "No parameters.")?;
        return Ok(());
    }
    writeln!(out, "Parameters:")?;
    let width = params.iter().map(|p| p.id.chars().count()).max().unwrap_or(0);
    for spec in params {
        let id = spec.id.as_str();
        let text = describe(spec);
        writeln!(out, "  {id:<width$}  {text}")?;
    }
    Ok(())
}

fn describe(spec: &ParamSpec) -> String {
    match &spec.kind {
        ParamKind::Bool { default } => format!("bool, default {default}"),
        ParamKind::Int { default, min, max } => {
            format!("int{}, default {default}", range(*min, *max))
        }
        ParamKind::Float { default, min, max } => {
            format!("float{}, default {default}", range(*min, *max))
        }
        ParamKind::Text { default } => format!("text, default {default:?}"),
        ParamKind::Choice { options, default } => {
            format!("one of {}, default {default}", options.join("|"))
        }
    }
}

fn range<T: Display>(min: Option<T>, max: Option<T>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!(", {lo}..{hi}"),
        (Some(lo), None) => format!(", >= {lo}"),
        (None, Some(hi)) => format!(", <= {hi}"),
        (None, None) => String::new(),
    }
}
