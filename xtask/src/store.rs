//! `cargo xtask store`: what `.dev/` holds, area by area, with what each keeps, and what is
//! there that no command writes any more.

use std::path::Path;
use std::process::ExitCode;

use crate::common::json::Json;
use crate::common::store;

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  if crate::cli::on(m, "json") {
    println!("{}", json(root).render());
    return ExitCode::SUCCESS;
  }
  print!("{}", text(root));
  ExitCode::SUCCESS
}

/// The store as `mtindo.xtask.store/1`: each area's size, files, what it holds and keeps,
/// and what is there in no area.
fn json(root: &Path) -> Json {
  let areas: Vec<Json> = store::AREAS
    .iter()
    .map(|area| {
      let (bytes, files) = store::size(&store::dir(root, area.name));
      Json::object(vec![
        ("name", area.name.into()),
        ("bytes", bytes.into()),
        ("files", files.into()),
        ("holds", area.holds.into()),
        ("kept", area.kept.into()),
      ])
    })
    .collect();
  let unknown: Vec<Json> = store::unknown(root)
    .iter()
    .map(|path| {
      Json::object(vec![
        ("path", path.display().to_string().into()),
        ("bytes", store::size(path).0.into()),
      ])
    })
    .collect();
  Json::document(
    "mtindo.xtask.store/1",
    vec![
      ("areas", Json::List(areas)),
      ("unknown", Json::List(unknown)),
    ],
  )
}

fn text(root: &Path) -> String {
  let mut out = String::new();
  let (mut bytes, mut files) = (0, 0);
  let mut rows = String::new();
  for area in store::AREAS {
    let (b, f) = store::size(&store::dir(root, area.name));
    bytes += b;
    files += f;
    rows.push_str(&format!(
      "\n{:<10} {:>9} {:>7} files\n  {}\n  kept: {}\n",
      area.name,
      store::shown_bytes(b),
      f,
      area.holds,
      area.kept
    ));
  }
  let unknown = store::unknown(root);
  for path in &unknown {
    let (b, f) = store::size(path);
    bytes += b;
    files += f;
  }
  out.push_str(&format!(
    ".dev/: {} in {} files\n",
    store::shown_bytes(bytes),
    files
  ));
  out.push_str(&rows);
  if !unknown.is_empty() {
    out.push_str("\nNot written by any command (left by one that is gone; safe to remove):\n");
    for path in &unknown {
      let name = path.file_name().unwrap_or_default().to_string_lossy();
      out.push_str(&format!(
        "  .dev/{:<5} {:>9}\n",
        name,
        store::shown_bytes(store::size(path).0)
      ));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_the_store_shows_each_area_and_what_is_left_over() {
    let root = std::env::temp_dir().join(format!("mtindo-store-text-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(store::dir(&root, store::RUNS)).unwrap();
    std::fs::write(store::dir(&root, store::RUNS).join("a"), vec![0; 2000]).unwrap();
    std::fs::create_dir_all(store::root(&root).join("hub")).unwrap();
    let shown = text(&root);
    assert!(shown.starts_with(".dev/: 2 KB in 1 files\n"), "{}", shown);
    assert!(shown.contains("\nruns            2 KB       1 files\n  a folder per command run"));
    // The folder no area names is listed.
    let left = shown.split("safe to remove):\n").nth(1).unwrap_or("");
    assert!(
      left.trim_start().starts_with(".dev/") && left.contains("hub"),
      "{}",
      shown
    );
    let _ = std::fs::remove_dir_all(&root);
  }
}
