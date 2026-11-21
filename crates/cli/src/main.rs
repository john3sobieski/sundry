use std::ffi::OsString;
use std::process::ExitCode;

use sundry_cli::{Io, builtin, error::EXIT_FAILURE, error::EXIT_USAGE};
use sundry_core::{Settings, logging, settings};

fn main() -> ExitCode {
    let args: Result<Vec<String>, OsString> =
        std::env::args_os().skip(1).map(OsString::into_string).collect();
    let Ok(args) = args else {
        eprintln!("error: arguments must be valid UTF-8");
        return ExitCode::from(EXIT_USAGE);
    };

    let config = load_settings();
    if let Err(err) = logging::init(config.log_level) {
        eprintln!("warning: {err}");
    }

    let registry = match builtin::registry() {
        Ok(registry) => registry,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };

    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let mut io = Io { stdin: &mut stdin, stdout: &mut stdout, stderr: &mut stderr };
    ExitCode::from(sundry_cli::run(&registry, &args, &mut io))
}

/// A missing or unreadable settings file must not stop the CLI: warn and use defaults.
fn load_settings() -> Settings {
    let Ok(path) = settings::default_path() else {
        return Settings::default();
    };
    match Settings::load(&path) {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("warning: ignoring {}: {err}", path.display());
            Settings::default()
        }
    }
}
