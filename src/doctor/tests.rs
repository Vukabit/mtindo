use super::*;
use crate::options::format::FormatConfig;
use crate::options::layout::LayoutConfig;

/// A pretend build: it breaks while `broken` says so of any file, and counts its runs.
fn broke() -> Tried {
  Tried {
    broke: true,
    errors: vec!["error[E0433]: failed to resolve".into()],
  }
}

fn build<'a>(
  broken: &'a dyn Fn(&str) -> bool,
  runs: &'a mut usize,
) -> impl FnMut(&[String]) -> Option<Tried> + 'a {
  move |texts: &[String]| {
    *runs += 1;
    let broke = texts.iter().any(|t| broken(t));
    Some(Tried {
      broke,
      errors: if broke {
        vec!["error[E0433]: failed to resolve".into()]
      } else {
        Vec::new()
      },
    })
  }
}

fn file(name: &str, source: &str, settings: Arc<Settings>) -> File {
  File {
    path: PathBuf::from(name),
    source: source.to_string(),
    settings,
    held: Held::default(),
  }
}

fn format_only() -> Arc<Settings> {
  Arc::new(Settings {
    layout: None,
    format: Some(FormatConfig::default()),
  })
}

fn layout_only() -> Arc<Settings> {
  Arc::new(Settings {
    layout: Some(LayoutConfig::default()),
    format: None,
  })
}

/// A build that breaks whatever mto does was not broken by mto: nothing is left to hold,
/// and the narrowing says it did not pass.
#[test]
fn test_a_build_broken_without_mto_is_not_passed() {
  let mut files = vec![file("a.rs", "fn z() {}\nstruct Y;\n", layout_only())];
  let mut runs = 0;
  let always = |_: &str| true;
  let narrowed = narrow(&mut files, broke(), &mut build(&always, &mut runs));
  assert!(!narrowed.passed);
  assert_eq!(narrowed.texts[0], files[0].source);
}

/// Layout moves a module above the struct a pretend build needs it below; the narrowing
/// holds that move back alone, and the other file keeps its edits.
#[test]
fn test_a_move_that_breaks_the_build_is_held_back_alone() {
  let mut files = vec![
    file(
      "a.rs",
      "struct A;\nfn b() {}\nmod generated {}\n",
      layout_only(),
    ),
    file("b.rs", "fn z() {}\nstruct Y;\n", layout_only()),
  ];
  let laid_b = files[1].text(&Held::default());
  assert_ne!(laid_b, files[1].source);
  let broken = |t: &str| {
    matches!(
      (t.find("mod generated"), t.find("struct A")),
      (Some(m), Some(a)) if m < a
    )
  };
  let mut runs = 0;
  let narrowed = narrow(&mut files, broke(), &mut build(&broken, &mut runs));
  assert!(narrowed.passed);
  assert!(!narrowed.ran_out);
  assert_eq!(narrowed.found.len(), 1);
  let found = &narrowed.found[0];
  assert_eq!(found.file, 0);
  assert_eq!(
    found.what,
    "moved: mod generated (line 3, now before struct A)"
  );
  assert_eq!(found.errors, ["error[E0433]: failed to resolve"]);
  assert!(found.moved.is_some());
  assert_eq!(narrowed.texts[1], laid_b);
  assert!(!broken(&narrowed.texts[0]));
  assert!(runs < 12, "{} runs", runs);
}

/// Formatting changes a line a pretend build reads as written; the narrowing keeps those
/// lines as written and the rest formatted.
#[test]
fn test_changed_lines_that_break_the_build_are_held_back() {
  let source = "fn a() {\nlet x=1;\n}\nfn b() {\nlet y=2;\n}\n";
  let mut files = vec![file("a.rs", source, format_only())];
  let formatted = files[0].text(&Held::default());
  assert!(formatted.contains("    let y = 2;"), "{}", formatted);
  let broken = |t: &str| t.contains("let y = 2;");
  let mut runs = 0;
  let narrowed = narrow(&mut files, broke(), &mut build(&broken, &mut runs));
  assert!(narrowed.passed);
  assert_eq!(narrowed.found.len(), 1);
  assert!(
    narrowed.found[0].what.starts_with("formatted: line 5"),
    "{}",
    narrowed.found[0].what
  );
  assert!(narrowed.texts[0].contains("    let x = 1;"));
  assert!(narrowed.texts[0].contains("let y=2;"));
}

