//! `cargo xtask mcp`: the tooling as an MCP server over stdio, for an agent to ask it
//! directly ("what fails for wrap_comments?") rather than read a terminal. JSON-RPC 2.0, one
//! message a line: `initialize`, `ping`, `tools/list` and `tools/call`. Each tool runs this
//! same xtask with its arguments and answers with what it printed: JSON for those that
//! have it (`next`, `gaps`, `checks`, `store`), text for the others (`probe`, `why`).

use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::common::json::{self, Json};
use crate::common::process;

/// The protocol version this server speaks.
const PROTOCOL: &str = "2025-06-18";

const TOOLS: &[Tool] = &[
  Tool {
    name: "next",
    about: "What to work on next, most urgent first, each with the command that acts on it",
    arguments: &[],
    command: |_| words(&["next", "--json"]),
  },
  Tool {
    name: "gaps",
    about: "The standard formatter's test pairs mto does not match yet, with their settings, \
            where they first differ, and why one is excused or its keys disagree",
    arguments: &[("setting", "Only the pairs that set this setting", false)],
    command: |args| {
      let mut all = words(&["gaps", "--json"]);
      if let Some(setting) = text(args, "setting") {
        all.extend(["--setting".to_string(), setting]);
      }
      all
    },
  },
  Tool {
    name: "checks",
    about: "Every check by its id: what it checks, and the test group and stages it runs in",
    arguments: &[],
    command: |_| words(&["checks", "--json"]),
  },
  Tool {
    name: "store",
    about: "What .dev/ holds, area by area, with what each keeps",
    arguments: &[],
    command: |_| words(&["store", "--json"]),
  },
  Tool {
    name: "probe",
    about: "One of the standard formatter's test pairs against every answer key",
    arguments: &[
      (
        "case",
        "The pair as the diff names it, e.g. tests/configs/wrap_comments/true.rs",
        true,
      ),
      (
        "show",
        "An answer key to show mto's output beside (tests@stable, run@nightly...)",
        false,
      ),
    ],
    command: |args| {
      let mut all = vec!["probe".to_string(), text(args, "case").unwrap_or_default()];
      if let Some(key) = text(args, "show") {
        all.extend(["--show".to_string(), key]);
      }
      all
    },
  },
  Tool {
    name: "why",
    about: "What differs between what produced two runs (tree, tools, machine, records)",
    arguments: &[
      ("run", "A run id, or its end", true),
      (
        "other",
        "The run to compare with (default: the one before of the same command)",
        false,
      ),
    ],
    command: |args| {
      let mut all = words(&["runs", "why"]);
      all.push(text(args, "run").unwrap_or_default());
      if let Some(other) = text(args, "other") {
        all.push(other);
      }
      all
    },
  },
];

/// One tool: its name, what it does, its arguments (name, what, required), and the xtask
/// command it runs.
struct Tool {
  name: &'static str,
  about: &'static str,
  arguments: &'static [(&'static str, &'static str, bool)],
  command: fn(&Json) -> Vec<String>,
}

pub fn run(root: &Path, _m: &clap::ArgMatches) -> ExitCode {
  let stdin = std::io::stdin();
  let stdout = std::io::stdout();
  serve(stdin.lock(), stdout.lock(), |tool, args| {
    call(root, tool, args)
  });
  ExitCode::SUCCESS
}

/// The reply to one message; none to a notification.
fn answer(message: &Json, call: &impl Fn(&Tool, &Json) -> (bool, String)) -> Option<Json> {
  let id = message.get("id").cloned()?;
  let method = message.get("method").and_then(Json::as_str).unwrap_or("");
  let params = message
    .get("params")
    .cloned()
    .unwrap_or(Json::Object(Vec::new()));
  let result = match method {
    "initialize" => Json::object(vec![
      ("protocolVersion", PROTOCOL.into()),
      (
        "capabilities",
        Json::object(vec![("tools", Json::Object(Vec::new()))]),
      ),
      (
        "serverInfo",
        Json::object(vec![
          ("name", "mtindo-xtask".into()),
          ("version", "1".into()),
        ]),
      ),
    ]),
    "ping" => Json::Object(Vec::new()),
    "tools/list" => Json::object(vec![(
      "tools",
      Json::List(TOOLS.iter().map(describe).collect()),
    )]),
    "tools/call" => {
      let name = params.get("name").and_then(Json::as_str).unwrap_or("");
      let Some(tool) = TOOLS.iter().find(|t| t.name == name) else {
        return Some(error(id, -32602, &format!("no tool `{}`", name)));
      };
      let args = params
        .get("arguments")
        .cloned()
        .unwrap_or(Json::Object(Vec::new()));
      if let Some((missing, _, _)) = tool
        .arguments
        .iter()
        .find(|(name, _, required)| *required && text(&args, name).is_none())
      {
        return Some(error(
          id,
          -32602,
          &format!("`{}` needs `{}`", tool.name, missing),
        ));
      }
      let (ok, printed) = call(tool, &args);
      Json::object(vec![
        (
          "content",
          Json::List(vec![Json::object(vec![
            ("type", "text".into()),
            ("text", printed.into()),
          ])]),
        ),
        ("isError", (!ok).into()),
      ])
    }
    other => return Some(error(id, -32601, &format!("no method `{}`", other))),
  };
  Some(Json::object(vec![
    ("jsonrpc", "2.0".into()),
    ("id", id),
    ("result", result),
  ]))
}

