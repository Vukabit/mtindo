//! Running the build guard's command and reading what it says.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// The command the guard runs, where, and for how long at most.
#[derive(Clone, Debug)]
pub struct Check {
  pub words: Vec<String>,
  /// The project's folder.
  pub dir: PathBuf,
  /// `None`: no limit.
  pub timeout: Option<Duration>,
}

impl Check {
  /// The command line, as words joined by spaces.
  pub fn line(&self) -> String {
    self.words.join(" ")
  }

  /// Runs the command once and reads what it says.
  pub fn run(&self) -> Said {
    let started = Instant::now();
    let Some((program, args)) = self.words.split_first() else {
      return Said {
        stopped: Some("the command is empty".into()),
        ..Said::default()
      };
    };
    let child = Command::new(program)
      .args(args)
      .current_dir(&self.dir)
      // Errors are read as text, so they come without color.
      .env("CARGO_TERM_COLOR", "never")
      // What `cargo mto` told this run is not for the command.
      .env_remove("MTINDO_COMMAND")
      .env_remove("MTINDO_PATHS_FILE")
      .stdin(Stdio::null())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn();
    let mut child = match child {
      Ok(child) => child,
      Err(e) => {
        return Said {
          stopped: Some(format!("`{}` could not start: {}", self.line(), e)),
          took: started.elapsed(),
          ..Said::default()
        };
      }
    };
    let read = |pipe: Option<Box<dyn Read + Send>>| {
      thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut pipe) = pipe {
          let mut bytes = Vec::new();
          let _ = pipe.read_to_end(&mut bytes);
          text = String::from_utf8_lossy(&bytes).into_owned();
        }
        text
      })
    };
    let stdout = read(
      child
        .stdout
        .take()
        .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = read(
      child
        .stderr
        .take()
        .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let mut stopped = None;
    let status = loop {
      match child.try_wait() {
        Ok(Some(status)) => break Some(status),
        Ok(None) if self.timeout.is_some_and(|t| started.elapsed() > t) => {
          stop_tree(&mut child);
          stopped = Some(format!(
            "`{}` ran past its {} s and was stopped",
            self.line(),
            self.timeout.map_or(0, |t| t.as_secs())
          ));
          break None;
        }
        Ok(None) => thread::sleep(Duration::from_millis(20)),
        Err(e) => {
          stopped = Some(format!("`{}` could not be waited for: {}", self.line(), e));
          break None;
        }
      }
    };
    let text = format!(
      "{}\n{}",
      stdout.join().unwrap_or_default(),
      stderr.join().unwrap_or_default()
    );
    let (errors, first) = errors_in(&text);
    Said {
      passed: status.is_some_and(|s| s.success()),
      errors,
      first,
      stopped,
      took: started.elapsed(),
    }
  }
}

/// What one run of the command said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Said {
  pub passed: bool,
  /// Its errors, each as the compiler names it (`error[E0425]: cannot find value `x` in
  /// this scope`), without where.
  pub errors: BTreeSet<String>,
  /// The first error with the lines that show where, as printed.
  pub first: Option<String>,
  /// Why it did not finish, when it did not: it could not start, or ran out of time.
  pub stopped: Option<String>,
  pub took: Duration,
}

impl Said {
  /// Whether the command speaks the compiler's way: when it fails, its errors are known.
  pub fn readable(&self) -> bool {
    self.passed || !self.errors.is_empty()
  }

  /// Whether this run, against `before`, shows a broken build: it passed before and fails
  /// now, or fails now with an error it did not have.
  pub fn broke(&self, before: &Said) -> bool {
    if self.passed {
      return false;
    }
    before.passed || self.errors.difference(&before.errors).next().is_some()
  }

  /// The errors it has that `before` did not.
  pub fn new_errors<'a>(&'a self, before: &'a Said) -> impl Iterator<Item = &'a String> {
    self.errors.difference(&before.errors)
  }

  /// One line for the run's record and for the terminal: `pass`, or `fail` with how many
  /// errors.
  pub fn summary(&self) -> String {
    if self.passed {
      return "pass".into();
    }
    match (&self.stopped, self.errors.len()) {
      (Some(why), _) => format!("fail ({})", why),
      (None, 0) => "fail".into(),
      (None, 1) => "fail, 1 error".into(),
      (None, n) => format!("fail, {} errors", n),
    }
  }
}

/// The errors in a compiler's output, each line `error[CODE]: message` or `error: message`
/// once, without the summaries that only count them; and the first error as printed, up to
/// the blank line after it.
pub fn errors_in(text: &str) -> (BTreeSet<String>, Option<String>) {
  let mut errors = BTreeSet::new();
  let mut first: Option<String> = None;
  let mut taking = false;
  for line in text.lines() {
    let is_error = line.starts_with("error[") || line.starts_with("error: ");
    let summary = line.starts_with("error: could not compile")
      || line.starts_with("error: aborting due to")
      || line.starts_with("error: build failed");
    if is_error && !summary {
      errors.insert(line.trim_end().to_string());
      if first.is_none() {
        first = Some(String::new());
        taking = true;
      }
    }
    if taking {
      if line.trim().is_empty() || (summary && first.as_ref().is_some_and(|f| !f.is_empty())) {
        taking = false;
        continue;
      }
      let first = first.as_mut().expect("set above");
      first.push_str(line.trim_end());
      first.push('\n');
    }
  }
  (errors, first)
}

