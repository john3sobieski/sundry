//! Command-line parsing. Tool parameters are not interpreted here: that needs
//! the tool's specs, so they stay raw until the command runs.

use sundry_core::Category;

use crate::error::CliError;

/// Words that are CLI commands, so no tool may use them as an id.
pub const RESERVED_IDS: [&str; 3] = ["help", "info", "list"];

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    List { category: Option<Category>, query: Option<String> },
    Info { tool: String },
    Run(RunArgs),
}

#[derive(Debug, PartialEq, Eq)]
pub enum InputSource {
    File(String),
    Text(String),
    Stdin,
}

/// `-p name=value`; a bare `-p name` has no value (shorthand for a bool `true`).
#[derive(Debug, PartialEq, Eq)]
pub struct RawParam {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RunArgs {
    pub tool: String,
    pub inputs: Vec<InputSource>,
    pub params: Vec<RawParam>,
    pub output: Option<String>,
    pub force: bool,
}

fn usage(message: impl Into<String>) -> CliError {
    CliError::Usage(message.into())
}

/// Parses the arguments after the program name.
pub fn parse(args: &[String]) -> Result<Command, CliError> {
    let Some((first, rest)) = args.split_first() else {
        return Ok(Command::Help);
    };
    match first.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "-V" | "--version" => Ok(Command::Version),
        "list" => parse_list(rest),
        "info" => parse_info(rest),
        other if other.starts_with('-') => Err(usage(format!("unknown option `{other}`"))),
        tool => parse_run(tool, rest),
    }
}

fn parse_list(args: &[String]) -> Result<Command, CliError> {
    let mut category = None;
    let mut query: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (name, inline) = split_option(arg);
        match name {
            "-h" | "--help" => return Ok(Command::Help),
            "-c" | "--category" => {
                let value = next_value(name, inline, &mut it)?;
                category = Some(parse_category(value)?);
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown option `{other}` for `list`")));
            }
            _ => {
                if query.is_some() {
                    return Err(usage("`list` takes at most one search query"));
                }
                query = Some(arg.clone());
            }
        }
    }
    Ok(Command::List { category, query })
}

fn parse_info(args: &[String]) -> Result<Command, CliError> {
    match args {
        [tool] if !tool.starts_with('-') => Ok(Command::Info { tool: tool.clone() }),
        [flag] if matches!(flag.as_str(), "-h" | "--help") => Ok(Command::Help),
        _ => Err(usage("usage: sundry info <tool>")),
    }
}

fn parse_run(tool: &str, args: &[String]) -> Result<Command, CliError> {
    let mut run = RunArgs {
        tool: tool.to_owned(),
        inputs: Vec::new(),
        params: Vec::new(),
        output: None,
        force: false,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (name, inline) = split_option(arg);
        match name {
            "-h" | "--help" => {
                no_value(name, inline)?;
                return Ok(Command::Info { tool: run.tool });
            }
            "-f" | "--force" => {
                no_value(name, inline)?;
                run.force = true;
            }
            "-i" | "--input" => {
                let value = next_value(name, inline, &mut it)?;
                run.inputs.push(if value == "-" {
                    InputSource::Stdin
                } else {
                    InputSource::File(value.to_owned())
                });
            }
            "-t" | "--text" => {
                let value = next_value(name, inline, &mut it)?;
                run.inputs.push(InputSource::Text(value.to_owned()));
            }
            "-p" | "--param" => {
                let value = next_value(name, inline, &mut it)?;
                run.params.push(split_param(value)?);
            }
            "-o" | "--output" => {
                let value = next_value(name, inline, &mut it)?;
                if run.output.is_some() {
                    return Err(usage("`--output` given more than once"));
                }
                run.output = Some(value.to_owned());
            }
            other => {
                return Err(usage(format!(
                    "unexpected argument `{other}` (tool parameters are passed as `-p name=value`)"
                )));
            }
        }
    }
    let stdin_uses = run.inputs.iter().filter(|s| matches!(s, InputSource::Stdin)).count();
    if stdin_uses > 1 {
        return Err(usage("stdin (`-i -`) can be used only once"));
    }
    Ok(Command::Run(run))
}

/// Splits `--name=value` into its parts; anything else is returned as is.
fn split_option(arg: &str) -> (&str, Option<&str>) {
    if arg.starts_with("--") {
        if let Some((name, value)) = arg.split_once('=') {
            return (name, Some(value));
        }
    }
    (arg, None)
}

/// The value of an option: inline (`--name=value`) or the next argument.
fn next_value<'a>(
    name: &str,
    inline: Option<&'a str>,
    it: &mut std::slice::Iter<'a, String>,
) -> Result<&'a str, CliError> {
    match inline {
        Some(value) => Ok(value),
        None => it
            .next()
            .map(String::as_str)
            .ok_or_else(|| usage(format!("option `{name}` needs a value"))),
    }
}

