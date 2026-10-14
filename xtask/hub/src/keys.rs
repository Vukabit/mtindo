//! Keys as `.hub.toml` writes them: `q`, `?`, `ctrl+c`, `shift+tab`, `enter`, `f1`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A key and its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
  pub code: KeyCode,
  pub modifiers: KeyModifiers,
}

impl Key {
  /// Whether a pressed key is this one. A character matches itself exactly (`R` is not
  /// `r`), with shift ignored, since shift is how it was typed; any other key needs its
  /// modifiers exactly.
  pub fn matches(&self, event: &KeyEvent) -> bool {
    let event = &canonical(event);
    let ctrl = KeyModifiers::CONTROL;
    match (self.code, event.code) {
      // A terminal sends ctrl+i as tab.
      (KeyCode::Char('i'), KeyCode::Tab) => self.modifiers == ctrl && event.modifiers.is_empty(),
      (KeyCode::Tab, KeyCode::BackTab) => self.modifiers.contains(KeyModifiers::SHIFT),
      (KeyCode::Char(a), KeyCode::Char(b)) => {
        a == b && event.modifiers - KeyModifiers::SHIFT == self.modifiers - KeyModifiers::SHIFT
      }
      (a, b) => a == b && event.modifiers == self.modifiers,
    }
  }
}

/// A pressed key's name, as `.hub.toml` writes it.
pub fn name(event: &KeyEvent) -> String {
  let event = canonical(event);
  let mut out = String::new();
  for (m, word) in [
    (KeyModifiers::CONTROL, "ctrl+"),
    (KeyModifiers::ALT, "alt+"),
  ] {
    if event.modifiers.contains(m) {
      out.push_str(word);
    }
  }
  out.push_str(&match event.code {
    KeyCode::Char(' ') => "space".to_string(),
    KeyCode::Char(c) => c.to_string(),
    KeyCode::F(n) => format!("f{}", n),
    KeyCode::BackTab => "shift+tab".to_string(),
    KeyCode::PageUp => "pageup".to_string(),
    KeyCode::PageDown => "pagedown".to_string(),
    other => format!("{:?}", other).to_lowercase(),
  });
  out
}

/// Reads a key's name; `None` if it is not one.
pub fn parse(text: &str) -> Option<Key> {
  let mut modifiers = KeyModifiers::NONE;
  let parts: Vec<&str> = if text == "+" {
    vec!["+"]
  } else {
    text.split('+').collect()
  };
  let (last, mods) = parts.split_last()?;
  for m in mods {
    modifiers |= match *m {
      "ctrl" => KeyModifiers::CONTROL,
      "alt" => KeyModifiers::ALT,
      "shift" => KeyModifiers::SHIFT,
      _ => return None,
    };
  }
  let code = match *last {
    "enter" => KeyCode::Enter,
    "esc" => KeyCode::Esc,
    "tab" => KeyCode::Tab,
    "backspace" => KeyCode::Backspace,
    "delete" => KeyCode::Delete,
    "space" => KeyCode::Char(' '),
    "up" => KeyCode::Up,
    "down" => KeyCode::Down,
    "left" => KeyCode::Left,
    "right" => KeyCode::Right,
    "home" => KeyCode::Home,
    "end" => KeyCode::End,
    "pageup" => KeyCode::PageUp,
    "pagedown" => KeyCode::PageDown,
    f if f.len() > 1 && f.starts_with('f') => KeyCode::F(f[1..].parse().ok()?),
    c if c.chars().count() == 1 => KeyCode::Char(c.chars().next()?),
    _ => return None,
  };
  Some(Key { code, modifiers })
}

/// A key as its name is written: terminals send ctrl with `\ ] ^ _` as ctrl with `4` to
/// `7`.
fn canonical(event: &KeyEvent) -> KeyEvent {
  let mut out = *event;
  if event.modifiers.contains(KeyModifiers::CONTROL)
    && let KeyCode::Char(c) = event.code
  {
    out.code = KeyCode::Char(match c {
      '4' => '\\',
      '5' => ']',
      '6' => '^',
      '7' => '_',
      c => c,
    });
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
  }

  #[test]
  fn test_keys_parse_and_match() {
    let q = parse("q").unwrap();
    assert!(q.matches(&press(KeyCode::Char('q'), KeyModifiers::NONE)));
    assert!(!q.matches(&press(KeyCode::Char('q'), KeyModifiers::CONTROL)));
    let ctrl_c = parse("ctrl+c").unwrap();
    assert!(ctrl_c.matches(&press(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    assert!(!ctrl_c.matches(&press(KeyCode::Char('c'), KeyModifiers::NONE)));
    // `?` arrives with shift held on most keyboards.
    assert!(
      parse("?")
        .unwrap()
        .matches(&press(KeyCode::Char('?'), KeyModifiers::SHIFT))
    );
    assert!(
      parse("shift+tab")
        .unwrap()
        .matches(&press(KeyCode::BackTab, KeyModifiers::SHIFT))
    );
    assert!(
      parse("R")
        .unwrap()
        .matches(&press(KeyCode::Char('R'), KeyModifiers::SHIFT))
    );
    assert!(
      !parse("r")
        .unwrap()
        .matches(&press(KeyCode::Char('R'), KeyModifiers::SHIFT))
    );
    assert_eq!(parse("f5").unwrap().code, KeyCode::F(5));
    let ctrl = KeyModifiers::CONTROL;
    assert!(
      parse("ctrl+]")
        .unwrap()
        .matches(&press(KeyCode::Char('5'), ctrl))
    );
    assert!(
      parse("ctrl+\\")
        .unwrap()
        .matches(&press(KeyCode::Char('4'), ctrl))
    );
    assert!(
      parse("ctrl+i")
        .unwrap()
        .matches(&press(KeyCode::Tab, KeyModifiers::NONE))
    );
    assert!(
      !parse("ctrl+i")
        .unwrap()
        .matches(&press(KeyCode::BackTab, KeyModifiers::SHIFT))
    );
    assert!(parse("hyper+x").is_none());
    assert!(parse("nonsense").is_none());
  }
}
