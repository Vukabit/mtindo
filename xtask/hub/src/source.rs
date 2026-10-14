//! Sources: files and commands turned into rows (SPEC.md section 3), each reloaded no more
//! often than its refresh.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::{Loaded, SourceSpec};
use crate::expr::{Row, Scope, Value};

/// The most sources loading at once in the background.
const AT_ONCE: usize = 8;

/// The longest a source's command may run.
const COMMAND_LIMIT: Duration = Duration::from_secs(30);

/// The most of a log or file kept in memory: its end.
const KEPT: u64 = 16 << 20;

/// A finished background load: its key, and its rows or why not.
type Done = (String, Result<Vec<Row>, String>);

/// Loads that run on threads of their own, so the interface never waits for a source.
struct Background {
  loaded: Arc<Loaded>,
  send: Sender<Done>,
  done: Receiver<Done>,
}

/// One source's rows as last loaded.
#[derive(Clone, Debug, Default)]
pub struct Data {
  pub rows: Vec<Row>,
  /// Why the last load failed, if it did; the rows are then the previous ones.
  pub error: Option<String>,
  /// When its rows were last read; `None` before the first load ends.
  pub updated: Option<Instant>,
  /// A load is under way, and since when.
  pub loading: Option<Instant>,
  /// How often it is read again.
  pub refresh: Duration,
}

impl Data {
  /// Whether its rows are older than three of its refreshes (and a few seconds): its loads
  /// fail or take long.
  pub fn stale(&self) -> bool {
    self
      .updated
      .is_some_and(|at| at.elapsed() > self.refresh * 3 + Duration::from_secs(5))
  }
}

/// Every source's rows, reloaded as their refresh allows.
pub struct Store {
  data: HashMap<String, Data>,
  loaded_at: HashMap<String, Instant>,
  background: Option<Background>,
  tails: HashMap<PathBuf, Tail>,
  /// Rows added to each source's limit (`page`), by name.
  more: HashMap<String, usize>,
  /// The stream sources' programs, by key.
  streams: HashMap<String, Stream>,
}

impl Store {
  /// A store that loads a source when it is asked for, before answering: for rendering
  /// once and for checks.
  pub fn new() -> Store {
    Store {
      data: HashMap::new(),
      loaded_at: HashMap::new(),
      background: None,
      tails: HashMap::new(),
      more: HashMap::new(),
      streams: HashMap::new(),
    }
  }

  /// A store that loads in the background: asked for a source due a reload, it starts the
  /// load and answers with the rows it has; [`Store::receive`] takes what finished.
  #[cfg(test)]
  pub fn background(loaded: Arc<Loaded>) -> Store {
    let mut store = Store::new();
    store.go_background(loaded);
    store
  }

  /// From now on loads in the background, as [`Store::background`]; keeps the rows it has.
  pub fn go_background(&mut self, loaded: Arc<Loaded>) {
    let (send, done) = channel();
    self.background = Some(Background { loaded, send, done });
  }

  /// Takes the background loads that finished; whether any did.
  pub fn receive(&mut self) -> bool {
    let Some(background) = &self.background else {
      return false;
    };
    let finished: Vec<Done> = background.done.try_iter().collect();
    for (key, result) in &finished {
      let entry = self.data.entry(key.clone()).or_default();
      entry.loading = None;
      match result {
        Ok(rows) => {
          entry.rows = rows.clone();
          entry.error = None;
          entry.updated = Some(Instant::now());
        }
        Err(e) => entry.error = Some(e.clone()),
      }
      self.loaded_at.insert(key.clone(), Instant::now());
    }
    !finished.is_empty()
  }

  /// Whether a background load is under way.
  pub fn busy(&self) -> bool {
    self.data.values().any(|d| d.loading.is_some())
  }

  /// The text of the file at `path`, reading only what was added since it was last read
  /// (all of it again when it shrank or was replaced), and at most its last 16 MB.
  pub fn tail(&mut self, path: &Path) -> Result<&str, String> {
    use std::io::{Read, Seek, SeekFrom};
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    let known = self.tails.get(path);
    let unchanged = known.is_some_and(|t| t.len == len && t.modified == modified);
    if !unchanged {
      let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
      let grown = known.filter(|t| len > t.len && t.len > 0);
      let mut text = match grown {
        Some(t) => {
          file
            .seek(SeekFrom::Start(t.len))
            .map_err(|e| e.to_string())?;
          t.text.clone()
        }
        None => {
          if len > KEPT {
            file
              .seek(SeekFrom::Start(len - KEPT))
              .map_err(|e| e.to_string())?;
          }
          String::new()
        }
      };
      let mut bytes = Vec::new();
      file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
      text.push_str(&String::from_utf8_lossy(&bytes));
      if text.len() as u64 > KEPT {
        let cut = text.len() - KEPT as usize;
        let cut = (cut..text.len())
          .find(|&i| text.is_char_boundary(i))
          .unwrap_or(text.len());
        text.drain(..cut);
      }
      self.tails.insert(
        path.to_path_buf(),
        Tail {
          len,
          modified,
          text,
        },
      );
    }
    Ok(self.tails.get(path).map_or("", |t| t.text.as_str()))
  }