/// Runs this xtask with the tool's command: whether it passed, and what it printed.
fn call(root: &Path, tool: &Tool, args: &Json) -> (bool, String) {
  let Ok(exe) = std::env::current_exe() else {
    return (false, "cannot find this xtask's binary".to_string());
  };
  let done = process::output(
    Command::new(exe)
      .args((tool.command)(args))
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::LONG,
  );
  match done {
    Ok(done) => {
      let mut printed = String::from_utf8_lossy(&done.stdout).into_owned();
      if done.code != 0 {
        printed.push_str(&String::from_utf8_lossy(&done.stderr));
      }
      (done.code == 0, printed)
    }
    Err(e) => (false, format!("cannot run xtask: {}", e)),
  }
}

/// A tool as `tools/list` describes it: its name, what it does and its input schema.
fn describe(tool: &Tool) -> Json {
  let properties: Vec<(&str, Json)> = tool
    .arguments
    .iter()
    .map(|(name, about, _)| {
      (
        *name,
        Json::object(vec![
          ("type", "string".into()),
          ("description", (*about).into()),
        ]),
      )
    })
    .collect();
  let required: Vec<&str> = tool
    .arguments
    .iter()
    .filter(|(_, _, required)| *required)
    .map(|(name, _, _)| *name)
    .collect();
  Json::object(vec![
    ("name", tool.name.into()),
    ("description", tool.about.into()),
    (
      "inputSchema",
      Json::object(vec![
        ("type", "object".into()),
        ("properties", Json::object(properties)),
        ("required", required.into()),
      ]),
    ),
  ])
}

fn error(id: Json, code: i64, message: &str) -> Json {
  Json::object(vec![
    ("jsonrpc", "2.0".into()),
    ("id", id),
    (
      "error",
      Json::object(vec![
        ("code", Json::Number(code as f64)),
        ("message", message.into()),
      ]),
    ),
  ])
}

/// Answers each message of `input` on `output` until the input ends; `call` runs a tool
/// and says whether it succeeded, with what it printed.
fn serve(
  input: impl BufRead,
  mut output: impl Write,
  call: impl Fn(&Tool, &Json) -> (bool, String),
) {
  for line in input.lines().map_while(Result::ok) {
    if line.trim().is_empty() {
      continue;
    }
    let reply = match json::parse(&line) {
      Ok(message) => answer(&message, &call),
      Err(e) => Some(error(Json::Null, -32700, &format!("not JSON: {}", e))),
    };
    if let Some(reply) = reply {
      let _ = writeln!(output, "{}", reply.render());
      let _ = output.flush();
    }
  }
}

/// A string argument, when given and not empty.
fn text(args: &Json, name: &str) -> Option<String> {
  args
    .get(name)
    .and_then(Json::as_str)
    .filter(|s| !s.is_empty())
    .map(str::to_string)
}

fn words(list: &[&str]) -> Vec<String> {
  list.iter().map(|w| w.to_string()).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn session(lines: &[&str]) -> Vec<Json> {
    let input = lines.join("\n");
    let mut output = Vec::new();
    serve(input.as_bytes(), &mut output, |tool, args| {
      (true, (tool.command)(args).join(" "))
    });
    String::from_utf8(output)
      .unwrap()
      .lines()
      .map(|l| json::parse(l).unwrap())
      .collect()
  }

  #[test]
  fn test_a_session_lists_and_calls_the_tools() {
    let replies = session(&[
      r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
      r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
      r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
      r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"gaps","arguments":{"setting":"wrap_comments"}}}"#,
      r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"probe","arguments":{}}}"#,
      r#"{"jsonrpc":"2.0","id":5,"method":"nope"}"#,
      "not json",
    ]);
    // The notification has no reply.
    assert_eq!(replies.len(), 6);
    let result = |i: usize| replies[i].get("result").unwrap();
    assert_eq!(
      result(0).get("protocolVersion").and_then(Json::as_str),
      Some(PROTOCOL)
    );
    let Some(Json::List(tools)) = result(1).get("tools") else {
      panic!("no tools: {:?}", replies[1]);
    };
    assert_eq!(tools.len(), TOOLS.len());
    let called = result(2).get("content").unwrap().render();
    assert!(
      called.contains("gaps --json --setting wrap_comments"),
      "{}",
      called
    );
    let missing = replies[3].get("error").unwrap().render();
    assert!(missing.contains("`probe` needs `case`"), "{}", missing);
    assert!(replies[4].get("error").unwrap().render().contains("-32601"));
    assert!(replies[5].get("error").unwrap().render().contains("-32700"));
  }

  /// Every tool's command is one xtask takes.
  #[test]
  fn test_every_tool_runs_a_command() {
    let args = json::parse(r#"{"case":"tests/a.rs","run":"x","setting":"s"}"#).unwrap();
    for tool in TOOLS {
      let command = (tool.command)(&args);
      assert!(
        matches!(crate::cli::parse(&command), crate::cli::Parsed::Run(..)),
        "{}: {:?}",
        tool.name,
        command
      );
    }
  }
}
