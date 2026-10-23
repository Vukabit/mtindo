//! Each page under docs/generated/metrics/: its front matter and generated regions, from the records.
//! A paper's prose sections are written by hand in its file; only what is built here is
//! generated.

use std::fs;
use std::path::Path;

use super::dictionary::{self, Entry, SETS};
use super::records::{self, Compat};
use super::{DIR, Front, Page, Region, quote, wrap};
use crate::metrics;

const COMPATIBILITY: &str = "Compatibility with the standard formatter";

const FUZZING: &str = "Fuzzing";

const MEANING: &str = "Meaning and stability";

const PERFORMANCE: &str = "Performance";

/// The papers whose data the repository does not record yet: id and title.
const PLANNED: &[(&str, &str)] = &[
  ("scrambled-pairs", "Scrambled pairs"),
  ("compile-safety", "Compile safety"),
  ("layout", "Layout"),
];

/// The widest a generated line may be: public Markdown's limit.
const WIDTH: usize = 80;

/// A built paper and its headline number for the index.
struct Paper {
  page: Page,
  headline: String,
}

/// Every page, papers first, then the index and the dictionary. `local` reads the
/// machine-local records too; without it, every local region is left as written.
pub fn all(root: &Path, local: bool) -> Result<Vec<Page>, String> {
  let mut papers = vec![
    compatibility(root, local)?,
    meaning(root, local)?,
    fuzzing(root, local)?,
    performance(root, local)?,
  ];
  for (id, title) in PLANNED {
    papers.push(planned(id, title)?);
  }
  let index = index(&papers);
  let mut pages: Vec<Page> = papers.into_iter().map(|p| p.page).collect();
  pages.push(index);
  pages.push(dictionary_page());
  Ok(pages)
}

/// One row of the compatibility history: lines matching per set, and files matching on
/// the scrambled set.
fn compat_row(date: &str, commit: &str, compat: &Compat) -> Vec<String> {
  let mut row = vec![date.to_string(), commit.to_string()];
  for set in SETS {
    row.push(
      compat
        .sets
        .get(*set)
        .map_or("-".to_string(), |t| fixed(t.lines_pct(), 2)),
    );
  }
  row.push(
    compat
      .sets
      .get("corpus-unformatted")
      .map_or("-".to_string(), |t| fixed(t.files_pct(), 2)),
  );
  row
}

