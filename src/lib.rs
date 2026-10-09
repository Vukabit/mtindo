pub mod syntax;
pub mod text;

use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::str::Chars;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const DIFF_CONTEXT: usize = 3;

const DIFF_TABLE_LIMIT: usize = 4_000_000;

static UNIQUE_COUNTER: AtomicUsize = AtomicUsize::new(0);

enum DiffOp {
    Equal,
    Delete(usize),
    Insert(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportGroup {
    Std = 0,
    External = 1,
    Local = 2,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ItemKind {
    ModDecl,
    UseDecl,
    MacroDecl,
    ConstDecl,
    StaticDecl,
    TraitDecl,
    EnumDecl,
    StructDecl,
    ImplBlock,
    FnDecl,
    Unknown,
}

impl ItemKind {
    pub fn from_name(s: &str) -> Self {
        let cleaned = s.trim().trim_matches(|c| c == '"' || c == '\'');
        match cleaned {
            "ModDecl" => ItemKind::ModDecl,
            "UseDecl" => ItemKind::UseDecl,
            "MacroDecl" => ItemKind::MacroDecl,
            "ConstDecl" => ItemKind::ConstDecl,
            "StaticDecl" => ItemKind::StaticDecl,
            "TraitDecl" => ItemKind::TraitDecl,
            "EnumDecl" => ItemKind::EnumDecl,
            "StructDecl" => ItemKind::StructDecl,
            "ImplBlock" => ItemKind::ImplBlock,
            "FnDecl" => ItemKind::FnDecl,
            _ => ItemKind::Unknown,
        }
    }
}

/// Errors that end a run. Each maps to a process exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MtoError {
    /// Bad arguments or configuration (exit code 2).
    Usage(String),
    /// A read or write failed (exit code 3).
    Io(String),
}

impl MtoError {
    pub fn exit_code(&self) -> i32 {
        match self {
            MtoError::Usage(_) => 2,
            MtoError::Io(_) => 3,
        }
    }
}

impl fmt::Display for MtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MtoError::Usage(msg) | MtoError::Io(msg) => write!(f, "{}", msg),
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
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Visibility {
    Pub,
    PubSuper,
    PubCrate,
    Private,
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub kind: ItemKind,
    pub visibility: Visibility,
    pub identifier: String,
    pub parent_target: String,
    pub raw_content: String,
    pub skip: bool,
    pub original_index: usize,
}

pub struct LayoutConfig {
    /// Reserved for the formatter. Has no effect on layout.
    pub max_width: usize,
    /// Reserved for the formatter. Has no effect on layout.
    pub tab_width: usize,
    pub sort_alphabetically: bool,
    pub type_order: Vec<ItemKind>,
    pub group_impls: bool,
    pub sort_imports: bool,
    pub import_grouping: bool,
    pub blank_lines_between_blocks: usize,
    pub ignore_dirs: Vec<String>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        LayoutConfig {
            max_width: 100,
            tab_width: 4,
            sort_alphabetically: true,
            type_order: vec![
                ItemKind::ModDecl,
                ItemKind::UseDecl,
                ItemKind::MacroDecl,
                ItemKind::ConstDecl,
                ItemKind::StaticDecl,
                ItemKind::TraitDecl,
                ItemKind::EnumDecl,
                ItemKind::StructDecl,
                ItemKind::ImplBlock,
                ItemKind::FnDecl,
                ItemKind::Unknown,
            ],
            group_impls: true,
            sort_imports: true,
            import_grouping: true,
            blank_lines_between_blocks: 1,
            ignore_dirs: vec![
                ".git".to_string(),
                "target".to_string(),
                "node_modules".to_string(),
                "vendor".to_string(),
                "dist".to_string(),
                ".venv".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions {
    pub dry_run: bool,
    pub check: bool,
    pub debug: bool,
    pub follow_links: bool,
}

struct Scan {
    blocks: Vec<CodeBlock>,
    problem: Option<String>,
}

pub fn apply_config_value(config: &mut LayoutConfig, key: &str, value: &str) -> bool {
    let cleaned_value = value.trim().trim_matches(|c| c == '"' || c == '\'');
    match key {
        "max_width" => {
            if let Ok(v) = cleaned_value.parse() {
                config.max_width = v;
            } else {
                return false;
            }
        }
        "tab_width" => {
            if let Ok(v) = cleaned_value.parse() {
                config.tab_width = v;
            } else {
                return false;
            }
        }
        "blank_lines_between_blocks" => {
            if let Ok(v) = cleaned_value.parse() {
                config.blank_lines_between_blocks = v;
            } else {
                return false;
            }
        }
        "sort_alphabetically" => {
            if let Ok(v) = cleaned_value.parse() {
                config.sort_alphabetically = v;
            } else {
                return false;
            }
        }
        "group_impls" => {
            if let Ok(v) = cleaned_value.parse() {
                config.group_impls = v;
            } else {
                return false;
            }
        }
        "sort_imports" => {
            if let Ok(v) = cleaned_value.parse() {
                config.sort_imports = v;
            } else {
                return false;
            }
        }
        "import_grouping" => {
            if let Ok(v) = cleaned_value.parse() {
                config.import_grouping = v;
            } else {
                return false;
            }
        }
        "type_order" => {
            if !value.trim().starts_with('[') || !value.trim().ends_with(']') {
                return false;
            }
            let array_clean = value.trim().trim_matches(|c| c == '[' || c == ']');
            let parts: Vec<&str> = array_clean.split(',').collect();
            config.type_order.clear();
            for part in parts {
                let p = part.trim().trim_matches(|c| c == '"' || c == '\'');
                if !p.is_empty() {
                    config.type_order.push(ItemKind::from_name(p));
                }
            }
        }
        "ignore_dirs" => {
            if !value.trim().starts_with('[') || !value.trim().ends_with(']') {
                return false;
            }
            let array_clean = value.trim().trim_matches(|c| c == '[' || c == ']');
            let parts: Vec<&str> = array_clean.split(',').collect();
            config.ignore_dirs.clear();
            for part in parts {
                let p = part.trim().trim_matches(|c| c == '"' || c == '\'');
                if !p.is_empty() {
                    config.ignore_dirs.push(p.to_string());
                }
            }
        }
        _ => {}
    }
    true
}

pub fn c_cyan() -> &'static str {
    if std::io::stdout().is_terminal() {
        "\x1b[1;36m"
    } else {
        ""
    }
}

pub fn c_green() -> &'static str {
    if std::io::stdout().is_terminal() {
        "\x1b[1;32m"
    } else {
        ""
    }
}

pub fn c_red() -> &'static str {
    if std::io::stderr().is_terminal() {
        "\x1b[1;31m"
    } else {
        ""
    }
}

pub fn c_reset(is_err: bool) -> &'static str {
    let tty = if is_err {
        std::io::stderr().is_terminal()
    } else {
        std::io::stdout().is_terminal()
    };
    if tty { "\x1b[0m" } else { "" }
}

pub fn c_yellow(is_err: bool) -> &'static str {
    let tty = if is_err {
        std::io::stderr().is_terminal()
    } else {
        std::io::stdout().is_terminal()
    };
    if tty { "\x1b[1;33m" } else { "" }
}

