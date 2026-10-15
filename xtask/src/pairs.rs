//! `cargo xtask pairs`: the scrambled-pair suite. Each run draws corpus files with a fresh
//! seed and scrambles them (`xtask-tools pairs`, which links mtindo): items shuffled where
//! their order carries no meaning, then whitespace, each pass checked token for token, so
//! every run samples other states real files may be in. The seed is printed, and `--seed`
//! replays a run.
//!
//! Each scrambled file is formatted by mto and by the standard formatter, and scored as
//! `cargo xtask diff` scores: files and lines matching (formatting only; the standard
//! formatter does not reorder items).
//!
//! The crates that compile offline (those of `domain_13_rw`) are also built three times,
//! with `cargo check --offline` in copies outside the repository, their target folders
//! kept warm between runs:
//!
//! 1. scrambled: it must compile, or the scrambler is at fault. A crate that fails here
//!    while it compiles as fetched is filed as a `scrambler-bug` and left out;
//! 2. laid out and formatted by mto: failing here is a `compile-break`, filed with the seed
//!    that shows it;
//! 3. formatted by the standard formatter: failing here is noted, as not mto's fault.
//!
//! The figures are kept as `pairs.*` series; the files' scores of the latest run in
//! `.dev/logs/pairs-files.tsv`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::common::log::Logger;
use crate::common::{process, tree};
use crate::{corpus, incidents, metrics};

/// The crates checked to compile: the real-world compile check's (tests/domain_13_rw.rs),
/// which all build with `cargo check --offline` once fetched.
pub const COMPILED: &[&str] = &[
  "lazy_static",
  "itoa",
  "termcolor",
  "logos",
  "once_cell",
  "bitflags",
  "bytes",
  "crossbeam",
  "uuid",
  "anyhow",
  "clap",
  "serde",
  "tokio",
  "rayon",
  "rand",
];

/// How many crates a run draws when none are named: this many that are compiled, and as
/// many that are only scored.
const DRAWN_PER_KIND: usize = 3;

/// What mto's copy is laid out and formatted with.
const MTO_SETTINGS: &str =
  "# Written by `cargo xtask pairs`.\nroot = true\nlayout = true\nformat = true\n";

/// How a crate's copies built.
#[derive(Clone, Debug, PartialEq)]
pub enum Build {
  /// Not built: why (`--no-compile`, not a crate that compiles offline).
  NotChecked(&'static str),
  /// Not even the crate as fetched compiles offline here.
  NotOffline,
  /// The scrambled copy does not compile: what the compiler said.
  Scrambled(String),
  /// The scrambled copy compiled; how mto's and the standard formatter's copies did, each
  /// what the compiler (or mto) said when they failed.
  Checked {
    mto: Result<(), String>,
    standard: Result<(), String>,
  },
}

/// One crate's results.
#[derive(Clone, Debug, PartialEq)]
pub struct Crate {
  pub name: String,
  pub scored: Vec<Scored>,
  /// Files the standard formatter refused, left unscored.
  pub refused: usize,
  /// Files not scrambled (syntax errors, carriage returns, not UTF-8).
  pub skipped: usize,
  /// Files whose scramble failed its own checks: (file, what failed).
  pub bugs: Vec<(String, String)>,
  /// Files mto panicked on: (file, message).
  pub panics: Vec<(String, String)>,
  pub build: Build,
}

impl Crate {
  fn new(name: &str) -> Crate {
    Crate {
      name: name.to_string(),
      scored: Vec::new(),
      refused: 0,
      skipped: 0,
      bugs: Vec::new(),
      panics: Vec::new(),
      build: Build::NotChecked("not run"),
    }
  }

  /// Left out of the figures: its scrambled copy did not compile.
  fn excluded(&self) -> bool {
    matches!(self.build, Build::Scrambled(_))
  }
}

/// What a run was asked to do.
#[derive(Debug, PartialEq)]
pub struct Options {
  pub seed: Option<u64>,
  pub crates: Vec<String>,
  /// Files drawn per crate; 0 for all.
  pub files: u32,
  pub compile: bool,
  pub keep: Option<PathBuf>,
}

/// One scored file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scored {
  pub file: String,
  pub moved: usize,
  pub matched: usize,
  pub lines: usize,
  pub same: bool,
}

