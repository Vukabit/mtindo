//! What a release tag's workflow (`.github/workflows/release.yml`) runs, so that its
//! steps are written once here: each target's archive and a try of it, the bill of
//! materials, and the checksums and notes the GitHub release takes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::checks::{self, Scope};
use super::config::Flow;
use super::git;
use crate::common::json::Json;
use crate::common::process;

/// What each archive holds beside the binaries.
const BESIDE: &[&str] = &[
  "README.md",
  "CREDITS.md",
  "LICENSE-MIT",
  "LICENSE-APACHE",
  "LICENSE-UNICODE",
];

/// The release's archives, checksums and notes, under `dist/`.
pub fn dist(root: &Path) -> PathBuf {
  root.join("dist")
}

/// `package TARGET`: `dist/mto-VERSION-TARGET.tar.gz` (`.zip` for Windows) with `mto`,
/// `cargo-mto` and the files [`BESIDE`], from `cargo build --release --target TARGET`.
pub fn package(root: &Path, flow: &Flow, target: &str) -> Result<(), String> {
  if !flow.release.targets.iter().any(|t| t == target) {
    return Err(format!(
      "{} is not a release target (flow.toml release.targets)",
      target
    ));
  }
  let version = version(root)?;
  let name = format!("mto-{}-{}", version, target);
  let windows = target.contains("windows");
  let built = root.join("target").join(target).join("release");
  let folder = dist(root).join(&name);
  let _ = fs::remove_dir_all(&folder);
  fs::create_dir_all(&folder).map_err(|e| format!("{}: {}", folder.display(), e))?;
  for bin in ["mto", "cargo-mto"] {
    let file = if windows {
      format!("{}.exe", bin)
    } else {
      bin.to_string()
    };
    fs::copy(built.join(&file), folder.join(&file)).map_err(|e| {
      format!(
        "{}: {} (build it: cargo build --release --locked --target {})",
        file, e, target
      )
    })?;
  }
  for file in BESIDE {
    fs::copy(root.join(file), folder.join(file)).map_err(|e| format!("{}: {}", file, e))?;
  }
  let archive = if windows {
    format!("{}.zip", name)
  } else {
    format!("{}.tar.gz", name)
  };
  // bsdtar writes a zip from the name (`-a`); every runner has a tar.
  let args: Vec<&str> = if windows {
    vec!["-a", "-cf", &archive, &name]
  } else {
    vec!["-czf", &archive, &name]
  };
  let done = process::output(
    Command::new("tar").args(&args).current_dir(dist(root)),
    process::QUICK,
  )
  .map_err(|e| format!("tar could not be run: {}", e))?;
  let _ = fs::remove_dir_all(&folder);
  if done.code != 0 {
    return Err(format!("tar failed for {}", archive));
  }
  println!("dist/{}", archive);
  Ok(())
}

/// `prepare TAG`: the publish stage's checks, then `dist/SHA256SUMS` over the archives
/// there, `dist/notes.md` from the changelog's section, and whether the release is left
/// a draft (`draft=true`), on stdout and in `$GITHUB_OUTPUT` when set. A dry run (no
/// tag: a `dry-run/` branch) checks only that the crate packages, for Cargo.toml's
/// version.
pub fn prepare(root: &Path, flow: &Flow, tag: &str, dry_run: bool) -> Result<(), String> {
  let tag = if dry_run {
    format!("v{}", version(root)?)
  } else {
    tag.to_string()
  };
  let tag = tag.as_str();
  let only_package = ["package".to_string()];
  let names = if dry_run {
    &only_package[..]
  } else {
    flow.stage("publish").unwrap_or_default()
  };
  let scope = Scope {
    stage: "publish".to_string(),
    files: None,
    staged: false,
    ranges: Vec::new(),
    pushed: Vec::new(),
  };
  checks::run_stage(root, flow, names, &scope)?;
  let version = tag.trim_start_matches('v');
  // The bill of materials, beside the archives and summed with them.
  let sbom = dist(root).join(format!("mto-{}.spdx.json", version));
  fs::write(&sbom, sbom_text(root, flow, version)?.render())
    .map_err(|e| format!("{}: {}", sbom.display(), e))?;
  let mut archives: Vec<PathBuf> = fs::read_dir(dist(root))
    .map_err(|e| format!("dist: {}", e))?
    .filter_map(|e| e.ok().map(|e| e.path()))
    .filter(|p| {
      p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("mto-") && (n.ends_with(".tar.gz") || n.ends_with(".zip")))
    })
    .collect();
  archives.sort();
  if archives.len() != flow.release.targets.len() {
    return Err(format!(
      "dist holds {} archive(s), flow.toml lists {} target(s)",
      archives.len(),
      flow.release.targets.len()
    ));
  }
  let mut sums = String::new();
  for path in archives.iter().chain([&sbom]) {
    let bytes = fs::read(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    sums.push_str(&format!(
      "{}  {}\n",
      crate::sha256::hex_digest(&bytes),
      name
    ));
  }
  fs::write(dist(root).join("SHA256SUMS"), &sums).map_err(|e| format!("SHA256SUMS: {}", e))?;
  let changelog = fs::read_to_string(root.join("CHANGELOG.md")).unwrap_or_default();
  let notes =
    section(&changelog, version).ok_or(format!("CHANGELOG.md has no ## [{}] section", version))?;
  fs::write(dist(root).join("notes.md"), notes).map_err(|e| format!("notes.md: {}", e))?;
  let earlier = git::git(root, &["tag", "--list", "v[0-9]*"])?
    .lines()
    .filter(|t| t.trim() != tag)
    .count();
  let draft = flow.release.first_as_draft && earlier == 0;
  let line = format!("draft={}", draft);
  println!("{}", line);
  if let Some(output) = std::env::var_os("GITHUB_OUTPUT") {
    let mut text = fs::read_to_string(&output).unwrap_or_default();
    text.push_str(&format!("{}\n", line));
    fs::write(&output, text).map_err(|e| format!("GITHUB_OUTPUT: {}", e))?;
  }
  Ok(())
}

