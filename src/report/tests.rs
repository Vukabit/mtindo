use super::*;
use crate::verify::Rule;

fn bug(said: &str, minimal: &str, file: &str) -> Bug {
  Bug {
    failure: Failure::Panicked { said: said.into() },
    file: file.into(),
    path: Some(PathBuf::from("/work").join(file)),
    minimal: Some(minimal.into()),
    backtrace: Some("0: mtindo::shape".into()),
    source_hash: "ab".repeat(32),
    settings: "layout: on\n".into(),
    command: "mto .".into(),
  }
}

fn scratch(name: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!("mtindo-report-{}-{}", name, std::process::id()));
  let _ = fs::remove_dir_all(&dir);
  dir
}

/// The same bug seen again is counted, with each file it was seen in; another bug is
/// another report; a prefix of an id finds it.
#[test]
fn test_one_report_per_bug_counted() {
  let dir = scratch("count");
  let store = Store::new(dir.clone());
  let first = store
    .save(&bug("boom (at src/a.rs:1)", "fn a() {}", "./a.rs"))
    .unwrap();
  let again = store
    .save(&bug("boom (at src/a.rs:1)", "fn a() {}", "./b.rs"))
    .unwrap();
  let other = store
    .save(&bug("boom (at src/a.rs:1)", "fn b() {}", "./a.rs"))
    .unwrap();
  assert_eq!(first, again);
  assert_ne!(first, other);
  let entry = store.find(&first[..6]).unwrap();
  assert_eq!(entry.count, 2);
  assert_eq!(
    entry
      .files
      .iter()
      .map(|s| s.shown.as_str())
      .collect::<Vec<_>>(),
    ["./a.rs", "./b.rs"]
  );
  let text = entry.read().unwrap();
  assert!(text.contains("```rust\nfn a() {}\n```"), "{}", text);
  assert!(text.contains("<summary>Backtrace</summary>"), "{}", text);
  assert!(text.contains(crate::version::BUILD), "{}", text);
  assert_eq!(store.list().len(), 2);
  assert!(store.find("zz").is_err());
  assert_eq!(store.clear().unwrap(), 2);
  assert!(store.list().is_empty());
  let _ = fs::remove_dir_all(&dir);
}

/// A verifier stop is keyed by the rules broken, not their details; reports unseen for
/// [`KEEP_DAYS`] go when another is saved.
#[test]
fn test_verifier_reports_and_pruning() {
  let dir = scratch("prune");
  let store = Store::new(dir.clone());
  let stop = |detail: &str| Bug {
    failure: Failure::Verifier {
      violations: vec![Violation {
        rule: Rule::CommentLost,
        detail: detail.into(),
      }],
    },
    ..bug("", "fn a() {}", "./a.rs")
  };
  let id = store.save(&stop("lost `// x`")).unwrap();
  assert_eq!(store.save(&stop("lost `// y`")).unwrap(), id);
  assert!(
    store
      .find(&id)
      .unwrap()
      .title
      .starts_with("verifier: comment-lost")
  );
  // Last seen long ago: the next save prunes it.
  let entry = store.find(&id).unwrap();
  let old = Entry { last: 1, ..entry };
  fs::write(old.dir.join("meta"), meta_text(&old)).unwrap();
  let fresh = store.save(&bug("other", "", "./c.rs")).unwrap();
  assert_eq!(
    store
      .list()
      .iter()
      .map(|e| e.id.clone())
      .collect::<Vec<_>>(),
    [fresh]
  );
  let _ = fs::remove_dir_all(&dir);
}
