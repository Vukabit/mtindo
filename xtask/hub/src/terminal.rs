//! The `terminal` widget: a program (a shell by default) on a pseudo-terminal, its screen
//! kept by a terminal emulator and drawn in the widget's pane. While the hub hands it the
//! keyboard, every key goes to the program.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Lines kept above the screen for scrolling back.
const SCROLLBACK: usize = 2000;

/// Answers to what a program asks its terminal (what it is, where the cursor is, its
/// colors), sent back as the program's input. Without them a program such as nvim waits
/// for answers that never come.
#[derive(Default)]
pub struct Replies {
  out: Vec<u8>,
  /// The foreground and background colors told to a program that asks.
  colors: [Option<[u8; 3]>; 2],
  /// The window title the program set, if any: its tab's name.
  pub title: String,
}

impl vt100::Callbacks for Replies {
  fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
    self.title = String::from_utf8_lossy(title).into_owned();
  }

  fn unhandled_csi(
    &mut self,
    screen: &mut vt100::Screen,
    i1: Option<u8>,
    i2: Option<u8>,
    params: &[&[u16]],
    c: char,
  ) {
    let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
    let (row, col) = screen.cursor_position();
    let reply = match (i1, i2, c) {
      // What the terminal is: a VT220 with colors.
      (None, None, 'c') if first == 0 => "\x1b[?62;22c".to_string(),
      (Some(b'>'), None, 'c') => "\x1b[>0;0;0c".to_string(),
      (None, None, 'n') if first == 5 => "\x1b[0n".to_string(),
      (None, None, 'n') if first == 6 => format!("\x1b[{};{}R", row + 1, col + 1),
      (Some(b'?'), None, 'n') if first == 6 => format!("\x1b[?{};{}R", row + 1, col + 1),
      // A mode asked about: not known to this terminal.
      (Some(b'?'), Some(b'$'), 'p') => format!("\x1b[?{};0$y", first),
      (None, Some(b'$'), 'p') => format!("\x1b[{};0$y", first),
      (Some(b'>'), None, 'q') => "\x1bP>|hub\x1b\\".to_string(),
      _ => return,
    };
    self.out.extend_from_slice(reply.as_bytes());
  }

  fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
    let which = match params {
      [b"10", b"?"] => 0,
      [b"11", b"?"] => 1,
      _ => return,
    };
    if let Some([r, g, b]) = self.colors[which] {
      let reply = format!(
        "\x1b]{};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\",
        10 + which,
        r,
        r,
        g,
        g,
        b,
        b
      );
      self.out.extend_from_slice(reply.as_bytes());
    }
  }
}

pub struct Terminal {
  pub parser: Arc<Mutex<vt100::Parser<Replies>>>,
  master: Box<dyn MasterPty + Send>,
  writer: Arc<Mutex<Box<dyn Write + Send>>>,
  child: Box<dyn Child + Send + Sync>,
  pub exited: Arc<AtomicBool>,
  /// When the program last wrote, in milliseconds since the epoch.
  wrote: Arc<std::sync::atomic::AtomicU64>,
  size: (u16, u16),
  /// The program's name, for its tab until it sets a title.
  program: String,
  /// The name given the tab, over any the program sets.
  pub custom: Option<String>,
  /// The tmux session its program runs in, which outlives the hub.
  pub session: Option<String>,
}

