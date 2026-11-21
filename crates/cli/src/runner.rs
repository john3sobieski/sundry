//! Running a tool: parameters from the command line, input in, output out.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sundry_core::{
    Error, Input, InputKind, Item, Job, Output, ParamKind, ParamSpec, Params, Registry, Value,
};

use crate::Io;
use crate::args::{InputSource, RawParam, RunArgs};
use crate::error::CliError;

pub fn run_tool(registry: &Registry, args: &RunArgs, io: &mut Io<'_>) -> Result<(), CliError> {
    let entry = registry.get(&args.tool)?;
    let params = build_params(entry.params(), &args.params)?;
    check_input_kind(&entry.info().id, entry.info().input, &args.inputs)?;
    let input = read_input(&args.inputs, &mut *io.stdin)?;
    let output = registry.run(&args.tool, &input, &params, &Job::new())?;
    emit(&output, args.output.as_deref(), args.force, &mut *io.stdout)
}

/// Rejects input that the tool would ignore.
fn check_input_kind(id: &str, kind: InputKind, sources: &[InputSource]) -> Result<(), CliError> {
    let has_text = sources.iter().any(|s| matches!(s, InputSource::Text(_)));
    let has_files = sources.iter().any(|s| matches!(s, InputSource::File(_)));
    let problem = match kind {
        _ if sources.is_empty() => return Ok(()),
        InputKind::None => "takes no input",
        InputKind::Text if has_files => "takes text input only (-t or -i -)",
        InputKind::Files if has_text => "takes files only (-i)",
        _ => return Ok(()),
    };
    Err(CliError::Usage(format!("tool `{id}` {problem}")))
}

fn build_params(specs: &[ParamSpec], raw: &[RawParam]) -> Result<Params, CliError> {
    let mut params = Params::new();
    for given in raw {
        let spec = specs.iter().find(|s| s.id == given.name).ok_or_else(|| {
            CliError::Core(Error::InvalidParam {
                name: given.name.clone(),
                reason: "unknown parameter".to_owned(),
            })
        })?;
        if params.get(&spec.id).is_some() {
            return Err(CliError::Usage(format!("parameter `{}` given more than once", spec.id)));
        }
        params.set(spec.id.clone(), parse_value(spec, given.value.as_deref())?);
    }
    Ok(params)
}

/// Turns command-line text into a typed value. Ranges and choices are
/// checked later by the registry, not here.
fn parse_value(spec: &ParamSpec, raw: Option<&str>) -> Result<Value, CliError> {
    let bad =
        |reason: String| CliError::Core(Error::InvalidParam { name: spec.id.clone(), reason });
    match (&spec.kind, raw) {
        (ParamKind::Bool { .. }, None) => Ok(Value::Bool(true)),
        (_, None) => Err(bad("expected name=value".to_owned())),
        (ParamKind::Bool { .. }, Some(text)) => parse_bool(text)
            .map(Value::Bool)
            .ok_or_else(|| bad(format!("expected true or false, got `{text}`"))),
        (ParamKind::Int { .. }, Some(text)) => text
            .trim()
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| bad(format!("expected an integer, got `{text}`"))),
        (ParamKind::Float { .. }, Some(text)) => text
            .trim()
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| bad(format!("expected a number, got `{text}`"))),
        (ParamKind::Text { .. } | ParamKind::Choice { .. }, Some(text)) => {
            Ok(Value::Text(text.to_owned()))
        }
    }
}

