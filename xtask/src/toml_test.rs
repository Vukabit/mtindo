//! `cargo xtask toml-test`: runs the shared TOML conformance suite (`toml-test`, pinned in
//! `compat/toml-test.txt`) against mto's TOML parser. The suite is checked out into
//! `compat/cache/toml-test/` and run by `xtask-tools`, which links mtindo.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::common::log::Logger;
use crate::common::process;
use crate::corpus::{checkout_into, parse_pins};

const PINS_PATH: &str = "compat/toml-test.txt";

pub fn run(root: &Path, args: &[String]) -> ExitCode {
  let log = Logger::new(root, "toml-test");
  if !args.is_empty() {
    eprintln!("Usage: cargo xtask toml-test");
    return ExitCode::from(2);
  }
  let pins = match fs::read_to_string(root.join(PINS_PATH))
    .map_err(|e| format!("cannot read {}: {}", PINS_PATH, e))
    .and_then(|text| parse_pins(&text))
  {
    Ok(pins) => pins,
    Err(e) => {
      println!("{}", e);
      return ExitCode::from(1);
    }
  };
  let Some(pin) = pins.first() else {
    println!("{} pins nothing", PINS_PATH);
    return ExitCode::from(1);
  };
  let base = root.join("compat").join("cache").join("toml-test");
  match checkout_into(&base, pin, &log) {
    Ok(what) => log.info(&format!("toml-test {} at {}", what, pin.commit)),
    Err(e) => {
      println!("cannot fetch toml-test: {}", e);
      log.error(&e);
      return ExitCode::from(1);
    }
  }
  let dir = base.join(&pin.name).display().to_string();
  process::forward_to_tools(root, &["toml-test".to_string(), dir])
}
