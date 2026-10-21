//! A character cursor over source text, with lookahead.

pub(super) struct Cursor<'a> {
  text: &'a str,
  pos: usize,
}

impl<'a> Cursor<'a> {
  pub(super) fn new(text: &'a str) -> Self {
    Cursor { text, pos: 0 }
  }

  pub(super) fn bump(&mut self) -> Option<char> {
    let c = self.first()?;
    self.pos += c.len_utf8();
    Some(c)
  }

  /// Consumes characters while `predicate` holds.
  pub(super) fn eat_while(&mut self, predicate: impl Fn(char) -> bool) {
    let bytes = self.text.as_bytes();
    while let Some(&byte) = bytes.get(self.pos) {
      // ASCII, nearly all of any source, read as it is.
      let c = if byte.is_ascii() {
        byte as char
      } else {
        self
          .rest()
          .chars()
          .next()
          .expect("a character at a boundary")
      };
      if !predicate(c) {
        break;
      }
      self.pos += c.len_utf8();
    }
  }

  /// Consumes ASCII spaces, tabs and line ends, eight spaces at a time where it can: an
  /// indentation, read faster than character by character.
  pub(super) fn eat_ascii_spaces(&mut self) {
    let bytes = self.text.as_bytes();
    while bytes[self.pos..].starts_with(b"        ") {
      self.pos += 8;
    }
    while bytes
      .get(self.pos)
      .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
    {
      self.pos += 1;
    }
  }

  /// The text consumed since byte position `start`.
  pub(super) fn consumed_since(&self, start: usize) -> &'a str {
    &self.text[start..self.pos]
  }

  pub(super) fn first(&self) -> Option<char> {
    let byte = *self.text.as_bytes().get(self.pos)?;
    if byte.is_ascii() {
      return Some(byte as char);
    }
    self.rest().chars().next()
  }

  pub(super) fn is_eof(&self) -> bool {
    self.pos >= self.text.len()
  }

  /// The character `n` positions ahead; `nth(0)` is `first()`.
  pub(super) fn nth(&self, n: usize) -> Option<char> {
    self.rest().chars().nth(n)
  }

  pub(super) fn pos(&self) -> usize {
    self.pos
  }

  pub(super) fn rest(&self) -> &'a str {
    &self.text[self.pos..]
  }
}