/// Stops `child` and every process it started: cargo's compilers hold the output pipes
/// open, so stopping cargo alone would leave them running and the reading waiting.
fn stop_tree(child: &mut std::process::Child) {
  let quietly = |program: &str, args: &[String]| {
    let _ = Command::new(program)
      .args(args)
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status();
  };
  if cfg!(windows) {
    quietly(
      "taskkill",
      &[
        "/T".into(),
        "/F".into(),
        "/PID".into(),
        child.id().to_string(),
      ],
    );
  } else {
    // Every descendant, found before any is stopped: once a parent goes, its children
    // are another's.
    let mut tree = vec![child.id()];
    if let Ok(out) = Command::new("ps")
      .args(["-A", "-o", "pid=", "-o", "ppid="])
      .stderr(Stdio::null())
      .output()
    {
      let pairs: Vec<(u32, u32)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
          let mut words = line.split_whitespace().map(|w| w.parse::<u32>().ok());
          Some((words.next()??, words.next()??))
        })
        .collect();
      let mut at = 0;
      while at < tree.len() {
        let parent = tree[at];
        tree.extend(pairs.iter().filter(|p| p.1 == parent).map(|p| p.0));
        at += 1;
      }
    }
    let mut args = vec!["-KILL".to_string()];
    args.extend(tree.iter().map(u32::to_string));
    quietly("kill", &args);
  }
  let _ = child.kill();
  let _ = child.wait();
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_break_is_a_failure_or_a_new_error() {
    let said = |passed: bool, errors: &[&str]| Said {
      passed,
      errors: errors.iter().map(|e| e.to_string()).collect(),
      ..Said::default()
    };
    let green = said(true, &[]);
    let red = said(false, &["error: a"]);
    assert!(red.broke(&green));
    assert!(!green.broke(&red));
    assert!(!said(false, &["error: a"]).broke(&red));
    assert!(said(false, &["error: a", "error: b"]).broke(&red));
    assert!(!said(false, &[]).readable());
    assert_eq!(
      said(false, &["error: a", "error: b"]).summary(),
      "fail, 2 errors"
    );
  }

  #[test]
  fn test_errors_are_read_the_compilers_way() {
    let text = "   Compiling a v0.1.0\n\
                error[E0425]: cannot find value `x` in this scope\n \
                --> src/lib.rs:3:5\n  |\n3 |     x\n  |     ^ not found\n\n\
                error: unused import\n\
                error: could not compile `a` (lib) due to 2 previous errors\n";
    let (errors, first) = errors_in(text);
    assert_eq!(
      errors.into_iter().collect::<Vec<_>>(),
      [
        "error: unused import",
        "error[E0425]: cannot find value `x` in this scope"
      ]
    );
    let first = first.unwrap();
    assert!(first.starts_with("error[E0425]"));
    assert!(first.contains("--> src/lib.rs:3:5"));
    assert!(!first.contains("unused import"));
  }

  /// What [`test_the_command_runs_in_its_folder_and_is_stopped_in_time`] checks, with the
  /// commands Windows has.
  #[cfg(windows)]
  #[test]
  fn test_the_command_runs_and_is_stopped_in_time_on_windows() {
    let dir = std::env::temp_dir();
    let check = |words: &[&str], timeout: Option<Duration>| Check {
      words: words.iter().map(|w| w.to_string()).collect(),
      dir: dir.clone(),
      timeout,
    };
    let said = check(
      &[
        "cmd",
        "/C",
        "echo",
        "error[E0425]:",
        "nope",
        "1>&2",
        "&",
        "exit",
        "1",
      ],
      None,
    )
    .run();
    assert!(!said.passed);
    assert_eq!(said.errors.len(), 1, "{:?}", said);
    assert!(check(&["cmd", "/C", "exit", "0"], None).run().passed);
    // `ping` is started by `cmd` and holds its output open: both are stopped in time.
    let started = Instant::now();
    let slow = check(
      &["cmd", "/C", "ping", "-n", "30", "127.0.0.1"],
      Some(Duration::from_millis(300)),
    )
    .run();
    assert!(!slow.passed);
    assert!(slow.stopped.unwrap().contains("ran past"));
    assert!(
      started.elapsed() < Duration::from_secs(10),
      "{:?}",
      started.elapsed()
    );
    let missing = check(&["mto-no-such-program"], None).run();
    assert!(missing.stopped.unwrap().contains("could not start"));
  }

  #[cfg(unix)]
  #[test]
  fn test_the_command_runs_in_its_folder_and_is_stopped_in_time() {
    let dir = std::env::temp_dir();
    let check = |words: &[&str], timeout: Option<Duration>| Check {
      words: words.iter().map(|w| w.to_string()).collect(),
      dir: dir.clone(),
      timeout,
    };
    let said = check(&["sh", "-c", "echo 'error: nope' >&2; exit 1"], None).run();
    assert!(!said.passed);
    assert_eq!(said.errors.len(), 1);
    assert!(check(&["true"], None).run().passed);
    let slow = check(&["sleep", "5"], Some(Duration::from_millis(100))).run();
    assert!(!slow.passed);
    assert!(slow.stopped.unwrap().contains("ran past"));
    let missing = check(&["mto-no-such-program"], None).run();
    assert!(missing.stopped.unwrap().contains("could not start"));
    // What the command started is stopped with it, so its output ends in time.
    let started = Instant::now();
    let tree = check(
      &["sh", "-c", "sleep 30 & sleep 30; wait"],
      Some(Duration::from_millis(200)),
    )
    .run();
    assert!(tree.stopped.unwrap().contains("ran past"));
    assert!(
      started.elapsed() < Duration::from_secs(10),
      "{:?}",
      started.elapsed()
    );
  }
}
