//! Running child processes: captured for the test runner, or passed through for commands
//! that forward to another binary.
//!
//! Every captured child runs under a time limit. A child still running at its limit is
//! stopped with its whole process tree (SIGTERM, then SIGKILL after [`GRACE`]), and the
//! result says it timed out, so a hung tool fails a run instead of hanging it.

use std::env;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitCode, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long a stopped child has to exit after SIGTERM before it gets SIGKILL.
pub const GRACE: Duration = Duration::from_secs(5);

/// For whole-corpus runs, real-world crate builds and benchmarks.
pub const HEAVY: Duration = Duration::from_secs(3 * 60 * 60);

/// For builds, test suites and other long commands.
pub const LONG: Duration = Duration::from_secs(60 * 60);

/// For clones, fetches and downloads.
pub const NETWORK: Duration = Duration::from_secs(15 * 60);

/// For quick queries: `git rev-parse`, `--version` checks, listings.
pub const QUICK: Duration = Duration::from_secs(60);

/// The exit code reported for a child stopped at its time limit, as `timeout(1)` reports.
pub const TIMED_OUT: i32 = 124;

/// How long the output is still read after the child exits, for grandchildren that keep
/// the pipe open.
const DRAIN: Duration = Duration::from_secs(2);

/// What a captured child did.
#[derive(Debug)]
pub struct Outcome {
  pub code: i32,
  /// Stdout and stderr interleaved, as a terminal shows them.
  pub output: String,
  pub timed_out: bool,
}

/// What [`output`] collected: stdout and stderr apart, unlike [`capture`].
#[derive(Debug)]
pub struct Separate {
  /// The exit code, as [`exit_code`] gives it; [`TIMED_OUT`] for a child stopped at its
  /// limit.
  pub code: i32,
  pub stdout: Vec<u8>,
  pub stderr: Vec<u8>,
  pub timed_out: bool,
}

/// Runs `program` in `root` with extra environment variables, for at most `limit`. Returns
/// the exit code and the output; a child stopped at its limit gives [`TIMED_OUT`] and a
/// last line saying so.
pub fn capture(
  root: &Path,
  program: &str,
  args: &[String],
  env_extra: &[(String, String)],
  limit: Duration,
) -> (i32, String) {
  let outcome = run(root, program, args, env_extra, limit);
  (outcome.code, outcome.output)
}

/// `90s` as `1m30s`, for messages.
pub fn duration(limit: Duration) -> String {
  let seconds = limit.as_secs();
  match (seconds / 3600, seconds / 60 % 60, seconds % 60) {
    (0, 0, s) => format!("{}s", s),
    (0, m, 0) => format!("{}m", m),
    (0, m, s) => format!("{}m{}s", m, s),
    (h, 0, _) => format!("{}h", h),
    (h, m, _) => format!("{}h{}m", h, m),
  }
}

/// The exit code of a finished process; a process killed by signal N gives 128 + N, as in
/// a shell.
pub fn exit_code(status: ExitStatus) -> i32 {
  if let Some(code) = status.code() {
    return code;
  }
  #[cfg(unix)]
  {
    use std::os::unix::process::ExitStatusExt;
    if let Some(signal) = status.signal() {
      return 128 + signal;
    }
  }
  -1
}

/// Runs an `xtask-tools` command with this terminal's stdin, stdout and stderr. The one
/// child without a time limit: it is interactive, and the person at the terminal sees it
/// and can stop it.
pub fn forward_to_tools(root: &Path, args: &[String]) -> ExitCode {
  let status = Command::new("cargo")
    .args(["run", "--quiet", "--package", "xtask-tools", "--"])
    .args(args)
    .current_dir(root)
    .status();
  match status {
    Ok(status) => ExitCode::from(exit_code(status).clamp(0, 255) as u8),
    Err(e) => {
      eprintln!("cannot run cargo: {}", e);
      ExitCode::from(1)
    }
  }
}

