//! A hand-written recursive-descent parser that turns tokens into [`Event`]s.
//!
//! The parser sees only significant tokens; the tree builder puts trivia back. Keywords are
//! identifiers to the lexer: the parser recognises them by text and edition and gives them
//! keyword kinds. Multi-character punctuation such as `::` is glued from adjacent single
//! characters, only when nothing separates them.

pub mod event;
mod grammar;

use event::Event;

use crate::syntax::lexer::Token;
use crate::syntax::{Edition, MAX_DEPTH, SyntaxKind, TOO_DEEP};

/// How many lookahead calls without progress count as a parser bug.
const STEP_LIMIT: u32 = 10_000_000;

/// A finished node, which can still be wrapped in a parent with [`CompletedMarker::precede`].
#[derive(Clone, Copy)]
pub(crate) struct CompletedMarker {
  pos: u32,
  kind: SyntaxKind,
}

impl CompletedMarker {
  pub(crate) fn kind(&self) -> SyntaxKind {
    self.kind
  }

  /// Starts a new node that will become this node's parent.
  pub(crate) fn precede(self, p: &mut Parser) -> Marker {
    let parent = p.start();
    match &mut p.events[self.pos as usize] {
      Event::Start { forward_parent, .. } => {
        *forward_parent = std::num::NonZeroU32::new(parent.pos - self.pos);
      }
      other => unreachable!("completed marker points at {:?}", other),
    }
    parent
  }
}

/// The significant tokens of a file, with whether each one touches the next.
pub(crate) struct Input<'t> {
  kinds: Vec<SyntaxKind>,
  texts: Vec<&'t str>,
  joint: Vec<bool>,
}

impl<'t> Input<'t> {
  fn new(text: &'t str, tokens: &[Token]) -> Self {
    let mut kinds = Vec::with_capacity(tokens.len());
    let mut texts = Vec::with_capacity(tokens.len());
    let mut joint = Vec::with_capacity(tokens.len());
    let mut offset = 0;
    let mut previous_end: Option<usize> = None;
    for token in tokens.iter() {
      let end = offset + token.len as usize;
      if !token.kind.is_trivia() {
        if let Some(prev) = previous_end {
          let last = joint.len() - 1;
          joint[last] = prev == offset;
        }
        kinds.push(token.kind);
        texts.push(&text[offset..end]);
        joint.push(false);
        previous_end = Some(end);
      }
      offset = end;
    }
    Input {
      kinds,
      texts,
      joint,
    }
  }
}

/// A started node whose kind is not known yet.
pub(crate) struct Marker {
  pos: u32,
}

impl Marker {
  pub(crate) fn abandon(self, p: &mut Parser) {
    p.open -= 1;
    let idx = self.pos as usize;
    if idx == p.events.len() - 1 {
      p.events.pop();
    } else {
      p.events[idx] = Event::Tombstone;
    }
  }

  pub(crate) fn complete(self, p: &mut Parser, kind: SyntaxKind) -> CompletedMarker {
    p.open -= 1;
    match &mut p.events[self.pos as usize] {
      Event::Start { kind: slot, .. } => *slot = kind,
      other => unreachable!("marker points at {:?}", other),
    }
    p.events.push(Event::Finish);
    CompletedMarker {
      pos: self.pos,
      kind,
    }
  }
}

/// The parser's events for a whole file, and the error messages they name.
pub struct Parsed {
  pub events: Vec<Event>,
  pub messages: Vec<String>,
}

pub(crate) struct Parser<'t> {
  input: Input<'t>,
  pos: usize,
  events: Vec<Event>,
  /// The error messages, which [`Event::Error`] names by their place here.
  messages: Vec<String>,
  edition: Edition,
  steps: std::cell::Cell<u32>,
  /// Nodes started and not yet completed or abandoned: how deep the grammar has recursed.
  open: usize,
  /// The grammar recursed past [`MAX_DEPTH`]: every token from here on reads as the end of
  /// the file, so it returns at once, and the rest of the file is kept as it is.
  too_deep: bool,
}

impl<'t> Parser<'t> {
  /// True if the current tokens form `kind`, gluing punctuation such as `::` when adjacent.
  #[inline]
  pub(crate) fn at(&self, kind: SyntaxKind) -> bool {
    self.nth_at(0, kind)
  }

  /// True if the current token is `keyword` and acts as one here: strict keywords only in
  /// the editions that reserve them, contextual keywords whenever the grammar asks.
  pub(crate) fn at_contextual_kw(&self, keyword: &str) -> bool {
    self.current() == SyntaxKind::Ident && self.nth_text(0) == keyword
  }