fn compatibility(root: &Path, local: bool) -> Result<Paper, String> {
  let text = read(root, "compat/compatibility.txt")?;
  let compat = records::parse_compat(&text);
  let (fingerprint, repositories) = records::corpus_fingerprint(root)?;
  let mut figures = Vec::new();
  for set in SETS {
    let Some(t) = compat.sets.get(*set) else {
      continue;
    };
    let name = |figure: &str| format!("compat.{}.{}", set, figure);
    figures.push((name("files"), fixed(t.files_pct(), 2)));
    figures.push((name("files_matched"), t.matched_files.to_string()));
    figures.push((name("files_scored"), t.files.to_string()));
    figures.push((name("lines"), fixed(t.lines_pct(), 2)));
    figures.push((name("lines_matched"), t.matched_lines.to_string()));
    figures.push((name("lines_total"), t.lines.to_string()));
    if let Some(skipped) = t.skipped {
      figures.push((name("skipped"), skipped.to_string()));
    }
  }
  figures.push((
    "compat.corpus.repositories".to_string(),
    repositories.to_string(),
  ));
  let front = Front::new("compatibility", COMPATIBILITY, "measured")
    .text("command", "cargo xtask diff --update")
    .list(
      "sources",
      &["compat/compatibility.txt", "compat/corpus.txt"],
    )
    .map("toolchain", reference(&compat))
    .text("corpus_fingerprint", &fingerprint)
    .text("seed", "FNV-1a of each file's text (scrambled set only)")
    .map("metrics", figures)
    .done()?;
  let get = |set: &str| compat.sets.get(set).copied().unwrap_or_default();
  let (tests, corpus, scrambled) = (get("tests"), get("corpus"), get("corpus-unformatted"));
  let summary = format!(
    "Against the standard formatter {}, mto's output equals the expected output for {} \
     of {} test files ({}%) and for {} of {} corpus files ({}%). On whitespace-scrambled \
     corpus files it matches {} of {} files ({}%) and keeps {}% of the expected lines.",
    records::reference_version(&compat.reference),
    tests.matched_files,
    tests.files,
    fixed(tests.files_pct(), 2),
    corpus.matched_files,
    corpus.files,
    fixed(corpus.files_pct(), 2),
    scrambled.matched_files,
    scrambled.files,
    fixed(scrambled.files_pct(), 2),
    fixed(scrambled.lines_pct(), 2)
  );
  let mut rows = Vec::new();
  for set in SETS {
    if let Some(t) = compat.sets.get(*set) {
      rows.push(vec![
        shown(set).to_string(),
        format!("{} of {}", t.matched_files, t.files),
        fixed(t.files_pct(), 2),
        format!("{} of {}", t.matched_lines, t.lines),
        fixed(t.lines_pct(), 2),
        t.skipped.map_or("-".to_string(), |k| k.to_string()),
      ]);
    }
  }
  let results = table(
    &[
      "set",
      "files matching",
      "%",
      "lines matching",
      "%",
      "skipped",
    ],
    "lrrrrr",
    &rows,
  );
  let versions = recent(
    records::file_history(root, "compat/compatibility.txt")?,
    |v| records::parse_compat(&v.text) == compat,
  );
  let mut rows = vec![compat_row("current", "", &compat)];
  for v in &versions {
    rows.push(compat_row(
      &v.date,
      &v.commit,
      &records::parse_compat(&v.text),
    ));
  }
  let history = table(
    &[
      "date",
      "commit",
      "tests",
      "with settings",
      "corpus",
      "scrambled",
      "scrambled files",
    ],
    "llrrrrr",
    &rows,
  );
  let headline = format!(
    "{}% of scrambled-corpus lines match",
    fixed(scrambled.lines_pct(), 2)
  );
  Ok(Paper {
    page: Page {
      path: format!("{}/compatibility.md", DIR),
      title: COMPATIBILITY.to_string(),
      front: Some(front),
      regions: vec![
        Region::tracked("abstract", wrap(&summary, WIDTH, "", "")),
        Region::tracked("results", results),
        Region::tracked("history", history),
        Region::local("last-run", if local { last_diff(root) } else { None }),
      ],
    },
    headline,
  })
}

/// Every file under `dir`, at any depth.
fn count_files(dir: &Path) -> usize {
  fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .map(|entry| {
      let path = entry.path();
      if path.is_dir() { count_files(&path) } else { 1 }
    })
    .sum()
}

/// The data dictionary, rendered from [`dictionary`].
fn dictionary_page() -> Page {
  let render = |entries: &[Entry]| {
    let mut out = String::new();
    for entry in entries {
      out.push_str(&format!("### `{}`\n\n", entry.name));
      out.push_str(&wrap(&format!("Unit: {}.", entry.unit), WIDTH, "- ", "  "));
      out.push_str(&wrap(
        &format!("Definition: {}", entry.definition),
        WIDTH,
        "- ",
        "  ",
      ));
      out.push_str(&wrap(
        &format!("Measured: {}", entry.measured),
        WIDTH,
        "- ",
        "  ",
      ));
      if let Some(hole) = dictionary::placeholder(entry.name) {
        let values: Vec<String> = entry.values.iter().map(|v| format!("`{}`", v)).collect();
        out.push_str(&wrap(
          &format!("`{}` is one of {}.", hole, values.join(", ")),
          WIDTH,
          "- ",
          "  ",
        ));
      }
      out.push('\n');
    }
    out.trim_end().to_string() + "\n"
  };
  Page {
    path: format!("{}/dictionary.md", DIR),
    title: "Data dictionary".to_string(),
    front: None,
    regions: vec![
      Region::tracked("fields", render(dictionary::FIELDS)),
      Region::tracked("metrics", render(dictionary::METRICS)),
    ],
  }
}

