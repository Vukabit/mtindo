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

/// A guard command a project declares in its mto.toml (`doctor.command`): it runs only
/// once the user trusts it, as written, for that project.
#[derive(Clone, Debug)]
pub struct Project {
  pub words: Vec<String>,
  /// Where it is set: the mto.toml and line.
  pub set_in: String,
  /// The project's folder.
  pub root: PathBuf,
  /// The user trusts this command, word for word, for this project.
  pub trusted: bool,
}

/// The guard's settings: from the user config, and the project's command when it
/// declares one.
#[derive(Clone, Debug)]
pub struct Settings {
  pub enabled: bool,
  pub command: Vec<String>,
  /// The project's own command, when its mto.toml declares one.
  pub project: Option<Project>,
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
      project: None,
    }
  }

  /// These settings with what the project of `here` declares in its mto.toml: its
  /// command, trusted or not, and its limits, which win over the user's.
  pub fn with_project(mut self, resolver: &mut Resolver, here: &Path) -> Settings {
    let root = crate::cli::history::project_root(here);
    let Ok(config) = resolver.config_for(&root) else {
      return self;
    };
    // Only what the project's own mto.toml sets: the user config's values are the user's.
    let from_file = |name: &str| match config.source(name) {
      Some(crate::config::Source::File { path, line })
        if path
          .file_name()
          .is_some_and(|f| f == "mto.toml" || f == ".mto.toml") =>
      {
        Some(format!("{}:{}", path.display(), line))
      }
      _ => None,
    };
    if from_file("doctor.timeout").is_some()
      && let Some(seconds) = config.get("doctor.timeout").and_then(|v| v.as_integer())
    {
      self.timeout = (seconds > 0).then(|| Duration::from_secs(seconds));
    }
    if from_file("doctor.max_checks").is_some()
      && let Some(checks) = config.get("doctor.max_checks").and_then(|v| v.as_integer())
    {
      self.max_checks = checks as usize;
    }
    if let Some(set_in) = from_file("doctor.command")
      && let Some(words) = config.get("doctor.command").and_then(|v| v.as_list())
      && !words.is_empty()
    {
      let words: Vec<String> = words.to_vec();
      let trusted = self
        .dir
        .as_deref()
        .is_some_and(|dir| trust_list(dir).contains(&trust_line(&root, &words)));
      self.project = Some(Project {
        words,
        set_in,
        root,
        trusted,
      });
    }
    self
  }

  /// The project's command when the user has not trusted it: then a guarded run writes
  /// nothing.
  pub fn untrusted(&self) -> Option<&Project> {
    self.project.as_ref().filter(|p| !p.trusted)
  }

  /// Whether a doctor command may build the project: only with the guard on and, when the
  /// project names its command, that command trusted, as a guarded run would. Building runs
  /// the project's code (its build scripts), so a command that looks read-only never does
  /// it unasked. Why not, otherwise.
  pub fn builds(&self) -> Result<(), String> {
    if !self.enabled {
      return Err("the build guard is off, so no build runs (`mto doctor on` turns it on)".into());
    }
    if let Some(project) = self.untrusted() {
      return Err(format!(
        "this project asks to build with `{}` ({}), which is not trusted here, so no build \
         runs (`mto doctor trust` allows it)",
        project.words.join(" "),
        project.set_in
      ));
    }
    Ok(())
  }

  /// Trusts the project's command, word for word, for its project: it may run from now on,
  /// until it changes.
  pub fn trust(&self) -> Result<String, String> {
    let Some(project) = &self.project else {
      return Err(
        "this project declares no guard command (`doctor.command` in its mto.toml); the \
         guard uses yours"
          .into(),
      );
    };
    let Some(dir) = &self.dir else {
      return Err("there is no folder to keep the trust in: no home folder".into());
    };
    let mut list = trust_list(dir);
    let line = trust_line(&project.root, &project.words);
    if !list.contains(&line) {
      list.push(line);
    }
    std::fs::create_dir_all(dir)
      .and_then(|_| crate::write_atomic(&dir.join("trusted.txt"), &(list.join("\n") + "\n")))
      .map_err(|e| format!("cannot keep the trust: {}", e))?;
    Ok(format!(
      "Trusted for {}:\n\n    {}\n\n(set in {}). A guarded run there now runs it; if the \
       project changes it, mto asks again.\n",
      project.root.display(),
      project.words.join(" "),
      project.set_in
    ))
  }

  /// The guard for a run in `here`, checking with `command` (when `None`, the project's
  /// trusted command, else the user's).
  pub fn guard(&self, here: &Path, command: Option<Vec<String>>, quiet: bool) -> Guard {
    let ours = self
      .project
      .as_ref()
      .filter(|p| p.trusted)
      .map(|p| p.words.clone());
    Guard {
      check: Check {
        words: command.or(ours).unwrap_or_else(|| self.command.clone()),
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

/// `mto doctor keep N|all`: each held move, confirmed by trying it again (twice, the build
/// must fail both times), written into its file as `// mto: hold -- why` above the item,
/// and dropped from the holds the user keeps; a held change of lines is named, as more
/// likely a bug in mto, with how to keep it.
pub fn keep(settings: &Settings, resolver: &mut Resolver, which: &str, here: &Path) -> String {
  let mut holds = settings.holds();
  let mine = in_project(&holds, here);
  let count = mine.len();
  let picked: Vec<usize> = if which == "all" {
    mine.clone()
  } else {
    match which.parse::<usize>() {
      Ok(n) if (1..=count).contains(&n) => vec![mine[n - 1]],
      _ => {
        return format!(
          "`{}` is not a hold: give all, or a number from 1 to {} (`mto doctor holds`)\n",
          which, count
        );
      }
    }
  };
  if picked.is_empty() {
    return "No edits are held back.\n".into();
  }
  // Keeping one is confirmed by building with it moved, twice.
  if let Err(why) = settings.builds() {
    return format!("nothing was kept: {}\n", why);
  }
  let check = settings.guard(here, None, true).check;
  // The lines it writes are a run of its own, recorded as any write is, so `mto restore
  // last` takes them out again.
  let history = crate::cli::history::Settings::read(resolver, false).ok();
  let mut recorder = history
    .as_ref()
    .and_then(|h| h.history.as_ref())
    .map(|h| h.recorder(&crate::cli::history::command_line(), here, None));
  let mut out = String::new();
  let mut kept = Vec::new();
  for at in picked {
    let hold = &holds.list[at];
    let crate::doctor::holds::Kind::Move(id) = &hold.kind else {
      out.push_str(&format!(
        "{}. {}: {}\n    a changed line held back is more likely a bug in mto: the guard's \
         report for run {} shows it (`mto report` keeps mto's own); to keep the lines as \
         written, put `// mto: off` and `// mto: on` around them\n",
        at + 1,
        hold.path.display(),
        hold.what,
        hold.run
      ));
      continue;
    };
    match keep_move(resolver, &hold.path, id, &check, &holds, recorder.as_mut()) {
      Ok(reason) => {
        out.push_str(&format!(
          "{}. {}: {}\n    confirmed (the build failed twice with it moved); wrote \
           `// mto: hold -- {}` above it\n",
          at + 1,
          hold.path.display(),
          hold.what,
          reason
        ));
        kept.push(at);
      }
      Err(why) => out.push_str(&format!(
        "{}. {}: {}\n    not kept: {}\n",
        at + 1,
        hold.path.display(),
        hold.what,
        why
      )),
    }
  }
  for at in kept.into_iter().rev() {
    holds.list.remove(at);
  }
  if let Err(e) = holds.save() {
    out.push_str(&format!("cannot update the holds: {}\n", e));
  }
  if let (Some(recorder), Some(history)) = (recorder.as_mut(), &history) {
    crate::cli::history::finish_record(recorder, history.limits, false);
  }
  out
}

pub fn list(settings: &Settings, here: &Path) -> String {
  let holds = settings.holds();
  let mine = in_project(&holds, here);
  let others = holds.list.len() - mine.len();
  let elsewhere = match others {
    0 => String::new(),
    n => format!(
      "{} more held in other projects; run `mto doctor holds` there to see them\n",
      n
    ),
  };
  if mine.is_empty() {
    return format!("No edits are held back here.\n{}", elsewhere);
  }
  let mut out = String::new();
  for (n, hold) in mine.iter().map(|&at| &holds.list[at]).enumerate() {
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
  out.push_str(&elsewhere);
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

/// `mto doctor recheck`: each `// mto: hold` in the project of `here`, tried without it:
/// still needed when the build fails with the item moved, else named as no longer needed.
/// Nothing is changed: the line is the project's, for a person to remove.
pub fn recheck(settings: &Settings, resolver: &mut Resolver, here: &Path) -> String {
  let root = crate::cli::history::project_root(here);
  let check = settings.guard(here, None, true).check;
  // Whether a hold is still needed is told by building without it, when building is
  // allowed; one nothing would move without needs no build.
  let builds = settings.builds();
  let opts = crate::RunOptions::default();
  let jobs = match crate::collect_jobs(std::slice::from_ref(&root), resolver, &opts) {
    Ok(collected) => collected.jobs,
    Err(e) => return format!("cannot walk {}: {}\n", root.display(), e),
  };
  let holds = settings.holds();
  let mut out = String::new();
  let mut seen = 0;
  for (path, file_settings) in jobs {
    let (Ok(source), Some(file_settings)) = (std::fs::read_to_string(&path), file_settings) else {
      continue;
    };
    if !source.contains("mto: hold") {
      continue;
    }
    let parsed = crate::syntax::parse(&source, file_settings.edition);
    let found = crate::syntax::marker::scan(&parsed.syntax_node(), &source);
    for start in found.holds {
      seen += 1;
      let line_start = source[..start].rfind('\n').map_or(0, |at| at + 1);
      let line_end = source[start..]
        .find('\n')
        .map_or(source.len(), |at| start + at + 1);
      let line = source[..start].matches('\n').count() + 1;
      let without = format!("{}{}", &source[..line_start], &source[line_end..]);
      let verdict = match crate::shape(&without, &file_settings, &holds.held_for(&path)) {
        Err(e) => format!("cannot tell: {}", e),
        Ok(free) if free.moves.is_empty() => {
          "no longer needed: without it nothing would move".to_string()
        }
        Ok(_) if builds.is_err() => format!(
          "cannot tell: {}",
          builds
            .as_ref()
            .err()
            .map(String::as_str)
            .unwrap_or_default()
        ),
        Ok(free) => match try_twice(&path, &source, &free.text, &check) {
          Ok(true) => "still needed: the build fails with the item moved".to_string(),
          Ok(false) => "likely no longer needed: the build passed with the item moved".to_string(),
          Err(e) => format!("cannot tell: {}", e),
        },
      };
      out.push_str(&format!("{}:{}  {}\n", path.display(), line, verdict));
    }
  }
  if seen == 0 {
    return format!("No `// mto: hold` lines in {}.\n", root.display());
  }
  out
}

/// `mto doctor release N|all`.
pub fn release(settings: &Settings, which: &str, here: &Path) -> Result<String, String> {
  let mut holds = settings.holds();
  let total = holds.list.len();
  let mine = in_project(&holds, here);
  let count = mine.len();
  if which == "all" {
    for at in mine.into_iter().rev() {
      holds.list.remove(at);
    }
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
    holds.list.remove(mine[n - 1]);
  }
  let count = total;
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
  use crate::text::color::{paint, tone};
  let on = crate::color_enabled(false);
  let key = |name: &str| paint(on, tone::KEY, &format!("{:<12}", format!("{}:", name)));
  let mut out = format!(
    "{} {}\n{} {} {}\n",
    key("build guard"),
    if settings.enabled {
      paint(on, tone::GOOD, "on")
    } else {
      paint(on, tone::QUIET, "off")
    },
    key("command"),
    settings.command.join(" "),
    paint(on, tone::QUIET, "(in the project's folder)")
  );
  if let Some(project) = &settings.project {
    out.push_str(&format!(
      "{} {} {}\n",
      key("project's"),
      project.words.join(" "),
      if project.trusted {
        paint(
          on,
          tone::GOOD,
          &format!(
            "(set in {}; trusted, so runs in place of yours)",
            project.set_in
          ),
        )
      } else {
        paint(
          on,
          tone::WARN,
          &format!(
            "(set in {}; not trusted: guarded runs write nothing until `mto doctor trust`)",
            project.set_in
          ),
        )
      }
    ));
  }
  out.push_str(&format!(
    "{} {} per check, {} checks per narrowing\n",
    key("limits"),
    settings
      .timeout
      .map_or("no limit".to_string(), |t| format!("{} s", t.as_secs())),
    settings.max_checks
  ));
  let holds = settings.holds();
  out.push_str(&format!(
    "{} {}\n",
    key("holds"),
    match holds.list.len() {
      0 => paint(on, tone::QUIET, "none"),
      n => format!(
        "{} {}",
        paint(on, tone::WARN, &n.to_string()),
        paint(on, tone::QUIET, "(`mto doctor holds` lists them)")
      ),
    }
  ));
  if let Some(config) = config {
    out.push_str(&format!(
      "{} {}\n",
      key("set in"),
      paint(on, tone::QUIET, &config.display().to_string())
    ));
  }
  if !settings.enabled {
    out.push_str("`mto doctor on` turns it on; mto runs no command until then.\n");
  }
  out
}

/// `mto doctor --json`: whether the guard is on, its command and limits, each held edit,
/// and the user config that sets it.
pub fn status_json(settings: &Settings, config: Option<&Path>) -> crate::text::json::Json {
  use crate::text::json::Json;
  let holds: Vec<Json> = settings
    .holds()
    .list
    .iter()
    .map(|hold| {
      Json::object()
        .with("path", hold.path.display().to_string())
        .with("what", hold.what.as_str())
        .with("run", hold.run.as_str())
        .with("fix", hold.fix.as_str())
    })
    .collect();
  let project = settings.project.as_ref().map(|p| {
    Json::object()
      .with("command", p.words.clone())
      .with("set_in", p.set_in.as_str())
      .with("trusted", p.trusted)
  });
  Json::output("doctor")
    .with("enabled", settings.enabled)
    .with("command", settings.command.clone())
    .with("project", project)
    .with("timeout_s", settings.timeout.map(|t| t.as_secs()))
    .with("max_checks", settings.max_checks)
    .with("holds", holds)
    .with("set_in", config.map(|c| c.display().to_string()))
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

/// `mto doctor holds`: each held edit, numbered for `release`.
/// The holds on files of the project `here` is in, by their places in the list: what the
/// doctor's commands run there see and act on, as `mto history` shows this project's runs.
fn in_project(holds: &Holds, here: &Path) -> Vec<usize> {
  let root = crate::cli::history::project_root(here);
  let real_root = crate::real_path(&root).unwrap_or_else(|_| root.clone());
  (0..holds.list.len())
    .filter(|&at| {
      let path = &holds.list[at].path;
      path.starts_with(&root) || path.starts_with(&real_root)
    })
    .collect()
}

/// Confirms the hold on the move `id` of `path` and writes its directive: the reason, from
/// the first error the build gave with the move made.
fn keep_move(
  resolver: &mut Resolver,
  path: &Path,
  id: &crate::doctor::MoveId,
  check: &Check,
  holds: &Holds,
  recorder: Option<&mut crate::history::Recorder>,
) -> Result<String, String> {
  let source = std::fs::read_to_string(path).map_err(|e| format!("cannot read it: {}", e))?;
  let dir = path.parent().unwrap_or(Path::new("."));
  let settings = resolver
    .config_for(dir)
    .ok()
    .and_then(|c| crate::Settings::of(&c))
    .ok_or("layout is off there")?;
  // The file's other holds still hold; this one is let go.
  let mut held = holds.held_for(path);
  held.move_ids.retain(|other| other != id);
  let free = crate::shape(&source, &settings, &held).map_err(|e| e.to_string())?;
  let Some(moved) = free
    .moves
    .iter()
    .find(|m| &crate::doctor::move_id(&source, m) == id)
  else {
    return Err("the item no longer moves; `mto doctor release` drops the hold".into());
  };
  let mut errors = Vec::new();
  for _ in 0..2 {
    crate::write_atomic(path, &free.text).map_err(|e| format!("cannot try it: {}", e))?;
    let said = check.run();
    crate::write_atomic(path, &source).map_err(|e| format!("cannot put it back: {}", e))?;
    if said.passed {
      return Err(
        "the build passed with it moved, so the hold may come from a flaky command; \
         `mto doctor release` drops it"
          .into(),
      );
    }
    errors.extend(said.errors.iter().cloned());
  }
  let reason = errors
    .first()
    .map(|e| e.chars().take(70).collect::<String>())
    .unwrap_or_else(|| "moving it breaks the build".into());
  // Above the unit's first line, at its indentation, with the file's line ending.
  let text = crate::text::source::normalize(&source).map_err(|e| e.to_string())?;
  let line = text.text[..moved.span.0.min(text.text.len())]
    .matches('\n')
    .count();
  let ending = if source.contains("\r\n") {
    "\r\n"
  } else {
    "\n"
  };
  let mut lines: Vec<&str> = source.split_inclusive('\n').collect();
  let indent: String = lines
    .get(line)
    .map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect())
    .unwrap_or_default();
  let directive = format!("{}// mto: hold -- {}{}", indent, reason, ending);
  lines.insert(line.min(lines.len()), &directive);
  let text = lines.concat();
  // Recorded before it is written, as a run records every file.
  if let Some(recorder) = recorder {
    let before = recorder
      .history()
      .store(source.as_bytes())
      .map_err(|e| format!("cannot record it in history: {}", e))?;
    let entry = crate::history::Entry {
      path: std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
      before,
      after: crate::history::sha256::hex(text.as_bytes()),
    };
    recorder
      .record(&entry)
      .and_then(|_| recorder.order())
      .map_err(|e| format!("cannot record it in history: {}", e))?;
  }
  crate::write_atomic(path, &text).map_err(|e| format!("cannot write it: {}", e))?;
  Ok(reason)
}

/// A trusted command, as the list keeps it: the project's folder, then a hash of its words,
/// so a command changed by a pull is not trusted.
fn trust_line(root: &Path, words: &[String]) -> String {
  format!(
    "{}\t{}",
    root.display(),
    crate::history::sha256::hex(words.join("\u{0}").as_bytes())
  )
}

/// The lines of the trust list: each project and command the user trusts.
fn trust_list(dir: &Path) -> Vec<String> {
  std::fs::read_to_string(dir.join("trusted.txt"))
    .map(|text| text.lines().map(str::to_string).collect())
    .unwrap_or_default()
}

/// Whether the build fails twice with `path` holding `text`; the file is put back as
/// `source` after each try.
fn try_twice(path: &Path, source: &str, text: &str, check: &Check) -> Result<bool, String> {
  for _ in 0..2 {
    crate::write_atomic(path, text).map_err(|e| format!("cannot try it: {}", e))?;
    let said = check.run();
    crate::write_atomic(path, source).map_err(|e| format!("cannot put it back: {}", e))?;
    if said.passed {
      return Ok(false);
    }
  }
  Ok(true)
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