pub fn classify_import(content: &str) -> ImportGroup {
    let trimmed = content.trim();
    let path_part = if trimmed.starts_with("pub") {
        if let Some(idx) = trimmed.find("use") {
            &trimmed[idx + 3..]
        } else {
            trimmed
        }
    } else if let Some(rest) = trimmed.strip_prefix("use") {
        rest
    } else {
        trimmed
    }
    .trim();
    if path_part.starts_with("std::")
        || path_part.starts_with("core::")
        || path_part.starts_with("alloc::")
        || path_part.starts_with("alloc ")
        || path_part.starts_with("core ")
        || path_part.starts_with("std ")
    {
        ImportGroup::Std
    } else if path_part.starts_with("crate::")
        || path_part.starts_with("super::")
        || path_part.starts_with("self::")
    {
        ImportGroup::Local
    } else {
        ImportGroup::External
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

/// Returns the text of an item after its leading whitespace, comments and outer attributes.
pub fn decl_head(content: &str) -> &str {
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    loop {
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
        } else if i < len && bytes[i] == b'#' {
            let next = skip_attribute(bytes, i);
            if next == i {
                break;
            }
            i = next;
        } else {
            break;
        }
    }
    &content[i..]
}

pub fn extract_blocks(source: &str) -> Vec<CodeBlock> {
    scan_blocks(source).blocks
}

pub fn parse_block_metadata(content: &str, skip: bool, original_index: usize) -> CodeBlock {
    let mut kind = ItemKind::Unknown;
    let mut visibility = Visibility::Private;
    let mut identifier = String::new();
    let mut parent_target = String::new();
    if skip {
        return CodeBlock {
            kind: ItemKind::Unknown,
            visibility,
            identifier: "SkippedBlock".to_string(),
            parent_target,
            raw_content: content.to_string(),
            skip: true,
            original_index,
        };
    }
    let mut decl_line = String::new();
    for line in decl_head(content).lines() {
        decl_line.push_str(line.trim());
        decl_line.push(' ');
        if decl_line.contains('{') || decl_line.contains(';') || decl_line.contains('=') {
            break;
        }
    }
    let trimmed_decl = decl_line.trim_start();
    if trimmed_decl.starts_with("const ")
        || trimmed_decl.starts_with("pub const ")
        || trimmed_decl.starts_with("pub(crate) const ")
        || trimmed_decl.starts_with("pub(super) const ")
    {
        kind = ItemKind::ConstDecl;
        let tokens: Vec<&str> = decl_line.split_whitespace().collect();
        for (i, token) in tokens.iter().enumerate() {
            if *token == "const" && i + 1 < tokens.len() {
                identifier = tokens[i + 1]
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(';')
                    .to_string();
                break;
            }
        }
    }
    if decl_line.contains("pub(crate) ") {
        visibility = Visibility::PubCrate;
    } else if decl_line.contains("pub(super) ") {
        visibility = Visibility::PubSuper;
    } else if decl_line.contains("pub ") {
        visibility = Visibility::Pub;
    }
    let tokens: Vec<&str> = decl_line.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        let base_token = token.split('<').next().unwrap_or(*token);
        match base_token {
            "mod" => {
                kind = ItemKind::ModDecl;
                if i + 1 < tokens.len() {
                    identifier = tokens[i + 1]
                        .trim_end_matches(';')
                        .trim_end_matches('{')
                        .to_string();
                }
                break;
            }
            "use" => {
                kind = ItemKind::UseDecl;
                if i + 1 < tokens.len() {
                    identifier = tokens[i + 1].trim_end_matches(';').to_string();
                }
                break;
            }
            "static" => {
                kind = ItemKind::StaticDecl;
                let mut id_idx = i + 1;
                if id_idx < tokens.len() && tokens[id_idx] == "mut" {
                    id_idx += 1;
                }
                if id_idx < tokens.len() {
                    identifier = tokens[id_idx]
                        .split(':')
                        .next()
                        .unwrap_or("")
                        .trim_end_matches(';')
                        .to_string();
                }
                break;
            }
            "struct" => {
                kind = ItemKind::StructDecl;
                if i + 1 < tokens.len() {
                    identifier = tokens[i + 1].trim_end_matches('{').to_string();
                }
                break;
            }
            "enum" => {
                kind = ItemKind::EnumDecl;
                if i + 1 < tokens.len() {
                    identifier = tokens[i + 1].trim_end_matches('{').to_string();
                }
                break;
            }
            "fn" => {
                kind = ItemKind::FnDecl;
                if i + 1 < tokens.len() {
                    identifier = tokens[i + 1].split('(').next().unwrap_or("").to_string();
                }
                break;
            }
            "trait" => {
                kind = ItemKind::TraitDecl;
                if i + 1 < tokens.len() {
                    identifier = tokens[i + 1].trim_end_matches('{').to_string();
                }
                break;
            }
            "impl" => {
                kind = ItemKind::ImplBlock;
                let for_pos = tokens.iter().position(|&t| t == "for");
                if let Some(pos) = for_pos {
                    if pos + 1 < tokens.len() {
                        parent_target = tokens[pos + 1]
                            .split('<')
                            .next()
                            .unwrap_or("")
                            .trim_end_matches('{')
                            .to_string();
                        let trait_name = if pos > 0 {
                            tokens[pos - 1].split('<').next().unwrap_or("")
                        } else {
                            ""
                        };
                        identifier = format!("{} for {}", trait_name, parent_target);
                    }
                } else {
                    let target_token = if i + 1 < tokens.len() {
                        tokens[i + 1]
                    } else {
                        ""
                    };
                    parent_target = target_token
                        .split('<')
                        .next()
                        .unwrap_or("")
                        .trim_end_matches('{')
                        .to_string();
                    identifier = parent_target.clone();
                }
                break;
            }
            "macro_rules!" => {
                kind = ItemKind::MacroDecl;
                if i + 1 < tokens.len() {
                    identifier = tokens[i + 1].trim_end_matches('{').to_string();
                }
                break;
            }
            _ => {}
        }
    }
    CodeBlock {
        kind,
        visibility,
        identifier,
        parent_target,
        raw_content: content.to_string(),
        skip: false,
        original_index,
    }
}