  #[inline]
  pub(crate) fn at_kw(&self, keyword: &str) -> bool {
    self.nth_at_kw(0, keyword)
  }

  /// Consumes the current tokens as `kind`, which the caller has already checked for.
  /// A mismatch is a parser bug: debug builds (tests, fuzzing) panic so it gets found;
  /// release builds report an error instead of crashing on the user's file.
  pub(crate) fn bump(&mut self, kind: SyntaxKind) {
    if !self.eat(kind) && !self.too_deep {
      debug_assert!(false, "parser bug: expected {:?}", kind);
      self.error(format!("expected {}", describe(kind)));
    }
  }

  /// Consumes the current token as it is.
  pub(crate) fn bump_any(&mut self) {
    let kind = self.current();
    if kind == SyntaxKind::Eof {
      return;
    }
    self.do_bump(kind, 1);
  }

  /// Consumes the current token, giving it `kind` in the tree (for keywords).
  pub(crate) fn bump_remap(&mut self, kind: SyntaxKind) {
    if self.current() == SyntaxKind::Eof {
      return;
    }
    self.do_bump(kind, 1);
  }

  #[inline]
  pub(crate) fn current(&self) -> SyntaxKind {
    self.nth(0)
  }

  pub(crate) fn eat(&mut self, kind: SyntaxKind) -> bool {
    if !self.at(kind) {
      return false;
    }
    let n_raw = composite_parts(kind).map_or(1, |parts| parts.len());
    self.do_bump(kind, n_raw as u8);
    true
  }

  /// Consumes `keyword` as `kind` if it is next.
  pub(crate) fn eat_contextual_kw(&mut self, keyword: &str, kind: SyntaxKind) -> bool {
    if self.at_contextual_kw(keyword) {
      self.bump_remap(kind);
      true
    } else {
      false
    }
  }

  pub(crate) fn eat_kw(&mut self, keyword: &str, kind: SyntaxKind) -> bool {
    if self.at_kw(keyword) {
      self.bump_remap(kind);
      true
    } else {
      false
    }
  }

  pub(crate) fn edition(&self) -> Edition {
    self.edition
  }

  /// Reports an error at the current position.
  pub(crate) fn error(&mut self, message: impl Into<String>) {
    self.events.push(Event::Error {
      message: self.messages.len() as u32,
    });
    self.messages.push(message.into());
  }

  /// Consumes `kind`, or reports that it was expected.
  pub(crate) fn expect(&mut self, kind: SyntaxKind) -> bool {
    if self.eat(kind) {
      return true;
    }
    self.error(format!("expected {}", describe(kind)));
    false
  }

  pub(crate) fn is_eof(&self) -> bool {
    self.current() == SyntaxKind::Eof
  }

  #[inline]
  pub(crate) fn nth(&self, n: usize) -> SyntaxKind {
    let steps = self.steps.get();
    assert!(steps < STEP_LIMIT, "the parser made no progress");
    self.steps.set(steps + 1);
    if self.too_deep {
      return SyntaxKind::Eof;
    }
    self
      .input
      .kinds
      .get(self.pos + n)
      .copied()
      .unwrap_or(SyntaxKind::Eof)
  }

  #[inline]
  pub(crate) fn nth_at(&self, n: usize, kind: SyntaxKind) -> bool {
    match composite_parts(kind) {
      None => self.nth(n) == kind,
      Some(parts) => parts.iter().enumerate().all(|(idx, part)| {
        self.nth(n + idx) == *part && (idx + 1 == parts.len() || self.joint(n + idx))
      }),
    }
  }

  #[inline]
  pub(crate) fn nth_at_kw(&self, n: usize, keyword: &str) -> bool {
    self.nth(n) == SyntaxKind::Ident
      && self.nth_text(n) == keyword
      && is_keyword_in(keyword, self.edition)
  }

  #[inline]
  pub(crate) fn nth_text(&self, n: usize) -> &'t str {
    self.input.texts.get(self.pos + n).copied().unwrap_or("")
  }

  /// How many significant tokens have been consumed.
  pub(crate) fn token_pos(&self) -> usize {
    self.pos
  }

  pub(crate) fn start(&mut self) -> Marker {
    self.open += 1;
    if self.open > MAX_DEPTH && !self.too_deep {
      self.too_deep = true;
      self.error(TOO_DEEP);
    }
    let pos = self.events.len() as u32;
    self.events.push(Event::Start {
      kind: SyntaxKind::Error,
      forward_parent: None,
    });
    Marker { pos }
  }

  fn do_bump(&mut self, kind: SyntaxKind, n_raw_tokens: u8) {
    self.pos += n_raw_tokens as usize;
    self.steps.set(0);
    self.events.push(Event::Token { kind, n_raw_tokens });
  }

  /// True if token `n` touches token `n + 1`, with no whitespace or comment between.
  fn joint(&self, n: usize) -> bool {
    self.input.joint.get(self.pos + n).copied().unwrap_or(false)
  }
}

