//! `cargo xtask capabilities report`: how much of rustfmt mto supports, written for people.
//!
//! The headline comes first (progress bars for the stable and unstable surface and for
//! rustfmt's own tests), then the counts by kind, then the stable items still missing,
//! which is the work list, then how nightly differs from the stable release, and last the
//! exact rustfmt versions compared. Terminal output fits 80 columns; `--markdown` gives the
//! same content as Markdown.

use std::collections::BTreeMap;

use super::manifest::{Manifest, STATUSES, Status};
use super::surface::{Kind, Surface};

const BAR_WIDTH: usize = 16;

const LINE_WIDTH: usize = 80;

/// What running rustfmt's own test cases through mto gave.
pub enum Cases {
    Ran {
        configured: usize,
        passed: usize,
        runnable: usize,
    },
    Unavailable(String),
}

#[derive(Clone, Copy)]
pub enum Style {
    Terminal { color: bool },
    Markdown,
}

struct Painter {
    style: Style,
}

impl Painter {
    fn paint(&self, code: &str, text: &str) -> String {
        match self.style {
            Style::Terminal { color: true } if !text.is_empty() => {
                format!("\x1b[{}m{}\x1b[0m", code, text)
            }
            _ => text.to_string(),
        }
    }

    fn heading(&self, text: &str) -> String {
        match self.style {
            Style::Markdown => format!("## {}\n\n", text),
            Style::Terminal { .. } => format!("{}\n", self.paint("1", text)),
        }
    }

    fn markdown(&self) -> bool {
        matches!(self.style, Style::Markdown)
    }

    fn status(&self, status: Status, text: &str) -> String {
        let code = match status {
            Status::Supported => "32",
            Status::Partial => "33",
            Status::Missing => "31",
            Status::NotPlanned => "90",
        };
        self.paint(code, text)
    }
}

/// One table row: (kind label, stability label, counts in [`STATUSES`] order).
struct Row {
    kind: &'static str,
    stability: &'static str,
    counts: [usize; 4],
}

impl Row {
    fn total(&self) -> usize {
        self.counts.iter().sum()
    }
}

