//! The hub against a small fixture project (tests/fixture): screens and pages drawn as
//! text match the expected drawings, and a broken config is refused with every problem.
//! `HUB_BLESS=1 cargo test -p hub` writes the drawings again.

use std::path::Path;
use std::process::Command;

fn fixture(name: &str) -> String {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixture")
    .join(name)
    .display()
    .to_string()
}

fn hub(args: &[&str]) -> (bool, String, String) {
  let out = Command::new(env!("CARGO_BIN_EXE_hub"))
    .args(args)
    .output()
    .expect("the hub runs");
  (
    out.status.success(),
    String::from_utf8_lossy(&out.stdout).into_owned(),
    String::from_utf8_lossy(&out.stderr).into_owned(),
  )
}

fn matches_drawing(screen: &str, size: &str) {
  matches_expected(screen, &["--render", screen, "--size", size]);
}

/// The hub's output for `args` on the fixture, against tests/expected/`name`.txt.
fn matches_expected(name: &str, args: &[&str]) {
  let config = fixture(".hub.toml");
  let mut all = vec!["--config", config.as_str()];
  all.extend_from_slice(args);
  let (ok, drawn, error) = hub(&all);
  assert!(ok, "{}", error);
  let screen = name;
  let expected = Path::new(env!("CARGO_MANIFEST_DIR"))
    .join("tests/expected")
    .join(format!("{}.txt", name));
  if std::env::var_os("HUB_BLESS").is_some() {
    std::fs::write(&expected, &drawn).unwrap();
  }
  let want = std::fs::read_to_string(&expected).unwrap_or_default();
  assert_eq!(drawn, want, "{} drew differently:\n{}", screen, drawn);
}

#[test]
fn test_a_broken_config_is_refused_with_every_problem() {
  let (ok, _, error) = hub(&["--config", &fixture("bad.hub.toml"), "--check"]);
  assert!(!ok);
  for problem in [
    "no color `nocolor`",
    "no kind `csv`",
    "widget w: no source `missing`",
    "1 sizes for 2 children",
    "no widget `ghost`",
    "[hub] start: no screen `nowhere`",
    "`hyper+x` is not a key",
    "no format `nope`",
    "`a ===`",
    "[bindings.default] `x` is bound to both up and down",
    "`{nothing}` names no action",
    "13 problem(s)",
  ] {
    assert!(
      error.contains(problem),
      "missing `{}` in:\n{}",
      problem,
      error
    );
  }
}

#[test]
fn test_a_page_draws_for_its_row() {
  matches_drawing("run", "60x14");
}

#[test]
fn test_a_screen_draws_tables_sparklines_and_gauges() {
  matches_drawing("main", "60x16");
}

/// The plain drawing, for screen readers: each pane's title and its rows as text, the
/// selected one marked, no drawing characters.
#[test]
fn test_a_screen_reads_plainly() {
  matches_expected(
    "main-plain",
    &["--render", "main", "--size", "60x16", "--plain"],
  );
  let text = std::fs::read_to_string(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/expected/main-plain.txt"
  ))
  .unwrap();
  assert!(text.contains("> failed | test"));
  assert!(
    !text.chars().any(|c| ('\u{2500}'..='\u{257f}').contains(&c)),
    "{}",
    text
  );
}

#[test]
fn test_an_unknown_key_is_refused_with_its_line() {
  let dir = std::env::temp_dir().join(format!("hub-unknown-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let config = dir.join(".hub.toml");
  std::fs::write(
    &config,
    "[hub]\nname = \"x\"\nstart = \"m\"\ncolour = \"red\"\n",
  )
  .unwrap();
  let (ok, _, error) = hub(&["--config", &config.display().to_string(), "--check"]);
  assert!(!ok);
  assert!(
    error.contains("colour") && error.contains("line 4"),
    "{}",
    error
  );
  let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_check_loads_every_source() {
  let (ok, out, error) = hub(&["--config", &fixture(".hub.toml"), "--check"]);
  assert!(ok, "{}", error);
  let has = |words: &[&str]| {
    out
      .lines()
      .any(|l| l.split_whitespace().collect::<Vec<_>>() == words)
  };
  assert!(has(&["source", "runs", "2", "row(s)"]), "{}", out);
  assert!(has(&["source", "series", "2", "row(s)"]), "{}", out);
}

/// An included file adds its screens, widgets and actions; its styles fill in what the
/// including file leaves out, never over it; a mistake in it is reported with its own
/// file and line.
#[test]
fn test_includes_add_and_fill_in() {
  let dir = std::env::temp_dir().join(format!("hub-include-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let main = dir.join(".hub.toml");
  std::fs::write(
    &main,
    "[hub]\nname = \"m\"\nstart = \"a\"\ninclude = [\"more.toml\"]\n\n[styles]\nok = { fg = \"blue\" }\n\n[[widget]]\nid = \"a\"\ntype = \"text\"\nlines = [\"main\"]\n\n[[screen]]\nid = \"a\"\ntitle = \"A\"\nkey = \"1\"\nlayout = \"a\"\n",
  )
  .unwrap();
  std::fs::write(
    dir.join("more.toml"),
    "[styles]\nok = { fg = \"green\" }\nnew = { fg = \"red\" }\n\n[[widget]]\nid = \"b\"\ntype = \"text\"\nlines = [\"included\"]\nstyle = \"new\"\n\n[[screen]]\nid = \"b\"\ntitle = \"B\"\nlayout = \"b\"\n",
  )
  .unwrap();
  let config = main.display().to_string();
  let (ok, out, error) = hub(&["--config", &config, "--check"]);
  assert!(ok, "{}", error);
  assert!(out.contains("2 screens, 2 widgets"), "{}", out);
  let (ok, drawn, error) = hub(&["--config", &config, "--render", "b", "--size", "40x10"]);
  assert!(ok && drawn.contains("included"), "{}{}", drawn, error);
  // A mistake in the included file names it and its line.
  std::fs::write(
    dir.join("more.toml"),
    "[[screen]]\nid = \"b\"\ncolour = 1\n",
  )
  .unwrap();
  let (ok, _, error) = hub(&["--config", &config, "--check"]);
  assert!(!ok);
  assert!(
    error.contains("more.toml") && error.contains("line 3"),
    "{}",
    error
  );
  let _ = std::fs::remove_dir_all(&dir);
}

/// Help's keys come from `[bindings]` and its defaults, one list with vim keys off and
/// another with them on; the footer's keys follow.
#[test]
fn test_keys_render_from_the_bindings() {
  matches_expected("keys", &["--keys"]);
  matches_expected("keys-vim", &["--keys", "--vim"]);
  matches_expected(
    "main-vim",
    &["--render", "main", "--size", "60x16", "--vim"],
  );
}