/// The figures over the crates not left out.
#[derive(Debug, Default, PartialEq)]
pub struct Totals {
  pub files: usize,
  pub same: usize,
  pub lines: usize,
  pub matched: usize,
  pub moved: usize,
  pub refused: usize,
  pub compiled: usize,
  pub mto_breaks: usize,
  pub standard_breaks: usize,
  pub scrambler_bugs: usize,
  pub panics: usize,
}

/// The crates `seed` draws from `available`: some that are compiled and as many that are
/// only scored, in the order of `available`.
pub fn draw_crates(seed: u64, available: &[String]) -> Vec<String> {
  let mut state = seed | 1;
  let mut next = || {
    // xorshift64*
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    state.wrapping_mul(0x2545_f491_4f6c_dd1d)
  };
  let mut chosen = Vec::new();
  for compiled in [true, false] {
    let mut pool: Vec<&String> = available
      .iter()
      .filter(|n| COMPILED.contains(&n.as_str()) == compiled)
      .collect();
    for _ in 0..DRAWN_PER_KIND.min(pool.len()) {
      let at = (next() % pool.len() as u64) as usize;
      chosen.push(pool.remove(at).clone());
    }
  }
  chosen.sort_by_key(|n| available.iter().position(|a| a == n));
  chosen
}

/// The options of a run, from its arguments.
pub fn options(m: &clap::ArgMatches) -> Options {
  Options {
    seed: m.get_one::<u64>("seed").copied(),
    crates: crate::cli::texts(m, "crates"),
    files: m.get_one::<u32>("files").copied().unwrap_or(20),
    compile: !crate::cli::on(m, "no-compile"),
    keep: crate::cli::text(m, "keep").map(PathBuf::from),
  }
}

/// Each crate's scores, from what `xtask-tools pairs` printed.
pub fn read_results(names: &[String], output: &str) -> Vec<Crate> {
  let mut crates: Vec<Crate> = names.iter().map(|n| Crate::new(n)).collect();
  for line in output.lines() {
    let fields: Vec<&str> = line.split('\t').collect();
    let Some(result) = fields
      .get(1)
      .and_then(|name| crates.iter_mut().find(|c| c.name == *name))
    else {
      continue;
    };
    let file = fields.get(2).unwrap_or(&"").to_string();
    let rest = fields.get(3).unwrap_or(&"").to_string();
    match fields[0] {
      "file" if fields.len() == 7 => {
        let number = |at: usize| fields[at].parse::<usize>().unwrap_or(0);
        result.scored.push(Scored {
          file,
          moved: number(3),
          matched: number(4),
          lines: number(5),
          same: fields[6] == "1",
        });
      }
      "refused" => result.refused += 1,
      "skipped" => result.skipped += 1,
      "scrambler-bug" => result.bugs.push((file, rest)),
      "panic" => result.panics.push((file, rest)),
      _ => {}
    }
  }
  crates
}