fn exception_row(date: &str, commit: &str, list: &[records::Exception]) -> Vec<String> {
  let unmodelled = list.iter().filter(|e| unmodelled(&e.reason)).count();
  vec![
    date.to_string(),
    commit.to_string(),
    list.len().to_string(),
    unmodelled.to_string(),
    (list.len() - unmodelled).to_string(),
  ]
}

/// `value` with `places` decimals.
fn fixed(value: f64, places: usize) -> String {
  format!("{:.*}", places, value)
}

/// The latest figures of each target's fuzz runs on this machine, and of differential
/// fuzzing and coverage.
fn fuzz_runs(root: &Path, targets: &[String]) -> Option<String> {
  let latest = |name: &str| metrics::series(root, name).pop();
  let cell =
    |name: &str, places: usize| latest(name).map_or("-".to_string(), |p| fixed(p.value, places));
  let rows: Vec<Vec<String>> = targets
    .iter()
    .map(|t| {
      vec![
        format!("`{}`", t),
        cell(&format!("fuzz.{}.exec_per_s", t), 0),
        cell(&format!("fuzz.{}.address.peak_rss", t), 0),
        cell(&format!("coverage.fuzz.target.{}.lines", t), 2),
      ]
    })
    .collect();
  if rows.iter().all(|r| r[1..].iter().all(|c| c == "-")) {
    return None;
  }
  let mut out = wrap(
    "The latest figures kept on the machine that generated this page: inputs per \
     second and peak memory in MB of the last run of each target (with the address \
     sanitizer), and the share of src/ lines its corpus reaches.",
    WIDTH,
    "",
    "",
  );
  out.push('\n');
  out.push_str(&table(
    &["target", "inputs/s", "peak MB", "lines reached %"],
    "lrrr",
    &rows,
  ));
  let mut notes = Vec::new();
  if let Some(p) = latest("coverage.fuzz.lines") {
    notes.push(format!(
      "every corpus together reaches {}% of the lines of src/ ({})",
      fixed(p.value, 2),
      &p.when[..10.min(p.when.len())]
    ));
  }
  if let Some(p) = latest("fuzz.diff.agreed.pct") {
    notes.push(format!(
      "in the last differential run, mto and the standard formatter agreed on {}% of \
       inputs ({})",
      fixed(p.value, 2),
      &p.when[..10.min(p.when.len())]
    ));
  }
  if !notes.is_empty() {
    out.push('\n');
    out.push_str(&wrap(
      &format!("Also: {}.", notes.join("; ")),
      WIDTH,
      "",
      "",
    ));
  }
  Some(out)
}

fn fuzzing(root: &Path, local: bool) -> Result<Paper, String> {
  let targets = stems(&root.join("fuzz/fuzz_targets"));
  if targets.is_empty() {
    return Err("no fuzz targets in fuzz/fuzz_targets/".to_string());
  }
  let kept_dir = root.join("tests/fuzz_regressions");
  let mut kept = Vec::new();
  for target in &targets {
    kept.push((target.clone(), count_files(&kept_dir.join(target))));
  }
  let total: usize = count_files(&kept_dir);
  let front = Front::new("fuzzing", FUZZING, "measured")
    .text("command", "cargo xtask fuzz run TARGET")
    .list(
      "sources",
      &["fuzz/fuzz_targets/", "tests/fuzz_regressions/"],
    )
    .text("seed", "chosen by libFuzzer for each run; not recorded")
    .map(
      "metrics",
      vec![
        ("fuzz.targets".to_string(), targets.len().to_string()),
        ("fuzz.regressions".to_string(), total.to_string()),
      ],
    )
    .done()?;
  let with_inputs = kept.iter().filter(|(_, n)| *n > 0).count();
  let summary = format!(
    "{} fuzz targets drive mto's lexer, parser, layout, formatting, meaning check and \
     TOML reader with generated inputs. {} inputs that once failed are kept as regression \
     tests, from {} of the targets; each still passes its target's checks in every test \
     run.",
    targets.len(),
    total,
    with_inputs
  );
  let rows: Vec<Vec<String>> = kept
    .iter()
    .map(|(target, n)| vec![format!("`{}`", target), n.to_string()])
    .collect();
  let results = table(&["target", "kept inputs"], "lr", &rows);
  let path = "tests/fuzz_regressions";
  let versions = recent(
    records::history(root, path, |hash| records::files_at(root, hash, path))?,
    |(_, _, n)| *n == total,
  );
  let mut rows = vec![vec![
    "current".to_string(),
    String::new(),
    total.to_string(),
  ]];
  for (commit, date, n) in versions {
    rows.push(vec![date, commit, n.to_string()]);
  }
  let history = table(&["date", "commit", "kept inputs"], "llr", &rows);
  Ok(Paper {
    page: Page {
      path: format!("{}/fuzzing.md", DIR),
      title: FUZZING.to_string(),
      front: Some(front),
      regions: vec![
        Region::tracked("abstract", wrap(&summary, WIDTH, "", "")),
        Region::tracked("results", results),
        Region::tracked("history", history),
        Region::local(
          "incidents",
          if local {
            incidents(root, |kind| kind.starts_with("fuzz-") || kind == "mismatch")
          } else {
            None
          },
        ),
        Region::local(
          "runs",
          if local {
            fuzz_runs(root, &targets)
          } else {
            None
          },
        ),
      ],
    },
    headline: format!(
      "{} targets, {} kept regression inputs",
      targets.len(),
      total
    ),
  })
}

