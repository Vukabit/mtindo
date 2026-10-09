//! mto's own surface, read from the repository so it cannot drift from the code:
//! `flag:` items from `FLAGS` and `command:` items from `COMMANDS` in `src/main.rs`, and
//! `config:` items from the keys `apply_config_value` accepts in `src/lib.rs`.

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

/// The keys matched in `apply_config_value`.
pub fn config_keys(lib: &str) -> Result<Vec<String>, String> {
    let body = block(lib, "pub fn apply_config_value", "\n}\n")?;
    let keys: Vec<String> = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|rest| rest.split_once("\" =>"))
        .map(|(key, _)| key.to_string())
        .collect();
    if keys.is_empty() {
        return Err("apply_config_value in src/lib.rs matches no keys".to_string());
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
    let lib = fs::read_to_string(root.join("src").join("lib.rs"))
        .map_err(|e| format!("cannot read src/lib.rs: {}", e))?;
    let mut items = Vec::new();
    for flag in flags(&main)? {
        items.push(format!("flag:{}", flag));
    }
    for command in commands(&main)? {
        items.push(format!("command:{}", command));
    }
    for key in config_keys(&lib)? {
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

    #[test]
    fn test_parsers_on_small_inputs() {
        let main = "const COMMANDS: &[(&str, &str, &str)] = &[\n    (\n        \"init\",\n        \"[DIR]\",\n        \"Write\",\n    ),\n];\nconst FLAGS: &[FlagSpec] = &[\n    FlagSpec {\n        long: \"--check\",\n    },\n];\n";
        assert_eq!(commands(main).unwrap(), vec!["init"]);
        assert_eq!(flags(main).unwrap(), vec!["--check"]);
        let lib = "pub fn apply_config_value(c: &mut C, key: &str) -> bool {\n    match key {\n        \"max_width\" => {}\n        _ => {}\n    }\n}\n";
        assert_eq!(config_keys(lib).unwrap(), vec!["max_width"]);
        assert!(commands("").is_err());
    }
}
