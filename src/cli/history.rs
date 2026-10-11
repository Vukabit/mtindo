//! `mto history` and `mto restore`, and what a run that writes files records: the user
//! config's history settings read once, the record finished and pruned at the end.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{Resolver, user};
use crate::history::{self, Entry, History, Limits, Run, State, id};
use crate::options::user::bytes;
use crate::{c_green, c_red, c_reset, c_yellow, write_atomic};

/// What `mto history prune` was asked to keep, over the user config.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PruneArgs {
    pub older_than_days: Option<u64>,
    pub keep: Option<usize>,
}

/// The user's history settings, with the command line's `--no-history` applied.
#[derive(Clone, Debug)]
pub struct Settings {
    /// `None` when history is off, or there is no folder for it.
    pub history: Option<History>,
    pub limits: Limits,
    /// Why history is off although the settings ask for it.
    pub note: Option<String>,
    /// `safety.allow_dirty`.
    pub allow_dirty: bool,
}

impl Settings {
    pub fn read(resolver: &Resolver, no_history: bool) -> Result<Settings, String> {
        let flag = |name: &str| resolver.user_value(name).and_then(|v| v.as_bool());
        let number = |name: &str| {
            resolver
                .user_value(name)
                .and_then(|v| v.as_integer())
                .unwrap_or(0)
        };
        let max_size = resolver
            .user_value("history.max_size")
            .and_then(|v| v.as_text().map(str::to_string))
            .unwrap_or_default();
        let limits = Limits {
            keep_days: number("history.keep_days"),
            keep_runs: number("history.keep_runs") as usize,
            max_bytes: bytes(&max_size)
                .ok_or_else(|| format!("history.max_size: `{}` is not a size", max_size))?,
        };
        let enabled = flag("history.enabled").unwrap_or(true) && !no_history;
        let dir = user::history_dir(resolver.env());
        let note = (enabled && dir.is_none())
            .then(|| "history is off: no home folder to keep it in".to_string());
        Ok(Settings {
            history: dir.filter(|_| enabled).map(History::new),
            limits,
            note,
            allow_dirty: flag("safety.allow_dirty").unwrap_or(false),
        })
    }
}

/// The command line a run is recorded with: `$MTINDO_COMMAND` (which `cargo mto` sets to
/// its own), else this process's arguments.
pub fn command_line() -> String {
    if let Some(command) = std::env::var_os("MTINDO_COMMAND").filter(|c| !c.is_empty()) {
        return command.to_string_lossy().to_string();
    }
    let mut words = vec!["mto".to_string()];
    for arg in std::env::args().skip(1) {
        if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || c == '\'') {
            words.push(format!("'{}'", arg.replace('\'', "'\\''")));
        } else {
            words.push(arg);
        }
    }
    words.join(" ")
}

/// Days in a duration such as `30d`, `2w`, `12h` or `30` (days); hours round up to whole
/// days.
pub fn days(text: &str) -> Option<u64> {
    let text = text.trim();
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let number: u64 = text[..digits].parse().ok()?;
    match &text[digits..] {
        "" | "d" => Some(number),
        "w" => number.checked_mul(7),
        "h" => Some(number.div_ceil(24)),
        _ => None,
    }
}

/// Ends a recorded run: syncs the record, says how to undo it, and prunes.
pub fn finish_record(recorder: &mut history::Recorder, limits: Limits, quiet: bool) {
    match recorder.finish() {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            eprintln!(
                "{}warning: cannot finish the history record: {}{}",
                c_yellow(true),
                e,
                c_reset(true)
            );
            return;
        }
    }
    if !quiet {
        let ids = recorder.history().ids().unwrap_or_default();
        let handle = id::handle(recorder.id(), &ids);
        eprintln!(
            "Recorded as run {}; `mto restore {}` undoes it.",
            handle, handle
        );
    }
    if let Err(e) = recorder.history().prune(limits, false) {
        eprintln!(
            "{}warning: cannot prune history: {}{}",
            c_yellow(true),
            e,
            c_reset(true)
        );
    }
}

