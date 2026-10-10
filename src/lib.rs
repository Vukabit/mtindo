pub mod cli;
pub mod config;
pub mod layout;
pub mod options;
pub mod syntax;
pub mod text;
pub mod verify;

use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub use layout::import_group_of_text as classify_import;
pub use options::layout::{ImportGroup, ItemKind, LayoutConfig, Visibility};
pub use text::color::{HelpPalette, color_enabled};

const DIFF_CONTEXT: usize = 3;

const DIFF_TABLE_LIMIT: usize = 4_000_000;

static UNIQUE_COUNTER: AtomicUsize = AtomicUsize::new(0);

enum DiffOp {
    Equal,
    Delete(usize),
    Insert(usize),
}

/// Errors that end a run. Each maps to a process exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MtoError {
    /// Bad arguments or configuration (exit code 2).
    Usage(String),
    /// A read or write failed (exit code 3).
    Io(String),
}

impl fmt::Display for MtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MtoError::Usage(msg) | MtoError::Io(msg) => write!(f, "{}", msg),
        }
    }
}

impl From<config::Error> for MtoError {
    fn from(error: config::Error) -> MtoError {
        MtoError::Usage(error.message)
    }
}

impl MtoError {
    pub fn exit_code(&self) -> i32 {
        match self {
            MtoError::Usage(_) => 2,
            MtoError::Io(_) => 3,
        }
    }
}

/// What happened to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Unchanged,
    Changed,
    /// Left untouched because it could not be split into items safely.
    Skipped,
    Failed,
    /// Left untouched because the verifier found the new text unsafe: a bug in mto.
    VerifierFailed,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions {
    pub dry_run: bool,
    pub check: bool,
    pub debug: bool,
    pub follow_links: bool,
}

pub fn c_cyan() -> &'static str {
    if color_enabled(false) {
        "\x1b[1;36m"
    } else {
        ""
    }
}

pub fn c_green() -> &'static str {
    if color_enabled(false) {
        "\x1b[1;32m"
    } else {
        ""
    }
}

pub fn c_red() -> &'static str {
    if color_enabled(true) {
        "\x1b[1;31m"
    } else {
        ""
    }
}

pub fn c_reset(is_err: bool) -> &'static str {
    if color_enabled(is_err) { "\x1b[0m" } else { "" }
}

pub fn c_yellow(is_err: bool) -> &'static str {
    if color_enabled(is_err) {
        "\x1b[1;33m"
    } else {
        ""
    }
}

pub fn create_backup(file_path: &Path, content: &str) -> Result<PathBuf, std::io::Error> {
    let mut backup_dir = env::temp_dir();
    backup_dir.push("mtindo_backups");
    fs::create_dir_all(&backup_dir)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let unique = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let safe_name = file_path.to_string_lossy().replace(['/', '\\'], "_");
    let backup_filename = format!(
        "{}_{}_{}_{}.bak",
        timestamp,
        std::process::id(),
        unique,
        safe_name
    );
    let backup_path = backup_dir.join(backup_filename);
    write_atomic(&backup_path, content)?;
    Ok(backup_path)
}

/// The `mto.toml` that `mto init` writes: every mto setting at its default, with its
/// summary.
pub fn init_template() -> String {
    let mut out = String::from("# mto configuration file; `mto --man` explains every setting.\n");
    for setting in options::all().filter(|s| s.file == options::File::Mto) {
        out.push_str(&format!(
            "\n# {}\n{} = {}\n",
            setting.summary, setting.name, setting.default
        ));
    }
    out
}

/// Reads the layout settings of one file, as `--config-path` does: no other file is searched
/// for.
pub fn parse_toml_config(path: &Path) -> Result<LayoutConfig, MtoError> {
    let overrides = config::Overrides {
        config_path: Some(path.to_path_buf()),
        ..config::Overrides::default()
    };
    let mut resolver = config::Resolver::new(&overrides, config::Environment::default())?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let config = resolver.config_for(dir)?.layout();
    print_warnings(&mut resolver);
    Ok(config)
}

/// Prints the configuration warnings found so far to stderr.
pub fn print_warnings(resolver: &mut config::Resolver) {
    for warning in resolver.take_warnings() {
        eprintln!("{}warning: {}{}", c_yellow(true), warning, c_reset(true));
    }
}

