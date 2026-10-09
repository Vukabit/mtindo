//! Running child processes: captured for the test runner, or passed through for commands
//! that forward to another binary.

use std::env;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitCode, ExitStatus, Stdio};

/// Runs `program` in `root` with extra environment variables. Returns the exit code and
/// stdout and stderr interleaved in the order they were written, as a terminal shows them.
pub fn capture(
    root: &Path,
    program: &str,
    args: &[String],
    env_extra: &[(String, String)],
) -> (i32, String) {
    let (mut reader, writer) = match std::io::pipe() {
        Ok(pair) => pair,
        Err(e) => return (-1, format!("cannot create a pipe: {}", e)),
    };
    let writer_err = match writer.try_clone() {
        Ok(clone) => clone,
        Err(e) => return (-1, format!("cannot create a pipe: {}", e)),
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
        Err(e) => return (-1, format!("cannot run {}: {}", program, e)),
    };
    // The parent's copies of the pipe's write end live in `command`; drop them so the read
    // below ends when the child exits.
    drop(command);
    let mut bytes = Vec::new();
    let _ = reader.read_to_end(&mut bytes);
    let code = match child.wait() {
        Ok(status) => exit_code(status),
        Err(e) => return (-1, format!("cannot wait for {}: {}", program, e)),
    };
    (code, String::from_utf8_lossy(&bytes).into_owned())
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

/// Runs an `xtask-tools` command with this terminal's stdin, stdout and stderr.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_interleaves_stdout_and_stderr() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let script = "echo one; echo two >&2; echo three; exit 3".to_string();
        let (code, output) = capture(root, "sh", &["-c".to_string(), script], &[]);
        assert_eq!(code, 3);
        assert_eq!(output, "one\ntwo\nthree\n");
    }

    #[test]
    fn test_capture_passes_environment_and_reports_missing_programs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let env = [("XTASK_PROBE".to_string(), "42".to_string())];
        let args = ["-c".to_string(), "echo $XTASK_PROBE".to_string()];
        assert_eq!(capture(root, "sh", &args, &env), (0, "42\n".to_string()));
        let (code, output) = capture(root, "no-such-program-xtask", &[], &[]);
        assert_eq!(code, -1);
        assert!(output.starts_with("cannot run no-such-program-xtask"));
    }

    #[test]
    fn test_on_path() {
        assert!(on_path("sh"));
        assert!(!on_path("no-such-program-xtask"));
    }
}