/// True if an executable named `tool` is on `PATH`.
pub fn on_path(tool: &str) -> bool {
  let Some(paths) = env::var_os("PATH") else {
    return false;
  };
  env::split_paths(&paths).any(|dir| {
    let candidate = dir.join(tool);
    candidate.is_file() || candidate.with_extension("exe").is_file()
  })
}

/// `Command::output` under a time limit: runs `command` with a null stdin, and stops it and
/// its process tree at `limit`. Only streams the caller set to `Stdio::piped()` are
/// collected; the others go where they were set (inherited, as `spawn` does, by default).
pub fn output(command: &mut Command, limit: Duration) -> std::io::Result<Separate> {
  output_with_input(command, None, limit)
}

/// [`output`], writing `input` to the child's stdin (null when `None`).
pub fn output_with_input(
  command: &mut Command,
  input: Option<&[u8]>,
  limit: Duration,
) -> std::io::Result<Separate> {
  command.stdin(if input.is_some() {
    Stdio::piped()
  } else {
    Stdio::null()
  });
  let mut child = command.spawn()?;
  if let (Some(mut stdin), Some(input)) = (child.stdin.take(), input) {
    let bytes = input.to_vec();
    // On its own thread: a child that writes before reading all its input would otherwise
    // block us both.
    thread::spawn(move || {
      use std::io::Write;
      let _ = stdin.write_all(&bytes);
    });
  }
  let read = |pipe: Option<Box<dyn Read + Send>>| {
    thread::spawn(move || {
      let mut bytes = Vec::new();
      if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes);
      }
      bytes
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
  let (status, timed_out) = wait(&mut child, limit);
  let status = status?;
  let collect = |handle: thread::JoinHandle<Vec<u8>>| {
    let drained = Instant::now();
    while !handle.is_finished() && drained.elapsed() < DRAIN {
      thread::sleep(Duration::from_millis(10));
    }
    if handle.is_finished() {
      handle.join().unwrap_or_default()
    } else {
      Vec::new()
    }
  };
  Ok(Separate {
    code: if timed_out {
      TIMED_OUT
    } else {
      exit_code(status)
    },
    stdout: collect(stdout),
    stderr: collect(stderr),
    timed_out,
  })
}

/// [`capture`], saying whether the child timed out.
pub fn run(
  root: &Path,
  program: &str,
  args: &[String],
  env_extra: &[(String, String)],
  limit: Duration,
) -> Outcome {
  let failed = |output: String| Outcome {
    code: -1,
    output,
    timed_out: false,
  };
  let (mut reader, writer) = match std::io::pipe() {
    Ok(pair) => pair,
    Err(e) => return failed(format!("cannot create a pipe: {}", e)),
  };
  let writer_err = match writer.try_clone() {
    Ok(clone) => clone,
    Err(e) => return failed(format!("cannot create a pipe: {}", e)),
  };
  let mut command = Command::new(program);
  command
    .args(args)
    .current_dir(root)
    .stdin(Stdio::null())
    .stdout(writer)
    .stderr(writer_err);
  for (key, value) in env_extra {
    command.env(key, value);
  }
  let mut child = match command.spawn() {
    Ok(child) => child,
    Err(e) => return failed(format!("cannot run {}: {}", program, e)),
  };
  // The parent's copies of the pipe's write end live in `command`; drop them so the read
  // ends when the child and everything it started have exited.
  drop(command);
  let bytes = Arc::new(Mutex::new(Vec::new()));
  let sink = Arc::clone(&bytes);
  let reading = thread::spawn(move || {
    let mut chunk = [0u8; 8192];
    while let Ok(n) = reader.read(&mut chunk) {
      if n == 0 {
        break;
      }
      sink.lock().unwrap().extend_from_slice(&chunk[..n]);
    }
  });
  let (status, timed_out) = wait(&mut child, limit);
  // A grandchild may hold the pipe open after the child exits; read what arrives shortly
  // after, then go on without it.
  let drained = Instant::now();
  while !reading.is_finished() && drained.elapsed() < DRAIN {
    thread::sleep(Duration::from_millis(10));
  }
  let mut output = String::from_utf8_lossy(&bytes.lock().unwrap()).into_owned();
  let code = match status {
    _ if timed_out => {
      if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
      }
      output.push_str(&format!(
        "{} was stopped at its time limit of {}\n",
        program,
        duration(limit)
      ));
      TIMED_OUT
    }
    Ok(status) => exit_code(status),
    Err(e) => return failed(format!("cannot wait for {}: {}", program, e)),
  };
  Outcome {
    code,
    output,
    timed_out,
  }
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
  Command::new("kill")
    .args(["-0", &pid.to_string()])
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|s| s.success())
}