/// The summary table.
pub fn render(seed: u64, crates: &[Crate], totals: &Totals) -> String {
  let percent = |a: usize, b: usize| {
    if b == 0 {
      0.0
    } else {
      a as f64 * 100.0 / b as f64
    }
  };
  let files = |same: usize, of: usize| format!("{} of {} ({:.1}%)", same, of, percent(same, of));
  let lines =
    |matched: usize, of: usize| format!("{} of {} ({:.2}%)", matched, of, percent(matched, of));
  let mut out = format!(
    "\n{:<12} {:>6} {:>22} {:>26}  {}\n",
    "crate", "moved", "files matching", "lines matching", "compiles (scrambled, mto, standard)"
  );
  for c in crates {
    let same = c.scored.iter().filter(|s| s.same).count();
    let total: usize = c.scored.iter().map(|s| s.lines).sum();
    let matched: usize = c.scored.iter().map(|s| s.matched).sum();
    let moved: usize = c.scored.iter().map(|s| s.moved).sum();
    let built = match &c.build {
      Build::NotChecked(why) => format!("not built: {}", why),
      Build::NotOffline => "not built: does not compile offline as fetched".to_string(),
      Build::Scrambled(_) => "SCRAMBLER BUG: the scrambled copy fails; left out".to_string(),
      Build::Checked { mto, standard } => format!(
        "yes, {}, {}",
        if mto.is_ok() { "yes" } else { "NO" },
        if standard.is_ok() { "yes" } else { "no" }
      ),
    };
    out.push_str(&format!(
      "{:<12} {:>6} {:>22} {:>26}  {}\n",
      c.name,
      moved,
      files(same, c.scored.len()),
      lines(matched, total),
      built
    ));
    let mut notes = Vec::new();
    if c.refused + c.skipped > 0 {
      notes.push(format!(
        "{} refused by the standard formatter, {} not scrambled",
        c.refused, c.skipped
      ));
    }
    for (file, what) in &c.bugs {
      notes.push(format!("scrambler bug in {}: {}", file, what));
    }
    for (file, message) in &c.panics {
      notes.push(format!("mto panicked on {}: {}", file, message));
    }
    match &c.build {
      Build::Scrambled(said) => notes.push(format!("scrambled copy: {}", first_error(said))),
      Build::Checked { mto, standard } => {
        if let Err(said) = mto {
          notes.push(format!("after mto: {}", first_error(said)));
        }
        if let Err(said) = standard {
          notes.push(format!(
            "after the standard formatter (not mto's fault): {}",
            first_error(said)
          ));
        }
      }
      _ => {}
    }
    for note in notes {
      out.push_str(&format!("  {}\n", note));
    }
  }
  out.push_str(&format!(
    "{:<12} {:>6} {:>22} {:>26}  {} built, {} broken by mto, {} by the standard formatter\n",
    "total",
    totals.moved,
    files(totals.same, totals.files),
    lines(totals.matched, totals.lines),
    totals.compiled,
    totals.mto_breaks,
    totals.standard_breaks
  ));
  out.push_str(&format!(
    "{} scrambler bug(s); replay with: cargo xtask pairs --seed {}\n",
    totals.scrambler_bugs, seed
  ));
  out
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let options = options(m);
  let log = Logger::new(root, "pairs");
  let corpus_dir = corpus::cache_dir(root);
  let available = match corpus::load_pins(root) {
    Ok(pins) => pins
      .into_iter()
      .filter(|p| !p.opt_in && corpus_dir.join(&p.name).is_dir())
      .map(|p| p.name)
      .collect::<Vec<_>>(),
    Err(e) => {
      eprintln!("{}", e);
      return ExitCode::from(2);
    }
  };
  if available.is_empty() {
    eprintln!("the corpus is not checked out: run `cargo xtask corpus fetch` first");
    return ExitCode::from(2);
  }
  if let Some(unknown) = options.crates.iter().find(|c| !available.contains(c)) {
    eprintln!(
      "no corpus crate `{}` is checked out; there are: {}",
      unknown,
      available.join(", ")
    );
    return ExitCode::from(2);
  }
  let seed = options.seed.unwrap_or_else(fresh_seed);
  let names = if options.crates.is_empty() {
    draw_crates(seed, &available)
  } else {
    options.crates.clone()
  };
  // Outside the repository, so cargo does not take a copy for a member of its workspace.
  let scratch = match &options.keep {
    Some(dir) => {
      let dir = if dir.is_absolute() {
        dir.clone()
      } else {
        std::env::current_dir().unwrap_or_default().join(dir)
      };
      if dir.starts_with(root) {
        eprintln!(
          "--keep needs a folder outside this repository (cargo would take the copies for members of its workspace)"
        );
        return ExitCode::from(2);
      }
      dir
    }
    None => {
      std::env::temp_dir()
        .join("mtindo-pairs")
        .join(format!("{}-{}", std::process::id(), seed))
    }
  };
  let _ = fs::remove_dir_all(scratch.join("files"));
  let _ = fs::remove_dir_all(scratch.join("crates"));
  let files_dir = scratch.join("files");
  if let Err(e) = fs::create_dir_all(&files_dir) {
    eprintln!("cannot make {}: {}", files_dir.display(), e);
    return ExitCode::from(1);
  }
  println!(
    "scrambled pairs, seed {} ({}, {} per crate)",
    seed,
    names.join(", "),
    match options.files {
      0 => "every file".to_string(),
      n => format!("{} files", n),
    }
  );
  log.info(&format!(
    "seed {}, crates {}, files {}",
    seed,
    names.join(","),
    options.files
  ));
  let mut args: Vec<String> = [
    "run",
    "--quiet",
    "--release",
    "--package",
    "xtask-tools",
    "--",
    "pairs",
  ]
  .iter()
  .map(|s| s.to_string())
  .collect();
  args.extend([
    "--seed".to_string(),
    seed.to_string(),
    "--out".to_string(),
    files_dir.display().to_string(),
    "--files".to_string(),
    options.files.to_string(),
  ]);
  args.extend(
    names
      .iter()
      .map(|n| format!("{}={}", n, corpus_dir.join(n).display())),
  );
  let (code, output) = process::capture(root, "cargo", &args, &[], process::LONG);
  if code != 0 {
    println!("xtask-tools pairs failed (exit {}):\n{}", code, output);
    return ExitCode::from(1);
  }
  let mut crates = read_results(&names, &output);
  if options.compile {
    let mto = build_mto(root);
    for result in &mut crates {
      result.build = if !COMPILED.contains(&result.name.as_str()) {
        Build::NotChecked("scored only (not a compile-checked crate)")
      } else {
        match &mto {
          Ok(mto) => {
            println!("  building {} three ways...", result.name);
            build(&corpus_dir.join(&result.name), &result.name, &scratch, mto)
          }
          Err(why) => {
            println!("{}", why);
            Build::NotChecked("mto did not build")
          }
        }
      };
    }
  } else {
    for result in &mut crates {
      result.build = Build::NotChecked("--no-compile");
    }
  }
  let replay = |name: &str| {
    format!(
      "cargo xtask pairs --seed {} --crates {} --files {}",
      seed, name, options.files
    )
  };
  let filed = file_incidents(root, &crates, seed, &scratch, &corpus_dir, &replay);
  let totals = totals(&crates);
  record(root, seed, &totals);
  write_scores(root, &crates);
  print!("{}", render(seed, &crates, &totals));
  for line in &filed {
    println!("{}", line);
  }
  log.info(&format!(
    "seed {}: {} of {} files, {} of {} lines; {} mto compile break(s), {} scrambler bug(s)",
    seed,
    totals.same,
    totals.files,
    totals.matched,
    totals.lines,
    totals.mto_breaks,
    totals.scrambler_bugs
  ));
  match &options.keep {
    Some(_) => println!("copies kept in {}", scratch.display()),
    None => {
      let _ = fs::remove_dir_all(&scratch);
    }
  }
  if totals.mto_breaks + totals.scrambler_bugs + totals.panics > 0 {
    ExitCode::from(1)
  } else {
    ExitCode::SUCCESS
  }
}