/// The parser events for a whole file.
pub fn parse_events(text: &str, tokens: &[Token], edition: Edition) -> Parsed {
  let input = Input::new(text, tokens);
  // Sized for the whole file at once: a file takes close to three events per significant
  // token (its own, and the starts and finishes of the nodes around it).
  let events = Vec::with_capacity(input.kinds.len() * 3);
  let mut p = Parser {
    input,
    pos: 0,
    events,
    messages: Vec::new(),
    edition,
    steps: std::cell::Cell::new(0),
    open: 0,
    too_deep: false,
  };
  grammar::source_file(&mut p);
  Parsed {
    events: p.events,
    messages: p.messages,
  }
}

/// The single-character tokens that make up a glued punctuation kind.
#[inline]
pub(crate) fn composite_parts(kind: SyntaxKind) -> Option<&'static [SyntaxKind]> {
  use SyntaxKind as K;
  let parts: &'static [SyntaxKind] = match kind {
    K::Colon2 => &[K::Colon, K::Colon],
    K::ThinArrow => &[K::Minus, K::Gt],
    K::FatArrow => &[K::Eq, K::Gt],
    K::Dot2 => &[K::Dot, K::Dot],
    K::Dot3 => &[K::Dot, K::Dot, K::Dot],
    K::Dot2Eq => &[K::Dot, K::Dot, K::Eq],
    K::EqEq => &[K::Eq, K::Eq],
    K::Neq => &[K::Bang, K::Eq],
    K::LtEq => &[K::Lt, K::Eq],
    K::GtEq => &[K::Gt, K::Eq],
    K::AmpAmp => &[K::Amp, K::Amp],
    K::PipePipe => &[K::Pipe, K::Pipe],
    K::PlusEq => &[K::Plus, K::Eq],
    K::MinusEq => &[K::Minus, K::Eq],
    K::StarEq => &[K::Star, K::Eq],
    K::SlashEq => &[K::Slash, K::Eq],
    K::PercentEq => &[K::Percent, K::Eq],
    K::CaretEq => &[K::Caret, K::Eq],
    K::AmpEq => &[K::Amp, K::Eq],
    K::PipeEq => &[K::Pipe, K::Eq],
    K::Shl => &[K::Lt, K::Lt],
    K::Shr => &[K::Gt, K::Gt],
    K::ShlEq => &[K::Lt, K::Lt, K::Eq],
    K::ShrEq => &[K::Gt, K::Gt, K::Eq],
    _ => return None,
  };
  Some(parts)
}

/// A short description of a token kind for error messages.
pub(crate) fn describe(kind: SyntaxKind) -> String {
  use SyntaxKind as K;
  let text = match kind {
    K::Semi => ";",
    K::Comma => ",",
    K::Colon => ":",
    K::Colon2 => "::",
    K::LParen => "(",
    K::RParen => ")",
    K::LBrace => "{",
    K::RBrace => "}",
    K::LBrack => "[",
    K::RBrack => "]",
    K::Lt => "<",
    K::Gt => ">",
    K::Eq => "=",
    K::Bang => "!",
    K::ThinArrow => "->",
    K::FatArrow => "=>",
    K::Pipe => "|",
    K::Dot2 => "..",
    K::At => "@",
    K::Pound => "#",
    K::Ident => return "an identifier".to_string(),
    other => return other.name().to_lowercase(),
  };
  format!("`{}`", text)
}

/// True if `word` is a keyword in `edition`: strict keywords always, `async`, `await` and
/// `dyn` from 2018, `gen` from 2024. Contextual keywords such as `union` are not included.
/// Inlined: called with a literal, the match folds away.
#[inline]
pub(crate) fn is_keyword_in(word: &str, edition: Edition) -> bool {
  match word {
    "as" | "break" | "const" | "continue" | "crate" | "else" | "enum" | "extern" | "false"
    | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut"
    | "pub" | "ref" | "return" | "self" | "Self" | "static" | "struct" | "super" | "trait"
    | "true" | "type" | "unsafe" | "use" | "where" | "while" => true,
    "async" | "await" | "dyn" => edition >= Edition::E2018,
    "gen" => edition >= Edition::E2024,
    _ => false,
  }
}