/// Incidents of the kinds `wanted` picks, by kind and status, from `.dev/incidents/`;
/// `None` when there is no such folder.
fn incidents(root: &Path, wanted: impl Fn(&str) -> bool) -> Option<String> {
  if !crate::incidents::root_dir(root).is_dir() {
    return None;
  }
  let all = crate::incidents::all(root);
  let statuses = crate::incidents::STATUSES;
  let rows: Vec<Vec<String>> = crate::incidents::KINDS
    .iter()
    .filter(|(kind, _, _)| wanted(kind))
    .map(|(kind, _, _)| {
      let mut row = vec![format!("`{}`", kind)];
      for status in statuses {
        let n = all
          .iter()
          .filter(|(_, i)| i.kind == *kind && i.status == *status)
          .count();
        row.push(n.to_string());
      }
      row
    })
    .collect();
  let mut out = wrap(
    "Incidents filed on the machine that generated this page (`cargo xtask incidents`), \
     each failure counted once however often it was seen, by kind and status:",
    WIDTH,
    "",
    "",
  );
  out.push('\n');
  let mut headers = vec!["kind"];
  headers.extend(statuses);
  out.push_str(&table(&headers, "lrrrr", &rows));
  Some(out)
}

/// The index: each paper with its status and headline number.
fn index(papers: &[Paper]) -> Page {
  let rows: Vec<Vec<String>> = papers
    .iter()
    .map(|p| {
      let id = p.page.path.rsplit('/').next().unwrap_or_default();
      vec![
        format!("[{}]({})", id.trim_end_matches(".md"), id),
        p.headline.clone(),
      ]
    })
    .collect();
  Page {
    path: format!("{}/README.md", DIR),
    title: "Metrics".to_string(),
    front: None,
    regions: vec![Region::tracked(
      "index",
      table(&["paper", "headline"], "ll", &rows),
    )],
  }
}

/// The last diff run on this machine: per-file scores by band, and the syntax where
/// mismatching files first differ.
fn last_diff(root: &Path) -> Option<String> {
  let (day, bands) = records::score_bands(root)?;
  let mut out = wrap(
    &format!(
      "From the last `cargo xtask diff` run recorded on the machine that generated this \
       page ({}), which need not be the run the record above was made from. Scored files \
       by the share of their expected lines mto's output keeps:",
      day
    ),
    WIDTH,
    "",
    "",
  );
  out.push('\n');
  let mut rows = Vec::new();
  for set in SETS {
    if let Some(counts) = bands.get(*set) {
      let mut row = vec![shown(set).to_string()];
      row.extend(counts.iter().map(|c| c.to_string()));
      rows.push(row);
    }
  }
  let mut headers = vec!["set"];
  headers.extend(records::SCORE_BANDS);
  out.push_str(&table(&headers, "lrrrr", &rows));
  if let Some(kinds) = records::first_kinds(root) {
    out.push('\n');
    out.push_str(&wrap(
      "The syntax at the first line where a mismatching file differs, the ten most \
       common over every set:",
      WIDTH,
      "",
      "",
    ));
    out.push('\n');
    let rows: Vec<Vec<String>> = kinds
      .iter()
      .take(10)
      .map(|(kind, files)| vec![format!("`{}`", kind), files.to_string()])
      .collect();
    out.push_str(&table(&["syntax kind", "files"], "lr", &rows));
  }
  Some(out)
}