pub fn totals(crates: &[Crate]) -> Totals {
  let mut t = Totals::default();
  for c in crates {
    t.scrambler_bugs += c.bugs.len() + usize::from(c.excluded());
    if c.excluded() {
      continue;
    }
    t.files += c.scored.len();
    t.same += c.scored.iter().filter(|s| s.same).count();
    t.lines += c.scored.iter().map(|s| s.lines).sum::<usize>();
    t.matched += c.scored.iter().map(|s| s.matched).sum::<usize>();
    t.moved += c.scored.iter().map(|s| s.moved).sum::<usize>();
    t.refused += c.refused;
    t.panics += c.panics.len();
    if let Build::Checked { mto, standard } = &c.build {
      t.compiled += 1;
      t.mto_breaks += usize::from(mto.is_err());
      t.standard_breaks += usize::from(standard.is_err());
    }
  }
  t
}

/// Builds a crate's three copies: scrambled, laid out and formatted by mto, formatted by
/// the standard formatter.
fn build(original: &Path, name: &str, scratch: &Path, mto: &Path) -> Build {
  let files = scratch.join("files");
  let copies = scratch.join("crates").join(name);
  // Kept between runs, so a check builds the crate and not its dependencies.
  let target = std::env::temp_dir().join("mtindo-pairs-target").join(name);
  let check = |dir: &Path| -> Result<(), String> {
    let env = [("CARGO_TARGET_DIR".to_string(), target.display().to_string())];
    let args = ["check", "--offline", "--quiet"].map(String::from);
    let (code, output) = process::capture(dir, "cargo", &args, &env, process::LONG);
    if code == 0 { Ok(()) } else { Err(output) }
  };
  let copy = |side: &str, overlay: Option<&str>| -> Result<PathBuf, String> {
    let dir = copies.join(side);
    copy_tree(original, &dir).map_err(|e| format!("cannot copy {}: {}", name, e))?;
    if let Some(overlay) = overlay {
      copy_tree(&files.join(overlay).join(name), &dir)
        .map_err(|e| format!("cannot copy {}'s {} files: {}", name, overlay, e))?;
    }
    Ok(dir)
  };
  let scrambled = match copy("scrambled", Some("scrambled")) {
    Ok(dir) => dir,
    Err(why) => {
      return Build::Checked {
        mto: Err(why.clone()),
        standard: Err(why),
      };
    }
  };
  if let Err(said) = check(&scrambled) {
    // The scrambler's fault only if the crate compiles as fetched.
    return match copy("as-fetched", None).and_then(|dir| check(&dir)) {
      Ok(()) => Build::Scrambled(said),
      Err(_) => Build::NotOffline,
    };
  }
  let mto_result = copy("mto", Some("scrambled")).and_then(|dir| {
    let state = scratch.join("state");
    let user = scratch.join("user.toml");
    let _ = fs::create_dir_all(&state);
    let _ = fs::write(&user, "");
    fs::write(dir.join("mto.toml"), MTO_SETTINGS).map_err(|e| e.to_string())?;
    let mut args: Vec<String> = ["-q", "--no-history", "--unguarded", "--skip-children"]
      .map(String::from)
      .to_vec();
    let mut scrambled_files = Vec::new();
    collect_files(&files.join("scrambled").join(name), &mut scrambled_files);
    let base = files.join("scrambled").join(name);
    args.extend(
      scrambled_files
        .iter()
        .filter_map(|f| f.strip_prefix(&base).ok())
        .map(|f| f.display().to_string()),
    );
    let env = [
      ("MTINDO_CONFIG".to_string(), user.display().to_string()),
      ("XDG_STATE_HOME".to_string(), state.display().to_string()),
    ];
    let (code, output) =
      process::capture(&dir, &mto.display().to_string(), &args, &env, process::LONG);
    // 3: files left alone (syntax errors); 4: the verifier stopped a write, file unchanged.
    if !matches!(code, 0 | 3 | 4) {
      return Err(format!("error: mto exited with {}\n{}", code, output));
    }
    check(&dir)
  });
  let standard = copy("standard", Some("standard")).and_then(|dir| check(&dir));
  Build::Checked {
    mto: mto_result,
    standard,
  }
}