fn parse_bool(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn read_input(sources: &[InputSource], stdin: &mut dyn Read) -> Result<Input, CliError> {
    let mut items = Vec::with_capacity(sources.len());
    for source in sources {
        let item = match source {
            InputSource::Text(text) => Item::text(text.clone()),
            InputSource::Stdin => {
                let mut buf = Vec::new();
                stdin.read_to_end(&mut buf)?;
                Item::bytes(buf)
            }
            InputSource::File(path) => {
                let bytes = std::fs::read(path)
                    .map_err(|source| CliError::File { path: path.clone(), source })?;
                let item = Item::bytes(bytes);
                match Path::new(path).file_name().and_then(|n| n.to_str()) {
                    Some(name) => item.named(name),
                    None => item,
                }
            }
        };
        items.push(item);
    }
    Ok(Input { items })
}

fn emit(
    output: &Output,
    dest: Option<&str>,
    force: bool,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    match dest {
        None => print_text(output, stdout),
        Some("-") => {
            for item in &output.items {
                stdout.write_all(item.data.as_bytes())?;
            }
            Ok(())
        }
        Some(path) => write_files(output, Path::new(path), force),
    }
}

/// Default output: text to stdout. Binary results are refused so that
/// they never end up as garbage in a terminal.
fn print_text(output: &Output, stdout: &mut dyn Write) -> Result<(), CliError> {
    let mut texts = Vec::with_capacity(output.items.len());
    for item in &output.items {
        let text = item.data.as_text().map_err(|_| {
            CliError::Output(
                "the result is binary data; use `-o FILE` (or `-o -` for raw stdout)".to_owned(),
            )
        })?;
        texts.push(text);
    }
    for text in texts.into_iter().filter(|t| !t.is_empty()) {
        stdout.write_all(text.as_bytes())?;
        if !text.ends_with('\n') {
            stdout.write_all(b"\n")?;
        }
    }
    Ok(())
}

/// One result goes to `dest` as a file (or into it, if `dest` is an existing
/// directory); several results always go into a directory, created if needed.
fn write_files(output: &Output, dest: &Path, force: bool) -> Result<(), CliError> {
    let count = output.items.len();
    if count == 0 {
        return Ok(());
    }
    let dest_is_dir = dest.is_dir();
    let targets: Vec<PathBuf> = if count == 1 && !dest_is_dir {
        vec![dest.to_path_buf()]
    } else {
        if dest.exists() && !dest_is_dir {
            return Err(CliError::Output(format!(
                "{} is not a directory, but the tool returned {count} files",
                dest.display()
            )));
        }
        if !dest_is_dir {
            std::fs::create_dir_all(dest).map_err(|source| file_error(dest, source))?;
        }
        output
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| dest.join(safe_name(item.name.as_deref(), i)))
            .collect()
    };
    for (i, target) in targets.iter().enumerate() {
        if targets[..i].contains(target) {
            return Err(CliError::Output(format!(
                "two results have the same file name: {}",
                target.display()
            )));
        }
        if !force && target.exists() {
            return Err(exists_error(target));
        }
    }
    for (target, item) in targets.iter().zip(&output.items) {
        write_file(target, item.data.as_bytes(), force)?;
    }
    Ok(())
}

/// A name suggested by a tool is untrusted: only its last component is
/// used, so it cannot point outside the output directory.
fn safe_name(name: Option<&str>, index: usize) -> String {
    name.and_then(|n| Path::new(n).file_name())
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .map_or_else(|| format!("output-{}", index + 1), str::to_owned)
}

fn write_file(path: &Path, bytes: &[u8], force: bool) -> Result<(), CliError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::AlreadyExists {
            exists_error(path)
        } else {
            file_error(path, source)
        }
    })?;
    file.write_all(bytes).map_err(|source| file_error(path, source))
}

fn file_error(path: &Path, source: std::io::Error) -> CliError {
    CliError::File { path: path.display().to_string(), source }
}

fn exists_error(path: &Path) -> CliError {
    CliError::Output(format!("{} already exists (use --force to overwrite)", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_keeps_only_the_last_component() {
        assert_eq!(safe_name(Some("a.png"), 0), "a.png");
        assert_eq!(safe_name(Some("../../evil.txt"), 0), "evil.txt");
        assert_eq!(safe_name(Some("dir/b.txt"), 0), "b.txt");
        assert_eq!(safe_name(Some(".."), 2), "output-3");
        assert_eq!(safe_name(Some(""), 0), "output-1");
        assert_eq!(safe_name(None, 1), "output-2");
    }

    #[test]
    fn bool_words() {
        assert_eq!(parse_bool("Yes"), Some(true));
        assert_eq!(parse_bool(" 0 "), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }
}
