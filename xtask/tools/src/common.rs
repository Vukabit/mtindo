//! Helpers shared by the tools.

use std::fs;
use std::path::Path;

use mtindo::syntax::Edition;

/// The edition to parse `file` with.
pub fn edition_for(file: &Path, text: &str) -> Edition {
    for line in text.lines().take(20) {
        if let Some(year) = line.trim().strip_prefix("// rustfmt-edition:")
            && let Some(edition) = Edition::from_year(year)
        {
            return edition;
        }
    }
    let mut dir = file.parent();
    while let Some(current) = dir {
        if let Ok(manifest) = fs::read_to_string(current.join("Cargo.toml")) {
            for line in manifest.lines() {
                let line = line.trim();
                if let Some(value) = line.strip_prefix("edition") {
                    let year = value
                        .trim()
                        .trim_start_matches('=')
                        .trim()
                        .trim_matches('"');
                    if let Some(edition) = Edition::from_year(year) {
                        return edition;
                    }
                }
            }
            if manifest.contains("[package]") && !manifest.contains("edition.workspace") {
                return Edition::E2015;
            }
        }
        dir = current.parent();
    }
    Edition::E2015
}
