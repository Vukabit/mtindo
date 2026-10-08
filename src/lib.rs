use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
    pub fn from_str(s: &str) -> Self {
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
    pub max_width: usize,
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

pub fn apply_width_constraints(content: &str, max_width: usize, tab_width: usize) -> String {
    let reflowed_content = reflow_delimited_blocks(content, max_width, tab_width);
    let mut min_indent = usize::MAX;
    for line in reflowed_content.lines() {
        let leading = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        if leading > 0 && leading < min_indent {
            min_indent = leading;
        }
    }
    let base_indent = if min_indent == usize::MAX || min_indent == 0 { 4 } else { min_indent };
    let mut processed_lines = Vec::new();
    for line in reflowed_content.lines() {
        let expanded_line = line.replace('\t', &" ".repeat(tab_width));
        let leading_spaces = expanded_line.chars().take_while(|c| *c == ' ').count();
        let trimmed_content = &expanded_line[leading_spaces..];
        let normalized_leading = if leading_spaces > 0 {
            let level = (leading_spaces + base_indent / 2) / base_indent;
            let new_indent_len = level * tab_width;
            " ".repeat(new_indent_len)
        } else {
            String::new()
        };
        let reconstructed_line = format!("{}{}", normalized_leading, trimmed_content);
        let wrapped = wrap_line(&reconstructed_line, max_width, tab_width);
        for wrapped_line in wrapped.lines() {
            processed_lines.push(wrapped_line.trim_end().to_string());
        }
    }
    while let Some(last) = processed_lines.last() {
        if last.is_empty() {
            processed_lines.pop();
        } else {
            break;
        }
    }
    processed_lines.join("\n")
}

pub fn c_cyan() -> &'static str {
    if std::io::stdout().is_terminal() { "\x1b[1;36m" } else { "" }
}

pub fn c_green() -> &'static str {
    if std::io::stdout().is_terminal() { "\x1b[1;32m" } else { "" }
}

pub fn c_red() -> &'static str {
    if std::io::stderr().is_terminal() { "\x1b[1;31m" } else { "" }
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
    } else if trimmed.starts_with("use") {
        &trimmed[3..]
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
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    let safe_name = file_path.to_string_lossy().replace('/', "_").replace('\\', "_");
    let backup_filename = format!("{}_{}.bak", timestamp, safe_name);
    let backup_path = backup_dir.join(backup_filename);
    write_atomic(&backup_path, content)?;
    Ok(backup_path)
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
    let lines: Vec<&str> = content.lines().collect();
    let mut decl_line = String::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*") {
            continue;
        }
        decl_line.push_str(trimmed);
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
                identifier = tokens[i + 1].split(':').next().unwrap_or("").trim_end_matches(';').to_string();
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
                    identifier = tokens[i + 1].trim_end_matches(';').trim_end_matches('{').to_string();
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
                    identifier = tokens[id_idx].split(':').next().unwrap_or("").trim_end_matches(';').to_string();
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
                        parent_target = tokens[pos + 1].split('<').next().unwrap_or("").trim_end_matches('{').to_string();
                        let trait_name = if pos > 0 { tokens[pos - 1].split('<').next().unwrap_or("") } else { "" };
                        identifier = format!("{} for {}", trait_name, parent_target);
                    }
                } else {
                    let target_token = if i + 1 < tokens.len() { tokens[i + 1] } else { "" };
                    parent_target = target_token.split('<').next().unwrap_or("").trim_end_matches('{').to_string();
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

pub fn walk_directory(
    dir: &std::path::Path,
    config: &LayoutConfig,
    dry_run: bool,
    check: bool,
    dbg: bool,
) {
    if !dir.is_dir() {
        return;
    }
    
    let dir_name = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
    if config.ignore_dirs.contains(&dir_name) {
        return;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let child_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if config.ignore_dirs.contains(&child_name) {
                    continue;
                }
                walk_directory(&path, config, dry_run, check, dbg);
            } else if path.is_file() && path.extension().unwrap_or_default() == "rs" {
                crate::process_file(&path, config, dry_run, check, dbg);
            }
        }
    }
}