/// The release mto, built.
fn build_mto(root: &Path) -> Result<PathBuf, String> {
  let args = ["build", "--quiet", "--release", "--bin", "mto"].map(String::from);
  let (code, output) = process::capture(root, "cargo", &args, &[], process::LONG);
  if code != 0 {
    return Err(format!("cannot build mto:\n{}", output));
  }
  Ok(root.join("target").join("release").join("mto"))
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) {
  for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
    let path = entry.path();
    if path.is_dir() {
      collect_files(&path, files);
    } else {
      files.push(path);
    }
  }
}

/// Copies the files under `from` into `to`, leaving out `.git` and `target`; symbolic links
/// are copied as links.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
  fs::create_dir_all(to)?;
  for entry in fs::read_dir(from)? {
    let entry = entry?;
    let name = entry.file_name();
    if name == ".git" || name == "target" {
      continue;
    }
    let path = entry.path();
    let kind = entry.file_type()?;
    let destination = to.join(&name);
    if kind.is_symlink() {
      let link = fs::read_link(&path)?;
      let _ = fs::remove_file(&destination);
      #[cfg(unix)]
      std::os::unix::fs::symlink(link, &destination)?;
      #[cfg(not(unix))]
      let _ = link;
    } else if kind.is_dir() {
      copy_tree(&path, &destination)?;
    } else if kind.is_file() {
      fs::copy(&path, &destination)?;
    }
  }
  Ok(())
}