  /// A source's rows, loading them first if they are older than its refresh (or `force`).
  pub fn get(&mut self, loaded: &Loaded, name: &str, force: bool) -> &Data {
    self.get_for(loaded, name, None, force)
  }

  /// A source's rows for `args` (its `args` template as rendered where it is shown), each
  /// value kept apart.
  pub fn get_for(&mut self, loaded: &Loaded, name: &str, args: Option<&str>, force: bool) -> &Data {
    let key = match args {
      Some(a) => format!("{}\u{1f}{}", name, a),
      None => name.to_string(),
    };
    // A selection moved over many rows leaves many keys: keep the store small.
    if args.is_some() && self.data.len() > 512 {
      self.data.retain(|k, _| !k.contains('\u{1f}'));
      self.loaded_at.retain(|k, _| !k.contains('\u{1f}'));
    }
    let spec = loaded.source(name);
    let refresh = spec
      .and_then(|s| s.refresh.as_deref())
      .or(loaded.config.hub.refresh.as_deref())
      .and_then(duration)
      .unwrap_or(Duration::from_secs(1));
    let stale = force
      || self
        .loaded_at
        .get(&key)
        .is_none_or(|at| at.elapsed() >= refresh);
    // A stream runs on its own while the hub does; read as it goes.
    if self.background.is_some()
      && let Some(spec) = spec.filter(|s| s.kind == "stream")
    {
      let spec = spec.clone();
      self.stream(loaded, &key, &spec, args);
      let entry = self.data.entry(key.clone()).or_default();
      entry.refresh = refresh;
      return self.data.entry(key).or_default();
    }
    let loading_now = self.data.values().filter(|d| d.loading.is_some()).count();
    let more = self.more.get(name).copied().unwrap_or(0);
    let entry = self.data.entry(key.clone()).or_default();
    entry.refresh = refresh;
    if stale
      && entry.loading.is_none()
      && let Some(spec) = spec
    {
      match &self.background {
        Some(background) if loading_now < AT_ONCE => {
          entry.loading = Some(Instant::now());
          let (shared, send) = (background.loaded.clone(), background.send.clone());
          let (name, args) = (name.to_string(), args.map(str::to_string));
          let key = key.clone();
          std::thread::spawn(move || {
            let result = match shared.source(&name) {
              Some(spec) => load(&shared, spec, args.as_deref(), more),
              None => Err(format!("no source {}", name)),
            };
            let _ = send.send((key, result));
          });
        }
        // Too many under way: asked again next frame.
        Some(_) => {}
        None => {
          match load(loaded, spec, args, more) {
            Ok(rows) => {
              entry.rows = rows;
              entry.error = None;
              entry.updated = Some(Instant::now());
            }
            Err(e) => entry.error = Some(e),
          }
          self.loaded_at.insert(key.clone(), Instant::now());
        }
      }
    }
    self.data.entry(key).or_default()
  }

  /// A source with `page` gives that many more rows, when its rows for `args` fill its
  /// limit (there may be more) and it is not loading. Whether it grew.
  pub fn grow(&mut self, loaded: &Loaded, name: &str, args: Option<&str>) -> bool {
    let Some(spec) = loaded.source(name) else {
      return false;
    };
    let Some(page) = spec.page else {
      return false;
    };
    let key = match args {
      Some(a) => format!("{}\u{1f}{}", name, a),
      None => name.to_string(),
    };
    let limit = spec.limit.unwrap_or(200) + self.more.get(name).copied().unwrap_or(0);
    let full = self
      .data
      .get(&key)
      .is_some_and(|d| d.loading.is_none() && d.rows.len() >= limit);
    if !full {
      return false;
    }
    *self.more.entry(name.to_string()).or_default() += page;
    self.loaded_at.remove(&key);
    true
  }

  /// Forgets every load time, so each source is read again when next asked for.
  pub fn invalidate(&mut self) {
    self.loaded_at.clear();
  }
}