/// `mto history`: the runs, newest first. With `here`, only the runs that wrote files in
/// that folder's project (see [`project_root`]), or every run with a note when there are
/// none; without it, every run.
pub fn list(history: &History, here: Option<&Path>) -> Result<String, String> {
    let ids = history
        .ids()
        .map_err(|e| format!("cannot read history: {}", e))?;
    if ids.is_empty() {
        return Ok("No runs recorded yet.\n".to_string());
    }
    let mut roots: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut runs = Vec::new();
    for id in ids.iter().rev() {
        let run = history.read(id);
        // The project its files are in; for files in several, the folder it ran in's.
        let project = run.as_ref().ok().map(|run| {
            let mut root_of = |dir: &Path| {
                roots
                    .entry(dir.to_path_buf())
                    .or_insert_with(|| project_root(dir))
                    .clone()
            };
            let found: HashSet<PathBuf> = run
                .entries
                .iter()
                .filter_map(|e| e.path.parent())
                .map(&mut root_of)
                .collect();
            match found.len() {
                1 => found.into_iter().next().unwrap_or_default(),
                _ => root_of(&run.cwd),
            }
        });
        runs.push((id, run, project));
    }
    let mut note = String::new();
    if let Some(here) = here {
        let root = project_root(here);
        let inside = |run: &Run| run.entries.iter().any(|e| is_under(&e.path, &root));
        if runs
            .iter()
            .any(|(_, run, _)| run.as_ref().is_ok_and(inside))
        {
            runs.retain(|(_, run, _)| run.as_ref().is_ok_and(inside));
        } else {
            note = format!(
                "No runs wrote files in {}; listing every run.\n\n",
                root.display()
            );
        }
    }
    let name = |project: &Option<PathBuf>| {
        project
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| shorten(&n.to_string_lossy(), 20))
            .unwrap_or_default()
    };
    // Cells: run, when, files, project, then the command, which is not padded.
    let mut table = vec![[
        "RUN".to_string(),
        "WHEN (UTC)".to_string(),
        "FILES".to_string(),
        "PROJECT".to_string(),
        "COMMAND".to_string(),
    ]];
    for (id, run, project) in &runs {
        let handle = id::handle(id, &ids);
        let when = utc(id::millis(id));
        table.push(match run {
            Ok(run) => {
                let restores = run
                    .restore_of
                    .as_ref()
                    .map(|of| format!(" (restores {})", id::handle(of, &ids)))
                    .unwrap_or_default();
                [
                    handle,
                    when,
                    run.entries.len().to_string(),
                    name(project),
                    format!("{}{}", shorten(&run.command, 60), restores),
                ]
            }
            Err(e) => [
                handle,
                when,
                String::new(),
                String::new(),
                format!("unreadable: {}", e),
            ],
        });
    }
    let width = |column: usize| {
        table
            .iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    // The file count right-aligned, its noun after it: "   1 file", "8754 files".
    let digits = table[1..].iter().map(|row| row[2].len()).max().unwrap_or(0);
    let files = |count: &str| match count {
        "" | "FILES" => count.to_string(),
        "1" => format!("{:>digits$} file", count),
        _ => format!("{:>digits$} files", count),
    };
    let widths = [
        width(0),
        width(1),
        (digits + " files".len()).max("FILES".len()),
        width(3),
    ];
    let mut out = note;
    for [run, when, count, project, command] in &table {
        out.push_str(&format!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}\n",
            run,
            when,
            files(count),
            project,
            command,
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3]
        ));
    }
    Ok(out)
}

/// The project a folder belongs to: the nearest folder at or above it holding `.git`, else
/// the nearest holding `Cargo.toml`, else the folder itself.
pub fn project_root(dir: &Path) -> PathBuf {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let nearest = |marker: &str| dir.ancestors().find(|a| a.join(marker).exists());
    nearest(".git")
        .or_else(|| nearest("Cargo.toml"))
        .unwrap_or(&dir)
        .to_path_buf()
}

/// `mto history prune`: the user config's limits, with the command line's over them.
pub fn prune(history: &History, mut limits: Limits, args: PruneArgs) -> Result<String, String> {
    if let Some(days) = args.older_than_days {
        limits.keep_days = days;
    }
    if let Some(keep) = args.keep {
        limits.keep_runs = keep;
    }
    let pruned = history
        .prune(limits, true)
        .map_err(|e| format!("cannot prune history: {}", e))?;
    Ok(format!(
        "Removed {} run(s) and {} stored file(s), freeing {}.\n",
        pruned.runs,
        pruned.objects,
        size(pruned.bytes)
    ))
}