/// What kind of error a message starts with, numbers aside: `error[E0425]`, for keys.
fn error_head(message: &str) -> String {
  let head = message.split(':').next().unwrap_or(message);
  head
    .chars()
    .take(60)
    .map(|c| {
      if c.is_ascii_digit() && !head.starts_with("error[") {
        '#'
      } else {
        c
      }
    })
    .collect()
}

/// Files what a run found: mto's compile breaks, and the scrambler's bugs. Marks fixed the
/// open ones this run's seed and crate replay that no longer fail. What was filed or fixed,
/// one line each.
fn file_incidents(
  root: &Path,
  crates: &[Crate],
  seed: u64,
  scratch: &Path,
  corpus_dir: &Path,
  replay: &dyn Fn(&str) -> String,
) -> Vec<String> {
  let tree = tree::identity(root);
  let run = std::env::var(crate::runs::RUN_DIR)
    .ok()
    .and_then(|d| {
      Path::new(&d)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    })
    .unwrap_or_default();
  let mut lines = Vec::new();
  let mut file = |kind: &str, name: &str, key: &str, said: &str, input: Option<Vec<u8>>| {
    let replay = replay(name);
    let report = incidents::Report {
      kind,
      target: name,
      said,
      input: input.as_deref(),
      replay: &replay,
      tree: &tree,
      run: &run,
      key: Some(key),
    };
    match incidents::file(root, &report) {
      Ok(filed) => lines.push(format!(
        "{} {} incident {} (cargo xtask incidents show {})",
        match filed {
          incidents::Filed::New(_) => "new",
          incidents::Filed::Again(_) => "again:",
          incidents::Filed::Reopened(_) => "reopened:",
        },
        kind,
        filed.id(),
        filed.id()
      )),
      Err(e) => lines.push(format!("cannot file a {} incident: {}", kind, e)),
    }
  };
  for c in crates {
    for (path, what) in &c.bugs {
      let said = format!(
        "pairs: the scrambler's check failed on {}/{} (seed {}): {}",
        c.name, path, seed, what
      );
      let key = format!("pairs check {}/{}: {}", c.name, path, error_head(what));
      let input = fs::read(corpus_dir.join(&c.name).join(path)).ok();
      file("scrambler-bug", &c.name, &key, &said, input);
    }
    let (kind, side, said) = match &c.build {
      Build::Scrambled(said) => ("scrambler-bug", "scrambled", said),
      Build::Checked { mto: Err(said), .. } => ("compile-break", "mto", said),
      _ => continue,
    };
    let headline = if kind == "compile-break" {
      format!(
        "pairs: {} compiles scrambled (seed {}) and not after mto lays it out and formats it",
        c.name, seed
      )
    } else {
      format!(
        "pairs: {} compiles as fetched and not scrambled (seed {})",
        c.name, seed
      )
    };
    let errors: String = said
      .lines()
      .skip_while(|l| !l.starts_with("error"))
      .take(40)
      .collect::<Vec<_>>()
      .join("\n");
    let shown = format!("{}\n{}", headline, errors);
    // The copy of the first file the compiler names, as the input.
    let input = said
      .lines()
      .find_map(|l| l.trim().strip_prefix("--> "))
      .and_then(|at| at.split(':').next())
      .and_then(|file| fs::read(scratch.join("crates").join(&c.name).join(side).join(file)).ok());
    let key = format!("pairs {}: {}", c.name, error_head(&first_error(said)));
    file(kind, &c.name, &key, &shown, input);
  }
  // A seed and crate that failed before and builds now: fixed.
  for c in crates {
    let (scrambled_ok, mto_ok) = match &c.build {
      Build::Checked { mto, .. } => (c.bugs.is_empty(), mto.is_ok()),
      _ => continue,
    };
    let replay = replay(&c.name);
    for (dir, mut incident) in incidents::all(root) {
      let fixed = match incident.kind.as_str() {
        "compile-break" => mto_ok,
        "scrambler-bug" => scrambled_ok,
        _ => false,
      };
      if fixed && incident.status == "open" && incident.replay == replay {
        incidents::mark(root, &dir, &mut incident, "fixed", &tree);
        lines.push(format!("fixed {} incident {}", incident.kind, incident.id));
      }
    }
  }
  lines
}

