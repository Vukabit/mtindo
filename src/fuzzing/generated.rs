//! Source built from the bytes by [`super::generate`] must parse, and then pass every
//! check the other targets make of what mto does to a file: layout, formatting, and the
//! two together as a run with both does them (each step's meaning check clean, and a second
//! run changing nothing). The first three bytes pick the formatting settings, as the
//! `format` target's do; the rest choose the source.

use super::generate::source;
use crate::doctor::Held;
use crate::options::format::FormatConfig;
use crate::syntax::{Edition, parse};
use crate::verify::format::verify_format;
use crate::verify::verify_layout;
use crate::{LayoutConfig, Settings, shape};

/// Checks one input; panics on any violation.
pub fn check(data: &[u8]) {
  // On the stack mto's binaries give this work ([`crate::STACK`]).
  crate::with_stack(|| checked(data))
}

fn checked(data: &[u8]) {
  let Some((settings, rest)) = data.split_first_chunk::<3>() else {
    return;
  };
  let text = source(rest);
  let parsed = parse(&text, Edition::E2021);
  assert!(
    parsed.errors().is_empty(),
    "generated source does not parse:\n{}\n{:?}",
    text,
    parsed.errors()
  );
  super::layout::check(text.as_bytes());
  let mut with_settings = settings.to_vec();
  with_settings.extend_from_slice(text.as_bytes());
  super::format::check(&with_settings);
  let both = Settings {
    layout: Some(LayoutConfig::default()),
    format: Some(FormatConfig::default()),
    edition: crate::syntax::Edition::E2021,
  };
  let Ok(once) = shape(&text, &both, &Held::default(), None) else {
    return;
  };
  let edition = once.edition;
  let moved = verify_layout(&text, &once.laid_out, edition);
  assert!(
    moved.is_empty(),
    "layout changed the meaning:\n{}\n{:?}",
    text,
    moved
  );
  // Each formatting against what it was given; a second layout pass, when formatting put
  // a body on lines, against the text it was given (as a run checks them).
  let mut formats = vec![(
    once.laid_out.as_str(),
    once
      .relaid
      .as_ref()
      .map_or(once.text.as_str(), |(first, _)| first.as_str()),
  )];
  if let Some((first, again)) = &once.relaid {
    let moved = verify_layout(first, again, edition);
    assert!(
      moved.is_empty(),
      "layout after formatting changed the meaning:\n{}\n{:?}",
      first,
      moved
    );
    formats.push((again.as_str(), once.text.as_str()));
  }
  for (given, gave) in formats {
    let formatted = verify_format(given, gave, edition);
    assert!(
      formatted.is_empty(),
      "formatting after layout changed the meaning:\n{}\n---\n{}\n{:?}",
      given,
      gave,
      formatted
    );
  }
  let twice = shape(&once.text, &both, &Held::default(), None).map(|s| s.text);
  assert_eq!(
    twice.as_deref(),
    Ok(once.text.as_str()),
    "a second run of layout and formatting changed it:\n{}",
    text
  );
}

#[cfg(test)]
mod tests {
  /// Bytes from a seed, as a fuzzer might give them.
  fn bytes(seed: u64, length: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..length)
      .map(|_| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as u8
      })
      .collect()
  }

  #[test]
  fn test_generated_sources_pass_every_check() {
    for seed in 0..500 {
      let data = bytes(seed, 64 + (seed as usize % 5) * 160);
      if let Err(e) = std::panic::catch_unwind(|| super::check(&data)) {
        let message = e.downcast_ref::<String>().cloned().unwrap_or_default();
        panic!("seed {}: {}", seed, message);
      }
    }
  }
}