pub fn render(surface: &Surface, manifest: &Manifest, cases: &Cases, style: Style) -> String {
    let p = Painter { style };
    let reference = ["stable", "local", "local-nightly", "nightly"]
        .into_iter()
        .find(|l| surface.has_source(l))
        .unwrap_or("stable");
    let statuses: BTreeMap<(Kind, String), Status> = manifest
        .lines
        .iter()
        .map(|l| ((l.kind, l.name.clone()), l.status))
        .collect();
    let is_stable = |kind: Kind, name: &str| {
        surface
            .items
            .get(&(kind, name.to_string()))
            .is_some_and(|e| e.stable.contains(reference))
    };
    let count = |kind: Kind, stable: Option<bool>| {
        let mut counts = [0; 4];
        for ((k, name), status) in statuses.iter() {
            if *k == kind && stable.is_none_or(|s| is_stable(kind, name) == s) {
                counts[status_index(*status)] += 1;
            }
        }
        counts
    };
    let mut rows = Vec::new();
    for (kind, label) in [
        (Kind::Option, "options"),
        (Kind::Value, "values"),
        (Kind::Flag, "flags"),
    ] {
        rows.push(Row {
            kind: label,
            stability: "stable",
            counts: count(kind, Some(true)),
        });
        rows.push(Row {
            kind: label,
            stability: "unstable",
            counts: count(kind, Some(false)),
        });
    }
    let own = Row {
        kind: "mto's own features",
        stability: "",
        counts: count(Kind::Mto, None),
    };
    let sum = |stability: &str| {
        let mut counts = [0; 4];
        for row in rows.iter().filter(|r| r.stability == stability) {
            for (idx, c) in row.counts.iter().enumerate() {
                counts[idx] += c;
            }
        }
        counts
    };
    let stable = sum("stable");
    let unstable = sum("unstable");

    let mut out = String::new();
    let release = version_of(surface, reference);
    let title = format!("mto against rustfmt {} ({})", release, role(reference));
    if p.markdown() {
        out.push_str(&format!("# {}\n\n", title));
    } else {
        out.push_str(&format!("{}\n\n", p.paint("1", &title)));
    }

    // Headline.
    let headline = [("Stable surface", stable), ("Unstable surface", unstable)];
    for (label, counts) in headline {
        let total: usize = counts.iter().sum();
        let text = format!(
            "{} of {} supported ({}), {} partial",
            counts[0],
            total,
            percent(counts[0], total),
            counts[1]
        );
        out.push_str(&headline_line(
            &p, label, counts[0], counts[1], total, &text,
        ));
    }
    match cases {
        Cases::Ran {
            configured,
            passed,
            runnable,
        } => {
            let text = format!(
                "{} of {} pass ({}); {} need options",
                passed,
                runnable,
                percent(*passed, *runnable),
                configured
            );
            out.push_str(&headline_line(
                &p,
                "rustfmt's tests",
                *passed,
                0,
                *runnable,
                &text,
            ));
        }
        Cases::Unavailable(why) => {
            if p.markdown() {
                out.push_str(&format!("- **rustfmt's tests:** {}\n", why));
            } else {
                out.push_str(&format!("  {:<18}{}\n", "rustfmt's tests", why));
            }
        }
    }

    // Counts by kind.
    out.push('\n');
    out.push_str(&p.heading("By kind"));
    out.push_str(&table(&p, &rows, &own));

    // The work list.
    out.push('\n');
    out.push_str(&p.heading("Still missing from stable rustfmt"));
    let missing = |kind: Kind| -> Vec<String> {
        statuses
            .iter()
            .filter(|((k, name), status)| {
                *k == kind && **status == Status::Missing && is_stable(kind, name)
            })
            .map(|((_, name), _)| name.clone())
            .collect()
    };
    // A value of a missing option is implied by the option, so only values of options
    // that are already started are listed; the rest are counted.
    let (values, implied): (Vec<String>, Vec<String>) =
        missing(Kind::Value).into_iter().partition(|value| {
            let option = value.split('=').next().unwrap_or("");
            statuses
                .get(&(Kind::Option, option.to_string()))
                .is_some_and(|s| *s != Status::Missing)
        });
    let lists = [
        ("options", missing(Kind::Option)),
        ("flags", missing(Kind::Flag)),
        ("values", values),
    ];
    let mut any = false;
    for (label, names) in lists.iter() {
        if names.is_empty() {
            continue;
        }
        any = true;
        out.push_str(&name_list(
            &p,
            &format!("{} ({})", label, names.len()),
            names,
        ));
    }
    if !any {
        out.push_str(&plain(
            &p,
            "Nothing: every stable item is supported, partial or not planned.",
        ));
    }
    if !implied.is_empty() {
        let note = format!("{} more values belong to the options above", implied.len());
        if p.markdown() {
            out.push_str(&format!("- {}\n", note));
        } else {
            out.push_str(&format!("  {:<16}{}\n", "", p.paint("90", &note)));
        }
    }

    // Nightly against the stable release.
    if surface.has_source("stable") && surface.has_source("nightly") {
        out.push('\n');
        out.push_str(&p.heading("Nightly against the stable release"));
        let mut added = Vec::new();
        let mut stabilized = Vec::new();
        let mut removed = Vec::new();
        for ((kind, name), entry) in surface.items.iter() {
            let item = format!("{} {}", kind.name(), name);
            let in_stable = entry.sources.contains("stable");
            let in_nightly = entry.sources.contains("nightly");
            if in_nightly && !in_stable {
                added.push(item);
            } else if in_stable && !in_nightly {
                removed.push(item);
            } else if in_stable
                && entry.stable.contains("nightly")
                && !entry.stable.contains("stable")
            {
                stabilized.push(item);
            }
        }
        if added.is_empty() && stabilized.is_empty() && removed.is_empty() {
            out.push_str(&plain(
                &p,
                "No difference: nightly has the same options, values and flags.",
            ));
        }
        for (label, names) in [
            ("new", &added),
            ("stabilized", &stabilized),
            ("removed", &removed),
        ] {
            if !names.is_empty() {
                out.push_str(&name_list(
                    &p,
                    &format!("{} ({})", label, names.len()),
                    names,
                ));
            }
        }
    }

    // Sources.
    out.push('\n');
    out.push_str(&p.heading("Compared against"));
    let mut sources: Vec<&(String, String)> = surface.sources.iter().collect();
    sources.sort_by_key(|(label, _)| {
        ["stable", "nightly", "local", "local-nightly"]
            .iter()
            .position(|l| l == label)
    });
    for (label, description) in sources {
        let (version, detail) = describe_source(label, description);
        if p.markdown() {
            out.push_str(&format!("- rustfmt {}: {}\n", version, detail));
        } else {
            out.push_str(&format!(
                "  {:<24}{}\n",
                format!("rustfmt {}", version),
                detail
            ));
        }
    }
    let footer = "Every item and its status: compat/capabilities.txt";
    if p.markdown() {
        out.push_str(&format!("\n{}\n", footer));
    } else {
        out.push_str(&format!("\n{}\n", p.paint("90", footer)));
    }
    out
}

