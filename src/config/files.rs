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

/// The `mto.toml` in `folder`, if any. When both names exist, `.mto.toml` is used, as
/// rustfmt prefers `.rustfmt.toml`, and `warn` is told.
pub(super) fn mto_file(folder: &Path, warn: &mut dyn FnMut(String)) -> Option<PathBuf> {
    let found: Vec<PathBuf> = MTO_NAMES
        .iter()
        .map(|name| folder.join(name))
        .filter(|path| path.is_file())
        .collect();
    if found.len() > 1 {
        warn(format!(
            "both {} and {} exist; using {}",
            found[0].display(),
            found[1].display(),
            found[0].display()
        ));
    }
    found.into_iter().next()
}

/// The `rustfmt.toml` rustfmt would use for files in `dir`: the nearest one in `dir` or a
/// parent, else one in the home folder, else one in the `rustfmt` configuration folder.
pub(super) fn rustfmt_file(dir: &Path, env: &Environment) -> Option<PathBuf> {
    let in_folder = |folder: &Path| {
        RUSTFMT_NAMES
            .iter()
            .map(|name| folder.join(name))
            .find(|path| path.is_file())
    };
    dir.ancestors()
        .find_map(in_folder)
        .or_else(|| env.home.as_deref().and_then(in_folder))
        .or_else(|| {
            env.os_config_dir
                .as_ref()
                .and_then(|d| in_folder(&d.join("rustfmt")))
        })
}