pub fn process_file(file_path: &Path, config: &LayoutConfig, opts: &RunOptions) -> Outcome {
    let source = match fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!(
                "{}Error reading {}: {}{}",
                c_red(),
                file_path.display(),
                e,
                c_reset(true)
            );
            return Outcome::Failed;
        }
    };
    if opts.debug {
        println!(
            "{}--- [DBG] Processing file: {} ---",
            c_cyan(),
            file_path.display()
        );
    }
    let (reorganized, edition) = match arrange(&source, config, opts.debug) {
        Ok(arranged) => arranged,
        Err(reason) => {
            eprintln!(
                "{}Skipped {}: {}{}",
                c_yellow(true),
                file_path.display(),
                reason,
                c_reset(true)
            );
            return Outcome::Skipped;
        }
    };
    if reorganized == source {
        return Outcome::Unchanged;
    }
    if !verified(
        &source,
        &reorganized,
        edition,
        config,
        &file_path.display().to_string(),
    ) {
        return Outcome::VerifierFailed;
    }
    if opts.check {
        println!("Diff in {}:", file_path.display());
        print!(
            "{}",
            unified_diff(&source, &reorganized, &file_path.display().to_string())
        );
        return Outcome::Changed;
    }
    if opts.dry_run {
        println!(
            "{}[DRY RUN]{} Would format: {}",
            c_cyan(),
            c_reset(false),
            file_path.display()
        );
        return Outcome::Changed;
    }
    let backup_path = match create_backup(file_path, &source) {
        Ok(path) => {
            if opts.debug {
                println!(
                    "{}--- [DBG] Created backup at: {} ---",
                    c_cyan(),
                    path.display()
                );
            }
            path
        }
        Err(e) => {
            eprintln!(
                "{}Failed to create backup, aborting {}: {}{}",
                c_red(),
                file_path.display(),
                e,
                c_reset(true)
            );
            return Outcome::Failed;
        }
    };
    if let Err(e) = write_atomic(file_path, &reorganized) {
        eprintln!(
            "{}Atomic write failed for {}: {}. Auto-restoring...{}",
            c_red(),
            file_path.display(),
            e,
            c_reset(true)
        );
        let _ = restore_backup(&backup_path, file_path);
        return Outcome::Failed;
    }
    println!(
        "{}Formatted:{} {}",
        c_green(),
        c_reset(false),
        file_path.display()
    );
    println!(
        "  {}Undo:{} mto --restore {} {}",
        c_cyan(),
        c_reset(false),
        backup_path.display(),
        file_path.display()
    );
    Outcome::Changed
}

pub fn process_init(target_dir: Option<PathBuf>) -> Result<PathBuf, MtoError> {
    let dir = target_dir.unwrap_or_else(|| PathBuf::from("."));
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| {
            MtoError::Io(format!("cannot create directory {}: {}", dir.display(), e))
        })?;
    }
    let config_file_path = dir.join("mto.toml");
    if config_file_path.exists() {
        return Err(MtoError::Usage(format!(
            "configuration file already exists at {}",
            config_file_path.display()
        )));
    }
    let default_config_template = init_template();
    write_atomic(&config_file_path, &default_config_template)
        .map_err(|e| MtoError::Io(format!("cannot write configuration file: {}", e)))?;
    Ok(config_file_path)
}

pub fn process_stdin(resolver: &mut config::Resolver, check: bool) -> Result<Outcome, MtoError> {
    let dir = env::current_dir()
        .map_err(|e| MtoError::Io(format!("cannot read the current directory: {}", e)))?;
    let config = resolver.config_for(&dir)?.layout();
    print_warnings(resolver);
    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .map_err(|e| MtoError::Io(format!("cannot read stdin: {}", e)))?;
    let (reorganized, edition) = match arrange(&source, &config, false) {
        Ok(arranged) => arranged,
        Err(reason) => {
            eprintln!(
                "{}Skipped <stdin>: {}{}",
                c_yellow(true),
                reason,
                c_reset(true)
            );
            if !check {
                print!("{}", source);
            }
            return Ok(Outcome::Skipped);
        }
    };
    if reorganized != source && !verified(&source, &reorganized, edition, &config, "<stdin>") {
        if !check {
            print!("{}", source);
        }
        return Ok(Outcome::VerifierFailed);
    }
    let outcome = if reorganized == source {
        Outcome::Unchanged
    } else {
        Outcome::Changed
    };
    if check {
        if outcome == Outcome::Changed {
            println!("Diff in <stdin>:");
            print!("{}", unified_diff(&source, &reorganized, "<stdin>"));
        }
    } else {
        print!("{}", reorganized);
    }
    Ok(outcome)
}

pub fn reorganize_file(source: String, config: &LayoutConfig, dbg: bool) -> String {
    match try_reorganize(&source, config, dbg) {
        Ok(text) => text,
        Err(_) => source,
    }
}

pub fn restore_backup(backup_path: &Path, target_path: &Path) -> Result<(), std::io::Error> {
    let content = fs::read_to_string(backup_path)?;
    write_atomic(target_path, &content)
}

