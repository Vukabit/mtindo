//! Layer 1: the record of every run that wrote files, which `mto restore` undoes.
//!
//! History lives in one folder:
//!
//! - `runs/ID.run`: one text record per run, named by its UUIDv7 id. It names the command,
//!   the folder it ran in, and each file written, with the SHA-256 of its contents before
//!   and after.
//! - `objects/AB/CDEF...`: file contents, named by their SHA-256, each stored once however
//!   many runs share it.
//!
//! A record is appended to as files are written, so a run cut short still records what it
//! did; a line cut short is ignored when read. Contents are stored, and synced, before the
//! file they belong to is replaced. Pruning removes whole runs, then the contents no remaining
//! run uses. Runs that happen at once stay apart: each has its own record, and contents are
//! written under a temporary name and renamed into place.

pub mod git;
pub mod id;
pub mod sha256;

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Pruning never removes contents or temporary files younger than this: a run going on at
/// the same time may have stored them without recording them yet.
const GRACE: Duration = Duration::from_secs(10 * 60);

/// The first line of every record.
const MAGIC: &str = "mto history 1";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Where a file a run wrote stands now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// It holds what the run wrote.
    AsWritten,
    /// It holds what it held before the run.
    AsBefore,
    /// It was changed since.
    Changed,
    Missing,
}

/// One file a run wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    /// The SHA-256 of the contents before the run.
    pub before: String,
    /// The SHA-256 of what the run wrote.
    pub after: String,
}

/// The folder history is kept in.
#[derive(Clone, Debug)]
pub struct History {
    dir: PathBuf,
}

