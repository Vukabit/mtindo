//! Generates `src/text/unicode/ident_tables.rs` and `width_tables.rs` from the Unicode
//! Character Database. Downloads are cached in `compat/cache/unicode/<version>/`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::sha256;

const BASE_URL: &str = "https://www.unicode.org/Public";

const CODE_POINTS: usize = 0x11_0000;

const SOFT_HYPHEN: usize = 0xAD;

/// The data files used, as (file name, path under `ucd/`).
const SOURCES: &[(&str, &str)] = &[
    ("DerivedCoreProperties.txt", "DerivedCoreProperties.txt"),
    (
        "DerivedGeneralCategory.txt",
        "extracted/DerivedGeneralCategory.txt",
    ),
    ("EastAsianWidth.txt", "EastAsianWidth.txt"),
    ("HangulSyllableType.txt", "HangulSyllableType.txt"),
];

struct DataFile {
    name: &'static str,
    url: String,
    sha256: String,
    text: String,
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let toolchain = char::UNICODE_VERSION;
    let mut version = format!("{}.{}.{}", toolchain.0, toolchain.1, toolchain.2);
    let mut check = false;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--check" => check = true,
            "--version" if idx + 1 < args.len() => {
                version = args[idx + 1].clone();
                idx += 1;
            }
            other => {
                eprintln!("Unknown unicode argument `{}`.", other);
                return ExitCode::from(2);
            }
        }
        idx += 1;
    }
    let version_parts: Vec<u8> = version.split('.').filter_map(|p| p.parse().ok()).collect();
    if version_parts.len() != 3 {
        eprintln!("Unicode version must look like 17.0.0, got `{}`.", version);
        return ExitCode::from(2);
    }

    let mut files = Vec::new();
    for (name, path) in SOURCES {
        match fetch(root, &version, name, path) {
            Ok(file) => files.push(file),
            Err(e) => {
                eprintln!("{}", e);
                return ExitCode::from(1);
            }
        }
    }

    let outputs = match render_tables(&version, &version_parts, &files) {
        Ok(outputs) => outputs,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(1);
        }
    };

    let table_dir = root.join("src").join("text").join("unicode");
    let mut stale = Vec::new();
    for (file_name, content) in outputs.iter() {
        let path = table_dir.join(file_name);
        if check {
            let current = fs::read_to_string(&path).unwrap_or_default();
            if &current != content {
                stale.push(path.display().to_string());
            }
        } else {
            if let Err(e) = fs::create_dir_all(&table_dir).and_then(|_| fs::write(&path, content)) {
                eprintln!("cannot write {}: {}", path.display(), e);
                return ExitCode::from(1);
            }
            println!("wrote {}", path.display());
        }
    }
    if check {
        if stale.is_empty() {
            println!("unicode tables match Unicode {}", version);
            return ExitCode::SUCCESS;
        }
        for path in stale.iter() {
            println!(
                "{} is stale; run `cargo xtask unicode --version {}`",
                path, version
            );
        }
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn cache_dir(root: &Path, version: &str) -> PathBuf {
    root.join("compat")
        .join("cache")
        .join("unicode")
        .join(version)
}

/// Downloads one data file (or reuses the cached copy) and checks it is the requested version.
fn fetch(root: &Path, version: &str, name: &'static str, path: &str) -> Result<DataFile, String> {
    let url = format!("{}/{}/ucd/{}", BASE_URL, version, path);
    let dir = cache_dir(root, version);
    let cached = dir.join(name);
    if !cached.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
        let partial = dir.join(format!("{}.partial", name));
        let status = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--retry",
                "2",
                "--output",
            ])
            .arg(&partial)
            .arg(&url)
            .status()
            .map_err(|e| format!("cannot run curl: {}", e))?;
        if !status.success() {
            let _ = fs::remove_file(&partial);
            return Err(format!("download failed: {}", url));
        }
        fs::rename(&partial, &cached)
            .map_err(|e| format!("cannot store {}: {}", cached.display(), e))?;
    }
    let bytes =
        fs::read(&cached).map_err(|e| format!("cannot read {}: {}", cached.display(), e))?;
    let text = String::from_utf8(bytes.clone())
        .map_err(|_| format!("{} is not UTF-8", cached.display()))?;
    let expected_header = format!("# {}-{}.txt", name.trim_end_matches(".txt"), version);
    if !text
        .lines()
        .next()
        .is_some_and(|line| line.trim() == expected_header)
    {
        return Err(format!(
            "{} does not start with `{}`; delete it and retry",
            cached.display(),
            expected_header
        ));
    }
    Ok(DataFile {
        name,
        url,
        sha256: sha256::hex_digest(&bytes),
        text,
    })
}