/// The first error a build printed, on one line.
fn first_error(said: &str) -> String {
  said
    .lines()
    .find(|l| l.starts_with("error"))
    .or_else(|| said.lines().find(|l| !l.trim().is_empty()))
    .unwrap_or("")
    .trim()
    .to_string()
}

/// A seed no run has had, short enough to type and to keep exactly as a metric.
fn fresh_seed() -> u64 {
  let nanos = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_nanos() as u64);
  (nanos ^ (u64::from(std::process::id()) << 20)).wrapping_mul(0x9e37_79b9_7f4a_7c15)
    % 1_000_000_000_000
}

/// Keeps the run's figures as `pairs.*` series; the seed too, so each run's figures can be
/// replayed.
fn record(root: &Path, seed: u64, t: &Totals) {
  let percent = |a: usize, b: usize| {
    if b == 0 {
      0.0
    } else {
      (a as f64 * 10000.0 / b as f64).round() / 100.0
    }
  };
  metrics::record(root, "pairs.seed", seed as f64, "seed");
  metrics::record(root, "pairs.likeness.files", percent(t.same, t.files), "%");
  metrics::record(
    root,
    "pairs.likeness.lines",
    percent(t.matched, t.lines),
    "%",
  );
  metrics::record(root, "pairs.files", t.files as f64, "files");
  metrics::record(root, "pairs.refused", t.refused as f64, "files");
  metrics::record(root, "pairs.moved", t.moved as f64, "items");
  metrics::record(root, "pairs.compile.crates", t.compiled as f64, "crates");
  metrics::record(
    root,
    "pairs.compile.mto_breaks",
    t.mto_breaks as f64,
    "crates",
  );
  metrics::record(
    root,
    "pairs.compile.standard_breaks",
    t.standard_breaks as f64,
    "crates",
  );
  metrics::record(
    root,
    "pairs.scrambler_bugs",
    t.scrambler_bugs as f64,
    "bugs",
  );
  metrics::record(root, "pairs.panics", t.panics as f64, "files");
}

/// Each scored file of the run, `.dev/logs/pairs-files.tsv`.
fn write_scores(root: &Path, crates: &[Crate]) {
  let mut out = String::from("crate\tfile\tmoved\tmatched\tlines\tsame\n");
  for c in crates.iter().filter(|c| !c.excluded()) {
    for s in &c.scored {
      out.push_str(&format!(
        "{}\t{}\t{}\t{}\t{}\t{}\n",
        c.name,
        s.file,
        s.moved,
        s.matched,
        s.lines,
        u8::from(s.same)
      ));
    }
  }
  let logs = root.join(".dev").join("logs");
  let _ = fs::create_dir_all(&logs);
  let _ = fs::write(logs.join("pairs-files.tsv"), out);
}

#[cfg(test)]
mod tests {
  use super::*;

  fn names(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
  }

  fn parsed(words: &[&str]) -> Options {
    match crate::cli::parse(&names(words)) {
      crate::cli::Parsed::Run(name, m) => {
        assert_eq!(name, "pairs");
        options(&m)
      }
      _ => panic!("not a run: {:?}", words),
    }
  }