/// `pid` and every process below it, parents before children.
#[cfg(unix)]
fn descendants(pid: u32) -> Vec<u32> {
  let mut tree = vec![pid];
  let mut idx = 0;
  while idx < tree.len() {
    if let Ok(out) = Command::new("pgrep")
      .args(["-P", &tree[idx].to_string()])
      .stderr(Stdio::null())
      .output()
    {
      tree.extend(
        String::from_utf8_lossy(&out.stdout)
          .lines()
          .filter_map(|l| l.trim().parse::<u32>().ok()),
      );
    }
    idx += 1;
  }
  tree
}

#[cfg(unix)]
fn signal(pids: &[u32], name: &str) {
  let mut args = vec![format!("-{}", name)];
  args.extend(pids.iter().map(u32::to_string));
  let _ = Command::new("kill")
    .args(&args)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status();
}

/// Stops a child and every process it started: SIGTERM to all, then SIGKILL to whatever
/// is left after [`GRACE`]. Elsewhere than Unix, `taskkill /T`.
fn stop_tree(child: &mut Child) {
  #[cfg(unix)]
  {
    let tree = descendants(child.id());
    signal(&tree, "TERM");
    let started = Instant::now();
    while started.elapsed() < GRACE {
      if matches!(child.try_wait(), Ok(Some(_))) && tree.iter().all(|pid| !alive(*pid)) {
        return;
      }
      thread::sleep(Duration::from_millis(50));
    }
    signal(&tree, "KILL");
  }
  #[cfg(not(unix))]
  {
    let _ = Command::new("taskkill")
      .args(["/T", "/F", "/PID", &child.id().to_string()])
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status();
  }
  let _ = child.kill();
}