/// `mto restore`: puts back what a run replaced, in the files that still hold what it wrote
/// (any file with `force`), and records that as a run of its own. Returns the exit code.
pub fn restore(
    settings: &Settings,
    history: &History,
    spec: &str,
    only: &[PathBuf],
    force: bool,
) -> Result<i32, String> {
    let id = resolve_here(history, spec)?;
    let run = history.read(&id).map_err(|e| e.to_string())?;
    let ids = history.ids().unwrap_or_default();
    let handle = id::handle(&id, &ids);
    let wanted = chosen(&run, only, &handle)?;
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut recorder = settings
        .history
        .as_ref()
        .map(|h| h.recorder(&command_line(), &cwd, Some(&id)));
    let mut skipped = 0;
    for entry in run.entries.iter().filter(|e| wanted.contains(&e.path)) {
        let shown = entry.path.display();
        match (Run::state(entry), force) {
            (State::AsBefore, _) => {
                println!("{} already holds what it held before run {}", shown, handle);
                continue;
            }
            (State::AsWritten, _) | (_, true) => {}
            (State::Changed, false) => {
                eprintln!(
                    "{}Skipped {}: changed since run {} wrote it (--force restores it \
                     anyway){}",
                    c_yellow(true),
                    shown,
                    handle,
                    c_reset(true)
                );
                skipped += 1;
                continue;
            }
            (State::Missing, false) => {
                eprintln!(
                    "{}Skipped {}: it no longer exists (--force recreates it){}",
                    c_yellow(true),
                    shown,
                    c_reset(true)
                );
                skipped += 1;
                continue;
            }
        }
        let before = history
            .load(&entry.before)
            .map_err(|e| format!("run {} lost {}'s contents: {}", handle, shown, e))?;
        let before = String::from_utf8(before)
            .map_err(|_| format!("run {} holds {} as text that is not UTF-8", handle, shown))?;
        // The file as it is now, so this restore can be undone too.
        let now = fs::read(&entry.path).ok();
        let record = match (&recorder, &now) {
            (Some(recorder), Some(now)) => {
                let now_hash = recorder
                    .history()
                    .store(now)
                    .map_err(|e| format!("cannot record {} in history: {}", shown, e))?;
                Some(Entry {
                    path: entry.path.clone(),
                    before: now_hash,
                    after: entry.before.clone(),
                })
            }
            _ => None,
        };
        if let Some(parent) = entry.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if let Err(e) = write_atomic(&entry.path, &before) {
            eprintln!("{}Cannot write {}: {}{}", c_red(), shown, e, c_reset(true));
            return Ok(3);
        }
        println!("{}Restored{} {}", c_green(), c_reset(false), shown);
        if let (Some(recorder), Some(record)) = (recorder.as_mut(), record)
            && let Err(e) = recorder.record(&record)
        {
            eprintln!(
                "{}warning: cannot add {} to the history record: {}{}",
                c_yellow(true),
                shown,
                e,
                c_reset(true)
            );
        }
    }
    if let Some(recorder) = recorder.as_mut() {
        finish_record(recorder, settings.limits, false);
    }
    Ok(if skipped > 0 { 1 } else { 0 })
}

