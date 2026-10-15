//! `cargo xtask docs metrics`: the metrics papers under `docs/metrics/`, one per category
//! of measurement, generated from the data the suites record.
//!
//! A paper is Markdown with YAML front matter. The front matter (the machine data) and the
//! body's generated regions, each between `<!-- generated: NAME -->` and
//! `<!-- end generated -->`, are written here; the rest of the page is prose written by hand
//! and never touched. Every front-matter field must be defined in the data dictionary
//! ([`dictionary`], rendered as `docs/metrics/dictionary.md`); writing one it does not
//! define is an error.
//!
//! Nothing is measured here. Most regions are read from tracked records (compat/, the fuzz
//! targets and the kept regression inputs) and their git history: the `metrics-docs` gate
//! regenerates those and fails when a page differs. Local regions are read from the
//! machine-local records under `.dev/` (the last diff run's per-file scores, incidents,
//! metric series). They are rewritten when those records exist and kept as written when
//! not, and the gate never compares them, since another checkout has other records or
//! none. `generated` and `commit` say when and from which tree a page last changed: a
//! regeneration that changes nothing else keeps them, and the gate does not compare them.

mod build;
pub mod dictionary;
mod records;

use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::SystemTime;

use crate::common::log::{Logger, timestamp};
use crate::common::tree;

/// Where the papers live.
pub const DIR: &str = "docs/metrics";

/// The line that ends a generated region.
const END: &str = "<!-- end generated -->";

/// What a local region says until the records it reads have been found once.
const NO_LOCAL: &str = "No machine-local records were found when this page was generated.";

/// The command that regenerates the papers, for messages.
const REGENERATE: &str = "cargo xtask docs metrics";

/// The prose a new paper starts with in each hand-written section.
const UNWRITTEN: &str = "_Not written yet._";

/// A front-matter value.
enum Value {
  /// Written as is: a number, or text [`quote`] has quoted.
  Raw(String),
  List(Vec<String>),
  /// Keys and values written as is.
  Map(Vec<(String, String)>),
}

/// The front matter of a paper: the fields in the order written. Each field is checked
/// against the dictionary as it is added; the first that is not defined there is kept as
/// the error [`Front::done`] returns.
pub struct Front {
  fields: Vec<(String, Value)>,
  error: Option<String>,
}

impl Front {
  /// The fields every paper starts with.
  pub fn new(paper: &str, title: &str, status: &str) -> Front {
    Front {
      fields: Vec::new(),
      error: None,
    }
    .text("paper", paper)
    .text("title", title)
    .text("status", status)
  }

  /// A map of keys to values already written as YAML ([`quote`] for text). The keys of
  /// `metrics` and `limits` must be metrics of the dictionary, the others fields under
  /// `key`.
  pub fn map(mut self, key: &str, entries: Vec<(String, String)>) -> Front {
    for (name, _) in &entries {
      let defined = match key {
        "metrics" | "limits" => dictionary::is_metric(name),
        _ => dictionary::is_field(&format!("{}.{}", key, name)),
      };
      if !defined {
        self.refuse(&format!("{}.{}", key, name));
      }
    }
    self.add(key, Value::Map(entries))
  }

  /// A list of plain words (paths, names).
  pub fn list(self, key: &str, items: &[&str]) -> Front {
    let items = items.iter().map(|i| i.to_string()).collect();
    self.add(key, Value::List(items))
  }

  /// A text field, quoted.
  pub fn text(self, key: &str, value: &str) -> Front {
    self.add(key, Value::Raw(quote(value)))
  }

  /// The front matter, or the first field the dictionary does not define.
  pub fn done(self) -> Result<Front, String> {
    match self.error {
      Some(e) => Err(e),
      None => Ok(self),
    }
  }

  fn add(mut self, key: &str, value: Value) -> Front {
    if !dictionary::is_field(key) {
      self.refuse(key);
    }
    self.fields.push((key.to_string(), value));
    self
  }

  fn refuse(&mut self, name: &str) {
    if self.error.is_none() {
      self.error = Some(format!(
        "the front-matter field `{}` is not defined in the data dictionary; define it in \
         xtask/src/papers/dictionary.rs first",
        name
      ));
    }
  }