impl Store {
  /// A stream source's rows: its program started the first time (and again a refresh
  /// after it ends), each line it prints a row as it comes, the newest `limit` (1000)
  /// kept.
  fn stream(&mut self, loaded: &Loaded, key: &str, spec: &SourceSpec, args: Option<&str>) {
    let refresh = spec
      .refresh
      .as_deref()
      .and_then(duration)
      .unwrap_or(Duration::from_secs(5));
    let ended = self
      .streams
      .get_mut(key)
      .is_some_and(|s| matches!(s.child.try_wait(), Ok(Some(_))));
    let due = self
      .streams
      .get(key)
      .is_none_or(|s| ended && s.started.elapsed() >= refresh);
    if due {
      self.streams.remove(key);
      let line = match args {
        Some(a) => format!("{} {}", spec.run.as_deref().unwrap_or(""), a),
        None => spec.run.clone().unwrap_or_default(),
      };
      let words = words(&line);
      let started = words
        .split_first()
        .ok_or_else(|| "an empty command".to_string())
        .and_then(|(program, rest)| {
          Command::new(program)
            .args(rest)
            .current_dir(&loaded.base)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run {}: {}", program, e))
        });
      match started {
        Ok(mut child) => {
          let rows = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
          let said = Arc::new(std::sync::Mutex::new(String::new()));
          let limit = spec.limit.unwrap_or(1000);
          if let Some(out) = child.stdout.take() {
            let (rows, spec) = (rows.clone(), spec.clone());
            std::thread::spawn(move || {
              use std::io::BufRead;
              for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(row) = stream_row(&line, &spec) {
                  let mut rows = rows.lock().expect("not poisoned");
                  rows.push_back(row);
                  while rows.len() > limit {
                    rows.pop_front();
                  }
                }
              }
            });
          }
          if let Some(err) = child.stderr.take() {
            let said = said.clone();
            std::thread::spawn(move || {
              use std::io::BufRead;
              for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                *said.lock().expect("not poisoned") = line;
              }
            });
          }
          self.streams.insert(
            key.to_string(),
            Stream {
              child,
              rows,
              said,
              started: Instant::now(),
            },
          );
        }
        Err(e) => {
          let entry = self.data.entry(key.to_string()).or_default();
          entry.error = Some(e);
          return;
        }
      }
    }
    let Some(stream) = self.streams.get(key) else {
      return;
    };
    let mut rows: Vec<Row> = stream
      .rows
      .lock()
      .expect("not poisoned")
      .iter()
      .cloned()
      .collect();
    for row in rows.iter_mut() {
      for (field, text) in &spec.computed {
        let value = loaded.expr(text).eval(&Scope {
          row: Some(row),
          ..Scope::default()
        });
        row.insert(field.clone(), value);
      }
    }
    let said = stream.said.lock().expect("not poisoned").clone();
    let entry = self.data.entry(key.to_string()).or_default();
    entry.rows = rows;
    entry.updated = Some(Instant::now());
    entry.error = (ended && !said.is_empty()).then_some(said);
  }
}

/// A stream's program, running, and the rows it has printed.
struct Stream {
  child: std::process::Child,
  rows: Arc<std::sync::Mutex<std::collections::VecDeque<Row>>>,
  /// What it last printed on its standard error.
  said: Arc<std::sync::Mutex<String>>,
  started: Instant,
}