/// `try-archive TARGET`: the target's archive unpacked, holding both binaries and the
/// files [`BESIDE`]; and where this machine runs the target, its mto saying the version,
/// and from a release tag (`GITHUB_REF_TYPE=tag`), that it is a release build (`-rel`).
pub fn try_archive(root: &Path, flow: &Flow, target: &str) -> Result<(), String> {
  if !flow.release.targets.iter().any(|t| t == target) {
    return Err(format!(
      "{} is not a release target (flow.toml release.targets)",
      target
    ));
  }
  let version = version(root)?;
  let name = format!("mto-{}-{}", version, target);
  let windows = target.contains("windows");
  let archive = dist(root).join(format!(
    "{}.{}",
    name,
    if windows { "zip" } else { "tar.gz" }
  ));
  if !archive.is_file() {
    return Err(format!(
      "no {}: run `cargo xtask flow package {}`",
      archive.display(),
      target
    ));
  }
  let scratch = std::env::temp_dir().join(format!("mtindo-try-{}", std::process::id()));
  let _ = fs::remove_dir_all(&scratch);
  fs::create_dir_all(&scratch).map_err(|e| format!("{}: {}", scratch.display(), e))?;
  // Every runner's tar reads a zip too (bsdtar on Windows and macOS).
  let unpacked = process::output(
    Command::new("tar")
      .arg("-xf")
      .arg(&archive)
      .current_dir(&scratch)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  )
  .map_err(|e| format!("cannot run tar: {}", e))?;
  if unpacked.code != 0 {
    return Err(format!(
      "cannot unpack {}: {}",
      archive.display(),
      String::from_utf8_lossy(&unpacked.stderr).trim()
    ));
  }
  let folder = scratch.join(&name);
  let exe = |bin: &str| {
    folder.join(if windows {
      format!("{}.exe", bin)
    } else {
      bin.to_string()
    })
  };
  let mut missing: Vec<String> = ["mto", "cargo-mto"]
    .iter()
    .map(|b| exe(b))
    .chain(BESIDE.iter().map(|f| folder.join(f)))
    .filter(|p| !p.is_file())
    .map(|p| p.display().to_string())
    .collect();
  missing.sort();
  if !missing.is_empty() {
    let _ = fs::remove_dir_all(&scratch);
    return Err(format!(
      "{} lacks: {}",
      archive.display(),
      missing.join(", ")
    ));
  }
  let host = host_triple(root);
  let said = if host.as_deref() == Some(target) {
    let ran = process::output(
      Command::new(exe("mto"))
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()),
      process::QUICK,
    )
    .map_err(|e| format!("cannot run the packed mto: {}", e))?;
    let line = String::from_utf8_lossy(&ran.stdout).trim().to_string();
    let tagged = std::env::var("GITHUB_REF_TYPE").is_ok_and(|t| t == "tag");
    let wanted = format!("mto {} (", version);
    if ran.code != 0 || !line.starts_with(&wanted) || (tagged && !line.ends_with("-rel)")) {
      let _ = fs::remove_dir_all(&scratch);
      return Err(format!(
        "the packed mto says `{}`; wanted `{}...{})`",
        line,
        wanted,
        if tagged { "-rel" } else { "" }
      ));
    }
    format!("runs: {}", line)
  } else {
    format!("not run here ({} on {})", target, host.unwrap_or_default())
  };
  let _ = fs::remove_dir_all(&scratch);
  println!(
    "{}: holds both binaries and {}; {}",
    archive.display(),
    BESIDE.join(", "),
    said
  );
  Ok(())
}

/// The target this machine builds for, as `rustc -vV` names it.
fn host_triple(root: &Path) -> Option<String> {
  let (code, said) = process::capture(root, "rustc", &["-vV".to_string()], &[], process::QUICK);
  (code == 0)
    .then(|| {
      said
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_string)
    })
    .flatten()
}