/// A bar of [`BAR_WIDTH`] cells: supported, then partial, then the rest. Any non-zero part
/// gets at least one cell, so a single supported item is visible.
fn bar(p: &Painter, done: usize, partial: usize, total: usize) -> String {
    if total == 0 {
        return p.paint("90", &"░".repeat(BAR_WIDTH));
    }
    let cells = |n: usize| {
        if n == 0 {
            0
        } else {
            (n * BAR_WIDTH / total).max(1)
        }
    };
    let full = cells(done).min(BAR_WIDTH);
    let half = cells(partial).min(BAR_WIDTH - full);
    let rest = BAR_WIDTH - full - half;
    format!(
        "{}{}{}",
        p.paint("32", &"█".repeat(full)),
        p.paint("33", &"▓".repeat(half)),
        p.paint("90", &"░".repeat(rest))
    )
}

fn describe_source(label: &str, description: &str) -> (String, String) {
    let words: Vec<&str> = description.split_whitespace().collect();
    let version = words
        .get(1)
        .map(|v| {
            v.trim_end_matches(',')
                .trim_end_matches("-stable")
                .to_string()
        })
        .unwrap_or_default();
    let date = description
        .split_once("released ")
        .map(|(_, d)| d.trim().to_string())
        .or_else(|| {
            let inside = description.split_once('(')?.1;
            inside
                .split_whitespace()
                .nth(1)
                .map(|d| d.trim_end_matches(')').trim_end_matches("),").to_string())
        })
        .unwrap_or_default();
    let commit = words
        .iter()
        .map(|w| w.trim_matches(|c: char| c == ',' || c == '(' || c == ')'))
        .find(|w| w.len() >= 10 && w.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|w| w[..w.len().min(12)].to_string())
        .unwrap_or_default();
    let mut detail = role(label).to_string();
    if !date.is_empty() {
        detail.push_str(&format!(", {}", date));
    }
    if !commit.is_empty() {
        detail.push_str(&format!(" ({})", commit));
    }
    (version, detail)
}

fn headline_line(
    p: &Painter,
    label: &str,
    done: usize,
    partial: usize,
    total: usize,
    text: &str,
) -> String {
    if p.markdown() {
        format!("- **{}:** {}\n", label, text)
    } else {
        format!(
            "  {:<18}{}  {}\n",
            label,
            bar(p, done, partial, total),
            text
        )
    }
}

/// `label  a, b, c` wrapped to the line width with a hanging indent.
fn name_list(p: &Painter, label: &str, names: &[String]) -> String {
    if p.markdown() {
        let quoted: Vec<String> = names.iter().map(|n| format!("`{}`", n)).collect();
        return format!("- **{}:** {}\n", label, quoted.join(", "));
    }
    let indent = 18;
    let mut out = format!("  {:<width$}", label, width = indent - 2);
    let mut column = indent;
    for (idx, name) in names.iter().enumerate() {
        let piece = if idx + 1 < names.len() {
            format!("{},", name)
        } else {
            name.clone()
        };
        let needed = piece.len() + if column > indent { 1 } else { 0 };
        if column > indent && column + needed > LINE_WIDTH {
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            column = indent;
        } else if column > indent {
            out.push(' ');
            column += 1;
        }
        out.push_str(&piece);
        column += piece.len();
    }
    out.push('\n');
    out
}

fn percent(part: usize, total: usize) -> String {
    let permille = (part * 1000).checked_div(total).unwrap_or(0);
    format!("{}.{}%", permille / 10, permille % 10)
}

fn plain(p: &Painter, text: &str) -> String {
    if p.markdown() {
        format!("{}\n", text)
    } else {
        format!("  {}\n", text)
    }
}

