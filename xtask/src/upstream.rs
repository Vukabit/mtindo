//! `cargo xtask upstream`: the standard formatter's bugs kept as cases in
//! `.dev/upstream/rustfmt/`, listed, rechecked against each toolchain, and drafted as issue
//! text for filing by hand. Runs in `xtask-tools`, which links mtindo for the meaning check.

use std::path::Path;
use std::process::ExitCode;

use crate::common::store;

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let dir = store::dir(root, store::UPSTREAM).join("rustfmt");
  let mut all = vec![
    "upstream".to_string(),
    "--dir".to_string(),
    dir.display().to_string(),
  ];
  match m.subcommand() {
    None | Some(("list", _)) => all.push("list".to_string()),
    Some(("recheck", sub)) => {
      all.push("recheck".to_string());
      all.extend(crate::cli::texts(sub, "toolchains"));
    }
    Some(("draft", sub)) => {
      all.push("draft".to_string());
      all.push(crate::cli::text(sub, "case").unwrap_or_default());
    }
    Some((other, _)) => unreachable!("upstream {} is in the tree but not here", other),
  }
  crate::common::process::forward_to_tools(root, &all)
}