fn file_text<'a>(files: &'a [DataFile], name: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.name == name)
        .map(|f| f.text.as_str())
        .unwrap_or("")
}

fn fill(set: &mut [bool], lo: usize, hi: usize, value: bool) {
    for slot in set.iter_mut().take(hi + 1).skip(lo) {
        *slot = value;
    }
}

fn header(version: &str, version_parts: &[u8], files: &[DataFile], used: &[&str]) -> String {
    let mut out = String::new();
    out.push_str("// Generated by `cargo xtask unicode`. Do not edit by hand.\n");
    out.push_str(&format!(
        "// Derived from Unicode {} data files; see LICENSE-UNICODE. Sources:\n",
        version
    ));
    for file in files.iter().filter(|f| used.contains(&f.name)) {
        out.push_str(&format!(
            "//   {}\n//     sha256 {}\n",
            file.url, file.sha256
        ));
    }
    out.push('\n');
    out.push_str("/// The Unicode version these tables were generated from.\n");
    out.push_str(&format!(
        "pub const UNICODE_VERSION: (u8, u8, u8) = ({}, {}, {});\n",
        version_parts[0], version_parts[1], version_parts[2]
    ));
    out
}

/// Marks every code point that has one of `values` in a UCD file.
///
/// Binary-property files (such as DerivedCoreProperties.txt) list many properties, so an entry
/// for another property never clears a mark. Enumerated files give each code point exactly one
/// value: their `@missing` defaults are applied first, then each entry sets its range either way.
fn mark(text: &str, values: &[&str], enumerated: bool) -> Vec<bool> {
    let mut set = vec![false; CODE_POINTS];
    if enumerated {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("# @missing:")
                && let Some((lo, hi, value)) = parse_entry(rest)
            {
                fill(&mut set, lo, hi, values.contains(&value.as_str()));
            }
        }
    }
    for line in text.lines() {
        let data = line.split('#').next().unwrap_or("");
        if let Some((lo, hi, value)) = parse_entry(data) {
            let listed = values.contains(&value.as_str());
            if listed || enumerated {
                fill(&mut set, lo, hi, listed);
            }
        }
    }
    set
}

/// Parses `0041..005A ; Value` or `00AD ; Value` into (start, end, value).
fn parse_entry(data: &str) -> Option<(usize, usize, String)> {
    let mut fields = data.split(';');
    let range = fields.next()?.trim();
    let value = fields.next()?.trim().to_string();
    if range.is_empty() || value.is_empty() {
        return None;
    }
    let (lo, hi) = match range.split_once("..") {
        Some((lo, hi)) => (lo, hi),
        None => (range, range),
    };
    let lo = usize::from_str_radix(lo.trim(), 16).ok()?;
    let hi = usize::from_str_radix(hi.trim(), 16).ok()?;
    if lo > hi || hi >= CODE_POINTS {
        return None;
    }
    Some((lo, hi, value))
}