impl Terminal {
  /// Starts `command` (split into words; the user's shell when empty) in `cwd` on a
  /// pseudo-terminal of `rows` by `cols`; `colors` (foreground, background) answer a
  /// program that asks for them. `before` is drawn first: a screen kept from an earlier
  /// session, which the program then writes under.
  pub fn start(
    command: &str,
    cwd: &Path,
    rows: u16,
    cols: u16,
    colors: [Option<[u8; 3]>; 2],
    before: Option<&str>,
  ) -> Result<Terminal, String> {
    let words = crate::source::words(command);
    let words = if words.is_empty() {
      vec![std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string())]
    } else {
      words
    };
    let size = PtySize {
      rows: rows.max(2),
      cols: cols.max(10),
      pixel_width: 0,
      pixel_height: 0,
    };
    let pair = native_pty_system()
      .openpty(size)
      .map_err(|e| e.to_string())?;
    let mut builder = CommandBuilder::new(&words[0]);
    builder.args(&words[1..]);
    builder.cwd(cwd);
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    let child = pair
      .slave
      .spawn_command(builder)
      .map_err(|e| e.to_string())?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = Arc::new(Mutex::new(
      pair.master.take_writer().map_err(|e| e.to_string())?,
    ));
    let replies = Replies {
      out: Vec::new(),
      colors,
      title: String::new(),
    };
    let mut first = vt100::Parser::new_with_callbacks(size.rows, size.cols, SCROLLBACK, replies);
    if let Some(screen) = before.filter(|s| !s.is_empty()) {
      first.process(screen.as_bytes());
      // The program starts on a line of its own below what was there.
      first.process(b"\r\n");
      first.callbacks_mut().out.clear();
    }
    let parser = Arc::new(Mutex::new(first));
    let exited = Arc::new(AtomicBool::new(false));
    let wrote = Arc::new(std::sync::atomic::AtomicU64::new(0));
    {
      let parser = Arc::clone(&parser);
      let writer = Arc::clone(&writer);
      let exited = Arc::clone(&exited);
      let wrote = Arc::clone(&wrote);
      std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
          match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
              wrote.store(now_millis(), Ordering::Relaxed);
              let replies = match parser.lock() {
                Ok(mut p) => {
                  p.process(&buffer[..n]);
                  std::mem::take(&mut p.callbacks_mut().out)
                }
                Err(_) => break,
              };
              if !replies.is_empty()
                && let Ok(mut w) = writer.lock()
              {
                let _ = w.write_all(&replies);
                let _ = w.flush();
              }
            }
          }
        }
        exited.store(true, Ordering::SeqCst);
      });
    }
    Ok(Terminal {
      parser,
      master: pair.master,
      writer,
      child,
      exited,
      wrote,
      size: (size.rows, size.cols),
      program: Path::new(&words[0])
        .file_name()
        .map_or_else(|| words[0].clone(), |n| n.to_string_lossy().into_owned()),
      custom: None,
      session: None,
    })
  }

  /// Follows the pane's size.
  pub fn resize(&mut self, rows: u16, cols: u16) {
    let (rows, cols) = (rows.max(2), cols.max(10));
    if (rows, cols) == self.size {
      return;
    }
    self.size = (rows, cols);
    let _ = self.master.resize(PtySize {
      rows,
      cols,
      pixel_width: 0,
      pixel_height: 0,
    });
    if let Ok(mut p) = self.parser.lock() {
      p.screen_mut().set_size(rows, cols);
    }
  }

  fn send(&self, bytes: &[u8]) {
    if !bytes.is_empty()
      && let Ok(mut w) = self.writer.lock()
    {
      let _ = w.write_all(bytes);
      let _ = w.flush();
    }
  }

  /// The screen as escape sequences that draw it again, to keep across a restart; empty
  /// while a full-screen program has it, whose screen would mean nothing under a new
  /// shell.
  pub fn snapshot(&self) -> String {
    self.parser.lock().map_or_else(
      |_| String::new(),
      |p| {
        if p.screen().alternate_screen() {
          String::new()
        } else {
          String::from_utf8_lossy(&p.screen().contents_formatted()).into_owned()
        }
      },
    )
  }

  /// Whether the program has taken the whole screen (nvim, less, htop), as such programs
  /// do by switching to the alternate screen.
  pub fn full_screen(&self) -> bool {
    self
      .parser
      .lock()
      .is_ok_and(|p| p.screen().alternate_screen())
  }

  /// Moves the view `by` lines back into what scrolled off the top (forward when
  /// negative), as far as there is.
  pub fn scroll_back(&mut self, by: i64) {
    if let Ok(mut p) = self.parser.lock() {
      let now = p.screen().scrollback() as i64;
      p.screen_mut().set_scrollback((now + by).max(0) as usize);
    }
  }

  /// Every line it has, oldest first: what scrolled off the top, then the screen. The
  /// view is left where it was.
  pub fn history(&self) -> Vec<String> {
    let Ok(mut p) = self.parser.lock() else {
      return Vec::new();
    };
    let was = p.screen().scrollback();
    p.screen_mut().set_scrollback(usize::MAX);
    let back = p.screen().scrollback();
    let (rows, cols) = p.screen().size();
    let rows = usize::from(rows.max(1));
    let mut lines: Vec<String> = Vec::with_capacity(back + rows);
    // At `offset` lines back, the screen shows lines `back - offset` onwards.
    let mut offset = back;
    loop {
      p.screen_mut().set_scrollback(offset);
      let first = back - offset;
      for (k, row) in p.screen().rows(0, cols).enumerate() {
        if first + k == lines.len() {
          lines.push(row.trim_end().to_string());
        }
      }
      if offset == 0 {
        break;
      }
      offset = offset.saturating_sub(rows);
    }
    p.screen_mut().set_scrollback(was);
    lines
  }

  /// Scrolls so that line `line` of [`Terminal::history`] is at the top, or the screen
  /// shows it.
  pub fn show_line(&mut self, line: usize) {
    if let Ok(mut p) = self.parser.lock() {
      p.screen_mut().set_scrollback(usize::MAX);
      let back = p.screen().scrollback();
      p.screen_mut().set_scrollback(back.saturating_sub(line));
    }
  }

  /// The text it shows, as scrolled.
  pub fn shown_text(&self) -> String {
    self
      .parser
      .lock()
      .map(|p| p.screen().contents())
      .unwrap_or_default()
  }

  /// How far back into its history the view is.
  pub fn scrolled_back(&self) -> usize {
    self.parser.lock().map_or(0, |p| p.screen().scrollback())
  }

  /// Whether the program asked for the mouse (nvim, htop, less with `--mouse`).
  pub fn wants_mouse(&self) -> bool {
    self
      .parser
      .lock()
      .is_ok_and(|p| p.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None)
  }

  /// Sends a mouse event at (`x`, `y`) of the screen, from 0, as the program asked for
  /// them; nothing it did not ask for.
  pub fn mouse(&mut self, event: &MouseEvent, x: u16, y: u16) {
    let (mode, encoding) = match self.parser.lock() {
      Ok(p) => (
        p.screen().mouse_protocol_mode(),
        p.screen().mouse_protocol_encoding(),
      ),
      Err(_) => return,
    };
    if let Some(bytes) = mouse_bytes(event, x, y, mode, encoding) {
      self.send(&bytes);
    }
  }

  /// Sends a key to the program, as a terminal sends it in the mode the program set; the
  /// view comes back from its history first.
  pub fn key(&mut self, event: &KeyEvent) {
    self.scroll_back(i64::MIN / 2);
    let application = self
      .parser
      .lock()
      .is_ok_and(|p| p.screen().application_cursor());
    self.send(&bytes_of(event, application));
  }

  /// Sends text, as pasted: marked as a paste when the program asked for that.
  pub fn paste(&mut self, text: &str) {
    let bracketed = self
      .parser
      .lock()
      .is_ok_and(|p| p.screen().bracketed_paste());
    if bracketed {
      self.send(format!("\x1b[200~{}\x1b[201~", text).as_bytes());
    } else {
      self.send(text.as_bytes());
    }
  }

  /// The tab's name: the one given it, else the title the program set, else the
  /// program's name.
  pub fn name(&self) -> String {
    if let Some(name) = &self.custom {
      return name.clone();
    }
    let title = self
      .parser
      .lock()
      .map(|p| p.callbacks().title.trim().to_string())
      .unwrap_or_default();
    if title.is_empty() {
      self.program.clone()
    } else {
      title
    }
  }

  /// Whether the program wrote in the last second: its screen is to be drawn often.
  pub fn busy(&self) -> bool {
    now_millis().saturating_sub(self.wrote.load(Ordering::Relaxed)) < 1000
  }

  pub fn running(&self) -> bool {
    !self.exited.load(Ordering::SeqCst)
  }

  pub fn has_exited(&mut self) -> Option<String> {
    if !self.exited.load(Ordering::SeqCst) {
      return None;
    }
    Some(match self.child.try_wait() {
      Ok(Some(status)) => format!("exited with {}", status.exit_code()),
      _ => "exited".to_string(),
    })
  }
}