impl Drop for Stream {
  fn drop(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

/// A text file as last read, kept to read only what was added since.
struct Tail {
  len: u64,
  modified: Option<SystemTime>,
  text: String,
}

/// Runs a command line in `base`, under [`COMMAND_LIMIT`]; its standard output.
pub fn command(base: &Path, line: &str) -> Result<String, String> {
  run(base, &words(line), None, &[], COMMAND_LIMIT, &[0])
}

/// `"500ms"`, `"2s"`, `"5m"`, `"1h"` as a duration.
pub fn duration(text: &str) -> Option<Duration> {
  let text = text.trim();
  let (number, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit() && c != '.')?);
  let n: f64 = number.parse().ok()?;
  Some(Duration::from_secs_f64(match unit {
    "ms" => n / 1000.0,
    "s" => n,
    "m" => n * 60.0,
    "h" => n * 3600.0,
    _ => return None,
  }))
}

/// Loads a source's rows, shaped by its `group` and `computed`.
/// With `args` (the source's `args` rendered), those words follow `run`; a source with an
/// `args` template that renders to nothing (no row selected) has no rows.
pub fn load(
  loaded: &Loaded,
  spec: &SourceSpec,
  args: Option<&str>,
  more: usize,
) -> Result<Vec<Row>, String> {
  let base = &loaded.base;
  if spec.args.is_some() && args.is_none_or(|a| a.trim().is_empty()) {
    return Ok(Vec::new());
  }
  let with_args = |run: &str| match args {
    Some(a) => format!("{} {}", run, a),
    None => run.to_string(),
  };
  let mut rows = match spec.kind.as_str() {
    "toml" => files(base, spec)?
      .iter()
      .filter_map(|path| toml_row(path).ok())
      .collect(),
    "tsv" => {
      let mut rows = Vec::new();
      for path in files(base, spec)? {
        rows.extend(tsv_rows(&path, spec));
      }
      rows
    }
    "lines" => {
      let pattern =
        regex::Regex::new(spec.pattern.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
      let mut rows = Vec::new();
      let mut texts: Vec<(Option<PathBuf>, String)> = Vec::new();
      if spec.glob.is_some() {
        for path in files(base, spec)? {
          texts.push((
            Some(path.clone()),
            std::fs::read_to_string(&path).unwrap_or_default(),
          ));
        }
      }
      if let Some(run) = &spec.run {
        texts.push((None, command(base, &with_args(run))?));
      }
      for (path, text) in texts {
        for line in text.lines() {
          if let Some(caps) = pattern.captures(line) {
            let mut row = path.as_deref().map(file_fields).unwrap_or_default();
            row.insert("line".into(), Value::Str(line.to_string()));
            for name in pattern.capture_names().flatten() {
              if let Some(m) = caps.name(name) {
                row.insert(name.to_string(), scalar(m.as_str()));
              }
            }
            rows.push(row);
          }
        }
      }
      rows
    }
    "command" => {
      let out = command(base, &with_args(spec.run.as_deref().unwrap_or("")))?;
      parse_output(&out, spec)?
    }
    "http" => {
      let mut words: Vec<String> = ["curl", "--fail", "--silent", "--show-error", "--location"]
        .iter()
        .map(|w| w.to_string())
        .collect();
      words.extend(["--max-time".to_string(), "20".to_string()]);
      for header in &spec.headers {
        words.push("--header".into());
        words.push(environment(header));
      }
      words.push(with_args(spec.url.as_deref().unwrap_or("")));
      let out = run(base, &words, None, &[], COMMAND_LIMIT, &[0])?;
      parse_output(&out, spec)?
    }
    // Read once for a moment: what a `--render` or `--check` sees of it.
    "stream" => sample_stream(base, &with_args(spec.run.as_deref().unwrap_or("")), spec)?,
    "files" => files(base, spec)?.iter().map(|p| file_fields(p)).collect(),
    "git" => {
      let args = args.map(words).unwrap_or_default();
      crate::git::rows(
        base,
        spec.of.as_deref().unwrap_or("head"),
        &args,
        spec.limit.unwrap_or(200) + more,
      )?
    }
    "commands" => {
      let out = command(base, spec.run.as_deref().unwrap_or(""))?;
      let tree: serde_json::Value =
        serde_json::from_str(&out).map_err(|e| format!("the command tree is not JSON: {}", e))?;
      let mut rows = Vec::new();
      command_rows(&tree, &mut Vec::new(), &mut rows);
      rows
    }
    other => return Err(format!("no source kind `{}`", other)),
  };
  // Paths relative to the config's folder, for display.
  for row in rows.iter_mut() {
    for (field, rel) in [("_path", "_rel"), ("_dir", "_reldir")] {
      if let Some(Value::Str(p)) = row.get(field) {
        let relative = Path::new(p)
          .strip_prefix(base)
          .map_or(p.clone(), |r| r.display().to_string());
        row.insert(rel.to_string(), Value::Str(relative));
      }
    }
  }
  if let Some(group) = &spec.group {
    rows = grouped(rows, &group.by, &group.list, &group.last);
  }
  for row in rows.iter_mut() {
    for (field, text) in &spec.computed {
      let value = loaded.expr(text).eval(&Scope {
        row: Some(row),
        ..Scope::default()
      });
      row.insert(field.clone(), value);
    }
  }
  Ok(rows)
}

/// Runs `words` in `base` with `input` on its standard input and `env` set, for no longer
/// than `limit`; its standard output if it exits with a code in `ok`. A failure says
/// what the command printed on its standard error.
pub fn run(
  base: &Path,
  words: &[String],
  input: Option<&str>,
  env: &[(&str, &str)],
  limit: Duration,
  ok: &[i32],
) -> Result<String, String> {
  run_cancellable(base, words, input, env, limit, ok, None)
}

/// [`run`], stopped early when `cancel` is set. The command runs in a process group of its
/// own, so stopping it (cancelled, or past `limit`) stops whatever it started too.
pub fn run_cancellable(
  base: &Path,
  words: &[String],
  input: Option<&str>,
  env: &[(&str, &str)],
  limit: Duration,
  ok: &[i32],
  cancel: Option<&AtomicBool>,
) -> Result<String, String> {
  let (program, args) = words.split_first().ok_or("an empty command")?;
  let line = words.join(" ");
  let mut command = Command::new(program);
  #[cfg(unix)]
  std::os::unix::process::CommandExt::process_group(&mut command, 0);
  let mut child = command
    .args(args)
    .current_dir(base)
    .envs(env.iter().copied())
    .stdin(if input.is_some() {
      Stdio::piped()
    } else {
      Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| format!("cannot run {}: {}", program, e))?;
  if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
    let text = text.to_string();
    std::thread::spawn(move || {
      let _ = std::io::Write::write_all(&mut stdin, text.as_bytes());
    });
  }
  let read = |pipe: Option<Box<dyn std::io::Read + Send>>| {
    std::thread::spawn(move || {
      let mut text = String::new();
      if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_string(&mut text);
      }
      text
    })
  };
  let stdout = read(
    child
      .stdout
      .take()
      .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
  );
  let stderr = read(
    child
      .stderr
      .take()
      .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
  );
  let started = Instant::now();
  loop {
    match child.try_wait() {
      Ok(Some(status)) => {
        let text = stdout.join().unwrap_or_default();
        let said = stderr.join().unwrap_or_default();
        return if status.code().is_some_and(|c| ok.contains(&c)) {
          Ok(text)
        } else {
          let said = said.trim();
          Err(if said.is_empty() {
            format!("`{}` exited with {}", line, status)
          } else {
            format!("`{}` exited with {}: {}", line, status, said)
          })
        };
      }
      Ok(None) if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) => {
        stop(&mut child);
        return Err(format!("`{}` was cancelled", line));
      }
      Ok(None) if started.elapsed() > limit => {
        stop(&mut child);
        return Err(format!("`{}` passed {}s", line, limit.as_secs()));
      }
      Ok(None) => std::thread::sleep(Duration::from_millis(5)),
      Err(e) => return Err(e.to_string()),
    }
  }
}