  /// The front matter between its `---` lines, `generated` and `commit` from `stamp`
  /// after the first three fields.
  fn render(&self, stamp: &Stamp) -> String {
    let mut out = String::from("---\n");
    for (at, (key, value)) in self.fields.iter().enumerate() {
      match value {
        Value::Raw(raw) => out.push_str(&format!("{}: {}\n", key, raw)),
        Value::List(items) if items.is_empty() => out.push_str(&format!("{}: []\n", key)),
        Value::List(items) => {
          out.push_str(&format!("{}:\n", key));
          for item in items {
            out.push_str(&format!("  - {}\n", item));
          }
        }
        Value::Map(entries) if entries.is_empty() => out.push_str(&format!("{}: {{}}\n", key)),
        Value::Map(entries) => {
          out.push_str(&format!("{}:\n", key));
          for (name, value) in entries {
            out.push_str(&format!("  {}: {}\n", name, value));
          }
        }
      }
      if at == 2 {
        out.push_str(&format!("generated: {}\n", stamp.generated));
        out.push_str(&format!("commit: {}\n", stamp.commit));
      }
    }
    out.push_str("---\n");
    out
  }
}

/// A page to generate: its front matter, if it has any, and its generated regions.
pub struct Page {
  /// Its path under the root.
  pub path: String,
  pub title: String,
  pub front: Option<Front>,
  pub regions: Vec<Region>,
}

/// One generated region of a page.
pub struct Region {
  pub name: &'static str,
  /// Its lines; `None` for a local region whose records are not here, kept as written.
  pub text: Option<String>,
  /// Read from machine-local records: never compared by the gate.
  pub local: bool,
}

impl Region {
  /// A region read from tracked records.
  pub fn tracked(name: &'static str, text: String) -> Region {
    Region {
      name,
      text: Some(text),
      local: false,
    }
  }

  /// A region read from machine-local records, `None` when they are not here.
  pub fn local(name: &'static str, text: Option<String>) -> Region {
    Region {
      name,
      text,
      local: true,
    }
  }
}

/// The two fields that say when a page last changed and from which tree, as written.
#[derive(Clone, Debug, PartialEq)]
struct Stamp {
  generated: String,
  commit: String,
}

impl Stamp {
  /// The stamp of a page not generated yet.
  fn blank() -> Stamp {
    Stamp {
      generated: quote(""),
      commit: quote(""),
    }
  }

  /// The stamp in a page's front matter, as written.
  fn read(page: &str) -> Option<Stamp> {
    let (front, _) = split_front(page);
    let front = front?;
    let field = |key: &str| {
      front
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{}: ", key)))
        .map(str::to_string)
    };
    Some(Stamp {
      generated: field("generated")?,
      commit: field("commit")?,
    })
  }
}

/// The `metrics-docs` gate: every page under docs/metrics/ matches what its tracked records
/// give, apart from its local regions and its stamp.
pub fn check(root: &Path) -> Vec<String> {
  let pages = match build::all(root, false) {
    Ok(pages) => pages,
    Err(e) => return vec![format!("metrics papers: {}", e)],
  };
  let mut problems = Vec::new();
  for page in &pages {
    let Ok(written) = fs::read_to_string(root.join(&page.path)) else {
      problems.push(format!(
        "{}: missing; run `{}` to write it",
        page.path, REGENERATE
      ));
      continue;
    };
    match splice(&written, page, None) {
      Ok(fresh) if fresh == written => {}
      Ok(fresh) => problems.push(format!(
        "{}: out of date with its recorded data ({}); run `{}`",
        page.path,
        differences(&written, &fresh, page).join(", "),
        REGENERATE
      )),
      Err(e) => problems.push(e),
    }
  }
  problems
}