/// The latest measurement of each figure the ratchet holds, on this machine.
fn latest_figures(root: &Path, limits: &[metrics::Limit]) -> Option<String> {
  let mut rows = Vec::new();
  for limit in limits {
    let Some(point) = metrics::series(root, &limit.name).pop() else {
      continue;
    };
    rows.push(vec![
      format!("`{}`", limit.name),
      metrics::short(point.value),
      if point.noisy() {
        "not judged".to_string()
      } else if limit.holds(point.value) {
        "yes".to_string()
      } else {
        "no".to_string()
      },
      point.when[..10.min(point.when.len())].to_string(),
    ]);
  }
  if rows.is_empty() {
    return None;
  }
  let mut out = wrap(
    &format!(
      "The latest measurement of each figure on the machine that generated this page \
       ({}); the limits apply only on the recording machine, and a timing taken while \
       the machine was busy is not judged:",
      metrics::machine()
    ),
    WIDTH,
    "",
    "",
  );
  out.push('\n');
  out.push_str(&table(
    &["series", "latest", "within", "date"],
    "lrll",
    &rows,
  ));
  Some(out)
}

/// One row of the ratchet's history: three of its limits at a version.
fn limit_row(date: &str, commit: &str, text: &str) -> Vec<String> {
  let (_, limits) = metrics::parse_record(text);
  let value = |name: &str| limits.iter().find(|l| l.name == name).map(|l| l.value);
  vec![
    date.to_string(),
    commit.to_string(),
    value("perf.scaling.one_thread.mb_per_s").map_or("-".to_string(), |v| v.to_string()),
    value("speed.speedup.stable").map_or("-".to_string(), |v| v.to_string()),
    value("memst.serde.peak_live").map_or("-".to_string(), |v| fixed(v / 1e6, 1)),
  ]
}