impl Drop for Terminal {
  fn drop(&mut self) {
    let _ = self.child.kill();
  }
}

/// The bytes a terminal sends for a key, as xterm sends them: with shift, alt or ctrl
/// on a special key, its modifier number (`\x1b[1;5A` is ctrl+up); with `application`
/// cursor keys set by the program, unmodified arrows, home and end as `\x1bO`.
pub fn bytes_of(event: &KeyEvent, application: bool) -> Vec<u8> {
  let m = event.modifiers;
  let ctrl = m.contains(KeyModifiers::CONTROL);
  let alt = m.contains(KeyModifiers::ALT);
  let shift = m.contains(KeyModifiers::SHIFT);
  // xterm's modifier number: 1, plus 1 for shift, 2 for alt, 4 for ctrl.
  let number = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;
  let letter = |c: char| -> Vec<u8> {
    if number > 1 {
      format!("\x1b[1;{}{}", number, c).into_bytes()
    } else if application {
      format!("\x1bO{}", c).into_bytes()
    } else {
      format!("\x1b[{}", c).into_bytes()
    }
  };
  let tilde = |n: u8| -> Vec<u8> {
    if number > 1 {
      format!("\x1b[{};{}~", n, number).into_bytes()
    } else {
      format!("\x1b[{}~", n).into_bytes()
    }
  };
  let function = |c: char| -> Vec<u8> {
    if number > 1 {
      format!("\x1b[1;{}{}", number, c).into_bytes()
    } else {
      format!("\x1bO{}", c).into_bytes()
    }
  };
  let mut out: Vec<u8> = match event.code {
    KeyCode::Char(c) if ctrl => {
      let lower = c.to_ascii_lowercase();
      match lower {
        'a'..='z' => vec![lower as u8 - b'a' + 1],
        '@' | ' ' | '2' => vec![0],
        '[' | '3' => vec![27],
        '\\' | '4' => vec![28],
        ']' | '5' => vec![29],
        '^' | '6' => vec![30],
        '_' | '7' | '/' => vec![31],
        _ => c.to_string().into_bytes(),
      }
    }
    KeyCode::Char(c) => c.to_string().into_bytes(),
    KeyCode::Enter => vec![b'\r'],
    KeyCode::Tab => vec![b'\t'],
    KeyCode::BackTab => b"\x1b[Z".to_vec(),
    KeyCode::Backspace if ctrl => vec![0x08],
    KeyCode::Backspace => vec![0x7f],
    KeyCode::Esc => vec![0x1b],
    KeyCode::Up => letter('A'),
    KeyCode::Down => letter('B'),
    KeyCode::Right => letter('C'),
    KeyCode::Left => letter('D'),
    KeyCode::Home => letter('H'),
    KeyCode::End => letter('F'),
    KeyCode::Insert => tilde(2),
    KeyCode::Delete => tilde(3),
    KeyCode::PageUp => tilde(5),
    KeyCode::PageDown => tilde(6),
    KeyCode::F(n) => match n {
      1 => function('P'),
      2 => function('Q'),
      3 => function('R'),
      4 => function('S'),
      5 => tilde(15),
      6 => tilde(17),
      7 => tilde(18),
      8 => tilde(19),
      9 => tilde(20),
      10 => tilde(21),
      11 => tilde(23),
      12 => tilde(24),
      _ => Vec::new(),
    },
    _ => Vec::new(),
  };
  // alt on a character is esc first; on a special key it is in the modifier number.
  if alt
    && matches!(
      event.code,
      KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace | KeyCode::Esc
    )
  {
    out.insert(0, 0x1b);
  }
  out
}