/// Splits a command line into words, honoring quotes; no shell.
pub fn words(line: &str) -> Vec<String> {
  let mut out = Vec::new();
  let mut current = String::new();
  let mut quote: Option<char> = None;
  let mut started = false;
  for c in line.chars() {
    match (quote, c) {
      (Some(q), c) if c == q => quote = None,
      (Some(_), c) => current.push(c),
      (None, '"' | '\'') => {
        quote = Some(c);
        started = true;
      }
      (None, c) if c.is_whitespace() => {
        if started || !current.is_empty() {
          out.push(std::mem::take(&mut current));
          started = false;
        }
      }
      (None, c) => current.push(c),
    }
  }
  if started || !current.is_empty() {
    out.push(current);
  }
  out
}

/// One row per runnable command of a command tree: those without commands below them,
/// or whose commands are optional.
fn command_rows(node: &serde_json::Value, path: &mut Vec<String>, out: &mut Vec<Row>) {
  let subs = node["commands"].as_array().cloned().unwrap_or_default();
  let required = node["subcommand_required"].as_bool().unwrap_or(false);
  if !path.is_empty() && (subs.is_empty() || !required) {
    let mut row = Row::new();
    row.insert("path".into(), Value::Str(path.join(" ")));
    row.insert(
      "name".into(),
      Value::Str(path.last().cloned().unwrap_or_default()),
    );
    row.insert("about".into(), json_value(&node["about"]));
    row.insert("args".into(), json_value(&node["args"]));
    out.push(row);
  }
  for sub in &subs {
    if let Some(name) = sub["name"].as_str() {
      path.push(name.to_string());
      command_rows(sub, path, out);
      path.pop();
    }
  }
}

/// `text` with each `$NAME` (letters, digits, `_`) the environment's value.
fn environment(text: &str) -> String {
  let mut out = String::new();
  let mut chars = text.chars().peekable();
  while let Some(c) = chars.next() {
    if c != '$' {
      out.push(c);
      continue;
    }
    let mut name = String::new();
    while let Some(&n) = chars.peek() {
      if n.is_ascii_alphanumeric() || n == '_' {
        name.push(n);
        chars.next();
      } else {
        break;
      }
    }
    if name.is_empty() {
      out.push('$');
    } else {
      out.push_str(&std::env::var(&name).unwrap_or_default());
    }
  }
  out
}

/// `_path`, `_dir`, `_name` and `_mtime` of a file.
fn file_fields(path: &Path) -> Row {
  let mut row = Row::new();
  row.insert("_path".into(), Value::Str(path.display().to_string()));
  row.insert(
    "_dir".into(),
    Value::Str(
      path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default(),
    ),
  );
  row.insert(
    "_name".into(),
    Value::Str(
      path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default(),
    ),
  );
  let mtime = std::fs::metadata(path)
    .and_then(|m| m.modified())
    .ok()
    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
    .map_or(0.0, |d| d.as_secs_f64());
  row.insert("_mtime".into(), Value::Num(mtime));
  row
}

/// The files a source's glob matches, sorted.
fn files(base: &Path, spec: &SourceSpec) -> Result<Vec<PathBuf>, String> {
  let pattern = base.join(spec.glob.as_deref().unwrap_or(""));
  let mut out: Vec<PathBuf> = glob::glob(&pattern.to_string_lossy())
    .map_err(|e| format!("glob: {}", e))?
    .flatten()
    .filter(|p| p.is_file())
    .collect();
  out.sort();
  Ok(out)
}

/// The checkout's branch, commit and whether it has uncommitted changes.
/// Rows grouped by field `by`: for each field in `list`, a `<field>_list` of every row's
/// value in order; for each in `last`, the group's last value.
fn grouped(rows: Vec<Row>, by: &str, list: &[String], last: &[String]) -> Vec<Row> {
  let mut groups: BTreeMap<String, Row> = BTreeMap::new();
  for row in rows {
    let key = row.get(by).map(Value::text).unwrap_or_default();
    let group = groups.entry(key.clone()).or_insert_with(|| {
      let mut g = Row::new();
      g.insert(by.to_string(), Value::Str(key.clone()));
      g
    });
    for field in list {
      let entry = group
        .entry(format!("{}_list", field))
        .or_insert(Value::List(Vec::new()));
      if let Value::List(items) = entry {
        items.push(row.get(field).cloned().unwrap_or(Value::Null));
      }
    }
    for field in last {
      if let Some(v) = row.get(field) {
        group.insert(field.clone(), v.clone());
      }
    }
  }
  groups.into_values().collect()
}

fn json_rows(text: &str) -> Result<Vec<Row>, String> {
  let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
  match value {
    serde_json::Value::Array(items) => Ok(
      items
        .iter()
        .filter_map(|i| i.as_object())
        .map(object_row)
        .collect(),
    ),
    serde_json::Value::Object(o) => Ok(vec![object_row(&o)]),
    _ => Err("the output is not a JSON object or array".into()),
  }
}