/// Text quoted for YAML: double quotes, with backslashes and quotes escaped.
pub fn quote(text: &str) -> String {
  format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `cargo xtask docs metrics [--check]`.
pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let Some(("metrics", sub)) = m.subcommand() else {
    unreachable!("clap knows the subcommands")
  };
  if crate::cli::on(sub, "check") {
    let problems = check(root);
    for problem in &problems {
      println!("{}", problem);
    }
    if problems.is_empty() {
      println!("{}: every page matches its recorded data", DIR);
      return ExitCode::SUCCESS;
    }
    return ExitCode::from(1);
  }
  let log = Logger::new(root, "docs");
  log.info(&format!("cargo xtask {}", crate::cli::line()));
  let pages = match build::all(root, true) {
    Ok(pages) => pages,
    Err(e) => {
      log.error(&e);
      println!("{}", e);
      return ExitCode::from(1);
    }
  };
  let now = Stamp {
    generated: quote(&timestamp(SystemTime::now())),
    commit: quote(&tree::identity(root)),
  };
  let mut unchanged = 0;
  for page in &pages {
    let path = root.join(&page.path);
    let written = fs::read_to_string(&path).ok();
    let base = written.clone().unwrap_or_else(|| skeleton(page));
    let fresh = splice(&base, page, None).and_then(|kept| {
      if Some(&kept) == written.as_ref() {
        Ok(None)
      } else {
        splice(&base, page, Some(&now)).map(Some)
      }
    });
    match fresh {
      Ok(None) => unchanged += 1,
      Ok(Some(text)) => {
        let made = path
          .parent()
          .map_or(Ok(()), fs::create_dir_all)
          .and_then(|_| fs::write(&path, text));
        if let Err(e) = made {
          println!("cannot write {}: {}", page.path, e);
          return ExitCode::from(1);
        }
        log.info(&format!("wrote {}", page.path));
        println!("wrote {}", page.path);
      }
      Err(e) => {
        log.error(&e);
        println!("{}", e);
        return ExitCode::from(1);
      }
    }
  }
  println!("{} page(s) already up to date", unchanged);
  ExitCode::SUCCESS
}

/// `text` wrapped at `width` columns: the first line starts with `first`, the others with
/// `rest`. A word longer than a line gets a line of its own.
pub fn wrap(text: &str, width: usize, first: &str, rest: &str) -> String {
  let mut out = String::new();
  let mut line = first.to_string();
  let mut empty = true;
  for word in text.split_whitespace() {
    let fits = line.chars().count() + 1 + word.chars().count() <= width;
    if !empty && !fits {
      out.push_str(line.trim_end());
      out.push('\n');
      line = rest.to_string();
      empty = true;
    }
    if !empty {
      line.push(' ');
    }
    line.push_str(word);
    empty = false;
  }
  out.push_str(line.trim_end());
  out.push('\n');
  out
}

/// What differs between a page as written and as it should be: the front matter, or
/// which regions.
fn differences(written: &str, fresh: &str, page: &Page) -> Vec<String> {
  let mut found = Vec::new();
  if split_front(written).0 != split_front(fresh).0 {
    found.push("front matter".to_string());
  }
  for region in &page.regions {
    if region_text(written, region.name) != region_text(fresh, region.name) {
      found.push(format!("region `{}`", region.name));
    }
  }
  if found.is_empty() {
    found.push("layout of the generated parts".to_string());
  }
  found
}

/// The lines of region `name` in `page`, if it has one.
fn region_text<'a>(page: &'a str, name: &str) -> Option<&'a str> {
  let open = format!("<!-- generated: {} -->\n", name);
  let start = page.find(&open)? + open.len();
  let end = page[start..].find(END)? + start;
  Some(&page[start..end])
}

/// A page that does not exist yet: its headings, its regions, and a placeholder for each
/// section to write by hand.
fn skeleton(page: &Page) -> String {
  let block = |region: &Region| {
    let text = if region.local {
      format!("{}\n", NO_LOCAL)
    } else {
      String::new()
    };
    format!("<!-- generated: {} -->\n{}{}\n", region.name, text, END)
  };
  let mut out = String::new();
  if let Some(front) = &page.front {
    out.push_str(&front.render(&Stamp::blank()));
    out.push('\n');
  }
  out.push_str(&format!("# {}\n\n", page.title));
  if page.front.is_none() {
    for region in &page.regions {
      out.push_str(&block(region));
      out.push('\n');
    }
    return out;
  }
  let (abstract_, results): (Vec<&Region>, Vec<&Region>) = page
    .regions
    .iter()
    .partition(|region| region.name == "abstract");
  out.push_str("## Abstract\n\n");
  for region in abstract_ {
    out.push_str(&block(region));
  }
  for heading in ["1 Question", "2 Method"] {
    out.push_str(&format!("\n## {}\n\n{}\n", heading, UNWRITTEN));
  }
  out.push_str("\n## 3 Results\n\n");
  for region in results {
    out.push_str(&block(region));
    out.push('\n');
  }
  for heading in ["4 Threats to validity", "5 Reproduction", "6 Changelog"] {
    out.push_str(&format!("## {}\n\n{}\n\n", heading, UNWRITTEN));
  }
  out.trim_end().to_string() + "\n"
}