/// `mto history show ID`: a run, and where each of its files stands now.
pub fn show(history: &History, spec: &str) -> Result<String, String> {
    let id = resolve_here(history, spec)?;
    let run = history.read(&id).map_err(|e| e.to_string())?;
    let ids = history.ids().unwrap_or_default();
    let mut out = format!(
        "run      {} ({})\nwhen     {} UTC\ncommand  {}\nin       {}\n",
        id::handle(&id, &ids),
        id,
        utc(id::millis(&id)),
        run.command,
        run.cwd.display()
    );
    if let Some(of) = &run.restore_of {
        out.push_str(&format!("restores {}\n", id::handle(of, &ids)));
    }
    out.push_str(&format!("{}:\n", files(run.entries.len())));
    for entry in &run.entries {
        let state = match Run::state(entry) {
            State::AsWritten => "as written; restore puts back what it replaced",
            State::AsBefore => "holds what it held before",
            State::Changed => "changed since; restore skips it without --force",
            State::Missing => "missing; restore skips it without --force",
        };
        out.push_str(&format!("  {}  ({})\n", entry.path.display(), state));
    }
    Ok(out)
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, for milliseconds since the Unix epoch.
pub fn utc(millis: u64) -> String {
    let secs = millis / 1000;
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days to a civil date (Howard Hinnant's algorithm), from 1970-01-01.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year,
        month,
        day,
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The run's files that `only` names (all of them when it is empty).
fn chosen(run: &Run, only: &[PathBuf], handle: &str) -> Result<HashSet<PathBuf>, String> {
    if only.is_empty() {
        return Ok(run.entries.iter().map(|e| e.path.clone()).collect());
    }
    let mut wanted = HashSet::new();
    for path in only {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        let real = fs::canonicalize(path).ok();
        let found = run.entries.iter().find(|e| {
            e.path == absolute
                || real
                    .as_ref()
                    .is_some_and(|r| fs::canonicalize(&e.path).ok().as_ref() == Some(r))
        });
        match found {
            Some(entry) => {
                wanted.insert(entry.path.clone());
            }
            None => {
                return Err(format!(
                    "run {} did not write {}; `mto history show {}` lists its files",
                    handle,
                    path.display(),
                    handle
                ));
            }
        }
    }
    Ok(wanted)
}

fn files(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{} files", count)
    }
}

/// Whether `path` lies in `root`, as written or with symlinks resolved (a record holds paths
/// as they were given, which may differ from the folder's resolved path).
fn is_under(path: &Path, root: &Path) -> bool {
    if path.starts_with(root) {
        return true;
    }
    let resolved = |p: &Path| fs::canonicalize(p).ok();
    let path = path
        .parent()
        .and_then(resolved)
        .map(|parent| parent.join(path.file_name().unwrap_or_default()));
    matches!((path, resolved(root)), (Some(path), Some(root)) if path.starts_with(&root))
}

/// A run named on the command line: `last` and `last~N` count the runs of the current
/// folder's project, as `mto history` lists them, so a restore never reaches into another
/// project; a handle names its run wherever mto runs.
fn resolve_here(history: &History, spec: &str) -> Result<String, String> {
    let Some(back) = history::steps_back(spec)? else {
        return history.resolve(spec);
    };
    let here =
        std::env::current_dir().map_err(|e| format!("cannot read the current folder: {}", e))?;
    let root = project_root(&here);
    let ids = history
        .ids()
        .map_err(|e| format!("cannot read history: {}", e))?;
    if ids.is_empty() {
        // Says the history is empty.
        return history.resolve(spec);
    }
    let mine: Vec<&String> = ids
        .iter()
        .rev()
        .filter(|id| {
            history
                .read(id)
                .is_ok_and(|run| run.entries.iter().any(|e| is_under(&e.path, &root)))
        })
        .collect();
    if mine.is_empty() {
        return Err(format!(
            "no run wrote files in {}; name one by its handle from `mto history --all`",
            root.display()
        ));
    }
    mine.get(back).map(|id| (*id).clone()).ok_or_else(|| {
        format!(
            "{} run(s) wrote files in {}; `{}` goes back too far",
            mine.len(),
            root.display(),
            spec
        )
    })
}

/// `text` cut to `width` characters, with `...` when cut.
fn shorten(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let cut: String = text.chars().take(width.saturating_sub(3)).collect();
    format!("{}...", cut)
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{} bytes", b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_durations_and_sizes() {
        assert_eq!(days("30"), Some(30));
        assert_eq!(days("2w"), Some(14));
        assert_eq!(days("25h"), Some(2));
        assert_eq!(days("3y"), None);
        assert_eq!(bytes("256MB"), Some(256 << 20));
        assert_eq!(bytes("0"), Some(0));
        assert_eq!(bytes("1 gb"), Some(1 << 30));
        assert_eq!(bytes("MB"), None);
        assert_eq!(shorten("abcdef", 5), "ab...");
    }

    #[test]
    fn test_utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00:00");
        assert_eq!(utc(951_782_400_000), "2000-02-29 00:00:00");
        assert_eq!(utc(1_760_111_236_000), "2025-10-10 15:47:16");
    }
}
