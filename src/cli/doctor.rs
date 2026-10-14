//! `mto doctor`: the build guard's settings, its holds, and narrowing down a run already
//! made.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{Environment, Resolver, user};
use crate::doctor::check::Check;
use crate::doctor::guard::Guard;
use crate::doctor::holds::Holds;
use crate::doctor::report;
use crate::history::History;

/// The command the guard runs when none is given.
pub const DEFAULT_COMMAND: [&str; 5] = [
  "cargo",
  "check",
  "--workspace",
  "--all-targets",
  "--keep-going",
];

/// The guard's settings, from the user config.
#[derive(Clone, Debug)]
pub struct Settings {
  pub enabled: bool,
  pub command: Vec<String>,
  /// `None`: no limit.
  pub timeout: Option<Duration>,
  pub max_checks: usize,
  /// The doctor's folder, next to history; `None` without a home folder.
  pub dir: Option<PathBuf>,
}

impl Settings {
  pub fn read(resolver: &Resolver) -> Settings {
    let command: Vec<String> = resolver
      .user_value("doctor.command")
      .and_then(|v| {
        v.as_list()
          .map(|l| l.iter().map(|s| s.to_string()).collect())
      })
      .filter(|words: &Vec<String>| !words.is_empty())
      .unwrap_or_else(|| DEFAULT_COMMAND.iter().map(|w| w.to_string()).collect());
    let number = |name: &str, default: u64| {
      resolver
        .user_value(name)
        .and_then(|v| v.as_integer())
        .unwrap_or(default)
    };
    let timeout = number("doctor.timeout", 600);
    Settings {
      enabled: resolver
        .user_value("doctor.enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false),
      command,
      timeout: (timeout > 0).then(|| Duration::from_secs(timeout)),
      max_checks: number("doctor.max_checks", 64) as usize,
      dir: dir(resolver.env()),
    }
  }

  /// The guard for a run in `here`, checking with `command` (the setting's when `None`).
  pub fn guard(&self, here: &Path, command: Option<Vec<String>>, quiet: bool) -> Guard {
    Guard {
      check: Check {
        words: command.unwrap_or_else(|| self.command.clone()),
        dir: crate::cli::history::project_root(here),
        timeout: self.timeout,
      },
      max_checks: self.max_checks,
      dir: self.dir.clone().unwrap_or_default(),
      quiet,
    }
  }

  pub fn holds(&self) -> Holds {
    self.dir.as_deref().map(Holds::load).unwrap_or_default()
  }
}

/// The doctor's folder: `doctor` next to the history folder.
pub fn dir(env: &Environment) -> Option<PathBuf> {
  user::history_dir(env).and_then(|h| beside(&h))
}

/// `mto doctor holds`: each held edit, numbered for `release`.
pub fn list(settings: &Settings) -> String {
  let holds = settings.holds();
  if holds.list.is_empty() {
    return "No edits are held back.\n".into();
  }
  let mut out = String::new();
  for (n, hold) in holds.list.iter().enumerate() {
    let what = hold.what.replace('\n', "\n     ");
    out.push_str(&format!(
      "{:>2}. {}\n    {}\n    held since run {}; to keep it for good: {}\n",
      n + 1,
      hold.path.display(),
      what,
      hold.run,
      hold.fix
    ));
  }
  out
}

/// Removes the reports of runs `history` no longer keeps, keeping those a remembered hold
/// names (see [`report::prune`]). Returns how many went, and their bytes.
pub fn prune_reports(history: &History) -> io::Result<(usize, u64)> {
  let Some(dir) = beside(history.dir()) else {
    return Ok((0, 0));
  };
  let runs: HashSet<String> = history.ids()?.into_iter().collect();
  let held: HashSet<String> = Holds::load(&dir).list.into_iter().map(|h| h.run).collect();
  report::prune(&dir.join("reports"), &runs, &held)
}

/// `mto doctor release N|all`.
pub fn release(settings: &Settings, which: &str) -> Result<String, String> {
  let mut holds = settings.holds();
  let count = holds.list.len();
  if which == "all" {
    holds.list.clear();
  } else {
    let n: usize = which
      .parse()
      .ok()
      .filter(|n| (1..=count).contains(n))
      .ok_or_else(|| {
        format!(
          "`{}` is not a hold: give all, or a number from 1 to {} (`mto doctor holds`)",
          which, count
        )
      })?;
    holds.list.remove(n - 1);
  }
  holds
    .save()
    .map_err(|e| format!("cannot write the holds: {}", e))?;
  Ok(format!(
    "Released {} hold(s); the edits are made again on the next run.\n",
    count - holds.list.len()
  ))
}

/// `mto doctor`: whether the guard is on, its command, and what it holds.
pub fn status(settings: &Settings, config: Option<&Path>) -> String {
  let mut out = format!(
    "build guard: {}\ncommand:     {} (in the project's folder)\n",
    if settings.enabled { "on" } else { "off" },
    settings.command.join(" ")
  );
  out.push_str(&format!(
    "limits:      {} per check, {} checks per narrowing\n",
    settings
      .timeout
      .map_or("no limit".to_string(), |t| format!("{} s", t.as_secs())),
    settings.max_checks
  ));
  let holds = settings.holds();
  out.push_str(&format!(
    "holds:       {}\n",
    match holds.list.len() {
      0 => "none".to_string(),
      n => format!("{} (`mto doctor holds` lists them)", n),
    }
  ));
  if let Some(config) = config {
    out.push_str(&format!("set in:      {}\n", config.display()));
  }
  if !settings.enabled {
    out.push_str("`mto doctor on` turns it on; mto runs no command until then.\n");
  }
  out
}

/// `mto doctor on` and `off`: sets `[doctor]` in the user config, keeping everything else
/// in it as written. Returns what the user config now says.
pub fn turn(path: &Path, on: bool, command: Option<&[String]>) -> Result<(), String> {
  let text = std::fs::read_to_string(path).unwrap_or_default();
  let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
  let header = lines.iter().position(|l| l.trim() == "[doctor]");
  let (start, end) = match header {
    Some(at) => {
      let end = lines[at + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |p| at + 1 + p);
      (at + 1, end)
    }
    None => {
      if lines.last().is_some_and(|l| !l.trim().is_empty()) {
        lines.push(String::new());
      }
      lines.push("[doctor]".into());
      (lines.len(), lines.len())
    }
  };
  let mut end = end;
  let mut set = |lines: &mut Vec<String>, key: &str, value: String| {
    let line = format!("{} = {}", key, value);
    let found = lines[start..end].iter().position(|l| {
      let l = l.trim_start().trim_start_matches('#').trim_start();
      l.strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
    });
    match found {
      Some(at) => lines[start + at] = line,
      None => {
        lines.insert(end, line);
        end += 1;
      }
    }
  };
  set(&mut lines, "enabled", on.to_string());
  if let Some(words) = command {
    let quoted: Vec<String> = words
      .iter()
      .map(|w| format!("\"{}\"", w.replace('\\', "\\\\").replace('"', "\\\"")))
      .collect();
    set(&mut lines, "command", format!("[{}]", quoted.join(", ")));
  }
  let mut out = lines.join("\n");
  out.push('\n');
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)
      .map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
  }
  std::fs::write(path, out).map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