fn role(label: &str) -> &'static str {
    match label {
        "stable" => "latest stable release",
        "nightly" => "latest nightly",
        "local" => "installed",
        "local-nightly" => "installed nightly",
        _ => "source",
    }
}

fn status_index(status: Status) -> usize {
    STATUSES.iter().position(|s| *s == status).unwrap_or(0)
}

fn table(p: &Painter, rows: &[Row], own: &Row) -> String {
    let names: Vec<&str> = STATUSES.iter().map(|s| s.name()).collect();
    let mut out = String::new();
    if p.markdown() {
        out.push_str(&format!(
            "| | | {} | total |\n|---|---|{}---:|\n",
            names.join(" | "),
            "---:|".repeat(names.len())
        ));
        for row in rows.iter().chain(std::iter::once(own)) {
            let cells: Vec<String> = row.counts.iter().map(|c| c.to_string()).collect();
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                row.kind,
                row.stability,
                cells.join(" | "),
                row.total()
            ));
        }
        return out;
    }
    let widths = [11, 9, 9, 13];
    out.push_str(&format!("  {:<20}", ""));
    for (idx, (name, status)) in names.iter().zip(STATUSES.iter()).enumerate() {
        out.push_str(&p.status(*status, &format!("{:>w$}", name, w = widths[idx])));
    }
    out.push_str(&format!("{:>7}\n", "total"));
    let mut previous = "";
    for row in rows.iter().chain(std::iter::once(own)) {
        let kind = if row.kind == previous { "" } else { row.kind };
        previous = row.kind;
        if row.stability.is_empty() {
            // The mto row has no stability column; its label runs into it.
            out.push_str(&format!("  {:<20}", row.kind));
        } else {
            out.push_str(&format!("  {:<9}{:<11}", kind, row.stability));
        }
        for (idx, count) in row.counts.iter().enumerate() {
            let cell = format!(
                "{:>w$}",
                if *count == 0 {
                    "·".to_string()
                } else {
                    count.to_string()
                },
                w = widths[idx]
            );
            if *count == 0 {
                out.push_str(&p.paint("90", &cell));
            } else {
                out.push_str(&p.status(STATUSES[idx], &cell));
            }
        }
        out.push_str(&format!("{:>7}\n", row.total()));
    }
    out
}