#[test]
fn test_ddmin_finds_the_fewest_that_fail_together() {
  let items: Vec<usize> = (0..16).collect();
  let mut runs = 0;
  let (found, tried) = ddmin(&items, Tried::default(), &mut |set: &[usize]| {
    runs += 1;
    let broke = set.contains(&3) && set.contains(&11);
    Some(Tried {
      broke,
      errors: vec![format!("{}", set.len())],
    })
  })
  .unwrap();
  assert_eq!(found, [3, 11]);
  assert!(tried.broke);
  assert!(runs < 40, "{} runs", runs);
  // Out of checks: no answer.
  assert!(ddmin(&items, Tried::default(), &mut |_: &[usize]| None).is_none());
}

#[test]
fn test_hunks_are_the_runs_of_changed_lines_and_apply_one_by_one() {
  let old = "a\nb\nc\nd\ne\n";
  let new = "a\nB\nc\nd\nE\nf\n";
  let found = hunks(old, new);
  assert_eq!(found.len(), 2);
  assert_eq!((found[0].old, found[0].new), ((1, 2), (1, 2)));
  assert_eq!((found[1].old, found[1].new), ((4, 5), (4, 6)));
  assert_eq!(apply(old, new, &found, |_| true), new);
  assert_eq!(apply(old, new, &found, |_| false), old);
  assert_eq!(
    apply(old, new, &found, |h| h.id == found[1].id),
    "a\nb\nc\nd\nE\nf\n"
  );
  assert!(hunks(old, old).is_empty());
}

/// Out of checks, every file still changed is held back whole.
#[test]
fn test_out_of_checks_files_are_held_back_whole() {
  let mut files = vec![file("a.rs", "fn z() {}\nstruct Y;\n", layout_only())];
  let mut left = 1;
  let narrowed = narrow(&mut files, broke(), &mut |_: &[String]| {
    if left == 0 {
      return None;
    }
    left -= 1;
    Some(Tried {
      broke: true,
      errors: Vec::new(),
    })
  });
  assert!(narrowed.ran_out);
  assert_eq!(narrowed.found[0].edit, Edit::All);
  assert_eq!(narrowed.texts[0], files[0].source);
}

#[test]
fn test_remembered_holds_are_found_by_content() {
  let source = "struct A;\nfn b() {}\nmod generated {}\n";
  let settings = layout_only();
  let free = shape(source, &settings, &Held::default(), None).unwrap();
  let moved = free
    .moves
    .iter()
    .find(|m| m.identifier == "generated")
    .unwrap();
  let held = Held {
    move_ids: vec![move_id(source, moved)],
    ..Held::default()
  };
  let kept = shape(source, &settings, &held, None).unwrap();
  assert_eq!(kept.found.len(), 1);
  assert!(kept.text.find("struct A") < kept.text.find("mod generated"));
  // Its text changed: the hold is not found, so it lapses.
  let changed = shape(&source.replace("{}", "{ }"), &settings, &held, None).unwrap();
  assert!(changed.found.is_empty());
}

#[test]
fn test_reports_go_with_their_runs() {
  use std::collections::HashSet;
  use std::fs;
  let dir = std::env::temp_dir().join(format!("mtindo-doctor-reports-{}", std::process::id()));
  let _ = fs::remove_dir_all(&dir);
  fs::create_dir_all(&dir).unwrap();
  let id = |n: u8| format!("0192f3a1-0000-7000-8000-0000000000{:02x}", n);
  let (kept, gone, held, young) = (id(1), id(2), id(3), id(4));
  for name in [&kept, &gone, &held, &young, &"latest".to_string()] {
    fs::write(dir.join(format!("{}.md", name)), "# report\n").unwrap();
  }
  fs::write(dir.join("notes.txt"), "mine").unwrap();
  // All but the young one were written before the grace period.
  let old = std::time::SystemTime::now() - crate::history::GRACE * 2;
  for entry in fs::read_dir(&dir).unwrap() {
    let path = entry.unwrap().path();
    if !path.ends_with(format!("{}.md", young)) {
      let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
      file.set_modified(old).unwrap();
    }
  }
  let runs: HashSet<String> = [kept.clone()].into();
  let holds: HashSet<String> = [held.clone()].into();
  let (count, bytes) = report::prune(&dir, &runs, &holds).unwrap();
  assert_eq!((count, bytes), (1, 9));
  let mut left: Vec<String> = fs::read_dir(&dir)
    .unwrap()
    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
    .collect();
  left.sort();
  let mut expected = vec![
    format!("{}.md", kept),
    format!("{}.md", held),
    format!("{}.md", young),
    "latest.md".to_string(),
    "notes.txt".to_string(),
  ];
  expected.sort();
  assert_eq!(left, expected);
  // No folder yet: nothing to do.
  assert_eq!(
    report::prune(&dir.join("none"), &runs, &holds).unwrap(),
    (0, 0)
  );
  let _ = fs::remove_dir_all(&dir);
}