/// `page` with its front matter and generated regions replaced by `fresh`'s; the prose
/// between them stays as written. The stamp is `stamp`, or the page's own; a local region
/// `fresh` has no text for keeps its own.
fn splice(page: &str, fresh: &Page, stamp: Option<&Stamp>) -> Result<String, String> {
  let fail = |what: String| Err(format!("{}: {}", fresh.path, what));
  let (_, body) = split_front(page);
  let mut out = String::new();
  if let Some(front) = &fresh.front {
    let stamp = stamp
      .cloned()
      .or_else(|| Stamp::read(page))
      .unwrap_or_else(Stamp::blank);
    out.push_str(&front.render(&stamp));
  }
  let mut seen: Vec<&str> = Vec::new();
  let mut lines = body.split_inclusive('\n');
  while let Some(line) = lines.next() {
    out.push_str(line);
    let Some(name) = line
      .trim_end()
      .strip_prefix("<!-- generated: ")
      .and_then(|rest| rest.strip_suffix(" -->"))
    else {
      continue;
    };
    let Some(region) = fresh.regions.iter().find(|r| r.name == name) else {
      return fail(format!(
        "a region `{}` the generator does not write; remove its markers",
        name
      ));
    };
    if seen.contains(&region.name) {
      return fail(format!("the region `{}` appears twice", name));
    }
    seen.push(region.name);
    let mut old = String::new();
    loop {
      match lines.next() {
        Some(l) if l.trim_end() == END => break,
        Some(l) => old.push_str(l),
        None => return fail(format!("the region `{}` has no `{}`", name, END)),
      }
    }
    match &region.text {
      Some(text) => {
        out.push_str(text);
        if !text.is_empty() && !text.ends_with('\n') {
          out.push('\n');
        }
      }
      None => out.push_str(&old),
    }
    out.push_str(END);
    out.push('\n');
  }
  if let Some(missing) = fresh.regions.iter().find(|r| !seen.contains(&r.name)) {
    return fail(format!(
      "no region `{}`; add `<!-- generated: {} -->` and `{}` where it belongs",
      missing.name, missing.name, END
    ));
  }
  Ok(out)
}

