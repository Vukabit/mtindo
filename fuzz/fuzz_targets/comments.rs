#![no_main]

//! Comments everywhere: layout must keep every comment, and never write what the verifier
//! would refuse.
//!
//! The first bytes pick the settings and where comments go; the rest is a source file. Each
//! gap between tokens that already holds whitespace may get a numbered comment, line or
//! block, so no token is split or joined. When the result parses and layout rearranges it:
//! - the verifier finds nothing wrong (a finding would stop a real write: a layout bug);
//! - every inserted comment is still there, as often as before;
//! - a second layout changes nothing.

use libfuzzer_sys::fuzz_target;
use mtindo::LayoutConfig;
use mtindo::layout::layout;
use mtindo::options::layout::{NameSort, TraitImplSort};
use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::{Edition, SyntaxKind, parse};
use mtindo::verify::verify_layout;

/// Bytes read before the source: settings, then where comments go.
const HEADER: usize = 4;

fuzz_target!(|data: &[u8]| {
  if data.len() < HEADER {
    return;
  }
  let (head, rest) = data.split_at(HEADER);
  let Ok(source) = std::str::from_utf8(rest) else {
    return;
  };
  if source.contains('\r') || !parse(source, Edition::E2021).errors().is_empty() {
    return;
  }
  let config = settings(head[0], head[1]);
  let (text, inserted) = with_comments(source, head[2], head[3]);
  if inserted.is_empty() || !parse(&text, Edition::E2021).errors().is_empty() {
    return;
  }
  let Ok(laid) = layout(&text, &config) else {
    return;
  };
  if laid.text == text {
    return;
  }
  let found = verify_layout(&text, &laid.text, laid.edition);
  assert!(
    found.is_empty(),
    "the verifier refuses layout's output:\n{}\n---\n{}\n{:?}",
    text,
    laid.text,
    found
  );
  for comment in &inserted {
    assert_eq!(
      text.matches(comment.as_str()).count(),
      laid.text.matches(comment.as_str()).count(),
      "`{}` lost or doubled:\n{}\n---\n{}",
      comment,
      text,
      laid.text
    );
  }
  let again = layout(&laid.text, &config).map(|l| l.text);
  assert_eq!(
    again.as_deref(),
    Ok(laid.text.as_str()),
    "a second layout changes it"
  );
});

/// Layout settings from two bytes, so the fuzzer explores them as it explores the source.
fn settings(a: u8, b: u8) -> LayoutConfig {
  let mut config = LayoutConfig {
    blank_lines_between_blocks: usize::from(a & 1),
    sort_alphabetically: a & 2 == 0,
    group_impls: a & 4 == 0,
    sort_imports: a & 8 == 0,
    import_grouping: a & 16 == 0,
    test_module_in_place: a & 32 != 0,
    reorder_impl_items: a & 64 != 0,
    impl_method_order_trait: a & 128 != 0,
    name_sort: match b % 3 {
      0 => NameSort::Version,
      1 => NameSort::Bytes,
      _ => NameSort::CaseInsensitive,
    },
    ..LayoutConfig::default()
  };
  if b & 4 != 0 {
    config.trait_impl_sort = TraitImplSort::Original;
  }
  if b & 8 != 0 {
    config.type_order.reverse();
  }
  if b & 16 != 0 {
    config.visibility_order.reverse();
  }
  config
}

/// `source` with numbered comments in some of its whitespace gaps: every `every`-th gap,
/// starting at `offset`, a block comment, or a line comment ending its line when `offset` is
/// odd. Returns the text and the comments put in.
fn with_comments(source: &str, every: u8, offset: u8) -> (String, Vec<String>) {
  let every = usize::from(every % 7) + 1;
  let mut out = String::with_capacity(source.len() * 2);
  let mut inserted = Vec::new();
  let mut at = 0;
  let mut gap = usize::from(offset);
  for token in tokenize(source, Edition::E2021) {
    let end = at + token.len as usize;
    let text = &source[at..end];
    if token.kind == SyntaxKind::Whitespace {
      gap += 1;
      if gap % every == 0 {
        let id = inserted.len();
        let comment = if (gap / every + usize::from(offset)) % 2 == 1 {
          format!("// c{}x\n", id)
        } else {
          format!("/* c{}x */", id)
        };
        inserted.push(comment.trim_end().to_string());
        out.push(' ');
        out.push_str(&comment);
      }
    }
    out.push_str(text);
    at = end;
  }
  (out, inserted)
}