/// The bytes for a mouse event as xterm reports them in `mode` and `encoding`, or none
/// when the mode leaves it out.
pub fn mouse_bytes(
  event: &MouseEvent,
  x: u16,
  y: u16,
  mode: vt100::MouseProtocolMode,
  encoding: vt100::MouseProtocolEncoding,
) -> Option<Vec<u8>> {
  use vt100::MouseProtocolMode as Mode;
  let button = |b: MouseButton| match b {
    MouseButton::Left => 0,
    MouseButton::Middle => 1,
    MouseButton::Right => 2,
  };
  let (code, release) = match event.kind {
    MouseEventKind::Down(b) => (button(b), false),
    MouseEventKind::Up(b) if mode != Mode::Press => (button(b), true),
    MouseEventKind::Drag(b) if matches!(mode, Mode::ButtonMotion | Mode::AnyMotion) => {
      (button(b) + 32, false)
    }
    MouseEventKind::Moved if mode == Mode::AnyMotion => (3 + 32, false),
    MouseEventKind::ScrollUp => (64, false),
    MouseEventKind::ScrollDown => (65, false),
    _ => return None,
  };
  if mode == Mode::None {
    return None;
  }
  let m = event.modifiers;
  let code = code
    + if m.contains(KeyModifiers::SHIFT) {
      4
    } else {
      0
    }
    + if m.contains(KeyModifiers::ALT) { 8 } else { 0 }
    + if m.contains(KeyModifiers::CONTROL) {
      16
    } else {
      0
    };
  Some(match encoding {
    vt100::MouseProtocolEncoding::Sgr => format!(
      "\x1b[<{};{};{}{}",
      code,
      x + 1,
      y + 1,
      if release { 'm' } else { 'M' }
    )
    .into_bytes(),
    _ => {
      // The old encoding: a release is button 3; places past 222 cannot be said.
      let code = if release { 3 + (code & !3) } else { code };
      let at = |v: u16| (v.min(222) + 33) as u8;
      vec![0x1b, b'[', b'M', (code + 32) as u8, at(x), at(y)]
    }
  })
}