/// Waits for `child` up to `limit`; past it, stops the child and its process tree.
fn wait(child: &mut Child, limit: Duration) -> (std::io::Result<ExitStatus>, bool) {
  let started = Instant::now();
  let mut pause = Duration::from_millis(5);
  loop {
    match child.try_wait() {
      Ok(Some(status)) => return (Ok(status), false),
      Ok(None) => {}
      Err(e) => return (Err(e), false),
    }
    if started.elapsed() >= limit {
      stop_tree(child);
      return (child.wait(), true);
    }
    thread::sleep(pause);
    pause = (pause * 2).min(Duration::from_millis(200));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
  }

  fn sh(script: &str) -> Vec<String> {
    vec!["-c".to_string(), script.to_string()]
  }

  #[test]
  fn test_a_child_past_its_limit_is_stopped_with_its_children() {
    let marker = std::env::temp_dir().join(format!("xtask-timeout-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    // The grandchild would write the marker after 3 s if it survived its parent's stop.
    let script = format!(
      "echo started; (sleep 3; touch {}) & sleep 30",
      marker.display()
    );
    let started = Instant::now();
    let outcome = run(root(), "sh", &sh(&script), &[], Duration::from_millis(300));
    assert!(outcome.timed_out);
    assert_eq!(outcome.code, TIMED_OUT);
    assert!(
      outcome.output.starts_with("started\n"),
      "{}",
      outcome.output
    );
    assert!(
      outcome
        .output
        .ends_with("sh was stopped at its time limit of 0s\n")
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    thread::sleep(Duration::from_secs(4));
    assert!(!marker.exists(), "the grandchild outlived the timeout");
  }

  #[test]
  fn test_a_lingering_grandchild_does_not_hold_the_result() {
    let started = Instant::now();
    let outcome = run(root(), "sh", &sh("echo done; sleep 20 &"), &[], QUICK);
    assert_eq!((outcome.code, outcome.timed_out), (0, false));
    assert_eq!(outcome.output, "done\n");
    assert!(started.elapsed() < Duration::from_secs(10));
  }

  #[test]
  fn test_capture_interleaves_stdout_and_stderr() {
    let script = "echo one; echo two >&2; echo three; exit 3";
    let (code, output) = capture(root(), "sh", &sh(script), &[], QUICK);
    assert_eq!(code, 3);
    assert_eq!(output, "one\ntwo\nthree\n");
  }

  #[test]
  fn test_capture_passes_environment_and_reports_missing_programs() {
    let env = [("XTASK_PROBE".to_string(), "42".to_string())];
    let args = sh("echo $XTASK_PROBE");
    assert_eq!(
      capture(root(), "sh", &args, &env, QUICK),
      (0, "42\n".to_string())
    );
    let (code, output) = capture(root(), "no-such-program-xtask", &[], &[], QUICK);
    assert_eq!(code, -1);
    assert!(output.starts_with("cannot run no-such-program-xtask"));
  }

  /// Every child xtask starts runs under a time limit: only this module spawns or waits on
  /// one, apart from the keep-awake guard, which runs as long as it lives.
  #[test]
  fn test_children_are_started_only_here() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let allowed = [src.join("common/process.rs"), src.join("common/awake.rs")];
    let mut files = vec![src.clone()];
    let mut found = Vec::new();
    while let Some(path) = files.pop() {
      if path.is_dir() {
        files.extend(
          std::fs::read_dir(&path)
            .unwrap()
            .flatten()
            .map(|e| e.path()),
        );
        continue;
      }
      if path.extension().is_none_or(|e| e != "rs") || allowed.contains(&path) {
        continue;
      }
      let text = std::fs::read_to_string(&path).unwrap();
      for (n, line) in text.lines().enumerate() {
        if [".spawn()", ".status()", ".output()", "wait_with_output"]
          .iter()
          .any(|call| line.contains(call))
        {
          found.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
        }
      }
    }
    assert!(
      found.is_empty(),
      "start children with process::run, capture or output, which take a time limit:\n{}",
      found.join("\n")
    );
  }

  #[test]
  fn test_duration() {
    assert_eq!(duration(Duration::from_secs(45)), "45s");
    assert_eq!(duration(Duration::from_secs(600)), "10m");
    assert_eq!(duration(Duration::from_secs(90)), "1m30s");
    assert_eq!(duration(Duration::from_secs(7200)), "2h");
    assert_eq!(duration(Duration::from_secs(5400)), "1h30m");
  }

  #[test]
  fn test_on_path() {
    assert!(on_path("sh"));
    assert!(!on_path("no-such-program-xtask"));
  }

  #[test]
  fn test_output_keeps_streams_apart_and_honors_the_limit() {
    let mut command = Command::new("sh");
    command
      .args(sh("echo out; echo err >&2; exit 2"))
      .stdout(Stdio::piped())
      .stderr(Stdio::piped());
    let done = output(&mut command, QUICK).unwrap();
    assert_eq!((done.code, done.timed_out), (2, false));
    assert_eq!(done.stdout, b"out\n");
    assert_eq!(done.stderr, b"err\n");
    let mut command = Command::new("sh");
    command.args(sh("sleep 30")).stdout(Stdio::null());
    let stopped = output(&mut command, Duration::from_millis(200)).unwrap();
    assert_eq!((stopped.code, stopped.timed_out), (TIMED_OUT, true));
  }
}