/// A page's front matter (between its `---` lines, those included) and the rest.
fn split_front(page: &str) -> (Option<&str>, &str) {
  if let Some(rest) = page.strip_prefix("---\n")
    && let Some(end) = rest.find("\n---\n")
  {
    let split = 4 + end + 5;
    return (Some(&page[..split]), &page[split..]);
  }
  (None, page)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn page(regions: Vec<Region>) -> Page {
    Page {
      path: "docs/metrics/x.md".to_string(),
      title: "X".to_string(),
      front: Some(
        Front::new("x", "X", "measured")
          .map(
            "metrics",
            vec![("fuzz.targets".to_string(), "8".to_string())],
          )
          .done()
          .unwrap(),
      ),
      regions,
    }
  }

  #[test]
  fn test_a_field_the_dictionary_lacks_is_refused() {
    let refused = Front::new("x", "X", "measured")
      .text("rustc", "1.90")
      .done();
    assert!(refused.is_err_and(|e| e.contains("`rustc`")));
    let refused = Front::new("x", "X", "measured")
      .map(
        "metrics",
        vec![("compat.nope.files".to_string(), "1".to_string())],
      )
      .done();
    assert!(refused.is_err_and(|e| e.contains("metrics.compat.nope.files")));
    let refused = Front::new("x", "X", "measured")
      .map("toolchain", vec![("rustc".to_string(), quote("1.90"))])
      .done();
    assert!(refused.is_err_and(|e| e.contains("toolchain.rustc")));
    assert!(
      Front::new("x", "X", "measured")
        .map(
          "toolchain",
          vec![("reference_formatter".to_string(), quote("1.9"))]
        )
        .done()
        .is_ok()
    );
  }

  #[test]
  fn test_a_skeleton_holds_every_region_and_section() {
    let fresh = page(vec![
      Region::tracked("abstract", "Short.\n".to_string()),
      Region::tracked("results", "| a |\n".to_string()),
      Region::local("local", None),
    ]);
    let made = splice(&skeleton(&fresh), &fresh, None).unwrap();
    for heading in [
      "# X\n",
      "## Abstract\n\n<!-- generated: abstract -->\nShort.\n",
      "## 1 Question\n",
      "## 3 Results\n\n<!-- generated: results -->\n| a |\n",
      NO_LOCAL,
      "## 6 Changelog\n",
    ] {
      assert!(made.contains(heading), "{}\n{}", heading, made);
    }
  }

  #[test]
  fn test_regions_must_match_the_generator() {
    let fresh = page(vec![Region::tracked("results", String::new())]);
    let unknown = "<!-- generated: other -->\n<!-- end generated -->\n";
    assert!(splice(unknown, &fresh, None).is_err_and(|e| e.contains("`other`")));
    assert!(splice("no regions\n", &fresh, None).is_err_and(|e| e.contains("no region")));
    let open = "<!-- generated: results -->\nnever closed\n";
    assert!(splice(open, &fresh, None).is_err_and(|e| e.contains("has no")));
    let twice = "<!-- generated: results -->\n<!-- end generated -->\n".repeat(2);
    assert!(splice(&twice, &fresh, None).is_err_and(|e| e.contains("twice")));
  }

  #[test]
  fn test_splicing_rewrites_only_the_generated_parts() {
    let fresh = page(vec![
      Region::tracked("results", "| a |\n".to_string()),
      Region::local("local", None),
    ]);
    let written = "---\npaper: \"old\"\ngenerated: \"then\"\ncommit: \"abc\"\n---\n\n# X\n\n\
                   Prose stays.\n\n<!-- generated: results -->\nold\n<!-- end generated -->\n\
                   More prose.\n<!-- generated: local -->\nkept\n<!-- end generated -->\n";
    let spliced = splice(written, &fresh, None).unwrap();
    assert!(spliced.starts_with("---\npaper: \"x\"\ntitle: \"X\"\nstatus: \"measured\"\n"));
    assert!(
      spliced.contains("generated: \"then\"\ncommit: \"abc\"\nmetrics:\n  fuzz.targets: 8\n")
    );
    assert!(spliced.contains("Prose stays.\n\n<!-- generated: results -->\n| a |\n<!-- end"));
    assert!(spliced.contains("More prose.\n<!-- generated: local -->\nkept\n"));
    // Splicing again changes nothing; a new stamp changes only the stamp.
    assert_eq!(splice(&spliced, &fresh, None).unwrap(), spliced);
    let now = Stamp {
      generated: quote("now"),
      commit: quote("def"),
    };
    let stamped = splice(&spliced, &fresh, Some(&now)).unwrap();
    assert_eq!(
      stamped
        .replace("\"now\"", "\"then\"")
        .replace("\"def\"", "\"abc\""),
      spliced
    );
    assert_eq!(Stamp::read(&stamped), Some(now));
  }

  #[test]
  fn test_wrapping_keeps_to_the_width() {
    let text = "one two three four five six seven eight nine ten eleven twelve";
    let wrapped = wrap(text, 20, "- ", "  ");
    assert!(
      wrapped.lines().all(|l| l.chars().count() <= 20),
      "{}",
      wrapped
    );
    assert!(wrapped.starts_with("- one two three four\n  five"));
    assert_eq!(wrap("a-very-long-word", 5, "", ""), "a-very-long-word\n");
    assert_eq!(quote("say \"hi\" \\"), "\"say \\\"hi\\\" \\\\\"");
  }
}
