//! memst: where mto's memory and time go. See `cargo xtask memst`, which builds this with
//! frame pointers forced on and runs it.
//!
//! - `memst run --out FILE [--config-path FILE] PATH...`: runs mto's pipeline on PATH as
//!   `mto --dry-run` does (nothing is written or printed), recording every allocation and sampling CPU
//!   time, and writes the profile to FILE.
//! - `memst report FILE [--view NAME|all] [--by bytes|allocs|time] [--top N]`: the views
//!   (overview, functions, hot, cold, tree, sites).

mod profile;
mod record;
mod report;
mod walk;

use std::path::PathBuf;
use std::process::ExitCode;

use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::{RunOptions, Writes, collect_jobs, run_jobs};

const USAGE: &str = "usage: memst run --out FILE [--config-path FILE] PATH...\n       memst report FILE [--view overview|functions|hot|cold|tree|sites|all] [--by bytes|allocs|time] [--top N]";

#[global_allocator]
static ALLOCATOR: record::Counting = record::Counting;

/// Stdout sent to the null device until dropped.
struct Silence(libc::c_int);

impl Silence {
  fn stdout() -> Silence {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    // SAFETY: duplicates and replaces this process's own descriptor 1; restored on drop.
    unsafe {
      let saved = libc::dup(1);
      let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
      if null >= 0 {
        libc::dup2(null, 1);
        libc::close(null);
      }
      Silence(saved)
    }
  }
}

impl Drop for Silence {
  fn drop(&mut self) {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    if self.0 >= 0 {
      // SAFETY: puts back the descriptor saved above.
      unsafe {
        libc::dup2(self.0, 1);
        libc::close(self.0);
      }
    }
  }
}

pub fn anchor_address() -> usize {
  memst_anchor as *const () as usize
}

/// A fixed point in this binary: where it lands in memory gives the load offset that turns
/// the symbol table's addresses into the profile's.
#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn memst_anchor() {}

fn main() -> ExitCode {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let result = match args.first().map(String::as_str) {
    Some("run") => run(&args[1..]),
    Some("report") => report(&args[1..]),
    _ => Err(USAGE.to_string()),
  };
  match result {
    Ok(()) => ExitCode::SUCCESS,
    Err(e) => {
      eprintln!("memst: {}", e);
      ExitCode::from(1)
    }
  }
}

fn report(args: &[String]) -> Result<(), String> {
  let file = args.first().ok_or(USAGE)?;
  let mut view = "all".to_string();
  let mut by = report::By::Bytes;
  let mut top = 15;
  let mut idx = 1;
  while idx < args.len() {
    let value = args.get(idx + 1).ok_or(USAGE)?;
    match args[idx].as_str() {
      "--view" if value == "all" || report::VIEWS.contains(&value.as_str()) => view = value.clone(),
      "--by" => by = report::By::parse(value).ok_or(USAGE)?,
      "--top" => top = value.parse().map_err(|_| USAGE)?,
      _ => return Err(USAGE.to_string()),
    }
    idx += 2;
  }
  let text = std::fs::read_to_string(file).map_err(|e| format!("cannot read {}: {}", file, e))?;
  let profile = profile::Profile::from_text(&text)?;
  let symbols = report::Symbols::load(&profile.exe, profile.anchor)?;
  let named = report::Named::new(&profile, &symbols);
  let views: Vec<&str> = if view == "all" {
    report::VIEWS.to_vec()
  } else {
    vec![view.as_str()]
  };
  for (idx, view) in views.iter().enumerate() {
    if idx > 0 {
      println!();
    }
    let text = match *view {
      "overview" => report::overview(&named),
      "functions" => report::functions_view(&named, by, top),
      "hot" => report::hot(&named, top),
      "cold" => report::cold(&named, &symbols, top),
      "tree" => report::tree(&named, by, 30),
      _ => report::sites(&named, top),
    };
    print!("{}", text);
  }
  Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
  let mut out = None;
  let mut overrides = Overrides::default();
  let mut paths = Vec::new();
  let mut idx = 0;
  while idx < args.len() {
    match args[idx].as_str() {
      "--out" => {
        idx += 1;
        out = args.get(idx).map(PathBuf::from);
      }
      "--config-path" => {
        idx += 1;
        overrides.config_path = args.get(idx).map(PathBuf::from);
      }
      path => paths.push(PathBuf::from(path)),
    }
    idx += 1;
  }
  let out = out.ok_or(USAGE)?;
  if paths.is_empty() {
    return Err(USAGE.to_string());
  }
  // As `mto --dry-run`: every file read, laid out, formatted and verified, nothing written
  // or printed.
  let opts = RunOptions {
    dry_run: true,
    quiet: true,
    no_history: true,
    ..RunOptions::default()
  };
  let mut resolver =
    Resolver::new(&overrides, Environment::from_process()).map_err(|e| e.to_string())?;
  let jobs = collect_jobs(&paths, &mut resolver, &opts)
    .map_err(|e| e.to_string())?
    .jobs;
  // `--dry-run` names every file it would change; nothing of that is wanted here.
  let quiet = Silence::stdout();
  // Time first, unslowed; then the same work again with every allocation recorded.
  record::start(true, false);
  run_jobs(&jobs, &opts, &mut Writes::default());
  let time = record::finish();
  record::start(false, true);
  let done = run_jobs(&jobs, &opts, &mut Writes::default());
  let mut profile = record::finish();
  drop(quiet);
  profile.absorb_time(time);
  profile.exe = std::env::current_exe()
    .map(|p| p.display().to_string())
    .unwrap_or_default();
  std::fs::write(&out, profile.to_text())
    .map_err(|e| format!("cannot write {}: {}", out.display(), e))?;
  eprintln!(
    "profiled {} file(s): {} unrecorded, {} recording memory: {} allocations, peak live {}; \
     profile in {}",
    done.len(),
    report::nanos(profile.time_elapsed_ns),
    report::nanos(profile.elapsed_ns),
    report::count(profile.allocs),
    report::bytes(profile.peak_live),
    out.display()
  );
  Ok(())
}