impl History {
    pub fn new(dir: PathBuf) -> History {
        History { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every run id, oldest first.
    pub fn ids(&self) -> io::Result<Vec<String>> {
        let mut ids = Vec::new();
        let entries = match fs::read_dir(self.dir.join("runs")) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ids),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let name = entry?.file_name().to_string_lossy().to_string();
            if let Some(id) = name.strip_suffix(".run") {
                ids.push(id.to_string());
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// The contents stored under `hash`.
    pub fn load(&self, hash: &str) -> io::Result<Vec<u8>> {
        fs::read(self.object_path(hash))
    }

    /// Removes the runs `limits` does not keep (always keeping the newest), then the
    /// contents no remaining run uses. Contents are looked through only when a run was
    /// removed, or with `sweep` (which also finds what a run cut short left behind), since
    /// otherwise none can have lost its last use.
    pub fn prune(&self, limits: Limits, sweep: bool) -> io::Result<Pruned> {
        let now = now_millis();
        let ids = self.ids()?;
        let mut runs = Vec::new();
        for id in ids.iter().rev() {
            if let Ok(run) = self.read(id) {
                runs.push(run);
            }
        }
        // Newest first: keep while within the count and the age.
        let mut kept: Vec<&Run> = Vec::new();
        let mut removed: Vec<&str> = Vec::new();
        for (i, run) in runs.iter().enumerate() {
            let age_days = now.saturating_sub(id::millis(&run.id)) / 86_400_000;
            let too_many = limits.keep_runs > 0 && i >= limits.keep_runs;
            let too_old = limits.keep_days > 0 && age_days >= limits.keep_days;
            if i > 0 && (too_many || too_old) {
                removed.push(&run.id);
            } else {
                kept.push(run);
            }
        }
        if limits.max_bytes > 0 {
            let mut uses: HashMap<&str, usize> = HashMap::new();
            for run in &kept {
                for hash in run.hashes() {
                    *uses.entry(hash).or_default() += 1;
                }
            }
            let size = |hash: &str| {
                fs::metadata(self.object_path(hash))
                    .map(|m| m.len())
                    .unwrap_or(0)
            };
            let mut total: u64 = uses.keys().map(|hash| size(hash)).sum();
            while total > limits.max_bytes && kept.len() > 1 {
                let oldest = kept.pop().expect("more than one run");
                for hash in oldest.hashes() {
                    let count = uses.get_mut(hash).expect("counted");
                    *count -= 1;
                    if *count == 0 {
                        total = total.saturating_sub(size(hash));
                    }
                }
                removed.push(&oldest.id);
            }
        }
        let mut pruned = Pruned::default();
        for id in &removed {
            let path = self.run_path(id);
            pruned.bytes += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            fs::remove_file(path)?;
            pruned.runs += 1;
        }
        // A record that could not be read may use any contents, so none are removed.
        let unreadable = ids.iter().any(|id| !runs.iter().any(|run| &run.id == id));
        if unreadable || (removed.is_empty() && !sweep) {
            return Ok(pruned);
        }
        let used: HashSet<&str> = kept.iter().flat_map(|run| run.hashes()).collect();
        let objects = self.dir.join("objects");
        let Ok(shards) = fs::read_dir(&objects) else {
            return Ok(pruned);
        };
        for shard in shards {
            let shard = shard?;
            let prefix = shard.file_name().to_string_lossy().to_string();
            let Ok(files) = fs::read_dir(shard.path()) else {
                continue;
            };
            for file in files {
                let file = file?;
                let name = file.file_name().to_string_lossy().to_string();
                let hash = format!("{}{}", prefix, name);
                if used.contains(hash.as_str()) {
                    continue;
                }
                let meta = file.metadata()?;
                let young = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.elapsed().ok())
                    .is_none_or(|age| age < GRACE);
                if young {
                    continue;
                }
                fs::remove_file(file.path())?;
                pruned.objects += 1;
                pruned.bytes += meta.len();
            }
            // Empty shards go too; one still in use stays.
            let _ = fs::remove_dir(shard.path());
        }
        Ok(pruned)
    }

    /// A run's record.
    pub fn read(&self, id: &str) -> io::Result<Run> {
        let text = fs::read_to_string(self.run_path(id))?;
        parse(&text).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a run record", self.run_path(id).display()),
            )
        })
    }

    /// Starts recording a run. Nothing is written until its first file.
    pub fn recorder(&self, command: &str, cwd: &Path, restore_of: Option<&str>) -> Recorder {
        let id = id::new();
        let mut header = format!(
            "{}\nid {}\ncommand {}\ncwd {}\n",
            MAGIC,
            id,
            escape(command),
            escape(&cwd.to_string_lossy())
        );
        if let Some(of) = restore_of {
            header.push_str(&format!("restore-of {}\n", of));
        }
        Recorder {
            history: self.clone(),
            id,
            header,
            file: None,
            count: 0,
        }
    }

    /// The run `spec` names: `last`, `last~N`, a whole id, or the end of one (a handle).
    pub fn resolve(&self, spec: &str) -> Result<String, String> {
        let ids = self
            .ids()
            .map_err(|e| format!("cannot read history: {}", e))?;
        if ids.is_empty() {
            return Err("history is empty: no run has written files yet".into());
        }
        let back = match spec {
            "last" => Some(0),
            _ => spec
                .strip_prefix("last~")
                .map(|n| {
                    n.parse::<usize>()
                        .map_err(|_| format!("`{}` is not last~N", spec))
                })
                .transpose()?,
        };
        if let Some(back) = back {
            return ids.iter().rev().nth(back).cloned().ok_or_else(|| {
                format!(
                    "history holds {} run(s); `{}` goes back too far",
                    ids.len(),
                    spec
                )
            });
        }
        let wanted = id::hex_digits(spec);
        if wanted.is_empty() || !wanted.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "`{}` is not a run: give last, last~N, or a handle from `mto history`",
                spec
            ));
        }
        let matches: Vec<&String> = ids
            .iter()
            .filter(|id| id::hex_digits(id).ends_with(&wanted))
            .collect();
        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(format!("no run ends with `{}`; see `mto history`", spec)),
            many => Err(format!(
                "`{}` names {} runs; give more of the handle: {}",
                spec,
                many.len(),
                many.iter()
                    .map(|id| id::handle(id, &ids))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Stores `contents` (once), synced, and returns its SHA-256. Contents already stored
    /// are touched, so a concurrent prune leaves them be.
    pub fn store(&self, contents: &[u8]) -> io::Result<String> {
        let hash = sha256::hex(contents);
        let path = self.object_path(&hash);
        if path.is_file() {
            if let Ok(file) = OpenOptions::new().write(true).open(&path) {
                let _ = file.set_modified(SystemTime::now());
            }
            return Ok(hash);
        }
        let parent = path.parent().expect("objects live in a shard");
        fs::create_dir_all(parent)?;
        let temp = parent.join(format!(
            ".{}.{}.{}.tmp",
            &hash[2..],
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = File::create(&temp)?;
            file.write_all(contents)?;
            file.sync_all()?;
            fs::rename(&temp, &path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map(|_| hash)
    }

    fn object_path(&self, hash: &str) -> PathBuf {
        let split = hash.len().min(2);
        self.dir
            .join("objects")
            .join(&hash[..split])
            .join(&hash[split..])
    }

    fn run_path(&self, id: &str) -> PathBuf {
        self.dir.join("runs").join(format!("{}.run", id))
    }
}

/// How much history to keep. Zero means no limit.
#[derive(Clone, Copy, Debug, Default)]
pub struct Limits {
    pub keep_days: u64,
    pub keep_runs: usize,
    pub max_bytes: u64,
}

/// What a prune removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub runs: usize,
    pub objects: usize,
    pub bytes: u64,
}

/// Writes one run's record, line by line, as its files are written.
#[derive(Debug)]
pub struct Recorder {
    history: History,
    id: String,
    header: String,
    file: Option<File>,
    count: usize,
}

impl Recorder {
    /// The files recorded so far.
    pub fn count(&self) -> usize {
        self.count
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Adds a written file to the record, creating the record with the first one.
    pub fn record(&mut self, entry: &Entry) -> io::Result<()> {
        if self.file.is_none() {
            let runs = self.history.dir.join("runs");
            fs::create_dir_all(&runs)?;
            let mut file = OpenOptions::new()
                .append(true)
                .create_new(true)
                .open(self.history.run_path(&self.id))?;
            file.write_all(self.header.as_bytes())?;
            self.file = Some(file);
        }
        let line = format!(
            "file {} {} {}\n",
            entry.before,
            entry.after,
            escape(&entry.path.to_string_lossy())
        );
        self.file
            .as_mut()
            .expect("opened above")
            .write_all(line.as_bytes())?;
        self.count += 1;
        Ok(())
    }

    /// Syncs the record. `true` when anything was recorded.
    pub fn finish(&mut self) -> io::Result<bool> {
        match &self.file {
            Some(file) => file.sync_all().map(|_| true),
            None => Ok(false),
        }
    }
}

/// A run, read back from its record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub id: String,
    pub command: String,
    pub cwd: PathBuf,
    /// The run this one restored, for `mto restore`.
    pub restore_of: Option<String>,
    pub entries: Vec<Entry>,
}

impl Run {
    /// Every content hash the run uses.
    pub fn hashes(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .flat_map(|e| [e.before.as_str(), e.after.as_str()])
    }

    /// Where `entry` stands now.
    pub fn state(entry: &Entry) -> State {
        match fs::read(&entry.path) {
            Ok(contents) => {
                let hash = sha256::hex(&contents);
                if hash == entry.after {
                    State::AsWritten
                } else if hash == entry.before {
                    State::AsBefore
                } else {
                    State::Changed
                }
            }
            Err(_) => State::Missing,
        }
    }
}

/// Milliseconds since the Unix epoch.
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `text` with backslashes, line feeds and carriage returns escaped, so it fits on a line.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Reads a record. Lines it does not know are skipped, and so is a last line cut short.
fn parse(text: &str) -> Option<Run> {
    let mut lines = text.split_inclusive('\n');
    if lines.next()?.trim_end() != MAGIC {
        return None;
    }
    let mut run = Run {
        id: String::new(),
        command: String::new(),
        cwd: PathBuf::new(),
        restore_of: None,
        entries: Vec::new(),
    };
    for line in lines {
        let Some(line) = line.strip_suffix('\n') else {
            break;
        };
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "id" => run.id = rest.to_string(),
            "command" => run.command = unescape(rest),
            "cwd" => run.cwd = PathBuf::from(unescape(rest)),
            "restore-of" => run.restore_of = Some(rest.to_string()),
            "file" => {
                let mut parts = rest.splitn(3, ' ');
                let (Some(before), Some(after), Some(path)) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    continue;
                };
                run.entries.push(Entry {
                    path: PathBuf::from(unescape(path)),
                    before: before.to_string(),
                    after: after.to_string(),
                });
            }
            _ => {}
        }
    }
    (!run.id.is_empty()).then_some(run)
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests;
