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
        while let Some(c) = self.first() {
            if !predicate(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    pub(super) fn first(&self) -> Option<char> {
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