/// Contiguous runs of `true` as inclusive ranges.
fn ranges(set: &[bool]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (cp, &inside) in set.iter().enumerate() {
        match (inside, start) {
            (true, None) => start = Some(cp),
            (false, Some(lo)) => {
                out.push((lo, cp - 1));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(lo) = start {
        out.push((lo, set.len() - 1));
    }
    out
}

fn render_tables(
    version: &str,
    version_parts: &[u8],
    files: &[DataFile],
) -> Result<Vec<(&'static str, String)>, String> {
    let core = file_text(files, "DerivedCoreProperties.txt");
    let categories = file_text(files, "DerivedGeneralCategory.txt");
    let widths = file_text(files, "EastAsianWidth.txt");
    let hangul = file_text(files, "HangulSyllableType.txt");

    let xid_start = mark(core, &["XID_Start"], false);
    let xid_continue = mark(core, &["XID_Continue"], false);
    let ignorable = mark(core, &["Default_Ignorable_Code_Point"], false);
    let marks_and_format = mark(categories, &["Mn", "Me", "Cf"], true);
    let jamo = mark(hangul, &["V", "T"], true);
    let wide = mark(widths, &["W", "F"], true);

    let mut zero = vec![false; CODE_POINTS];
    let mut double = vec![false; CODE_POINTS];
    for cp in 0..CODE_POINTS {
        zero[cp] = (marks_and_format[cp] || ignorable[cp] || jamo[cp]) && cp != SOFT_HYPHEN;
        double[cp] = wide[cp] && !zero[cp];
    }

    let mut ident = header(
        version,
        version_parts,
        files,
        &["DerivedCoreProperties.txt"],
    );
    ident.push_str(&table(
        "Characters that may continue an identifier (XID_Continue), as inclusive ranges.",
        "XID_CONTINUE",
        &xid_continue,
    ));
    ident.push_str(&table(
        "Characters that may start an identifier (XID_Start), as inclusive ranges.",
        "XID_START",
        &xid_start,
    ));

    let mut width = header(
        version,
        version_parts,
        files,
        &[
            "DerivedCoreProperties.txt",
            "DerivedGeneralCategory.txt",
            "EastAsianWidth.txt",
            "HangulSyllableType.txt",
        ],
    );
    width.push_str(&table(
        "Characters two columns wide: East_Asian_Width W or F, minus the zero-width set.",
        "DOUBLE_WIDTH",
        &double,
    ));
    width.push_str(&table(
        "Characters zero columns wide: general categories Mn, Me and Cf, Default_Ignorable_Code_Point,\n\
         and Hangul medial vowels and final consonants (Hangul_Syllable_Type V and T),\n\
         except U+00AD SOFT HYPHEN, which stays one column wide.",
        "ZERO_WIDTH",
        &zero,
    ));

    Ok(vec![
        ("ident_tables.rs", rustfmt(&ident)?),
        ("width_tables.rs", rustfmt(&width)?),
    ])
}

fn rustfmt(source: &str) -> Result<String, String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run rustfmt: {}", e))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(source.as_bytes())
            .map_err(|e| format!("cannot write to rustfmt: {}", e))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("rustfmt failed: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "rustfmt failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| "rustfmt produced invalid UTF-8".to_string())
}

fn table(doc: &str, name: &str, set: &[bool]) -> String {
    let mut out = String::new();
    out.push('\n');
    for line in doc.lines() {
        out.push_str(&format!("/// {}\n", line));
    }
    out.push_str(&format!("pub static {}: &[(u32, u32)] = &[\n", name));
    for (lo, hi) in ranges(set) {
        out.push_str(&format!("    (0x{:X}, 0x{:X}),\n", lo, hi));
    }
    out.push_str("];\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_entry_forms() {
        assert_eq!(
            parse_entry("0041..005A    ; XID_Start "),
            Some((0x41, 0x5A, "XID_Start".to_string()))
        );
        assert_eq!(
            parse_entry("00AD          ; Cf "),
            Some((0xAD, 0xAD, "Cf".to_string()))
        );
        assert_eq!(parse_entry(""), None);
        assert_eq!(parse_entry("not a range ; X"), None);
    }

    #[test]
    fn test_mark_applies_missing_defaults_then_entries() {
        let text = "# @missing: 0000..10FFFF; N\n# @missing: 3400..4DBF; W\n4E00..4E01 ; W # CJK\n3400 ; N\n";
        let set = mark(text, &["W"], true);
        assert!(!set[0x41]);
        assert!(!set[0x3400]);
        assert!(set[0x3401]);
        assert!(set[0x4E01]);
        assert!(!set[0x4E02]);
    }

    #[test]
    fn test_mark_binary_properties_never_clear() {
        let text =
            "0041..005A ; XID_Start # Lu\n0041..005A ; Alphabetic # Lu\n0030 ; XID_Continue # Nd\n";
        let set = mark(text, &["XID_Start"], false);
        assert!(set[0x41]);
        assert!(set[0x5A]);
        assert!(!set[0x30]);
    }

    #[test]
    fn test_ranges_merge_runs() {
        let mut set = vec![false; 10];
        set[2] = true;
        set[3] = true;
        set[7] = true;
        set[9] = true;
        assert_eq!(ranges(&set), vec![(2, 3), (7, 7), (9, 9)]);
    }
}
