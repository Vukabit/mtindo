//! The `man` gate: `mto --man` documents everything mto accepts, and mentions nothing it
//! does not. It runs the built `mto` and compares the manual with `mto -h` and
//! `mto config print --default`, which come from the same tables as the parser and the
//! settings checks:
//!
//! - every flag, command and exit code in `-h` is in the manual;
//! - every key `config print --default` prints has an entry (`#### \`key\``);
//! - every entry is a key mto knows (printed by `config print`, or one of rustfmt's hidden
//!   options in `compat/rustfmt-surface.txt`);
//! - every `--flag` the manual mentions is an mto flag, or a rustfmt or cargo-fmt flag that
//!   rustfmt's own descriptions refer to;
//! - every backticked `snake_case` name in the manual is a documented key or one of its
//!   values, so prose cannot keep a key that was renamed or removed.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

pub fn check_manual(root: &Path) -> Vec<String> {
    let run = |args: &[&str]| -> Result<String, String> {
        let output = Command::new("cargo")
            .args(["run", "--quiet", "--bin", "mto", "--"])
            .args(args)
            .current_dir(root)
            .env("NO_COLOR", "1")
            .env("CI", "1")
            .output()
            .map_err(|e| format!("cannot run mto: {}", e))?;
        if !output.status.success() {
            return Err(format!(
                "mto {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let outputs = (
        run(&["--man"]),
        run(&["--help"]),
        run(&["config", "print", "--default"]),
    );
    let (manual, help, defaults) = match outputs {
        (Ok(m), Ok(h), Ok(d)) => (m, h, d),
        (m, h, d) => {
            return [m.err(), h.err(), d.err()].into_iter().flatten().collect();
        }
    };
    let surface = fs::read_to_string(root.join("compat/rustfmt-surface.txt")).unwrap_or_default();
    problems(&manual, &help, &defaults, &surface)
}

fn problems(manual: &str, help: &str, defaults: &str, surface: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let section = |name: &str| -> Vec<&str> {
        let start = help
            .find(&format!("\n{}\n", name))
            .map(|i| i + name.len() + 2);
        let Some(start) = start else {
            return Vec::new();
        };
        help[start..]
            .lines()
            .take_while(|line| line.starts_with("  "))
            .collect()
    };

    // -h -> manual.
    let mto_flags: BTreeSet<&str> = section("options:")
        .iter()
        .flat_map(|line| line.split_whitespace())
        .map(|word| word.trim_end_matches(','))
        .filter(|word| word.starts_with('-') && word.len() > 1)
        .collect();
    for flag in &mto_flags {
        let documented =
            manual.contains(&format!("`{}", flag)) || manual.contains(&format!(", {}", flag));
        if !documented {
            problems.push(format!("the manual does not document flag {}", flag));
        }
    }
    // A command line starts two spaces in; lines indented further continue the
    // description above them.
    for line in section("commands:")
        .into_iter()
        .filter(|l| !l.starts_with("   "))
    {
        let name = line.split_whitespace().next().unwrap_or_default();
        if !manual.contains(&format!("`mto {}", name)) {
            problems.push(format!("the manual does not document command `{}`", name));
        }
    }
    for line in section("exit codes:") {
        let code = line.split_whitespace().next().unwrap_or_default();
        if !manual.contains(&format!("*   `{}`: ", code)) {
            problems.push(format!("the manual does not document exit code {}", code));
        }
    }

    // config print -> manual, and manual -> known keys.
    let entries: BTreeSet<&str> = manual
        .lines()
        .filter_map(|line| line.strip_prefix("#### `")?.strip_suffix('`'))
        .collect();
    let printed: BTreeSet<&str> = defaults
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split(" = ").next())
        .collect();
    let hidden: BTreeSet<&str> = surface
        .lines()
        .filter(|line| line.starts_with("option ") && line.ends_with(" hidden"))
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect();
    for key in &printed {
        if !entries.contains(key) {
            problems.push(format!("the manual has no entry for `{}`", key));
        }
    }
    for key in &entries {
        if !printed.contains(key) && !hidden.contains(key) {
            problems.push(format!(
                "the manual has an entry for `{}`, which mto does not accept",
                key
            ));
        }
    }

    // Mentions in the text.
    let rustfmt_flags: BTreeSet<&str> = surface
        .lines()
        .filter(|line| line.starts_with("flag "))
        .filter_map(|line| line.split_whitespace().nth(1)?.split(':').nth(1))
        .collect();
    let mut seen = BTreeSet::new();
    // Code blocks show other tools' commands too (`cargo install --path .`); only mto's
    // own command lines in them are checked.
    let mut in_code = false;
    let prose: String = manual
        .lines()
        .filter(|line| {
            if line.starts_with("```") {
                in_code = !in_code;
                return false;
            }
            !in_code || line.starts_with("mto ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    for word in prose.split(|c: char| c.is_whitespace() || "`(),.;:".contains(c)) {
        let flag = word.split(['=', '[']).next().unwrap_or_default();
        let is_flag = flag.starts_with("--")
            && flag.len() > 2
            && flag[2..]
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-');
        if is_flag
            && !mto_flags.contains(flag)
            && !rustfmt_flags.contains(flag)
            && seen.insert(flag.to_string())
        {
            problems.push(format!("the manual mentions {}, which is not a flag", flag));
        }
    }
    let values: BTreeSet<String> = manual
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("*   `"))
        .filter_map(|rest| rest.split('`').next())
        .map(str::to_string)
        .collect();
    for (idx, span) in manual.split('`').enumerate() {
        let key_like = span.contains('_')
            && span
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            && span.starts_with(|c: char| c.is_ascii_lowercase());
        if idx % 2 == 1
            && key_like
            && !entries.contains(span)
            && !values.contains(span)
            && seen.insert(span.to_string())
        {
            problems.push(format!(
                "the manual mentions `{}`, which is neither a key nor a value",
                span
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP: &str = "usage: mto\n\noptions:\n  -h, --help      Show\n  --check         Check\n\ncommands:\n  init [DIR]      Write\n\nexit codes:\n  0  success\n";

    #[test]
    fn test_a_consistent_manual_passes() {
        let manual = "*   `-h, --help`: Show.\n*   `--check`: Check.\n*   `mto init [DIR]`: Write.\n*   `0`: success.\n#### `max_width`\nSee `max_width`; rustfmt's `--emit`.\n#### `merge_imports`\n";
        let defaults = "# style edition 2015\nmax_width = 100\n";
        let surface = "option  merge_imports  in=all stable=none hidden\nflag    rustfmt:--emit  in=all stable=all\n";
        assert_eq!(
            problems(manual, HELP, defaults, surface),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_gaps_and_stale_mentions_are_reported() {
        let manual = "*   `--help`: Show.\n*   `0`: success.\n#### `old_key`\nUse `tab_width` with --gone.\n";
        let defaults = "max_width = 100\n";
        let found = problems(manual, HELP, defaults, "");
        assert_eq!(
            found,
            vec![
                "the manual does not document flag --check",
                "the manual does not document flag -h",
                "the manual does not document command `init`",
                "the manual has no entry for `max_width`",
                "the manual has an entry for `old_key`, which mto does not accept",
                "the manual mentions --gone, which is not a flag",
                "the manual mentions `tab_width`, which is neither a key nor a value",
            ]
        );
    }
}