pub fn parse_toml_config(path: &Path) -> Result<LayoutConfig, MtoError> {
    let mut config = LayoutConfig::default();
    let content = fs::read_to_string(path).map_err(|e| {
        MtoError::Usage(format!(
            "cannot read configuration {}: {}",
            path.display(),
            e
        ))
    })?;
    let invalid = |key: &str| {
        MtoError::Usage(format!(
            "invalid configuration value for `{}` in {}",
            key,
            path.display()
        ))
    };
    let mut in_array_key: Option<String> = None;
    let mut array_accumulator = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        if let Some(ref key) = in_array_key {
            array_accumulator.push(' ');
            array_accumulator.push_str(trimmed);

            if trimmed.ends_with(']') {
                if !apply_config_value(&mut config, key, &array_accumulator) {
                    return Err(invalid(key));
                }
                in_array_key = None;
                array_accumulator.clear();
            }
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.contains('=') {
            continue;
        }

        let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
        if parts.len() == 2 {
            let key = parts[0].trim();
            let value = parts[1].trim();

            if value.starts_with('[') && !value.ends_with(']') {
                in_array_key = Some(key.to_string());
                array_accumulator.push_str(value);
                continue;
            }

            if !apply_config_value(&mut config, key, value) {
                return Err(invalid(key));
            }
        }
    }

    if let Some(key) = in_array_key {
        return Err(MtoError::Usage(format!(
            "unterminated array for `{}` in {}",
            key,
            path.display()
        )));
    }

    Ok(config)
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
    let reorganized = match try_reorganize(&source, config, opts.debug) {
        Ok(text) => text,
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
    let default_config_template = r#"# mto configuration file
sort_alphabetically = true
group_impls = true
sort_imports = true
import_grouping = true
blank_lines_between_blocks = 1
type_order = [
  "ModDecl",
  "UseDecl",
  "MacroDecl",
  "ConstDecl",
  "StaticDecl",
  "TraitDecl",
  "EnumDecl",
  "StructDecl",
  "ImplBlock",
  "FnDecl",
  "Unknown"
]
"#;
    write_atomic(&config_file_path, default_config_template)
        .map_err(|e| MtoError::Io(format!("cannot write configuration file: {}", e)))?;
    Ok(config_file_path)
}

pub fn process_stdin(config_path: Option<PathBuf>, check: bool) -> Result<Outcome, MtoError> {
    let config = match resolve_config_path(config_path, None) {
        Some(path) => parse_toml_config(&path)?,
        None => LayoutConfig::default(),
    };
    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .map_err(|e| MtoError::Io(format!("cannot read stdin: {}", e)))?;
    let reorganized = match try_reorganize(&source, &config, false) {
        Ok(text) => text,
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

pub fn resolve_config_path(
    cli_path: Option<PathBuf>,
    target_path: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(path) = cli_path {
        return Some(path);
    }
    let start_dir = match target_path {
        Some(p) => {
            if p.is_file() {
                p.parent()
                    .map(|s| s.to_path_buf())
                    .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            } else {
                p.to_path_buf()
            }
        }
        None => env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    let mut current = Some(start_dir.as_path());
    while let Some(dir) = current {
        let candidate1 = dir.join("mto.toml");
        if candidate1.exists() {
            return Some(candidate1);
        }
        let candidate2 = dir.join(".mto.toml");
        if candidate2.exists() {
            return Some(candidate2);
        }
        current = dir.parent();
    }
    let home_env = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok();
    if let Some(home_str) = home_env {
        let home_path = PathBuf::from(home_str);
        let xdg_config = home_path.join(".config").join("mtindo").join("mto.toml");
        if xdg_config.exists() {
            return Some(xdg_config);
        }
        let xdg_config_old = home_path.join(".config").join("mtindo").join(".mto.toml");
        if xdg_config_old.exists() {
            return Some(xdg_config_old);
        }
        let global_config = home_path.join("mto.toml");
        if global_config.exists() {
            return Some(global_config);
        }
        let global_config_old = home_path.join(".mto.toml");
        if global_config_old.exists() {
            return Some(global_config_old);
        }
    }
    None
}

pub fn restore_backup(backup_path: &Path, target_path: &Path) -> Result<(), std::io::Error> {
    let content = fs::read_to_string(backup_path)?;
    write_atomic(target_path, &content)
}

pub fn sort_blocks(blocks: &mut Vec<CodeBlock>, config: &LayoutConfig) {
    let original = blocks.clone();
    arrange_blocks(blocks, config, false);
    if macro_moved_past_user(blocks) {
        *blocks = original;
        arrange_blocks(blocks, config, true);
    }
}

/// Splits off the file header: shebang, inner doc comments, inner attributes, and
/// comment runs that are separated from the first item by a blank line.
pub fn split_header(source: &str) -> (&str, &str) {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut cut = 0;
    let mut pending: Option<usize> = None;
    if source.starts_with("#!") && !source[2..].trim_start().starts_with('[') {
        pos = next_line(bytes, 0);
        cut = pos;
    }
    while pos < len {
        let line_end = next_line(bytes, pos);
        let line = &source[pos..line_end];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if let Some(end) = pending.take() {
                cut = end;
            }
            pos = line_end;
            continue;
        }
        if trimmed.starts_with("// mto:") {
            break;
        }
        let is_inner =
            trimmed.starts_with("//!") || trimmed.starts_with("/*!") || trimmed.starts_with("#![");
        if !is_inner && !is_plain_comment(trimmed) {
            break;
        }
        let start = pos + (line.len() - line.trim_start().len());
        let span_end = if trimmed.starts_with("#![") {
            skip_attribute(bytes, start)
        } else if trimmed.starts_with("/*") {
            skip_block_comment(bytes, start)
        } else {
            line_end
        };
        let end = if span_end >= len || bytes[span_end - 1] == b'\n' {
            span_end
        } else {
            let rest_end = next_line(bytes, span_end);
            let rest = source[span_end..rest_end].trim();
            if rest.is_empty() || rest.starts_with("//") {
                rest_end
            } else {
                // Code shares the line with the header element; stop here.
                if is_inner {
                    cut = span_end;
                }
                break;
            }
        };
        if is_inner {
            cut = end;
            pending = None;
        } else {
            pending = Some(end);
        }
        pos = end;
    }
    if pos >= len
        && let Some(end) = pending
    {
        cut = end;
    }
    (&source[..cut], &source[cut..])
}

/// Reorders the items of one source file, or explains why the file was left alone.
pub fn try_reorganize(source: &str, config: &LayoutConfig, dbg: bool) -> Result<String, String> {
    if source.trim_start_matches('\u{feff}').trim().is_empty() {
        return Ok(source.to_string());
    }
    let normalized = text::source::normalize(source)?;
    let (header, body) = split_header(&normalized.text);
    let scan = scan_blocks(body);
    if let Some(problem) = scan.problem {
        return Err(problem);
    }
    let mut blocks = scan.blocks;
    let mut macro_names = HashSet::new();
    for block in blocks.iter() {
        if block.kind == ItemKind::MacroDecl && !macro_names.insert(block.identifier.as_str()) {
            return Err(format!(
                "macro `{}` is defined more than once",
                block.identifier
            ));
        }
    }
    if dbg {
        println!(
            "{}--- [DBG] Extracted {} blocks from source ---",
            c_cyan(),
            blocks.len()
        );
        for (idx, block) in blocks.iter().enumerate() {
            println!(
                "  [Block #{}] Kind: {:?}, Visibility: {:?}, Identifier: '{}', Parent Target: '{}'",
                idx, block.kind, block.visibility, block.identifier, block.parent_target
            );
        }
    }
    sort_blocks(&mut blocks, config);
    if macro_moved_past_user(&blocks) {
        return Err("cannot keep macro definitions ahead of the items after them".to_string());
    }
    if dbg {
        println!("{}--- [DBG] Blocks after sorting ---", c_cyan());
        for (idx, block) in blocks.iter().enumerate() {
            println!(
                "  [Sorted #{}] Kind: {:?}, Identifier: '{}'",
                idx, block.kind, block.identifier
            );
        }
    }
    let mut output = String::new();
    let header = header.trim();
    if !header.is_empty() {
        output.push_str(header);
        if !blocks.is_empty() {
            output.push_str("\n\n");
        }
    }
    let mut previous_block: Option<&CodeBlock> = None;
    for block in blocks.iter() {
        if let Some(prev) = previous_block {
            if prev.kind == ItemKind::UseDecl && block.kind == ItemKind::UseDecl {
                let mut inject_blank = false;
                if config.sort_imports && config.import_grouping {
                    let g1 = classify_import(&prev.raw_content);
                    let g2 = classify_import(&block.raw_content);
                    if g1 != g2 {
                        inject_blank = true;
                    }
                }
                if inject_blank {
                    let newlines = "\n".repeat(config.blank_lines_between_blocks + 1);
                    output.push_str(&newlines);
                } else {
                    output.push('\n');
                }
            } else if prev.kind == ItemKind::ModDecl
                && block.kind == ItemKind::ModDecl
                && prev.raw_content.ends_with(';')
                && block.raw_content.ends_with(';')
            {
                output.push('\n');
            } else {
                let newlines = "\n".repeat(config.blank_lines_between_blocks + 1);
                output.push_str(&newlines);
            }
        }
        output.push_str(&block.raw_content);
        previous_block = Some(block);
    }
    let result = output.trim_end().to_string() + "\n";
    Ok(normalized.restore(&result))
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

pub fn walk_directory(dir: &Path, config: &LayoutConfig, opts: &RunOptions) -> Vec<Outcome> {
    let mut visited = HashSet::new();
    let mut outcomes = Vec::new();
    walk_inner(dir, config, opts, &mut visited, &mut outcomes);
    outcomes
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

fn arrange_blocks(blocks: &mut Vec<CodeBlock>, config: &LayoutConfig, macros_first: bool) {
    let mut skipped_items = Vec::new();
    let mut normal_blocks = Vec::new();
    for (idx, block) in blocks.drain(..).enumerate() {
        if block.skip || is_pinned(&block) {
            skipped_items.push((idx, block));
        } else {
            normal_blocks.push(block);
        }
    }
    let mut test_mods = HashSet::new();
    let mut macro_use_mods = HashSet::new();
    for block in normal_blocks.iter() {
        if block.kind == ItemKind::ModDecl {
            let attrs = outer_attributes(&block.raw_content);
            if attrs.iter().any(|a| a == "cfg(test)") {
                test_mods.insert(block.original_index);
            }
            if attrs.iter().any(|a| a == "macro_use") {
                macro_use_mods.insert(block.original_index);
            }
        }
    }
    let mut family_map: Vec<(String, ItemKind)> = Vec::new();
    for block in normal_blocks.iter() {
        if block.kind == ItemKind::StructDecl
            || block.kind == ItemKind::EnumDecl
            || block.kind == ItemKind::TraitDecl
        {
            let name = block.identifier.clone();
            let kind = block.kind.clone();
            family_map.push((name, kind));
        }
    }
    normal_blocks.sort_by(|a, b| {
        if macros_first {
            let macro_a = a.kind == ItemKind::MacroDecl;
            let macro_b = b.kind == ItemKind::MacroDecl;
            if macro_a != macro_b {
                return macro_b.cmp(&macro_a);
            }
        }
        let last_a = test_mods.contains(&a.original_index);
        let last_b = test_mods.contains(&b.original_index);
        if last_a != last_b {
            return last_a.cmp(&last_b);
        }
        if a.kind == ItemKind::UseDecl && b.kind == ItemKind::UseDecl && config.sort_imports {
            if config.import_grouping {
                let group_a = classify_import(&a.raw_content);
                let group_b = classify_import(&b.raw_content);
                if group_a != group_b {
                    return group_a.cmp(&group_b);
                }
            }
            return (import_path_rank(&a.identifier), &a.identifier)
                .cmp(&(import_path_rank(&b.identifier), &b.identifier));
        }
        let mut family_a = &a.identifier;
        if config.group_impls && a.kind == ItemKind::ImplBlock {
            family_a = &a.parent_target;
        }
        let mut family_b = &b.identifier;
        if config.group_impls && b.kind == ItemKind::ImplBlock {
            family_b = &b.parent_target;
        }
        let mut kind_a = &a.kind;
        if config.group_impls && a.kind == ItemKind::ImplBlock {
            for (name, kind) in family_map.iter() {
                if name == family_a {
                    kind_a = kind;
                    break;
                }
            }
        }
        let mut kind_b = &b.kind;
        if config.group_impls && b.kind == ItemKind::ImplBlock {
            for (name, kind) in family_map.iter() {
                if name == family_b {
                    kind_b = kind;
                    break;
                }
            }
        }
        let mut family_rank_a = 99;
        for (index, kind) in config.type_order.iter().enumerate() {
            if kind == kind_a {
                family_rank_a = index;
                break;
            }
        }
        let mut family_rank_b = 99;
        for (index, kind) in config.type_order.iter().enumerate() {
            if kind == kind_b {
                family_rank_b = index;
                break;
            }
        }
        if family_rank_a != family_rank_b {
            return family_rank_a.cmp(&family_rank_b);
        }
        if config.group_impls {
            let is_groupable = |k: &ItemKind| {
                matches!(
                    k,
                    ItemKind::StructDecl
                        | ItemKind::EnumDecl
                        | ItemKind::TraitDecl
                        | ItemKind::ImplBlock
                )
            };
            if is_groupable(&a.kind) && is_groupable(&b.kind) && family_a != family_b {
                return family_a.cmp(family_b);
            }
        }
        let mut block_rank_a = 99;
        for (index, kind) in config.type_order.iter().enumerate() {
            if kind == &a.kind {
                block_rank_a = index;
                break;
            }
        }
        let mut block_rank_b = 99;
        for (index, kind) in config.type_order.iter().enumerate() {
            if kind == &b.kind {
                block_rank_b = index;
                break;
            }
        }
        if block_rank_a != block_rank_b {
            return block_rank_a.cmp(&block_rank_b);
        }
        // Macros are scoped by textual order, so they never change places with each other.
        if a.kind == ItemKind::MacroDecl && b.kind == ItemKind::MacroDecl {
            return a.original_index.cmp(&b.original_index);
        }
        // A #[macro_use] module must stay ahead of the modules that rely on its macros.
        if a.kind == ItemKind::ModDecl && b.kind == ItemKind::ModDecl {
            let use_a = macro_use_mods.contains(&a.original_index);
            let use_b = macro_use_mods.contains(&b.original_index);
            if use_a != use_b {
                return use_b.cmp(&use_a);
            }
            if use_a {
                return a.original_index.cmp(&b.original_index);
            }
            // Modules are ordered by name alone, ignoring visibility, as rustfmt orders them.
            if config.sort_alphabetically {
                return a.identifier.cmp(&b.identifier);
            }
            return std::cmp::Ordering::Equal;
        }
        if a.visibility != b.visibility {
            return a.visibility.cmp(&b.visibility);
        }
        if config.sort_alphabetically {
            a.identifier.cmp(&b.identifier)
        } else {
            std::cmp::Ordering::Equal
        }
    });
    let total_len = normal_blocks.len() + skipped_items.len();
    let mut final_blocks: Vec<Option<CodeBlock>> =
        std::iter::repeat_with(|| None).take(total_len).collect();
    for (orig_idx, block) in skipped_items {
        if orig_idx < total_len {
            final_blocks[orig_idx] = Some(block);
        } else {
            final_blocks.push(Some(block));
        }
    }
    let mut normal_iter = normal_blocks.into_iter();
    for slot in final_blocks.iter_mut() {
        if slot.is_none()
            && let Some(normal_block) = normal_iter.next()
        {
            *slot = Some(normal_block);
        }
    }
    *blocks = final_blocks.into_iter().map(|b| b.unwrap()).collect();
}

/// Pulls a same-line trailing comment (`} // note`) into the block that just ended.
fn attach_trailing_comment(chars: &mut Peekable<Chars>, block: &mut String) {
    let mut look = chars.clone();
    let mut prefix_len = 0;
    while let Some(&c) = look.peek() {
        if c == ' ' || c == '\t' {
            prefix_len += 1;
            look.next();
        } else {
            break;
        }
    }
    let rest: String = look.take_while(|&c| c != '\n').collect();
    let take = if rest.starts_with("//") {
        let is_doc =
            rest.starts_with("//!") || (rest.starts_with("///") && !rest.starts_with("////"));
        if is_doc || rest.starts_with("// mto:") {
            0
        } else {
            rest.chars().count()
        }
    } else if rest.starts_with("/*") && !rest.starts_with("/*!") && !rest.starts_with("/**") {
        match rest.find("*/") {
            Some(close) if rest[close + 2..].trim().is_empty() => rest.chars().count(),
            _ => 0,
        }
    } else {
        0
    };
    if take == 0 {
        return;
    }
    for _ in 0..prefix_len + take {
        if let Some(c) = chars.next() {
            block.push(c);
        }
    }
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

/// True for an item that ends at a `;` even though it may contain braces.
fn ends_with_semicolon(block: &str) -> bool {
    let head = strip_visibility(decl_head(block));
    let mut words = head.split_whitespace();
    let first = words.next().unwrap_or("");
    let second = words.next().unwrap_or("");
    match first {
        "use" | "static" | "type" => true,
        "extern" => second.starts_with("crate"),
        "const" => !matches!(second, "fn" | "unsafe" | "async" | "extern"),
        _ => matches!(macro_call_delimiter(head), Some('(') | Some('[')),
    }
}

fn flush_block(blocks: &mut Vec<CodeBlock>, current_block: &mut String) {
    let trimmed = current_block.trim();
    if !trimmed.is_empty() {
        let idx = blocks.len();
        blocks.push(parse_block_metadata(trimmed, false, idx));
    }
    current_block.clear();
}

/// Where an import path sorts by its first segment, as rustfmt orders them in every edition:
/// `self::` first, then `super::`, then `crate::`, then everything else.
fn import_path_rank(path: &str) -> u8 {
    let first = path.split("::").next().unwrap_or("");
    match first {
        "self" => 0,
        "super" => 1,
        "crate" => 2,
        _ => 3,
    }
}

fn is_pinned(block: &CodeBlock) -> bool {
    let head = strip_visibility(decl_head(&block.raw_content));
    let mut words = head.split_whitespace();
    let is_extern_crate =
        words.next() == Some("extern") && words.next().is_some_and(|w| w.starts_with("crate"));
    is_extern_crate || macro_call_delimiter(head).is_some()
}

fn is_plain_comment(trimmed: &str) -> bool {
    let line = trimmed.starts_with("//")
        && !trimmed.starts_with("//!")
        && (!trimmed.starts_with("///") || trimmed.starts_with("////"));
    let block = trimmed.starts_with("/*")
        && !trimmed.starts_with("/*!")
        && (!trimmed.starts_with("/**")
            || trimmed.starts_with("/***")
            || trimmed.starts_with("/**/"));
    line || block
}

/// For an item-position macro call such as `name! { .. }`, returns its opening delimiter.
fn macro_call_delimiter(head: &str) -> Option<char> {
    let bang = head.find('!')?;
    let path = &head[..bang];
    if path.is_empty()
        || path == "macro_rules"
        || !path
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
    {
        return None;
    }
    head[bang + 1..]
        .trim_start()
        .chars()
        .next()
        .filter(|c| matches!(c, '(' | '[' | '{'))
}

/// A `macro_rules!` is only in scope for what follows it in the file, so nothing that
/// originally came after a macro definition may end up in front of it.
fn macro_moved_past_user(blocks: &[CodeBlock]) -> bool {
    let mut latest_before: Option<usize> = None;
    for block in blocks.iter() {
        if block.kind == ItemKind::MacroDecl
            && latest_before.is_some_and(|latest| latest > block.original_index)
        {
            return true;
        }
        latest_before = latest_before.max(Some(block.original_index));
    }
    false
}

fn next_line(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    (i + 1).min(bytes.len())
}

/// The outer attributes of an item, with whitespace removed, e.g. `cfg(test)`.
fn outer_attributes(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut attrs = Vec::new();
    let mut i = 0;
    loop {
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
        } else if i + 1 < len && bytes[i] == b'#' && bytes[i + 1] == b'[' {
            let next = skip_attribute(bytes, i);
            if next == i {
                break;
            }
            let inner = &content[i + 2..next.saturating_sub(1).max(i + 2)];
            attrs.push(inner.chars().filter(|c| !c.is_whitespace()).collect());
            i = next;
        } else {
            break;
        }
    }
    attrs
}

fn scan_blocks(source: &str) -> Scan {
    let mut blocks = Vec::new();
    let mut problem: Option<String> = None;
    let mut current_block = String::new();
    let mut chars = source.chars().peekable();
    let mut brace_depth: i64 = 0;
    let mut paren_depth: i64 = 0;
    let mut bracket_depth: i64 = 0;
    let mut in_string = false;
    let mut in_raw_string = false;
    let mut raw_hash_count = 0;
    let mut in_char = false;
    let mut in_line_comment = false;
    let mut block_comment_depth = 0;
    let mut escape_next = false;
    let mut in_mto_off = false;

    while let Some(c) = chars.next() {
        current_block.push(c);
        let at_line_end = matches!(chars.peek(), None | Some('\n'));

        if in_mto_off {
            if at_line_end && current_block.trim_end().ends_with("// mto: on") {
                in_mto_off = false;
                let trimmed = current_block.trim();
                if !trimmed.is_empty() {
                    let idx = blocks.len();
                    blocks.push(parse_block_metadata(trimmed, true, idx));
                }
                current_block.clear();
            }
            continue;
        }

        if escape_next {
            escape_next = false;
            continue;
        }

        if in_line_comment && at_line_end && current_block.trim_end().ends_with("// mto: off") {
            let prefix = current_block
                .trim_end()
                .trim_end_matches("// mto: off")
                .trim();
            if !prefix.is_empty() {
                let idx = blocks.len();
                blocks.push(parse_block_metadata(prefix, false, idx));
            }
            current_block.clear();
            current_block.push_str("// mto: off");
            in_mto_off = true;
            in_line_comment = false;
            continue;
        }

        let in_code = !in_string
            && !in_char
            && !in_line_comment
            && block_comment_depth == 0
            && !in_raw_string;

        match c {
            '\\' if in_string || in_char => {
                escape_next = true;
            }
            '\'' if in_code => {
                let mut peek_clone = chars.clone();
                let next1 = peek_clone.next();
                let next2 = peek_clone.next();
                let is_literal = match (next1, next2) {
                    (Some('\\'), Some(_)) => peek_clone.next() == Some('\''),
                    (Some(c1), Some('\'')) if c1 != '\'' && c1 != '\\' && c1 != '\n' => true,
                    _ => false,
                };
                if is_literal {
                    in_char = true;
                }
            }
            '\'' if in_char => {
                in_char = false;
            }
            'r' if in_code => {
                let mut peek_clone = chars.clone();
                let mut hashes = 0;
                while let Some(&'#') = peek_clone.peek() {
                    hashes += 1;
                    peek_clone.next();
                }
                if let Some(&'"') = peek_clone.peek() {
                    in_raw_string = true;
                    raw_hash_count = hashes;
                    for _ in 0..hashes {
                        current_block.push(chars.next().unwrap());
                    }
                    current_block.push(chars.next().unwrap());
                }
            }
            '"' if in_raw_string => {
                let mut peek_clone = chars.clone();
                let mut hashes = 0;
                while let Some(&'#') = peek_clone.peek() {
                    hashes += 1;
                    peek_clone.next();
                }
                if hashes >= raw_hash_count {
                    in_raw_string = false;
                    for _ in 0..raw_hash_count {
                        current_block.push(chars.next().unwrap());
                    }
                    raw_hash_count = 0;
                }
            }
            '"' if !in_char && !in_line_comment && block_comment_depth == 0 && !in_raw_string => {
                in_string = !in_string;
            }
            '/' if in_code => {
                if let Some(&'/') = chars.peek() {
                    in_line_comment = true;
                } else if let Some(&'*') = chars.peek() {
                    current_block.push(chars.next().unwrap());
                    block_comment_depth = 1;
                }
            }
            '/' if block_comment_depth > 0 && chars.peek() == Some(&'*') => {
                current_block.push(chars.next().unwrap());
                block_comment_depth += 1;
            }
            '*' if block_comment_depth > 0 && chars.peek() == Some(&'/') => {
                current_block.push(chars.next().unwrap());
                block_comment_depth -= 1;
            }
            '\n' if in_line_comment => {
                in_line_comment = false;
            }
            '(' if in_code => paren_depth += 1,
            ')' if in_code => paren_depth -= 1,
            '[' if in_code => bracket_depth += 1,
            ']' if in_code => bracket_depth -= 1,
            '{' if in_code => {
                brace_depth += 1;
            }
            '}' if in_code => {
                brace_depth -= 1;
                // Flush at the item's closing brace, unless the item ends at a `;`.
                if brace_depth == 0
                    && paren_depth == 0
                    && bracket_depth == 0
                    && !ends_with_semicolon(&current_block)
                {
                    attach_trailing_comment(&mut chars, &mut current_block);
                    flush_block(&mut blocks, &mut current_block);
                }
            }
            ';' if in_code && brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 => {
                attach_trailing_comment(&mut chars, &mut current_block);
                flush_block(&mut blocks, &mut current_block);
            }
            _ => {}
        }

        if (brace_depth < 0 || paren_depth < 0 || bracket_depth < 0) && problem.is_none() {
            problem = Some("unbalanced delimiters".to_string());
        }
    }

    if problem.is_none() {
        if brace_depth != 0 || paren_depth != 0 || bracket_depth != 0 {
            problem = Some("unbalanced delimiters".to_string());
        } else if in_string || in_raw_string || in_char || block_comment_depth > 0 {
            problem = Some("unterminated string, char or comment".to_string());
        }
    }

    let trailing = current_block.trim();
    if !trailing.is_empty() {
        if !in_mto_off && problem.is_none() {
            let head = decl_head(trailing).trim();
            if !head.is_empty() && !head.ends_with('}') && !head.ends_with(';') {
                problem = Some("code after the last complete item".to_string());
            }
        }
        let idx = blocks.len();
        blocks.push(parse_block_metadata(trailing, in_mto_off, idx));
    }

    Scan { blocks, problem }
}

/// Skips `#[...]` or `#![...]` starting at `start`; returns `start` if there is no attribute.
fn skip_attribute(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start + 1;
    if i < len && bytes[i] == b'!' {
        i += 1;
    }
    if i >= len || bytes[i] != b'[' {
        return start;
    }
    let mut depth = 0;
    let mut in_str = false;
    while i < len {
        let b = bytes[i];
        if in_str {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    len
}

/// Skips a block comment starting at `start`, honouring nesting.
fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut depth = 0;
    let mut i = start;
    while i + 1 < len {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    len
}

fn strip_visibility(head: &str) -> &str {
    let t = head.trim_start();
    if let Some(rest) = t.strip_prefix("pub") {
        let r = rest.trim_start();
        if r.starts_with('(') {
            if let Some(close) = r.find(')') {
                return r[close + 1..].trim_start();
            }
        } else if rest.starts_with(char::is_whitespace) {
            return r;
        }
    }
    t
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

fn walk_inner(
    dir: &Path,
    config: &LayoutConfig,
    opts: &RunOptions,
    visited: &mut HashSet<PathBuf>,
    outcomes: &mut Vec<Outcome>,
) {
    let real_dir = match fs::canonicalize(dir) {
        Ok(path) => path,
        Err(_) => return,
    };
    if !visited.insert(real_dir) {
        return;
    }
    let dir_name = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    if config.ignore_dirs.contains(&dir_name) {
        return;
    }
    let mut entries: Vec<fs::DirEntry> = match fs::read_dir(dir) {
        Ok(entries) => entries.flatten().collect(),
        Err(_) => return,
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
            walk_inner(&path, config, opts, visited, outcomes);
        } else if is_file && path.extension().unwrap_or_default() == "rs" {
            if let Ok(real_file) = fs::canonicalize(&path)
                && !visited.insert(real_file)
            {
                continue;
            }
            outcomes.push(process_file(&path, config, opts));
        }
    }
}
