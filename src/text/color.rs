//! Terminal colors for help screens and messages.
//!
//! This file is also compiled into the xtask runner with `#[path]`, so that the runner
//! builds even when the rest of mtindo does not. Keep it self-contained: std only, no
//! `crate::` paths.

use std::env;
use std::io::IsTerminal;

/// `--color`: 0 auto, 1 always, 2 never.
static CHOICE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Colors in the style of Python's argparse help, shared by every help screen in the project
/// so they look alike: bold blue headings, bold magenta program name, bold green short flags,
/// bold cyan long flags, bold yellow values; usage lines use the same hues without bold.
pub struct HelpPalette {
    on: bool,
    program: &'static str,
}

impl HelpPalette {
    pub fn new(on: bool, program: &'static str) -> Self {
        HelpPalette { on, program }
    }

    pub fn heading(&self, text: &str) -> String {
        self.paint("1;34", text)
    }

    pub fn long(&self, text: &str) -> String {
        self.paint("1;36", text)
    }

    pub fn paint(&self, code: &str, text: &str) -> String {
        if self.on && !text.is_empty() {
            format!("\x1b[{}m{}\x1b[0m", code, text)
        } else {
            text.to_string()
        }
    }

    pub fn program(&self, text: &str) -> String {
        self.paint("1;35", text)
    }

    /// A help row: `label` (whose visible width is `width`) padded to `column`, then `help`.
    /// A label too wide for the column puts the help on the next line.
    pub fn row(&self, label: &str, width: usize, column: usize, help: &str) -> String {
        if width + 2 <= column {
            format!("{}{}{}\n", label, " ".repeat(column - width), help)
        } else {
            format!("{}\n{}{}\n", label, " ".repeat(column), help)
        }
    }

    pub fn short(&self, text: &str) -> String {
        self.paint("1;32", text)
    }

    /// A usage line: the program name, then flags and placeholders in their colors.
    pub fn usage_line(&self, line: &str) -> String {
        line.split(' ')
            .map(|word| {
                if self.program.split(' ').any(|part| part == word) {
                    self.program(word)
                } else if word.starts_with("--") {
                    self.paint("36", word)
                } else if word.starts_with('-') {
                    self.paint("32", word)
                } else if word.starts_with('<')
                    || word.starts_with('[')
                    || (word.contains('|') && word.len() > 1)
                    || word.ends_with(']')
                {
                    self.paint("33", word)
                } else {
                    word.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn value(&self, text: &str) -> String {
        self.paint("1;33", text)
    }
}

/// Whether to color stdout (or stderr, with `is_err`). `--color always|never` decides when
/// given; otherwise `NO_COLOR` turns color off and wins, then `FORCE_COLOR` turns it on,
/// then color goes only to a terminal.
pub fn color_enabled(is_err: bool) -> bool {
    match CHOICE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    // Asked for every line of a diff, so worked out once per stream: the answer cannot
    // change during a run.
    static DETECTED: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    let (stdout, stderr) = *DETECTED.get_or_init(|| {
        let set = |name: &str| env::var_os(name).is_some_and(|value| !value.is_empty());
        if set("NO_COLOR") {
            return (false, false);
        }
        if set("FORCE_COLOR") {
            return (true, true);
        }
        (
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal(),
        )
    });
    if is_err { stderr } else { stdout }
}

/// Sets `--color`: `Some(true)` always, `Some(false)` never, `None` automatic.
pub fn set_color(choice: Option<bool>) {
    let value = match choice {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    CHOICE.store(value, std::sync::atomic::Ordering::Relaxed);
}