fn json_value(value: &serde_json::Value) -> Value {
  match value {
    serde_json::Value::Null => Value::Null,
    serde_json::Value::Bool(b) => Value::Bool(*b),
    serde_json::Value::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
    serde_json::Value::String(s) => Value::Str(s.clone()),
    serde_json::Value::Array(items) if items.iter().all(|i| i.is_object()) && !items.is_empty() => {
      Value::Rows(
        items
          .iter()
          .filter_map(|i| i.as_object())
          .map(object_row)
          .collect(),
      )
    }
    serde_json::Value::Array(items) => Value::List(items.iter().map(json_value).collect()),
    serde_json::Value::Object(o) => Value::Rows(vec![object_row(o)]),
  }
}

fn object_row(object: &serde_json::Map<String, serde_json::Value>) -> Row {
  object
    .iter()
    .map(|(k, v)| (k.clone(), json_value(v)))
    .collect()
}

/// A command's output as rows, as `parse` says: `lines` (field `line`), `tsv`, `json`, or
/// one row with field `text`.
fn parse_output(out: &str, spec: &SourceSpec) -> Result<Vec<Row>, String> {
  Ok(match spec.parse.as_deref().unwrap_or("text") {
    "lines" => out
      .lines()
      .map(|l| Row::from([("line".to_string(), Value::Str(l.to_string()))]))
      .collect(),
    "tsv" => parse_tsv(out, spec),
    "json" => json_rows(out)?,
    _ => vec![Row::from([(
      "text".to_string(),
      Value::Str(out.to_string()),
    )])],
  })
}

fn parse_tsv(text: &str, spec: &SourceSpec) -> Vec<Row> {
  let mut lines = text.lines();
  let columns: Vec<String> = match &spec.columns {
    Some(columns) => columns.clone(),
    None => lines
      .next()
      .map(|l| l.split('\t').map(str::to_string).collect())
      .unwrap_or_default(),
  };
  lines
    .filter(|l| !l.is_empty())
    .map(|line| {
      columns
        .iter()
        .cloned()
        .zip(line.split('\t').map(scalar))
        .collect()
    })
    .collect()
}

/// What a stream prints in its first second, for drawing once.
fn sample_stream(base: &Path, line: &str, spec: &SourceSpec) -> Result<Vec<Row>, String> {
  let words = words(line);
  let (program, args) = words.split_first().ok_or("an empty command")?;
  let mut child = Command::new(program)
    .args(args)
    .current_dir(base)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| format!("cannot run {}: {}", program, e))?;
  let out = child.stdout.take().ok_or("no output")?;
  let (send, read) = channel();
  std::thread::spawn(move || {
    use std::io::BufRead;
    for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
      if send.send(line).is_err() {
        break;
      }
    }
  });
  let started = Instant::now();
  let mut rows = Vec::new();
  while let Some(left) = Duration::from_secs(1).checked_sub(started.elapsed()) {
    match read.recv_timeout(left) {
      Ok(line) => rows.extend(stream_row(&line, spec)),
      Err(_) => break,
    }
  }
  let _ = child.kill();
  let _ = child.wait();
  Ok(rows)
}

/// A text value, as a number when it reads as one.
fn scalar(text: &str) -> Value {
  match text.trim().parse::<f64>() {
    Ok(n) if !text.trim().is_empty() => Value::Num(n),
    _ => Value::Str(text.to_string()),
  }
}

/// Stops a command and everything in its process group: asked to end, then made to a
/// second later.
fn stop(child: &mut std::process::Child) {
  #[cfg(unix)]
  {
    let group = -(child.id() as libc::pid_t);
    // SAFETY: signals the child's own process group, which it leads.
    unsafe { libc::kill(group, libc::SIGTERM) };
    let asked = Instant::now();
    while asked.elapsed() < Duration::from_secs(1) {
      if matches!(child.try_wait(), Ok(Some(_))) {
        // The group may outlive its leader.
        unsafe { libc::kill(group, libc::SIGKILL) };
        return;
      }
      std::thread::sleep(Duration::from_millis(20));
    }
    unsafe { libc::kill(group, libc::SIGKILL) };
  }
  let _ = child.kill();
  let _ = child.wait();
}

/// One line of a stream as a row: a JSON object, or the line as `line`.
fn stream_row(line: &str, spec: &SourceSpec) -> Option<Row> {
  match spec.parse.as_deref().unwrap_or("json") {
    "json" => match serde_json::from_str::<serde_json::Value>(line) {
      Ok(serde_json::Value::Object(o)) => Some(object_row(&o)),
      _ => None,
    },
    _ => Some(Row::from([(
      "line".to_string(),
      Value::Str(line.to_string()),
    )])),
  }
}

fn toml_row(path: &Path) -> Result<Row, String> {
  let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
  let table: toml::Table = toml::from_str(&text).map_err(|e| e.to_string())?;
  let mut row = file_fields(path);
  for (key, value) in &table {
    row.insert(key.clone(), toml_value(value));
  }
  Ok(row)
}