/// The release's SPDX 2.3 bill of materials: mto, its version, license and where it is
/// published, and nothing else, as it depends on no other crate at run time. Dated by the
/// commit released, so the same tag gives the same document; what Cargo.toml does not
/// say (the supplier) is left `NOASSERTION`.
fn sbom_text(root: &Path, flow: &Flow, version: &str) -> Result<Json, String> {
  let created = git::git(root, &["log", "-1", "--format=%cI", "HEAD"])?;
  let created = created.trim();
  // The license Cargo.toml declares, as crates.io shows it.
  let manifest = fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
  let license = manifest
    .lines()
    .find_map(|l| l.strip_prefix("license = "))
    .map(|l| l.trim().trim_matches('"').to_string())
    .ok_or("Cargo.toml declares no license")?;
  let license = license.as_str();
  let package = Json::object(vec![
    ("name", "mtindo".into()),
    ("SPDXID", "SPDXRef-Package-mtindo".into()),
    ("versionInfo", version.into()),
    (
      "downloadLocation",
      format!(
        "https://crates.io/api/v1/crates/mtindo/{}/download",
        version
      )
      .into(),
    ),
    ("supplier", "NOASSERTION".into()),
    ("licenseConcluded", license.into()),
    ("licenseDeclared", license.into()),
    ("copyrightText", "NOASSERTION".into()),
    ("filesAnalyzed", false.into()),
    (
      "externalRefs",
      Json::List(vec![Json::object(vec![
        ("referenceCategory", "PACKAGE-MANAGER".into()),
        ("referenceType", "purl".into()),
        (
          "referenceLocator",
          format!("pkg:cargo/mtindo@{}", version).into(),
        ),
      ])]),
    ),
  ]);
  Ok(Json::object(vec![
    ("spdxVersion", "SPDX-2.3".into()),
    ("dataLicense", "CC0-1.0".into()),
    ("SPDXID", "SPDXRef-DOCUMENT".into()),
    ("name", format!("mtindo-{}", version).into()),
    (
      "documentNamespace",
      format!(
        "https://github.com/{}/releases/tag/v{}#sbom",
        flow.release.repository, version
      )
      .into(),
    ),
    (
      "creationInfo",
      Json::object(vec![
        ("created", created.into()),
        ("creators", vec!["Tool: cargo-xtask-flow"].into()),
      ]),
    ),
    ("packages", Json::List(vec![package])),
    (
      "relationships",
      Json::List(vec![Json::object(vec![
        ("spdxElementId", "SPDXRef-DOCUMENT".into()),
        ("relationshipType", "DESCRIBES".into()),
        ("relatedSpdxElement", "SPDXRef-Package-mtindo".into()),
      ])]),
    ),
  ]))
}

/// The body of `changelog`'s `## [version]` section, up to the next release's heading.
fn section(changelog: &str, version: &str) -> Option<String> {
  let heading = format!("## [{}]", version);
  let start = changelog.find(&heading)?;
  let body = &changelog[start..];
  let body = body.split_once('\n').map_or("", |(_, rest)| rest);
  let end = body.find("\n## [").map_or(body.len(), |i| i + 1);
  Some(format!("{}\n", body[..end].trim()))
}

/// Cargo.toml's `[package]` version.
fn version(root: &Path) -> Result<String, String> {
  let text =
    fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {}", e))?;
  crate::toml::parse(&text)
    .ok()
    .and_then(|t| {
      t.get("package")?
        .as_table()?
        .get("version")?
        .as_str()
        .map(String::from)
    })
    .ok_or("Cargo.toml has no [package] version".to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_release_section_is_its_body_alone() {
    let changelog = "# Changelog\n\n## [0.2.0] - 2027-01-14\n\n### Fixed\n\n- A\n\n## [0.1.0] - 2026-11-01\n\nFirst.\n";
    assert_eq!(
      section(changelog, "0.2.0").as_deref(),
      Some("### Fixed\n\n- A\n")
    );
    assert_eq!(section(changelog, "0.1.0").as_deref(), Some("First.\n"));
    assert_eq!(section(changelog, "0.3.0"), None);
  }

  #[test]
  fn test_the_bill_of_materials_names_mtindo_alone() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let flow = super::super::config::load(root).unwrap();
    let text = sbom_text(root, &flow, "0.2.0").unwrap().render();
    let back = crate::common::json::parse(&text).unwrap();
    assert_eq!(
      back.get("spdxVersion").and_then(Json::as_str),
      Some("SPDX-2.3")
    );
    assert!(text.contains("\"pkg:cargo/mtindo@0.2.0\""), "{}", text);
    assert!(text.contains("releases/tag/v0.2.0#sbom"), "{}", text);
    let Some(Json::List(packages)) = back.get("packages") else {
      panic!("no packages: {}", text);
    };
    assert_eq!(packages.len(), 1);
  }
}