fn no_value(name: &str, inline: Option<&str>) -> Result<(), CliError> {
    match inline {
        Some(_) => Err(usage(format!("option `{name}` does not take a value"))),
        None => Ok(()),
    }
}

fn split_param(raw: &str) -> Result<RawParam, CliError> {
    let (name, value) = match raw.split_once('=') {
        Some((name, value)) => (name, Some(value.to_owned())),
        None => (raw, None),
    };
    if name.is_empty() {
        return Err(usage(format!("invalid parameter `{raw}`: empty name")));
    }
    Ok(RawParam { name: name.to_owned(), value })
}

fn parse_category(value: &str) -> Result<Category, CliError> {
    let wanted = value.to_lowercase();
    Category::ALL.into_iter().find(|c| c.as_str() == wanted).ok_or_else(|| {
        let keys: Vec<&str> = Category::ALL.iter().map(|c| c.as_str()).collect();
        usage(format!("unknown category `{value}` (expected one of: {})", keys.join(", ")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    fn run_args(list: &[&str]) -> RunArgs {
        match parse(&args(list)).unwrap() {
            Command::Run(run) => run,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse(&[]).unwrap(), Command::Help);
        for flag in ["-h", "--help", "help"] {
            assert_eq!(parse(&args(&[flag])).unwrap(), Command::Help);
        }
        assert_eq!(parse(&args(&["--version"])).unwrap(), Command::Version);
        assert_eq!(parse(&args(&["-V"])).unwrap(), Command::Version);
        assert!(matches!(parse(&args(&["--nope"])), Err(CliError::Usage(_))));
    }

    #[test]
    fn list_options() {
        let got = parse(&args(&["list", "-c", "Text", "rep"])).unwrap();
        let want = Command::List { category: Some(Category::Text), query: Some("rep".to_owned()) };
        assert_eq!(got, want);
        let got = parse(&args(&["list", "--category=image"])).unwrap();
        assert_eq!(got, Command::List { category: Some(Category::Image), query: None });
        assert!(parse(&args(&["list", "-c", "bogus"])).is_err());
        assert!(parse(&args(&["list", "-c"])).is_err());
        assert!(parse(&args(&["list", "a", "b"])).is_err());
        assert!(parse(&args(&["list", "--wat"])).is_err());
    }

    #[test]
    fn info_needs_exactly_one_tool() {
        let got = parse(&args(&["info", "text.repeat"])).unwrap();
        assert_eq!(got, Command::Info { tool: "text.repeat".to_owned() });
        assert!(parse(&args(&["info"])).is_err());
        assert!(parse(&args(&["info", "a", "b"])).is_err());
    }

    #[test]
    fn run_collects_inputs_in_order() {
        let run = run_args(&["t", "-t", "one", "-i", "a.txt", "--input=-", "--text=two"]);
        assert_eq!(run.tool, "t");
        assert_eq!(
            run.inputs,
            [
                InputSource::Text("one".to_owned()),
                InputSource::File("a.txt".to_owned()),
                InputSource::Stdin,
                InputSource::Text("two".to_owned()),
            ]
        );
    }

    #[test]
    fn run_params_output_and_force() {
        let run = run_args(&["t", "-p", "n=3", "--param=label=a=b", "-p", "flag", "-o", "out", "-f"]);
        assert_eq!(
            run.params,
            [
                RawParam { name: "n".to_owned(), value: Some("3".to_owned()) },
                RawParam { name: "label".to_owned(), value: Some("a=b".to_owned()) },
                RawParam { name: "flag".to_owned(), value: None },
            ]
        );
        assert_eq!(run.output.as_deref(), Some("out"));
        assert!(run.force);
    }

    #[test]
    fn option_values_may_start_with_a_dash() {
        let run = run_args(&["t", "-t", "-5"]);
        assert_eq!(run.inputs, [InputSource::Text("-5".to_owned())]);
    }

    #[test]
    fn run_rejects_mistakes() {
        for bad in [
            &["t", "-p", "=x"][..],
            &["t", "-p"],
            &["t", "stray"],
            &["t", "--unknown"],
            &["t", "-i", "-", "-i", "-"],
            &["t", "-o", "a", "-o", "b"],
            &["t", "--force=yes"],
        ] {
            assert!(matches!(parse(&args(bad)), Err(CliError::Usage(_))), "accepted: {bad:?}");
        }
    }

    #[test]
    fn tool_help_flag_shows_info() {
        let got = parse(&args(&["text.repeat", "--help"])).unwrap();
        assert_eq!(got, Command::Info { tool: "text.repeat".to_owned() });
    }
}