fn toml_value(value: &toml::Value) -> Value {
  match value {
    toml::Value::String(s) => Value::Str(s.clone()),
    toml::Value::Integer(i) => Value::Num(*i as f64),
    toml::Value::Float(f) => Value::Num(*f),
    toml::Value::Boolean(b) => Value::Bool(*b),
    toml::Value::Datetime(d) => Value::Str(d.to_string()),
    toml::Value::Array(items) if items.iter().all(|i| i.is_table()) && !items.is_empty() => {
      Value::Rows(
        items
          .iter()
          .filter_map(|i| i.as_table())
          .map(|t| t.iter().map(|(k, v)| (k.clone(), toml_value(v))).collect())
          .collect(),
      )
    }
    toml::Value::Array(items) => Value::List(items.iter().map(toml_value).collect()),
    toml::Value::Table(t) => Value::Rows(vec![
      t.iter().map(|(k, v)| (k.clone(), toml_value(v))).collect(),
    ]),
  }
}

fn tsv_rows(path: &Path, spec: &SourceSpec) -> Vec<Row> {
  let text = std::fs::read_to_string(path).unwrap_or_default();
  let fields = file_fields(path);
  parse_tsv(&text, spec)
    .into_iter()
    .map(|mut row| {
      row.extend(fields.clone());
      row
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A folder of its own for one test, with a `.hub.toml` holding `body`.
  fn config(name: &str, body: &str) -> (PathBuf, Arc<Loaded>) {
    let dir = std::env::temp_dir().join(format!("hub-source-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let toml = format!(
      "[hub]\nname = \"t\"\nstart = \"s\"\n{}\n[[widget]]\nid = \"w\"\ntype = \"list\"\n\
       source = \"slow\"\nitem = \"{{line}}\"\n[[screen]]\nid = \"s\"\ntitle = \"S\"\n\
       key = \"1\"\nlayout = \"w\"\n",
      body
    );
    std::fs::write(dir.join(".hub.toml"), toml).unwrap();
    let loaded = crate::config::load(&dir.join(".hub.toml")).unwrap();
    (dir, Arc::new(loaded))
  }

  #[cfg(unix)]
  #[test]
  fn test_a_cancelled_command_stops_with_what_it_started() {
    let dir = std::env::temp_dir().join(format!("hub-cancel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    std::thread::spawn(move || {
      std::thread::sleep(Duration::from_millis(300));
      stop.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let said = run_cancellable(
      &dir,
      &words("sh -c 'sleep 30 & echo $! > pid; wait'"),
      None,
      &[],
      Duration::from_secs(60),
      &[0],
      Some(&cancel),
    );
    assert!(said.unwrap_err().contains("was cancelled"));
    assert!(started.elapsed() < Duration::from_secs(5));
    // The sleep the command started went with it.
    let pid: libc::pid_t = std::fs::read_to_string(dir.join("pid"))
      .unwrap()
      .trim()
      .parse()
      .unwrap();
    let gone = (0..100).any(|_| {
      std::thread::sleep(Duration::from_millis(20));
      // SAFETY: signal 0 only asks whether the process is there.
      (unsafe { libc::kill(pid, 0) }) != 0
    });
    assert!(gone, "the command's child {} still runs", pid);
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn test_a_command_tree_gives_its_runnable_commands() {
    let tree: serde_json::Value = serde_json::from_str(
      r#"{"name": "x", "commands": [
        {"name": "fuzz", "subcommand_required": true, "commands": [
          {"name": "run", "about": "Fuzz", "args": [{"id": "target", "required": true}]}]},
        {"name": "status", "about": "Where it stands", "args": []}]}"#,
    )
    .unwrap();
    let mut rows = Vec::new();
    command_rows(&tree, &mut Vec::new(), &mut rows);
    let paths: Vec<String> = rows.iter().map(|r| r["path"].text()).collect();
    assert_eq!(paths, ["fuzz run", "status"]);
    assert!(matches!(&rows[0]["args"], Value::Rows(args) if args.len() == 1));
  }

  #[cfg(unix)]
  #[test]
  fn test_a_slow_source_loads_in_the_background() {
    let (dir, loaded) = config(
      "slow",
      "[[source]]\nname = \"slow\"\nkind = \"command\"\nrun = \"sh -c 'sleep 0.3; echo row'\"\n\
       parse = \"lines\"\nrefresh = \"1h\"",
    );
    let mut store = Store::background(loaded.clone());
    let asked = Instant::now();
    let data = store.get(&loaded, "slow", false);
    // Answered at once, with nothing yet and the load under way.
    assert!(asked.elapsed() < Duration::from_millis(200));
    assert!(data.rows.is_empty() && data.loading.is_some() && data.updated.is_none());
    assert!(store.busy());
    while !store.receive() {
      assert!(
        asked.elapsed() < Duration::from_secs(10),
        "the load never ended"
      );
      std::thread::sleep(Duration::from_millis(20));
    }
    let data = store.get(&loaded, "slow", false);
    assert_eq!(data.rows.len(), 1);
    assert_eq!(data.rows[0]["line"].text(), "row");
    assert!(data.updated.is_some() && data.loading.is_none() && !data.stale());
    assert!(!store.busy());
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// A source with `page` gives more rows once its rows fill its limit and are asked
  /// for to the last.
  #[test]
  fn test_a_source_pages_when_its_rows_fill_its_limit() {
    let repo = crate::git::tests::Repo::new("page");
    for n in 0..5 {
      repo.write("f.txt", &format!("{}\n", n));
      repo.git(&["add", "."]);
      repo.git(&["commit", "-qm", &format!("c{}", n)]);
    }
    std::fs::write(
      repo.0.join(".hub.toml"),
      "[hub]\nname = \"t\"\nstart = \"s\"\n\
       [[source]]\nname = \"log\"\nkind = \"git\"\nof = \"log\"\nlimit = 2\npage = 2\n\
       [[widget]]\nid = \"w\"\ntype = \"list\"\nsource = \"log\"\nitem = \"{subject}\"\n\
       [[screen]]\nid = \"s\"\ntitle = \"S\"\nkey = \"1\"\nlayout = \"w\"\n",
    )
    .unwrap();
    let loaded = crate::config::load(&repo.0.join(".hub.toml")).unwrap();
    let mut store = Store::new();
    assert_eq!(store.get(&loaded, "log", false).rows.len(), 2);
    assert!(store.grow(&loaded, "log", None));
    assert_eq!(store.get(&loaded, "log", false).rows.len(), 4);
    assert!(store.grow(&loaded, "log", None));
    // Five is all there is: it does not fill six, so it grows no more.
    assert_eq!(store.get(&loaded, "log", false).rows.len(), 5);
    assert!(!store.grow(&loaded, "log", None));
  }

  /// A stream's rows come as its program prints them, kept while it runs; drawn once,
  /// what it prints in its first second.
  #[cfg(unix)]
  #[test]
  fn test_a_stream_gives_rows_as_they_come() {
    let (dir, loaded) = config(
      "stream",
      "[[source]]\nname = \"slow\"\nkind = \"stream\"\n\
       run = \"sh -c 'while read l; do echo $l; sleep 0.1; done < lines.json; sleep 5'\"\n\
       computed = { twice = \"line * 2\" }",
    );
    std::fs::write(
      dir.join("lines.json"),
      "{\"line\": 1}\n{\"line\": 2}\n{\"line\": 3}\n",
    )
    .unwrap();
    let mut store = Store::background(loaded.clone());
    let started = Instant::now();
    loop {
      let rows = store.get(&loaded, "slow", false).rows.clone();
      if rows.len() == 3 {
        assert_eq!(rows[2]["line"], Value::Num(3.0));
        assert_eq!(rows[2]["twice"], Value::Num(6.0));
        break;
      }
      assert!(started.elapsed() < Duration::from_secs(5), "{:?}", rows);
      std::thread::sleep(Duration::from_millis(50));
    }
    let mut once = Store::new();
    assert_eq!(once.get(&loaded, "slow", false).rows.len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn test_a_tail_reads_only_what_was_added() {
    let dir = std::env::temp_dir().join(format!("hub-tail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("run.log");
    std::fs::write(&path, "one\n").unwrap();
    let mut store = Store::new();
    assert_eq!(store.tail(&path).unwrap(), "one\n");
    {
      use std::io::Write;
      let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
      file.write_all(b"two\n").unwrap();
    }
    assert_eq!(store.tail(&path).unwrap(), "one\ntwo\n");
    // Shorter than before: replaced, so read again whole.
    std::fs::write(&path, "new\n").unwrap();
    assert_eq!(store.tail(&path).unwrap(), "new\n");
    assert!(store.tail(&dir.join("missing.log")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn test_an_http_source_reads_a_url_with_headers() {
    let dir = std::env::temp_dir().join(format!("hub-http-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("runs.json"), "[{\"id\": 1}, {\"id\": 2}]").unwrap();
    let (cfg, loaded) = config(
      "http",
      &format!(
        "[[source]]\nname = \"slow\"\nkind = \"http\"\nurl = \"file://{}\"\n\
         parse = \"json\"\nheaders = [\"X-Who: $HUB_NOBODY_SET_THIS\"]",
        dir.join("runs.json").display()
      ),
    );
    let mut store = Store::new();
    let data = store.get(&loaded, "slow", false);
    assert_eq!(data.error, None);
    assert_eq!(data.rows.len(), 2);
    assert_eq!(environment("a $HUB_NOBODY_SET_THIS b $"), "a  b $");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cfg);
  }

  #[test]
  fn test_durations_and_words() {
    assert_eq!(duration("500ms"), Some(Duration::from_millis(500)));
    assert_eq!(duration("5m"), Some(Duration::from_secs(300)));
    assert_eq!(duration("soon"), None);
    assert_eq!(
      words("cargo xtask runs cancel \"a b\" ''"),
      ["cargo", "xtask", "runs", "cancel", "a b", ""]
    );
  }

  #[test]
  fn test_grouping_collects_lists_and_keeps_last_values() {
    let row = |name: &str, value: f64| {
      Row::from([
        ("_name".to_string(), Value::Str(name.into())),
        ("value".to_string(), Value::Num(value)),
      ])
    };
    let groups = grouped(
      vec![row("a", 1.0), row("b", 5.0), row("a", 2.0)],
      "_name",
      &["value".to_string()],
      &["value".to_string()],
    );
    assert_eq!(groups.len(), 2);
    assert_eq!(
      groups[0]["value_list"],
      Value::List(vec![Value::Num(1.0), Value::Num(2.0)])
    );
    assert_eq!(groups[0]["value"], Value::Num(2.0));
  }
}