fn meaning(root: &Path, local: bool) -> Result<Paper, String> {
  let text = read(root, "compat/meaning-exceptions.txt")?;
  let exceptions = records::parse_exceptions(&text);
  let compat = records::parse_compat(&read(root, "compat/compatibility.txt")?);
  let (fingerprint, _) = records::corpus_fingerprint(root)?;
  let counts = |list: &[records::Exception]| {
    let unmodelled = list.iter().filter(|e| unmodelled(&e.reason)).count();
    (list.len(), unmodelled, list.len() - unmodelled)
  };
  let (total, unmodelled_count, other) = counts(&exceptions);
  let mut figures = Vec::new();
  let mut rows = Vec::new();
  for set in SETS {
    let listed: Vec<&records::Exception> = exceptions.iter().filter(|e| e.set == *set).collect();
    let in_set = listed.iter().filter(|e| unmodelled(&e.reason)).count();
    figures.push((
      format!("meaning.exceptions.{}", set),
      listed.len().to_string(),
    ));
    rows.push(vec![
      shown(set).to_string(),
      listed.len().to_string(),
      in_set.to_string(),
      (listed.len() - in_set).to_string(),
    ]);
  }
  figures.push(("meaning.exceptions.total".to_string(), total.to_string()));
  figures.push((
    "meaning.exceptions.unmodelled".to_string(),
    unmodelled_count.to_string(),
  ));
  figures.push(("meaning.exceptions.other".to_string(), other.to_string()));
  let front = Front::new("meaning", MEANING, "measured")
    .text("command", "cargo xtask diff")
    .list(
      "sources",
      &["compat/meaning-exceptions.txt", "compat/compatibility.txt"],
    )
    .map("toolchain", reference(&compat))
    .text("corpus_fingerprint", &fingerprint)
    .text("seed", "FNV-1a of each file's text (scrambled set only)")
    .map("metrics", figures)
    .done()?;
  let summary = format!(
    "Every output of mto that `cargo xtask diff` scores must keep the input's meaning and \
     must not change when formatted again, or the run fails. The same meaning check, run \
     on the standard formatter's own outputs, flags {} files, each listed with its reason: \
     {} are rewrites the check does not model yet, and {} are the formatter changing \
     meaning, writing invalid code, or test inputs that differ from their expected files \
     beyond formatting.",
    total, unmodelled_count, other
  );
  let mut results = table(&["set", "listed", "not modelled", "other"], "lrrr", &rows);
  let mut reasons: Vec<(String, usize)> = Vec::new();
  for e in &exceptions {
    match reasons.iter_mut().find(|(r, _)| *r == e.reason) {
      Some((_, n)) => *n += 1,
      None => reasons.push((e.reason.clone(), 1)),
    }
  }
  reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
  results.push_str("\nThe reasons given, with how many files give each:\n\n");
  for (reason, n) in &reasons {
    results.push_str(&wrap(&format!("{} ({})", reason, n), WIDTH, "- ", "  "));
  }
  let versions = recent(
    records::file_history(root, "compat/meaning-exceptions.txt")?,
    |v| v.text == text,
  );
  let mut rows = vec![exception_row("current", "", &exceptions)];
  for v in &versions {
    rows.push(exception_row(
      &v.date,
      &v.commit,
      &records::parse_exceptions(&v.text),
    ));
  }
  let history = table(
    &["date", "commit", "listed", "not modelled", "other"],
    "llrrr",
    &rows,
  );
  Ok(Paper {
    page: Page {
      path: format!("{}/meaning.md", DIR),
      title: MEANING.to_string(),
      front: Some(front),
      regions: vec![
        Region::tracked("abstract", wrap(&summary, WIDTH, "", "")),
        Region::tracked("results", results),
        Region::tracked("history", history),
        Region::local(
          "incidents",
          if local {
            incidents(root, |kind| {
              ["meaning", "unstable", "panic"].contains(&kind)
            })
          } else {
            None
          },
        ),
      ],
    },
    headline: format!(
      "{} exceptions listed, {} not modelled yet",
      total, unmodelled_count
    ),
  })
}

