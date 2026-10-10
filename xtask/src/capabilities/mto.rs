//! mto's own surface, read from the repository so it cannot drift from the code:
//! `flag:` items from `FLAGS` and `command:` items from `COMMANDS` in `src/main.rs`, and
//! `config:` items from mto's own settings table, `SETTINGS` in `src/options/mto.rs`
//! (rustfmt's keys are tracked as `option` items).

use std::fs;
use std::path::Path;

/// The first string of each tuple in `COMMANDS`.
pub fn commands(main: &str) -> Result<Vec<String>, String> {
    let body = block(main, "const COMMANDS", "\n];")?;
    let mut names = Vec::new();
    let mut previous = "";
    for line in body.lines() {
        let trimmed = line.trim();
        if previous == "(" && trimmed.starts_with('"') {
            names.push(trimmed.trim_end_matches(',').trim_matches('"').to_string());
        }
        if !trimmed.is_empty() {
            previous = trimmed;
        }
    }
    if names.is_empty() {
        return Err("COMMANDS in src/main.rs lists nothing".to_string());
    }
    Ok(names)
}

/// The `name:` of each setting in mto's `SETTINGS` table.
pub fn config_keys(table: &str) -> Result<Vec<String>, String> {
    let body = block(table, "static SETTINGS", "\n];")?;
    let keys: Vec<String> = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("name: \""))
        .map(|rest| rest.trim_end_matches(',').trim_end_matches('"').to_string())
        .collect();
    if keys.is_empty() {
        return Err("SETTINGS in src/options/mto.rs lists nothing".to_string());
    }
    Ok(keys)
}

/// The `long:` names in `FLAGS`.
pub fn flags(main: &str) -> Result<Vec<String>, String> {
    let body = block(main, "const FLAGS", "\n];")?;
    let flags: Vec<String> = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("long: \""))
        .map(|rest| rest.trim_end_matches(',').trim_end_matches('"').to_string())
        .collect();
    if flags.is_empty() {
        return Err("FLAGS in src/main.rs lists nothing".to_string());
    }
    Ok(flags)
}

pub fn surface(root: &Path) -> Result<Vec<String>, String> {
    let main = fs::read_to_string(root.join("src").join("main.rs"))
        .map_err(|e| format!("cannot read src/main.rs: {}", e))?;
    let table = fs::read_to_string(root.join("src").join("options").join("mto.rs"))
        .map_err(|e| format!("cannot read src/options/mto.rs: {}", e))?;
    let mut items = Vec::new();
    for flag in flags(&main)? {
        items.push(format!("flag:{}", flag));
    }
    for command in commands(&main)? {
        items.push(format!("command:{}", command));
    }
    for key in config_keys(&table)? {
        items.push(format!("config:{}", key));
    }
    items.sort();
    Ok(items)
}

/// The text from `start` to the end of the item (`\n];` or `\n}`) that follows it.
fn block<'a>(text: &'a str, start: &str, end: &str) -> Result<&'a str, String> {
    let from = text
        .find(start)
        .ok_or_else(|| format!("`{}` not found", start))?;
    let rest = &text[from..];
    let to = rest
        .find(end)
        .ok_or_else(|| format!("end of `{}` not found", start))?;
    Ok(&rest[..to])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsers_on_small_inputs() {
        let main = "const COMMANDS: &[(&str, &str, &str)] = &[\n    (\n        \"init\",\n        \"[DIR]\",\n        \"Write\",\n    ),\n];\nconst FLAGS: &[FlagSpec] = &[\n    FlagSpec {\n        long: \"--check\",\n    },\n];\n";
        assert_eq!(commands(main).unwrap(), vec!["init"]);
        assert_eq!(flags(main).unwrap(), vec!["--check"]);
        let table = "pub(super) static SETTINGS: &[Setting] = &[\n    Setting {\n        name: \"group_impls\",\n        file: File::Mto,\n    },\n];\n";
        assert_eq!(config_keys(table).unwrap(), vec!["group_impls"]);
        assert!(commands("").is_err());
    }

    #[test]
    fn test_reads_the_real_repository() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let items = surface(root).unwrap();
        for expected in [
            "flag:--check",
            "flag:--restore",
            "command:init",
            "command:-",
            "config:sort_imports",
            "config:type_order",
        ] {
            assert!(
                items.iter().any(|i| i == expected),
                "{} in {:?}",
                expected,
                items
            );
        }
    }
}