/// A `#rrggbb` color's bytes.
pub fn rgb(text: &str) -> Option<[u8; 3]> {
  let hex = text.strip_prefix('#').filter(|h| h.len() == 6)?;
  let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
  Some([byte(0)?, byte(2)?, byte(4)?])
}

fn now_millis() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A real program on a real pseudo-terminal: its output reaches the emulator's screen,
  /// and keys sent reach it.
  #[test]
  fn test_a_program_runs_and_answers_keys() {
    let mut term = Terminal::start("sh", Path::new("."), 10, 40, [None; 2], None).unwrap();
    term.paste("echo hub-$((40+2))\r");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
      let shown = term.parser.lock().unwrap().screen().contents();
      if shown.contains("hub-42") {
        break;
      }
      assert!(
        std::time::Instant::now() < deadline,
        "the shell did not answer:\n{}",
        shown
      );
      std::thread::sleep(std::time::Duration::from_millis(50));
    }
    term.key(&KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while term.has_exited().is_none() {
      assert!(
        std::time::Instant::now() < deadline,
        "ctrl+d did not end the shell"
      );
      std::thread::sleep(std::time::Duration::from_millis(50));
    }
  }

  #[test]
  fn test_keys_become_terminal_bytes() {
    let key = |code, m| bytes_of(&KeyEvent::new(code, m), false);
    assert_eq!(key(KeyCode::Char('c'), KeyModifiers::CONTROL), vec![3]);
    assert_eq!(key(KeyCode::Char('x'), KeyModifiers::ALT), vec![0x1b, b'x']);
    assert_eq!(key(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
    assert_eq!(key(KeyCode::Enter, KeyModifiers::NONE), b"\r");
    assert_eq!(key(KeyCode::Char('é'), KeyModifiers::NONE), "é".as_bytes());
    assert_eq!(key(KeyCode::Up, KeyModifiers::CONTROL), b"\x1b[1;5A");
    assert_eq!(
      key(KeyCode::Left, KeyModifiers::SHIFT | KeyModifiers::ALT),
      b"\x1b[1;4D"
    );
    assert_eq!(key(KeyCode::Delete, KeyModifiers::CONTROL), b"\x1b[3;5~");
    assert_eq!(key(KeyCode::F(1), KeyModifiers::SHIFT), b"\x1b[1;2P");
    let app = bytes_of(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true);
    assert_eq!(app, b"\x1bOA");
  }

  #[test]
  fn test_mouse_events_are_reported_as_asked() {
    use vt100::{MouseProtocolEncoding as E, MouseProtocolMode as M};
    let click = MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column: 0,
      row: 0,
      modifiers: KeyModifiers::NONE,
    };
    assert_eq!(mouse_bytes(&click, 4, 2, M::None, E::Sgr), None);
    assert_eq!(
      mouse_bytes(&click, 4, 2, M::Press, E::Sgr).unwrap(),
      b"\x1b[<0;5;3M"
    );
    let up = MouseEvent {
      kind: MouseEventKind::Up(MouseButton::Left),
      ..click
    };
    assert_eq!(mouse_bytes(&up, 4, 2, M::Press, E::Sgr), None);
    assert_eq!(
      mouse_bytes(&up, 4, 2, M::PressRelease, E::Sgr).unwrap(),
      b"\x1b[<0;5;3m"
    );
    let wheel = MouseEvent {
      kind: MouseEventKind::ScrollDown,
      ..click
    };
    assert_eq!(
      mouse_bytes(&wheel, 0, 0, M::Press, E::Default).unwrap(),
      b"\x1b[Ma!!"
    );
  }

  /// What nvim asks its terminal at start is answered: what it is, where the cursor is,
  /// a mode, and the background color.
  #[test]
  fn test_questions_a_program_asks_are_answered() {
    let replies = Replies {
      colors: [None, rgb("#15171b")],
      ..Replies::default()
    };
    let mut p = vt100::Parser::new_with_callbacks(5, 20, 0, replies);
    p.process(b"ab\x1b[c\x1b[6n\x1b[?2026$p\x1b]11;?\x07\x1b]10;?\x07");
    let out = String::from_utf8(p.callbacks().out.clone()).unwrap();
    assert_eq!(
      out,
      "\x1b[?62;22c\x1b[1;3R\x1b[?2026;0$y\x1b]11;rgb:1515/1717/1b1b\x1b\\"
    );
  }
}
