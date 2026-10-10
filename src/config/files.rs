//! Where configuration files are looked for.

use std::env;
use std::path::{Path, PathBuf};

/// mto's file names, in the order tried in each folder: the dot file first, as rustfmt does.
const MTO_NAMES: [&str; 2] = [".mto.toml", "mto.toml"];

/// rustfmt's file names, in the order rustfmt tries them in each folder.
const RUSTFMT_NAMES: [&str; 2] = [".rustfmt.toml", "rustfmt.toml"];

/// The parts of the process environment configuration lookup depends on, gathered once so
/// tests can supply their own.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    /// The home folder.
    pub home: Option<PathBuf>,
    /// The operating system's per-user configuration folder, as rustfmt finds it:
    /// `$XDG_CONFIG_HOME` or `~/.config` on Linux, `~/Library/Application Support` on macOS,
    /// `%APPDATA%` on Windows.
    pub os_config_dir: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`, when set to an absolute path.
    pub xdg_config_home: Option<PathBuf>,
    /// `%APPDATA%` (Windows).
    pub app_data: Option<PathBuf>,
    /// `$MTINDO_CONFIG`: the exact user config file.
    pub mtindo_config: Option<PathBuf>,
    /// `$CI` is set: never create files outside the project.
    pub ci: bool,
}

impl Environment {
    pub fn from_process() -> Environment {
        let path = |name: &str| {
            env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = path("HOME").or_else(|| path("USERPROFILE"));
        let xdg_config_home = path("XDG_CONFIG_HOME").filter(|p| p.is_absolute());
        let app_data = path("APPDATA");
        let os_config_dir = if cfg!(windows) {
            app_data.clone()
        } else if cfg!(target_os = "macos") {
            home.as_ref()
                .map(|h| h.join("Library").join("Application Support"))
        } else {
            xdg_config_home
                .clone()
                .or_else(|| home.as_ref().map(|h| h.join(".config")))
        };
        Environment {
            home,
            os_config_dir,
            xdg_config_home,
            app_data,
            mtindo_config: path("MTINDO_CONFIG"),
            ci: env::var_os("CI").is_some(),
        }
    }
}

/// The configuration files one folder holds, found once per folder and remembered.
#[derive(Clone, Debug, Default)]
pub(super) struct Folder {
    pub rustfmt: Option<PathBuf>,
    pub mto: Option<PathBuf>,
    /// Both `.mto.toml` and `mto.toml` exist.
    pub both_mto: bool,
    pub cargo: Option<PathBuf>,
}

impl Folder {
    pub fn scan(folder: &Path) -> Folder {
        let first = |names: &[&str]| -> Vec<PathBuf> {
            names
                .iter()
                .map(|name| folder.join(name))
                .filter(|path| path.is_file())
                .collect()
        };
        let mto = first(&MTO_NAMES);
        let cargo = folder.join("Cargo.toml");
        Folder {
            rustfmt: first(&RUSTFMT_NAMES).into_iter().next(),
            both_mto: mto.len() > 1,
            mto: mto.into_iter().next(),
            cargo: cargo.is_file().then_some(cargo),
        }
    }
}