pub fn parse_toml_config(path: &std::path::Path) -> LayoutConfig {
    let mut config = LayoutConfig::default();
    
    if let Ok(content) = std::fs::read_to_string(path) {
        let mut in_array_key: Option<String> = None;
        let mut array_accumulator = String::new();

        for line in content.lines() {
            let trimmed = line.trim();
            
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }

            if let Some(ref key) = in_array_key {
                array_accumulator.push_str(" ");
                array_accumulator.push_str(trimmed);
                
                if trimmed.ends_with(']') {
                    if !apply_config_value(&mut config, key, &array_accumulator) {
                        eprintln!("Error: Invalid configuration syntax for {}", key);
                        std::process::exit(1);
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
                    eprintln!("Error: Invalid configuration syntax for {}", key);
                    std::process::exit(1);
                }
            }
        }
        
        if let Some(key) = in_array_key {
            eprintln!("Error: Unterminated array in configuration for {}", key);
            std::process::exit(1);
        }
    }
    
    config
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
                    config.type_order.push(ItemKind::from_str(p));
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

pub fn process_file(file_path: &Path, config: &LayoutConfig, dry_run: bool, dbg: bool, check: bool) {
    let source = match fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("{}Error reading {}: {}{}", c_red(), file_path.display(), e, c_reset(true));
            return;
        }
    };
    if dbg {
        println!("{}--- [DBG] Processing file: {} ---", c_cyan(), file_path.display());
    }
    let reorganized = reorganize_file(source.clone(), config, dbg);
    if check {
        if source != reorganized {
            eprintln!("{}Formatting drift detected in {}{}", c_red(), file_path.display(), c_reset(true));
            std::process::exit(1);
        }
        return;
    }
    if dry_run {
        println!("{}[DRY RUN]{} Would process: {}", c_cyan(), c_reset(false), file_path.display());
        return;
    }
    let backup_path = match create_backup(file_path, &source) {
        Ok(path) => {
            if dbg {
                println!("{}--- [DBG] Created backup at: {} ---", c_cyan(), path.display());
            }
            path
        }
        Err(e) => {
            eprintln!("{}Failed to create backup, aborting {}: {}{}", c_red(), file_path.display(), e, c_reset(true));
            return;
        }
    };
    if let Err(e) = write_atomic(file_path, &reorganized) {
        eprintln!("{}Atomic write failed for {}: {}. Auto-restoring...{}", c_red(), file_path.display(), e, c_reset(true));
        let _ = restore_backup(&backup_path, file_path);
        return;
    }
    println!("{}Formatted:{} {}", c_green(), c_reset(false), file_path.display());
    println!("  {}Undo:{} mto --restore {} {}", c_cyan(), c_reset(false), backup_path.display(), file_path.display());
}

pub fn process_init(target_dir: Option<PathBuf>) {
    let dir = target_dir.unwrap_or_else(|| PathBuf::from("."));
    if !dir.exists() {
        if let Err(e) = fs::create_dir_all(&dir) {
            eprintln!("{}Error creating directory {}: {}{}", c_red(), dir.display(), e, c_reset(true));
            std::process::exit(1);
        }
    }
    let config_file_path = dir.join("mto.toml");
    if config_file_path.exists() {
        eprintln!("{}Error: Configuration file already exists at {}{}", c_yellow(true), config_file_path.display(), c_reset(true));
        std::process::exit(1);
    }
    let default_config_template = r#"# mto configuration file
sort_alphabetically = true
group_impls = true
sort_imports = true
import_grouping = true
blank_lines_between_blocks = 1
max_width = 100
tab_width = 4
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
    match write_atomic(&config_file_path, default_config_template) {
        Ok(_) => {
            println!("{}Initialized default configuration:{} {}", c_green(), c_reset(false), config_file_path.display());
        }
        Err(e) => {
            eprintln!("{}Failed to write configuration file: {}{}", c_red(), e, c_reset(true));
            std::process::exit(1);
        }
    }
}

pub fn process_stdin(config_path: Option<PathBuf>, check: bool) {
    let resolved_config = resolve_config_path(config_path, None);
    let config = match resolved_config {
        Some(path) => {
            if let Some(parent) = path.parent() {
                let _ = parent;
            }
            parse_toml_config(&path)
        }
        None => LayoutConfig::default(),
    };
    let mut source = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut source) {
        eprintln!("{}Error reading from stdin: {}{}", c_red(), e, c_reset(true));
        std::process::exit(1);
    }
    let reorganized = reorganize_file(source.clone(), &config, false);
    if check {
        if source != reorganized {
            eprintln!("{}Formatting drift detected on stdin.{}", c_red(), c_reset(true));
            std::process::exit(1);
        } else {
            std::process::exit(0);
        }
    } else {
        print!("{}", reorganized);
    }
}

