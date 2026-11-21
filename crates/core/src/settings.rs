//! User settings stored as a TOML file; portable mode via a marker file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const FILE_NAME: &str = "settings.toml";
/// If this file lies next to the executable, settings are stored there too.
pub const PORTABLE_MARKER: &str = "sundry.portable";
const APP_DIR: &str = "Sundry";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Auto,
    En,
    Ru,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn to_tracing(self) -> tracing::Level {
        match self {
            LogLevel::Error => tracing::Level::ERROR,
            LogLevel::Warn => tracing::Level::WARN,
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Trace => tracing::Level::TRACE,
        }
    }
}

/// Missing fields fall back to defaults and unknown fields are ignored,
/// so files survive upgrades and downgrades.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    pub theme: Theme,
    pub log_level: LogLevel,
}

impl Settings {
    pub fn from_toml(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Loads settings; a missing file yields defaults, a broken one an error.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes via a temp file and rename, creating parent directories.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Where the settings file lives: next to the exe in portable mode,
/// otherwise in the user's config directory.
pub fn default_path() -> Result<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    locate(exe_dir.as_deref(), |key| std::env::var_os(key))
}

fn locate(exe_dir: Option<&Path>, env: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf> {
    if let Some(dir) = exe_dir {
        if dir.join(PORTABLE_MARKER).is_file() {
            return Ok(dir.join(FILE_NAME));
        }
    }
    let base = if cfg!(windows) {
        env("APPDATA").map(PathBuf::from)
    } else {
        env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    base.map(|b| b.join(APP_DIR).join(FILE_NAME)).ok_or(Error::NoConfigDir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("sundry-test-{}-{n}", std::process::id());
            let dir = std::env::temp_dir().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn toml_roundtrip() {
        let s = Settings { language: Language::Ru, theme: Theme::Dark, log_level: LogLevel::Debug };
        let text = s.to_toml().unwrap();
        assert_eq!(Settings::from_toml(&text).unwrap(), s);
    }

    #[test]
    fn partial_and_unknown_fields() {
        let s = Settings::from_toml("language = \"en\"\nfuture_option = 1\n").unwrap();
        assert_eq!(s.language, Language::En);
        assert_eq!(s.theme, Theme::Auto);
        assert_eq!(s.log_level, LogLevel::Info);
        assert_eq!(Settings::from_toml("").unwrap(), Settings::default());
    }

    #[test]
    fn invalid_value_is_an_error() {
        let res = Settings::from_toml("theme = \"purple\"");
        assert!(matches!(res, Err(Error::SettingsParse(_))));
    }

    #[test]
    fn missing_file_gives_defaults() {
        let tmp = TempDir::new();
        let s = Settings::load(&tmp.0.join("nope.toml")).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn save_creates_dirs_and_loads_back() {
        let tmp = TempDir::new();
        let path = tmp.0.join("a").join("b").join(FILE_NAME);
        let s = Settings { theme: Theme::Light, ..Settings::default() };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let tmp = TempDir::new();
        let path = tmp.0.join(FILE_NAME);
        std::fs::write(&path, "this is = not [valid").unwrap();
        assert!(matches!(Settings::load(&path), Err(Error::SettingsParse(_))));
    }

    #[test]
    fn portable_marker_wins() {
        let tmp = TempDir::new();
        std::fs::write(tmp.0.join(PORTABLE_MARKER), "").unwrap();
        let path = locate(Some(&tmp.0), |_| None).unwrap();
        assert_eq!(path, tmp.0.join(FILE_NAME));
    }

    #[test]
    fn falls_back_to_config_dir() {
        let tmp = TempDir::new();
        let wanted = if cfg!(windows) { "APPDATA" } else { "XDG_CONFIG_HOME" };
        let env = |key: &str| (key == wanted).then(|| OsString::from("cfg"));
        let path = locate(Some(&tmp.0), env).unwrap();
        assert_eq!(path, PathBuf::from("cfg").join(APP_DIR).join(FILE_NAME));
    }

    #[cfg(not(windows))]
    #[test]
    fn falls_back_to_home_config() {
        let env = |key: &str| (key == "HOME").then(|| OsString::from("home"));
        let path = locate(None, env).unwrap();
        assert_eq!(path, PathBuf::from("home").join(".config").join(APP_DIR).join(FILE_NAME));
    }

    #[test]
    fn no_location_is_an_error() {
        assert!(matches!(locate(None, |_| None), Err(Error::NoConfigDir)));
    }
}
