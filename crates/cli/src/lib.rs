//! Command-line frontend of Sundry: any registered tool runs without a UI.
//! Exit codes: 0 success, 1 failure, 2 wrong usage.

pub mod args;
pub mod builtin;
mod commands;
pub mod error;
mod runner;

use std::io::{Read, Write};

use sundry_core::Registry;

use crate::args::Command;
use crate::error::CliError;

const HELP: &str = "\
Sundry: offline toolbox for everyday tasks.

Usage:
  sundry list [-c CATEGORY] [QUERY]   List tools, optionally filtered
  sundry info <tool>                  Show a tool's description and parameters
  sundry <tool> [options]             Run a tool
  sundry --help | --version

Run options:
  -i, --input FILE         Input file; `-` reads stdin. Repeatable
  -t, --text TEXT          Input text. Repeatable
  -p, --param NAME=VALUE   Tool parameter; a bare NAME sets a bool to true. Repeatable
  -o, --output PATH        Write the result to a file or directory; `-` is raw stdout
  -f, --force              Overwrite existing output files
  -h, --help               Show the tool's description and parameters

Without -o the result is printed to stdout (text only).
Exit codes: 0 success, 1 failure, 2 wrong usage.
";

/// Streams the CLI works with; tests plug in buffers instead of the real ones.
pub struct Io<'a> {
    pub stdin: &'a mut dyn Read,
    pub stdout: &'a mut dyn Write,
    pub stderr: &'a mut dyn Write,
}

/// Runs the CLI with the arguments after the program name; returns the exit code.
pub fn run(registry: &Registry, args: &[String], io: &mut Io<'_>) -> u8 {
    match execute(registry, args, io) {
        Ok(()) => 0,
        Err(err) => {
            let _ = writeln!(io.stderr, "error: {err}");
            if let Some(hint) = err.hint() {
                let _ = writeln!(io.stderr, "{hint}");
            }
            err.exit_code()
        }
    }
}

fn execute(registry: &Registry, args: &[String], io: &mut Io<'_>) -> Result<(), CliError> {
    match args::parse(args)? {
        Command::Help => write!(io.stdout, "{HELP}")?,
        Command::Version => writeln!(io.stdout, "sundry {}", env!("CARGO_PKG_VERSION"))?,
        Command::List { category, query } => {
            commands::list(registry, category, query.as_deref(), &mut *io.stdout)?;
        }
        Command::Info { tool } => commands::info(registry, &tool, &mut *io.stdout)?,
        Command::Run(run) => runner::run_tool(registry, &run, io)?,
    }
    io.stdout.flush()?;
    Ok(())
}