  #[test]
  fn test_a_seed_draws_the_same_crates_some_of_them_compiled() {
    let available = names(&["anyhow", "axum", "itoa", "libc", "serde", "syn", "tokio"]);
    let drawn = draw_crates(9, &available);
    assert_eq!(drawn, draw_crates(9, &available));
    assert_eq!(drawn.len(), 6);
    let compiled = drawn
      .iter()
      .filter(|n| COMPILED.contains(&n.as_str()))
      .count();
    assert_eq!(compiled, 3);
    assert!((10..30).any(|seed| draw_crates(seed, &available) != drawn));
    assert_eq!(draw_crates(1, &names(&["syn"])), ["syn"]);
  }

  #[test]
  fn test_arguments() {
    assert_eq!(
      parsed(&["pairs"]),
      Options {
        seed: None,
        crates: Vec::new(),
        files: 20,
        compile: true,
        keep: None,
      }
    );
    let all = parsed(&[
      "pairs",
      "--seed",
      "42",
      "--crates",
      "itoa,serde",
      "--files",
      "0",
      "--no-compile",
      "--keep",
      "/tmp/k",
    ]);
    assert_eq!(all.seed, Some(42));
    assert_eq!(all.crates, ["itoa", "serde"]);
    assert_eq!(all.files, 0);
    assert!(!all.compile);
    assert_eq!(all.keep, Some(PathBuf::from("/tmp/k")));
    assert!(matches!(
      crate::cli::parse(&names(&["pairs", "--seed", "x"])),
      crate::cli::Parsed::Error(_)
    ));
  }

  #[test]
  fn test_error_heads_make_stable_keys() {
    assert_eq!(error_head("error[E0425]: cannot find"), "error[E0425]");
    assert_eq!(
      error_head("layout: the shuffled file does not parse"),
      "layout"
    );
    assert_eq!(error_head("token 12 is x"), "token ## is x");
  }

  #[test]
  fn test_results_are_read_and_totalled_leaving_out_failed_scrambles() {
    let output = "\
file\titoa\tsrc/lib.rs\t3\t90\t100\t0
file\titoa\tsrc/udiv128.rs\t0\t20\t20\t1
refused\titoa\tbenches/bench.rs\trefused: exit 1
skipped\tserde\tsrc/x.rs\tsyntax errors
scrambler-bug\tserde\tsrc/y.rs\twhitespace: token 3 differs
file\tserde\tsrc/lib.rs\t1\t10\t10\t1
noise from cargo
";
    let mut crates = read_results(&names(&["itoa", "serde"]), output);
    assert_eq!(crates[0].scored.len(), 2);
    assert_eq!(crates[0].refused, 1);
    assert_eq!(crates[1].skipped, 1);
    assert_eq!(crates[1].bugs.len(), 1);
    crates[0].build = Build::Checked {
      mto: Err("error[E0425]: cannot find value `x`\n --> src/lib.rs:1:1".to_string()),
      standard: Ok(()),
    };
    crates[1].build = Build::Scrambled("error: expected item".to_string());
    let t = totals(&crates);
    assert_eq!((t.files, t.same, t.lines, t.matched), (2, 1, 120, 110));
    assert_eq!((t.compiled, t.mto_breaks, t.scrambler_bugs), (1, 1, 2));
    let table = render(77, &crates, &t);
    assert!(table.contains("1 of 2 (50.0%)"), "{}", table);
    assert!(table.contains("110 of 120 (91.67%)"), "{}", table);
    assert!(table.contains("yes, NO, yes"), "{}", table);
    assert!(table.contains("after mto: error[E0425]: cannot find value `x`"));
    assert!(table.contains("SCRAMBLER BUG"));
    assert!(table.contains("scrambler bug in src/y.rs: whitespace: token 3 differs"));
    assert!(table.contains("cargo xtask pairs --seed 77"));
    assert!(!table.contains("rustfmt"));
  }
}