fn beside(history_dir: &Path) -> Option<PathBuf> {
  history_dir.parent().map(|p| p.join("doctor"))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_turning_the_guard_on_keeps_the_rest_of_the_user_config() {
    let dir = std::env::temp_dir().join(format!("mtindo-doctor-on-{}", std::process::id()));
    let path = dir.join("config.toml");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
      &path,
      "# mine\n[doctor]\n# The command\n# command = []\n# enabled = false\n\n[history]\nkeep_runs = 5\n",
    )
    .unwrap();
    let words = vec!["cargo".to_string(), "test".to_string(), "-q".to_string()];
    turn(&path, true, Some(&words)).unwrap();
    assert_eq!(
      std::fs::read_to_string(&path).unwrap(),
      "# mine\n[doctor]\n# The command\ncommand = [\"cargo\", \"test\", \"-q\"]\nenabled = true\n\n[history]\nkeep_runs = 5\n"
    );
    turn(&path, false, None).unwrap();
    assert!(
      std::fs::read_to_string(&path)
        .unwrap()
        .contains("enabled = false\n")
    );
    std::fs::write(&path, "[history]\nkeep_runs = 5").unwrap();
    turn(&path, true, None).unwrap();
    assert_eq!(
      std::fs::read_to_string(&path).unwrap(),
      "[history]\nkeep_runs = 5\n\n[doctor]\nenabled = true\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
  }
}