/// Reorders the items of one source file, or explains why the file was left alone.
pub fn try_reorganize(source: &str, config: &LayoutConfig, dbg: bool) -> Result<String, String> {
    arrange(source, config, dbg).map(|(text, _)| text)
}

/// A unified diff of two texts, with three lines of context.
pub fn unified_diff(old: &str, new: &str, label: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let ops = diff_ops(&a, &b);
    let mut a_before = Vec::with_capacity(ops.len() + 1);
    let mut b_before = Vec::with_capacity(ops.len() + 1);
    let (mut a_pos, mut b_pos) = (0, 0);
    for op in ops.iter() {
        a_before.push(a_pos);
        b_before.push(b_pos);
        match op {
            DiffOp::Equal => {
                a_pos += 1;
                b_pos += 1;
            }
            DiffOp::Delete(_) => a_pos += 1,
            DiffOp::Insert(_) => b_pos += 1,
        }
    }
    a_before.push(a_pos);
    b_before.push(b_pos);
    let changes: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| !matches!(op, DiffOp::Equal))
        .map(|(idx, _)| idx)
        .collect();
    let mut out = String::new();
    if changes.is_empty() {
        return out;
    }
    out.push_str(&format!("--- {}\n+++ {}\n", label, label));
    let mut group_start = 0;
    while group_start < changes.len() {
        let mut group_end = group_start;
        while group_end + 1 < changes.len()
            && changes[group_end + 1] - changes[group_end] <= 2 * DIFF_CONTEXT + 1
        {
            group_end += 1;
        }
        let start = changes[group_start].saturating_sub(DIFF_CONTEXT);
        let end = (changes[group_end] + DIFF_CONTEXT + 1).min(ops.len());
        let old_count = a_before[end] - a_before[start];
        let new_count = b_before[end] - b_before[start];
        let old_start = if old_count == 0 {
            a_before[start]
        } else {
            a_before[start] + 1
        };
        let new_start = if new_count == 0 {
            b_before[start]
        } else {
            b_before[start] + 1
        };
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_count, new_start, new_count
        ));
        for idx in start..end {
            match ops[idx] {
                DiffOp::Equal => out.push_str(&format!(" {}\n", a[a_before[idx]])),
                DiffOp::Delete(i) => out.push_str(&format!("-{}\n", a[i])),
                DiffOp::Insert(j) => out.push_str(&format!("+{}\n", b[j])),
            }
        }
        group_start = group_end + 1;
    }
    out
}

/// Lays out every `.rs` file under `dir`, each with the settings of its own folder.
pub fn walk_directory(
    dir: &Path,
    resolver: &mut config::Resolver,
    opts: &RunOptions,
) -> Result<Vec<Outcome>, MtoError> {
    let mut visited = HashSet::new();
    let mut outcomes = Vec::new();
    walk_inner(dir, resolver, opts, &mut visited, &mut outcomes)?;
    Ok(outcomes)
}