pub fn reflow_delimited_blocks(content: &str, max_width: usize, tab_width: usize) -> String {
    let mut result = String::new();
    let mut chars = content.chars().peekable();
    let indent = " ".repeat(tab_width);

    let mut in_string = false;
    let mut in_raw_string = false;
    let mut raw_hash_count = 0;
    let mut in_char = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut escape_next = false;

    while let Some(c) = chars.next() {
        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }
        if in_raw_string {
            result.push(c);
            if c == '"' {
                let mut peek_clone = chars.clone();
                let mut hashes = 0;
                while let Some(&'#') = peek_clone.peek() {
                    hashes += 1;
                    peek_clone.next();
                }
                if hashes >= raw_hash_count {
                    for _ in 0..raw_hash_count {
                        result.push(chars.next().unwrap());
                    }
                    in_raw_string = false;
                    raw_hash_count = 0;
                }
            }
            continue;
        }
        if c == 'r' && !in_string && !in_char && !in_line_comment && !in_block_comment {
            let mut peek_clone = chars.clone();
            let mut hashes = 0;
            while let Some(&'#') = peek_clone.peek() {
                hashes += 1;
                peek_clone.next();
            }
            if let Some(&'"') = peek_clone.peek() {
                in_raw_string = true;
                raw_hash_count = hashes;
                result.push(c);
                for _ in 0..hashes {
                    result.push(chars.next().unwrap());
                }
                result.push(chars.next().unwrap());
                continue;
            }
        }
        if c == '\\' && (in_string || in_char) {
            result.push(c);
            escape_next = true;
            continue;
        }
        if c == '"' && !in_char && !in_line_comment && !in_block_comment {
            in_string = !in_string;
            result.push(c);
            continue;
        }
        if c == '\'' && !in_string && !in_line_comment && !in_block_comment {
            in_char = !in_char;
            result.push(c);
            continue;
        }
        if c == '/' && !in_string && !in_char && !in_line_comment && !in_block_comment {
            if let Some(&'/') = chars.peek() {
                in_line_comment = true;
                result.push(c);
                continue;
            } else if let Some(&'*') = chars.peek() {
                in_block_comment = true;
                result.push(c);
                continue;
            }
        }
        if c == '\n' && in_line_comment {
            in_line_comment = false;
        }
        if c == '*' && in_block_comment {
            if let Some(&'/') = chars.peek() {
                result.push(c);
                result.push(chars.next().unwrap());
                in_block_comment = false;
                continue;
            }
        }

        let is_active = !in_string && !in_raw_string && !in_char && !in_line_comment && !in_block_comment;

        if is_active && (c == '(' || c == '[') {
            let open_char = c;
            let close_char = match c {
                '(' => ')',
                '[' => ']',
                _ => unreachable!(),
            };

            let mut inner = String::new();
            let mut depth = 1;
            
            let mut i_in_string = false;
            let mut i_in_char = false;
            let mut i_escape = false;

            while let Some(nc) = chars.next() {
                if i_escape {
                    inner.push(nc);
                    i_escape = false;
                    continue;
                }
                if nc == '\\' && (i_in_string || i_in_char) {
                    inner.push(nc);
                    i_escape = true;
                    continue;
                }
                if nc == '"' && !i_in_char {
                    i_in_string = !i_in_string;
                }
                if nc == '\'' && !i_in_string {
                    i_in_char = !i_in_char;
                }

                if !i_in_string && !i_in_char {
                    if nc == open_char {
                        depth += 1;
                    } else if nc == close_char {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                inner.push(nc);
            }

            let full_expr = format!("{}{}{}", open_char, inner, close_char);
            
            if full_expr.chars().count() > max_width && !inner.contains('"') && !inner.contains('\'') {
                result.push(open_char);
                result.push('\n');
                
                let mut item = String::new();
                let mut i_depth: usize = 0;
                for ic in inner.chars() {
                    match ic {
                        '(' | '[' | '{' | '<' => i_depth += 1,
                        ')' | ']' | '}' | '>' => i_depth = i_depth.saturating_sub(1),
                        ',' if i_depth == 0 => {
                            let trimmed = item.trim();
                            if !trimmed.is_empty() {
                                result.push_str(&indent);
                                result.push_str(trimmed);
                                result.push_str(",\n");
                            }
                            item.clear();
                            continue;
                        }
                        _ => {}
                    }
                    item.push(ic);
                }
                let trimmed = item.trim();
                if !trimmed.is_empty() {
                    result.push_str(&indent);
                    result.push_str(trimmed);
                    result.push_str(",\n");
                }
                result.push(close_char);
            } else {
                result.push_str(&full_expr);
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub fn resolve_config_path(cli_path: Option<PathBuf>, target_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = cli_path {
        return Some(path);
    }
    let start_dir = match target_path {
        Some(p) => {
            if p.is_file() {
                p.parent().map(|s| s.to_path_buf()).unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
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

pub fn extract_blocks(source: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let mut current_block = String::new();
    let mut chars = source.chars().peekable();
    let mut brace_depth = 0;
    let mut in_string = false;
    let mut in_raw_string = false;
    let mut raw_hash_count = 0;
    let mut in_char = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut escape_next = false;
    let mut in_mto_off = false;

    // Determine if a closing brace '}' should NOT flush the block 
    // because it is conceptually a semicolon-terminated statement.
    let is_semicolon_prefix = |block: &str| -> bool {
        let first_line = block
            .lines()
            .map(|l| l.trim())
            .find(|l| {
                !l.is_empty()
                    && !l.starts_with("//")
                    && !l.starts_with("#[")
                    && !l.starts_with("/*")
            })
            .unwrap_or("");
        let t = first_line;
        
        // Notice: ALL `mod` prefixes have been permanently removed.
        t.starts_with("use")
            || (t.starts_with("const") && !t.contains("fn"))
            || t.starts_with("static")
            || t.starts_with("type")
            || t.starts_with("extern")
            || t.starts_with("pub use")
            || t.starts_with("pub const")
            || t.starts_with("pub static")
            || t.starts_with("pub type")
            || t.starts_with("pub extern")
            || t.starts_with("pub(crate) use")
            || t.starts_with("pub(crate) const")
            || t.starts_with("pub(crate) static")
            || t.starts_with("pub(crate) type")
            || t.starts_with("pub(super) use")
            || t.starts_with("pub(super) const")
            || t.starts_with("pub(super) static")
            || t.starts_with("pub(super) type")
    };

    while let Some(c) = chars.next() {
        current_block.push(c);
        
        if in_mto_off {
            if current_block.trim_end().ends_with("// mto: on") {
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
        
        if current_block.trim_end().ends_with("// mto: off") {
            let prefix = current_block.trim_end_matches("// mto: off").trim();
            if !prefix.is_empty() {
                let idx = blocks.len();
                blocks.push(parse_block_metadata(prefix, false, idx));
            }
            current_block.clear();
            current_block.push_str("// mto: off\n");
            in_mto_off = true;
            continue;
        }
        
        let semicolon_stmt = is_semicolon_prefix(&current_block);
        
        match c {
            '\\' if in_string || in_char => {
                escape_next = true;
            }
            '\'' if !in_string && !in_line_comment && !in_block_comment && !in_raw_string && !in_char => {
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
            'r' if !in_string && !in_char && !in_line_comment && !in_block_comment && !in_raw_string => {
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
            '"' if !in_char && !in_line_comment && !in_block_comment && !in_raw_string => {
                in_string = !in_string;
            }
            '/' if !in_string && !in_char && !in_line_comment && !in_block_comment && !in_raw_string => {
                if let Some(&'/') = chars.peek() {
                    in_line_comment = true;
                } else if let Some(&'*') = chars.peek() {
                    in_block_comment = true;
                }
            }
            '\n' if in_line_comment => {
                in_line_comment = false;
            }
            '*' if in_block_comment => {
                if let Some(&'/') = chars.peek() {
                    current_block.push(chars.next().unwrap());
                    in_block_comment = false;
                }
            }
            // UNIVERSAL BRACE COUNTING
            '{' if !in_string && !in_char && !in_line_comment && !in_block_comment && !in_raw_string => {
                brace_depth += 1;
            }
            '}' if !in_string && !in_char && !in_line_comment && !in_block_comment && !in_raw_string => {
                brace_depth -= 1;
                // Only flush if brace hits 0 AND it's not waiting for a semicolon.
                if brace_depth == 0 && !semicolon_stmt {
                    let trimmed = current_block.trim();
                    if !trimmed.is_empty() {
                        let idx = blocks.len();
                        blocks.push(parse_block_metadata(trimmed, false, idx));
                    }
                    current_block.clear();
                }
            }
            ';' if brace_depth == 0 && !in_string && !in_char && !in_line_comment && !in_block_comment && !in_raw_string => {
                let trimmed = current_block.trim();
                if !trimmed.is_empty() {
                    let idx = blocks.len();
                    blocks.push(parse_block_metadata(trimmed, false, idx));
                }
                current_block.clear();
            }
            _ => {}
        }
    }
    
    let trailing = current_block.trim();
    if !trailing.is_empty() {
        let idx = blocks.len();
        blocks.push(parse_block_metadata(trailing, in_mto_off, idx));
    }
    
    blocks
}

pub fn reorganize_file(source: String, config: &LayoutConfig, dbg: bool) -> String {
    let mut blocks = extract_blocks(&source);
    if dbg {
        println!("{}--- [DBG] Extracted {} blocks from source ---", c_cyan(), blocks.len());
        for (idx, block) in blocks.iter().enumerate() {
            println!(
                "  [Block #{}] Kind: {:?}, Visibility: {:?}, Identifier: '{}', Parent Target: '{}'",
                idx, block.kind, block.visibility, block.identifier, block.parent_target
            );
        }
    }
    sort_blocks(&mut blocks, config);
    if dbg {
        println!("{}--- [DBG] Blocks after sorting ---", c_cyan());
        for (idx, block) in blocks.iter().enumerate() {
            println!("  [Sorted #{}] Kind: {:?}, Identifier: '{}'", idx, block.kind, block.identifier);
        }
    }
    let mut output = String::new();
    let mut previous_block: Option<CodeBlock> = None;
    for block in blocks {
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
            } else if prev.kind == ItemKind::ModDecl && block.kind == ItemKind::ModDecl {
                output.push('\n');
            } else {
                let newlines = "\n".repeat(config.blank_lines_between_blocks + 1);
                output.push_str(&newlines);
            }
        }
        let styled_content = apply_width_constraints(&block.raw_content, config.max_width, config.tab_width);
        output.push_str(&styled_content);
        previous_block = Some(block.clone());
    }
    output.trim_end().to_string() + "\n"
}

pub fn sort_blocks(blocks: &mut Vec<CodeBlock>, config: &LayoutConfig) {
    let mut skipped_items = Vec::new();
    let mut normal_blocks = Vec::new();
    for (idx, block) in blocks.drain(..).enumerate() {
        if block.skip {
            skipped_items.push((idx, block));
        } else {
            normal_blocks.push(block);
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
        if a.kind == ItemKind::UseDecl && b.kind == ItemKind::UseDecl {
            if config.sort_imports {
                if config.import_grouping {
                    let group_a = classify_import(&a.raw_content);
                    let group_b = classify_import(&b.raw_content);
                    if group_a != group_b {
                        return group_a.cmp(&group_b);
                    }
                }
                return a.identifier.cmp(&b.identifier);
            }
        }
        let mut family_a = &a.identifier;
        if config.group_impls {
            if a.kind == ItemKind::ImplBlock {
                family_a = &a.parent_target;
            }
        }
        let mut family_b = &b.identifier;
        if config.group_impls {
            if b.kind == ItemKind::ImplBlock {
                family_b = &b.parent_target;
            }
        }
        let mut kind_a = &a.kind;
        if config.group_impls {
            if a.kind == ItemKind::ImplBlock {
                for (name, kind) in family_map.iter() {
                    if name == family_a {
                        kind_a = kind;
                        break;
                    }
                }
            }
        }
        let mut kind_b = &b.kind;
        if config.group_impls {
            if b.kind == ItemKind::ImplBlock {
                for (name, kind) in family_map.iter() {
                    if name == family_b {
                        kind_b = kind;
                        break;
                    }
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
            let is_groupable = |k: &ItemKind| matches!(k, ItemKind::StructDecl | ItemKind::EnumDecl | ItemKind::TraitDecl | ItemKind::ImplBlock);
            if is_groupable(&a.kind) && is_groupable(&b.kind) {
                if family_a != family_b {
                    return family_a.cmp(family_b);
                }
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
    let mut final_blocks: Vec<Option<CodeBlock>> = std::iter::repeat_with(|| None).take(total_len).collect();
    for (orig_idx, block) in skipped_items {
        if orig_idx < total_len {
            final_blocks[orig_idx] = Some(block);
        } else {
            final_blocks.push(Some(block));
        }
    }
    let mut normal_iter = normal_blocks.into_iter();
    for slot in final_blocks.iter_mut() {
        if slot.is_none() {
            if let Some(normal_block) = normal_iter.next() {
                *slot = Some(normal_block);
            }
        }
    }
    *blocks = final_blocks.into_iter().map(|b| b.unwrap()).collect();
}

pub fn wrap_line(line: &str, max_width: usize, tab_width: usize) -> String {
    if line.chars().count() <= max_width {
        return line.to_string();
    }
    let leading = line.chars().take_while(|c| *c == ' ').count();
    let leading_str = &line[..leading];
    let content = &line[leading..];

    let mut in_string = false;
    let mut in_char = false;
    let mut in_raw_string = false;
    let mut raw_hash_count = 0;
    let mut escape_next = false;
    let mut comment_idx = None;

    let mut char_indices = content.char_indices().peekable();
    while let Some((i, c)) = char_indices.next() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if in_raw_string {
            if c == '"' {
                let mut peek_clone = char_indices.clone();
                let mut hashes = 0;
                while let Some(&(_, '#')) = peek_clone.peek() {
                    hashes += 1;
                    peek_clone.next();
                }
                if hashes >= raw_hash_count {
                    for _ in 0..raw_hash_count {
                        char_indices.next();
                    }
                    in_raw_string = false;
                    raw_hash_count = 0;
                }
            }
            continue;
        }
        if c == 'r' && !in_string && !in_char {
            let mut peek_clone = char_indices.clone();
            let mut hashes = 0;
            while let Some(&(_, '#')) = peek_clone.peek() {
                hashes += 1;
                peek_clone.next();
            }
            if let Some(&(_, '"')) = peek_clone.peek() {
                in_raw_string = true;
                raw_hash_count = hashes;
                for _ in 0..hashes {
                    char_indices.next();
                }
                char_indices.next();
                continue;
            }
        }
        if c == '\\' && (in_string || in_char) {
            escape_next = true;
            continue;
        }
        if c == '"' && !in_char {
            in_string = !in_string;
            continue;
        }
        if c == '\'' && !in_string {
            in_char = !in_char;
            continue;
        }
        if c == '/' && !in_string && !in_char {
            if let Some(&(_, '/')) = char_indices.peek() {
                comment_idx = Some(i);
                break;
            }
        }
    }

    let (code_part, comment_part) = if let Some(idx) = comment_idx {
        (&content[..idx], Some(&content[idx..]))
    } else {
        (content, None)
    };

    let wrap_prose = |text: &str, first_line_prefix: &str, ind: &str, cont_prefix: &str| -> Vec<String> {
        let mut lines = Vec::new();
        let clean = text.trim_start_matches('/').trim_start();
        
        let mut curr_line = first_line_prefix.to_string();
        let mut curr_len = curr_line.chars().count();
        let base_len_first = curr_len;
        let base_len_cont = ind.chars().count() + cont_prefix.chars().count();

        for word in clean.split_whitespace() {
            let w_len = word.chars().count();
            let base_len = if lines.is_empty() { base_len_first } else { base_len_cont };
            
            // Explicitly account for the space we are about to inject
            let needed_space = if curr_len > base_len { 1 } else { 0 };

            if curr_len + needed_space + w_len > max_width && curr_len > base_len {
                lines.push(curr_line.trim_end().to_string());
                curr_line = format!("{}{}{}", ind, cont_prefix, word);
                curr_len = base_len_cont + w_len;
            } else {
                if curr_len > base_len {
                    curr_line.push(' ');
                    curr_len += 1;
                }
                curr_line.push_str(word);
                curr_len += w_len;
            }
        }
        if !curr_line.trim().is_empty() {
            lines.push(curr_line.trim_end().to_string());
        }
        if lines.is_empty() {
            lines.push(first_line_prefix.trim_end().to_string());
        }
        lines
    };

    if code_part.trim().is_empty() && comment_part.is_some() {
        let first_prefix = format!("{}// ", leading_str);
        return wrap_prose(comment_part.unwrap(), &first_prefix, leading_str, "// ").join("\n");
    }

    let continuation_indent = leading_str.to_string() + &" ".repeat(tab_width);

    let mut tokens = Vec::new();
    let mut current_token = String::new();
    let mut paren_depth: usize = 0;
    let mut bracket_depth: usize = 0;
    let mut brace_depth: usize = 0;
    let mut angle_depth: usize = 0;
    in_string = false;
    in_raw_string = false;
    raw_hash_count = 0;
    escape_next = false;
    in_char = false;

    let mut chars_iter = code_part.chars().peekable();
    while let Some(c) = chars_iter.next() {
        if escape_next {
            current_token.push(c);
            escape_next = false;
            continue;
        }
        if in_raw_string {
            current_token.push(c);
            if c == '"' {
                let mut peek_clone = chars_iter.clone();
                let mut hashes = 0;
                while let Some(&'#') = peek_clone.peek() {
                    hashes += 1;
                    peek_clone.next();
                }
                if hashes >= raw_hash_count {
                    for _ in 0..raw_hash_count {
                        current_token.push(chars_iter.next().unwrap());
                    }
                    in_raw_string = false;
                    raw_hash_count = 0;
                }
            }
            continue;
        }
        if c == 'r' && !in_string && !in_char {
            let mut peek_clone = chars_iter.clone();
            let mut hashes = 0;
            while let Some(&'#') = peek_clone.peek() {
                hashes += 1;
                peek_clone.next();
            }
            if let Some(&'"') = peek_clone.peek() {
                in_raw_string = true;
                raw_hash_count = hashes;
                current_token.push(c);
                for _ in 0..hashes {
                    current_token.push(chars_iter.next().unwrap());
                }
                current_token.push(chars_iter.next().unwrap());
                continue;
            }
        }
        if c == '\\' && (in_string || in_char) {
            current_token.push(c);
            escape_next = true;
            continue;
        }
        if c == '"' && !in_char {
            in_string = !in_string;
            current_token.push(c);
            continue;
        }
        if c == '\'' && !in_string {
            in_char = !in_char;
            current_token.push(c);
            continue;
        }
        if !in_string && !in_char {
            match c {
                '(' => paren_depth += 1,
                ')' => paren_depth = paren_depth.saturating_sub(1),
                '[' => bracket_depth += 1,
                ']' => bracket_depth = bracket_depth.saturating_sub(1),
                '{' => brace_depth += 1,
                '}' => brace_depth = brace_depth.saturating_sub(1),
                '<' => {
                    if chars_iter.peek() != Some(&'=') && chars_iter.peek() != Some(&'<') {
                        angle_depth += 1;
                    }
                }
                '>' => {
                    if angle_depth > 0 && chars_iter.peek() != Some(&'=') {
                        angle_depth = angle_depth.saturating_sub(1);
                    }
                }
                _ => {}
            }
        }
        current_token.push(c);
        let at_safe_break = !in_string
            && !in_raw_string
            && !in_char
            && paren_depth == 0
            && bracket_depth == 0
            && brace_depth == 0
            && angle_depth == 0
            && (c == ',' || c == ' ');
        if at_safe_break {
            tokens.push(current_token.clone());
            current_token.clear();
        }
    }
    if !current_token.is_empty() {
        tokens.push(current_token);
    }
    let mut result_lines = Vec::new();
    let mut current_line = leading_str.to_string();
    let mut current_len = leading;
    for token in tokens {
        let token_len = token.chars().count();
        if current_len + token_len > max_width && current_len > leading {
            result_lines.push(current_line.trim_end().to_string());
            current_line = continuation_indent.clone() + &token;
            current_len = continuation_indent.chars().count() + token_len;
        } else {
            current_line.push_str(&token);
            current_len += token_len;
        }
    }

    if let Some(c_part) = comment_part {
        let sep = if current_line.ends_with(' ') { "// " } else { " // " };
        let first_prefix = format!("{}{}", current_line, sep);
        let wrapped_comment = wrap_prose(c_part, &first_prefix, leading_str, "// ");
        result_lines.extend(wrapped_comment);
    } else if !current_line.trim().is_empty() {
        result_lines.push(current_line.trim_end().to_string());
    }

    if result_lines.is_empty() {
        String::new()
    } else {
        result_lines.join("\n")
    }
}

pub fn write_atomic(target: &Path, content: &str) -> Result<(), std::io::Error> {
    let tmp_path = target.with_extension("tmp_mtindo");
    fs::write(&tmp_path, content)?;
    match fs::rename(&tmp_path, target) {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(tmp_path);
            Err(e)
        }
    }
}
