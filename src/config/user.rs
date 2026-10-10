//! The per-user configuration file, `config.toml`: tool behavior only, never layout or
//! formatting.
//!
//! It is `$MTINDO_CONFIG` when set (the file must exist), else
//! `$XDG_CONFIG_HOME/mtindo/config.toml`, else `~/.config/mtindo/config.toml` on macOS and
//! Linux and `%APPDATA%\mtindo\config.toml` on Windows.
//!
//! [`create_if_missing`] writes it with every setting commented out at its default, so a
//! later change of default still applies. It never overwrites a file, does nothing when `CI`
//! is set, and does nothing while the settings table has no user settings.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use crate::config::{Environment, Error};
use crate::options::{self, File};

/// Creates the user config if it is missing. `Ok(Some(path))` when a file was written.
pub fn create_if_missing(env: &Environment) -> Result<Option<PathBuf>, String> {
    if env.ci || env.mtindo_config.is_some() || !options::all().any(|s| s.file == File::User) {
        return Ok(None);
    }
    let Some(path) = path(env).map_err(|e| e.message)? else {
        return Ok(None);
    };
    if path.exists() {
        return Ok(None);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
    }
    // create_new: two runs racing to create it cannot clobber each other.
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(None),
        Err(e) => return Err(format!("cannot create {}: {}", path.display(), e)),
    };
    file.write_all(template().as_bytes())
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    Ok(Some(path))
}

/// The user config file, if the environment names a place for one.
pub fn path(env: &Environment) -> Result<Option<PathBuf>, Error> {
    if let Some(path) = &env.mtindo_config {
        if !path.is_file() {
            return Err(Error::new(format!(
                "MTINDO_CONFIG names {}, which is not a file",
                path.display()
            )));
        }
        return Ok(Some(path.clone()));
    }
    let base = if let Some(xdg) = &env.xdg_config_home {
        Some(xdg.clone())
    } else if cfg!(windows) {
        env.app_data.clone()
    } else {
        env.home.as_ref().map(|home| home.join(".config"))
    };
    Ok(base.map(|base| base.join("mtindo").join("config.toml")))
}

/// The contents of a new user config: every user setting, commented out at its default,
/// under its section.
pub fn template() -> String {
    let mut out = String::from(
        "# mto's user configuration: how mto behaves for you, on every project.\n\
         # Layout and formatting settings belong in a project's mto.toml instead.\n\
         # Every setting is shown commented out at its default; `mto --man` explains them.\n",
    );
    let mut sections: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for setting in options::all().filter(|s| s.file == File::User) {
        let (section, key) = setting.name.split_once('.').unwrap_or(("", setting.name));
        sections.entry(section).or_default().push(format!(
            "# {}\n# {} = {}\n",
            setting.summary, key, setting.default
        ));
    }
    for (section, lines) in sections {
        out.push('\n');
        if !section.is_empty() {
            out.push_str(&format!("[{}]\n", section));
        }
        out.push_str(&lines.join("\n"));
    }
    out
}