/// Replaces `target` atomically: a sibling temp file is written, synced and renamed over it.
/// A symlinked target keeps its link and its pointee is replaced; permissions are kept.
pub fn write_atomic(target: &Path, content: &str) -> Result<(), std::io::Error> {
    let real_target = match fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(target)?,
        _ => target.to_path_buf(),
    };
    let parent = match real_target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let file_name = real_target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let unique = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!(
        ".{}.{}.{}.tmp_mtindo",
        file_name,
        std::process::id(),
        unique
    ));
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = File::create(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        if let Ok(meta) = fs::metadata(&real_target) {
            fs::set_permissions(&tmp_path, meta.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, &real_target)?;
        sync_dir(&parent);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

/// The arranged text and the edition the file was parsed with.
fn arrange(
    source: &str,
    config: &LayoutConfig,
    dbg: bool,
) -> Result<(String, syntax::Edition), String> {
    if source.trim_start_matches('\u{feff}').trim().is_empty() {
        return Ok((source.to_string(), syntax::Edition::E2021));
    }
    let arranged = layout::layout(source, config)?;
    if dbg {
        println!(
            "{}--- [DBG] Extracted {} blocks from source ---",
            c_cyan(),
            arranged.before.len()
        );
        for (idx, unit) in arranged.before.iter().enumerate() {
            println!(
                "  [Block #{}] Kind: {:?}, Visibility: {:?}, Identifier: '{}', Parent Target: '{}'",
                idx, unit.kind, unit.visibility, unit.identifier, unit.parent_target
            );
        }
        println!("{}--- [DBG] Blocks after sorting ---", c_cyan());
        for (idx, unit) in arranged.after.iter().enumerate() {
            println!(
                "  [Sorted #{}] Kind: {:?}, Identifier: '{}'",
                idx, unit.kind, unit.identifier
            );
        }
    }
    let mut text = arranged.text;
    // Debug builds only: lets the tests prove the verifier stops a bad write, now that the
    // engine has no known way to produce one.
    if cfg!(debug_assertions) && env::var_os("MTINDO_INJECT_FAULT").is_some() {
        text.push_str("// injected by MTINDO_INJECT_FAULT\n");
    }
    Ok((text, arranged.edition))
}

fn diff_ops(a: &[&str], b: &[&str]) -> Vec<DiffOp> {
    let mut prefix = 0;
    while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix
        && suffix < b.len() - prefix
        && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let a_mid = &a[prefix..a.len() - suffix];
    let b_mid = &b[prefix..b.len() - suffix];
    let mut ops: Vec<DiffOp> = (0..prefix).map(|_| DiffOp::Equal).collect();
    let (n, m) = (a_mid.len(), b_mid.len());
    if n.saturating_mul(m) <= DIFF_TABLE_LIMIT {
        let width = m + 1;
        let mut table = vec![0u32; (n + 1) * width];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                table[i * width + j] = if a_mid[i] == b_mid[j] {
                    table[(i + 1) * width + j + 1] + 1
                } else {
                    table[(i + 1) * width + j].max(table[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if a_mid[i] == b_mid[j] {
                ops.push(DiffOp::Equal);
                i += 1;
                j += 1;
            } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
                ops.push(DiffOp::Delete(prefix + i));
                i += 1;
            } else {
                ops.push(DiffOp::Insert(prefix + j));
                j += 1;
            }
        }
        ops.extend((i..n).map(|i| DiffOp::Delete(prefix + i)));
        ops.extend((j..m).map(|j| DiffOp::Insert(prefix + j)));
    } else {
        ops.extend((0..n).map(|i| DiffOp::Delete(prefix + i)));
        ops.extend((0..m).map(|j| DiffOp::Insert(prefix + j)));
    }
    ops.extend((0..suffix).map(|_| DiffOp::Equal));
    ops
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// Checks the new text with the verifier before anything is written or shown. On a
/// violation, prints the bug report (with the smallest part of the input that still
/// fails) to stderr and returns false.
fn verified(
    source: &str,
    output: &str,
    edition: syntax::Edition,
    config: &LayoutConfig,
    path: &str,
) -> bool {
    let violations = verify::verify_layout(source, output, edition);
    if violations.is_empty() {
        return true;
    }
    let fails = |candidate: &str| match try_reorganize(candidate, config, false) {
        Ok(out) => !verify::verify_layout(candidate, &out, edition).is_empty(),
        Err(_) => false,
    };
    let minimal = verify::report::minimize(source, edition, &fails);
    eprint!(
        "{}{}{}",
        c_red(),
        verify::report::bug_report(path, &violations, Some(&minimal)),
        c_reset(true)
    );
    false
}

fn walk_inner(
    dir: &Path,
    resolver: &mut config::Resolver,
    opts: &RunOptions,
    visited: &mut HashSet<PathBuf>,
    outcomes: &mut Vec<Outcome>,
) -> Result<(), MtoError> {
    let real_dir = match fs::canonicalize(dir) {
        Ok(path) => path,
        Err(_) => return Ok(()),
    };
    if !visited.insert(real_dir) {
        return Ok(());
    }
    let config = resolver.config_for(dir)?.layout();
    print_warnings(resolver);
    let dir_name = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    if config.ignore_dirs.contains(&dir_name) {
        return Ok(());
    }
    let mut entries: Vec<fs::DirEntry> = match fs::read_dir(dir) {
        Ok(entries) => entries.flatten().collect(),
        Err(_) => return Ok(()),
    };
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        let (is_dir, is_file) = if file_type.is_symlink() {
            if !opts.follow_links {
                continue;
            }
            match fs::metadata(&path) {
                Ok(meta) => (meta.is_dir(), meta.is_file()),
                Err(_) => continue,
            }
        } else {
            (file_type.is_dir(), file_type.is_file())
        };
        if is_dir {
            let child_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if config.ignore_dirs.contains(&child_name) {
                continue;
            }
            walk_inner(&path, resolver, opts, visited, outcomes)?;
        } else if is_file && path.extension().unwrap_or_default() == "rs" {
            if let Ok(real_file) = fs::canonicalize(&path)
                && !visited.insert(real_file)
            {
                continue;
            }
            outcomes.push(process_file(&path, &config, opts));
        }
    }
    Ok(())
}