fn performance(root: &Path, local: bool) -> Result<Paper, String> {
  let text = read(root, metrics::RECORD)?;
  let (machine, limits) = metrics::parse_record(&text);
  let baseline = records::parse_baseline(&read(root, "compat/baseline.txt")?);
  let before = records::parse_before(&read(root, metrics::BASELINE)?);
  let mut figures = Vec::new();
  if let Some(lex) = baseline.lex {
    figures.push(("corpus.lex.mb_per_s".to_string(), lex.to_string()));
  }
  if let Some(both) = baseline.lex_parse {
    figures.push(("corpus.lex_parse.mb_per_s".to_string(), both.to_string()));
  }
  let front = Front::new("performance", PERFORMANCE, "measured")
    .text("command", "cargo xtask metrics ratchet --update")
    .list(
      "sources",
      &[metrics::RECORD, "compat/baseline.txt", metrics::BASELINE],
    )
    .text("machine", &machine)
    .text(
      "seed",
      "fixed in code for the doctor bench's draws; none elsewhere",
    )
    .map("metrics", figures)
    .map(
      "limits",
      limits
        .iter()
        .map(|l| (l.name.clone(), l.value.to_string()))
        .collect(),
    )
    .done()?;
  let value = |name: &str| limits.iter().find(|l| l.name == name).map(|l| l.value);
  let summary = format!(
    "On the recording machine ({}), the ratchet holds {} figures. At their recorded \
     values, mto lays out and formats {} MB/s on one thread, checks a laid-out corpus {} \
     times as fast as the standard formatter's stable release, and holds at most {} MB of \
     memory live at once while laying out and formatting serde.",
    machine,
    limits.len(),
    value("perf.scaling.one_thread.mb_per_s").map_or("-".to_string(), |v| v.to_string()),
    value("speed.speedup.stable").map_or("-".to_string(), |v| v.to_string()),
    value("memst.serde.peak_live").map_or("-".to_string(), |v| fixed(v / 1e6, 1)),
  );
  let rows: Vec<Vec<String>> = limits
    .iter()
    .map(|l| {
      vec![
        format!("`{}`", l.name),
        before
          .iter()
          .find(|b| b.name == l.name)
          .map_or("-".to_string(), |b| b.value.clone()),
        l.value.to_string(),
        format!("{}%", fixed(l.tolerance * 100.0, 0)),
        match l.better {
          metrics::Better::Higher => "higher",
          metrics::Better::Lower => "lower",
        }
        .to_string(),
      ]
    })
    .collect();
  let mut results = table(
    &["series", "before", "limit", "slack", "better"],
    "lrrrl",
    &rows,
  );
  results.push('\n');
  results.push_str(&wrap(
    &format!(
      "Parsing speed on one thread, release build, on {}, over {}:",
      baseline.machine, baseline.corpus
    ),
    WIDTH,
    "",
    "",
  ));
  results.push('\n');
  let speed = |v: Option<f64>| v.map_or("-".to_string(), |v| v.to_string());
  results.push_str(&table(
    &["stage", "MB/s"],
    "lr",
    &[
      vec!["lexer".to_string(), speed(baseline.lex)],
      vec!["lexer and parser".to_string(), speed(baseline.lex_parse)],
    ],
  ));
  let versions = recent(records::file_history(root, metrics::RECORD)?, |v| {
    v.text == text
  });
  let mut rows = vec![limit_row("current", "", &text)];
  for v in &versions {
    rows.push(limit_row(&v.date, &v.commit, &v.text));
  }
  let history = table(
    &[
      "date",
      "commit",
      "one thread MB/s",
      "speedup",
      "serde peak MB",
    ],
    "llrrr",
    &rows,
  );
  Ok(Paper {
    page: Page {
      path: format!("{}/performance.md", DIR),
      title: PERFORMANCE.to_string(),
      front: Some(front),
      regions: vec![
        Region::tracked("abstract", wrap(&summary, WIDTH, "", "")),
        Region::tracked("results", results),
        Region::tracked("history", history),
        Region::local(
          "latest",
          if local {
            latest_figures(root, &limits)
          } else {
            None
          },
        ),
      ],
    },
    headline: format!(
      "{} MB/s formatting on one thread",
      value("perf.scaling.one_thread.mb_per_s").map_or("-".to_string(), |v| v.to_string())
    ),
  })
}

/// A paper whose data the repository does not record yet.
fn planned(id: &str, title: &str) -> Result<Paper, String> {
  let front = Front::new(id, title, "planned")
    .text("command", "none yet")
    .list("sources", &[])
    .text("seed", "none")
    .map("metrics", Vec::new())
    .done()?;
  let none = "No data is recorded for this paper yet. Section 2 describes the planned \
              method; section 3 will hold the results once a command records them.";
  Ok(Paper {
    page: Page {
      path: format!("{}/{}.md", DIR, id),
      title: title.to_string(),
      front: Some(front),
      regions: vec![
        Region::tracked("abstract", wrap(none, WIDTH, "", "")),
        Region::tracked("results", "No data is recorded yet.\n".to_string()),
      ],
    },
    headline: "no data recorded yet".to_string(),
  })
}

/// A tracked file's text.
fn read(root: &Path, path: &str) -> Result<String, String> {
  fs::read_to_string(root.join(path)).map_err(|e| format!("cannot read {}: {}", path, e))
}

/// The history after the current record: the versions from the newest back, without the
/// newest ones that equal the current record (it is shown as the current row), so the
/// table reads the same before and after the record is committed.
fn recent<T>(versions: Vec<T>, current: impl Fn(&T) -> bool) -> Vec<T> {
  versions.into_iter().skip_while(|v| current(v)).collect()
}