fn version_of(surface: &Surface, label: &str) -> String {
    surface
        .sources
        .iter()
        .find(|(l, _)| l == label)
        .map(|(l, d)| describe_source(l, d).0)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::surface::{Found, Reading};
    use super::*;

    fn cases() -> Cases {
        Cases::Ran {
            configured: 559,
            passed: 191,
            runnable: 462,
        }
    }

    fn sample_manifest() -> Manifest {
        let (manifest, errors) = Manifest::parse(
            "option max_width supported # tests/x.rs\noption group_imports missing\noption newline_style partial # Unix only\noption brand_new missing\nvalue newline_style=Unix missing\nflag rustfmt:--check missing\nmto flag:--check supported # tests/x.rs\n",
        );
        assert!(errors.is_empty(), "{:?}", errors);
        manifest
    }

    fn sample_surface() -> Surface {
        let reading = |label: &str, description: &str, items: Vec<Found>| Reading {
            label: label.to_string(),
            description: description.to_string(),
            items,
        };
        Surface::from_readings(&[
            reading(
                "stable",
                "rustfmt 1.10.0, rust-lang/rust b940084d7eb6a299eb4bfeb8e34901bc051e7ac4, released 2026-10-01",
                vec![
                    Found::new(Kind::Option, "max_width", true),
                    Found::new(Kind::Option, "group_imports", false),
                    Found::new(Kind::Option, "newline_style", true),
                    Found::new(Kind::Value, "newline_style=Unix", true),
                    Found::new(Kind::Flag, "rustfmt:--check", true),
                ],
            ),
            reading(
                "nightly",
                "rustfmt 1.11.0-nightly, rust-lang/rust a30aa9064df6a504b6a323c6884a2f4cfbfff666, released 2026-10-09",
                vec![
                    Found::new(Kind::Option, "max_width", true),
                    Found::new(Kind::Option, "group_imports", true),
                    Found::new(Kind::Option, "newline_style", true),
                    Found::new(Kind::Value, "newline_style=Unix", true),
                    Found::new(Kind::Option, "brand_new", false),
                    Found::new(Kind::Flag, "rustfmt:--check", true),
                ],
            ),
            reading(
                "local",
                "rustfmt 1.9.0-stable (88d9e12ae1 2026-08-18), installed binary",
                vec![Found::new(Kind::Option, "max_width", true)],
            ),
        ])
    }

    #[test]
    fn test_color_is_only_decoration() {
        let colored = render(
            &sample_surface(),
            &sample_manifest(),
            &cases(),
            Style::Terminal { color: true },
        );
        let mut stripped = String::new();
        let mut rest = colored.as_str();
        while let Some(idx) = rest.find('\x1b') {
            stripped.push_str(&rest[..idx]);
            rest = &rest[idx..];
            rest = &rest[rest.find('m').unwrap() + 1..];
        }
        stripped.push_str(rest);
        assert_eq!(
            stripped,
            render(
                &sample_surface(),
                &sample_manifest(),
                &cases(),
                Style::Terminal { color: false }
            )
        );
    }

    #[test]
    fn test_markdown_report() {
        let report = render(
            &sample_surface(),
            &sample_manifest(),
            &Cases::Unavailable("not cached".to_string()),
            Style::Markdown,
        );
        assert!(report.starts_with("# mto against rustfmt 1.10.0 (latest stable release)\n"));
        assert!(report.contains("- **Stable surface:** 1 of 4 supported (25.0%), 1 partial\n"));
        assert!(report.contains("- **rustfmt's tests:** not cached\n"));
        assert!(
            report.contains("| options | stable | 1 | 1 | 0 | 0 | 2 |\n"),
            "{}",
            report
        );
        assert!(report.contains("## Still missing from stable rustfmt\n"));
    }

    #[test]
    fn test_name_lists_wrap_under_the_names() {
        let p = Painter {
            style: Style::Terminal { color: false },
        };
        let names: Vec<String> = (0..12).map(|i| format!("option_number_{}", i)).collect();
        let text = name_list(&p, "options (12)", &names);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.len() > 1);
        for line in lines.iter() {
            assert!(line.len() <= LINE_WIDTH, "{}", line);
        }
        assert!(lines[1].starts_with(&" ".repeat(18)));
        assert!(text.ends_with("option_number_11\n"));
    }

    #[test]
    fn test_terminal_report_reads_top_down() {
        let report = render(
            &sample_surface(),
            &sample_manifest(),
            &cases(),
            Style::Terminal { color: false },
        );
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(
            lines[0],
            "mto against rustfmt 1.10.0 (latest stable release)"
        );
        assert!(
            lines[2].starts_with("  Stable surface    ████▓▓▓▓░░░░░░░░  "),
            "{}",
            lines[2]
        );
        assert!(
            lines[2].ends_with("1 of 4 supported (25.0%), 1 partial"),
            "{}",
            lines[2]
        );
        assert!(
            lines[4].ends_with("191 of 462 pass (41.3%); 559 need options"),
            "{}",
            lines[4]
        );
        assert!(
            report.contains(
                "\n  options  stable               1        1        ·            ·      2\n"
            ),
            "{}",
            report
        );
        assert!(
            report.contains(
                "\n  mto's own features            1        ·        ·            ·      1\n"
            ),
            "{}",
            report
        );
        assert!(
            report.contains("\n  flags (1)       rustfmt:--check\n"),
            "{}",
            report
        );
        assert!(
            report.contains("\n  values (1)      newline_style=Unix\n"),
            "{}",
            report
        );
        assert!(
            report.contains("\n  new (1)         option brand_new\n"),
            "{}",
            report
        );
        assert!(
            report.contains("\n  stabilized (1)  option group_imports\n"),
            "{}",
            report
        );
        assert!(
            report.contains(
                "  rustfmt 1.10.0          latest stable release, 2026-10-01 (b940084d7eb6)\n"
            ),
            "{}",
            report
        );
        assert!(
            report.contains("  rustfmt 1.9.0           installed, 2026-08-18 (88d9e12ae1)\n"),
            "{}",
            report
        );
        for line in report.lines() {
            assert!(line.chars().count() <= LINE_WIDTH, "too wide: {}", line);
        }
    }
}