/// The reference formatter's version from the compatibility record, as a toolchain map.
fn reference(compat: &Compat) -> Vec<(String, String)> {
  vec![(
    "reference_formatter".to_string(),
    quote(&records::reference_version(&compat.reference)),
  )]
}

/// A set's name in prose and tables: the scrambled set by what it is.
fn shown(set: &str) -> &str {
  if set == "corpus-unformatted" {
    "corpus-scrambled"
  } else {
    set
  }
}

/// File stems of the `.rs` files directly in `dir`, sorted.
fn stems(dir: &Path) -> Vec<String> {
  let mut stems: Vec<String> = fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .map(|entry| entry.path())
    .filter(|path| path.extension().is_some_and(|e| e == "rs"))
    .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
    .collect();
  stems.sort();
  stems
}

/// A Markdown table; `align` has an `l` or `r` per column.
fn table(headers: &[&str], align: &str, rows: &[Vec<String>]) -> String {
  let line = |cells: Vec<&str>| format!("| {} |\n", cells.join(" | "));
  let mut out = line(headers.to_vec());
  let rule: Vec<&str> = align
    .chars()
    .map(|a| if a == 'r' { "---:" } else { "---" })
    .collect();
  out.push_str(&format!("|{}|\n", rule.join("|")));
  for row in rows {
    out.push_str(&line(row.iter().map(String::as_str).collect()));
  }
  out
}

/// Whether an exception's reason says the check does not model the rewrite.
fn unmodelled(reason: &str) -> bool {
  reason.contains("not modelled") || reason.contains("equivalent")
}

#[cfg(test)]
mod tests {
  use super::*;

  fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
  }

  #[test]
  fn test_every_page_keeps_to_the_rules_for_public_markdown() {
    for local in [false, true] {
      for page in all(root(), local).unwrap() {
        let mut text = page
          .front
          .as_ref()
          .map_or(String::new(), |f| f.render(&super::super::Stamp::blank()));
        for region in &page.regions {
          text.push_str(region.text.as_deref().unwrap_or_default());
        }
        for line in text.lines() {
          assert!(line.chars().count() <= WIDTH, "{}: {}", page.path, line);
          assert!(
            !line.contains('\u{2014}') && !line.contains(" -- "),
            "{}",
            line
          );
        }
        let stray = crate::gates::stray_names(&page.path, &text);
        assert!(stray.is_empty(), "{:?}", stray);
      }
    }
  }

  #[test]
  fn test_tables_and_history_rows() {
    let made = table(&["a", "b"], "lr", &[vec!["x".to_string(), "1".to_string()]]);
    assert_eq!(made, "| a | b |\n|---|---:|\n| x | 1 |\n");
    let versions = vec![3, 3, 2, 3, 1];
    assert_eq!(recent(versions, |v| *v == 3), vec![2, 3, 1]);
    assert!(unmodelled("an attributed body: equivalent, not modelled"));
    assert!(!unmodelled("the formatter drops `pub`"));
    assert_eq!(shown("corpus-unformatted"), "corpus-scrambled");
  }

  #[test]
  fn test_the_papers_are_built_from_the_tracked_records() {
    let pages = all(root(), false).unwrap();
    let paths: Vec<&str> = pages.iter().map(|p| p.path.as_str()).collect();
    for id in [
      "compatibility",
      "meaning",
      "fuzzing",
      "performance",
      "layout",
    ] {
      assert!(
        paths.contains(&format!("{}/{}.md", DIR, id).as_str()),
        "{}",
        id
      );
    }
    // Without the local records, every local region is left as written.
    assert!(
      pages
        .iter()
        .flat_map(|p| &p.regions)
        .all(|r| r.local == r.text.is_none())
    );
    let compat = &pages[0];
    let results = compat.regions[1].text.as_deref().unwrap();
    let record =
      records::parse_compat(&fs::read_to_string(root().join("compat/compatibility.txt")).unwrap());
    let corpus = record.sets["corpus"];
    assert!(results.contains(&format!("{} of {}", corpus.matched_files, corpus.files)));
    assert!(results.contains("corpus-scrambled"));
  }
}
