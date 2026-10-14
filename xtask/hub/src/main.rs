//! hub: a terminal hub for a project's development records, declared in `.hub.toml`. See
//! SPEC.md for the file; nothing here knows any project.

mod app;
mod bindings;
mod config;
mod expr;
mod git;
mod keys;
mod layout;
mod session;
mod source;
mod template;
mod terminal;
mod widgets;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{
  self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
  Event, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
  EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{CrosstermBackend, TestBackend};

const USAGE: &str = "usage: hub [--config FILE] [--fresh] [--no-color] [--check | --keys [--vim] | --render SCREEN [--size COLSxROWS] [--vim | --no-vim] [--plain]]";

type Term = Terminal<CrosstermBackend<std::io::Stdout>>;

/// How a run of the interface on one config ended.
enum Ended {
  Quit,
  /// The config was read again: go on with it.
  Reloaded(Box<config::Loaded>),
}

/// When each of the config's files last changed, to see that one did.
fn changed_at(files: &[PathBuf]) -> Vec<Option<std::time::SystemTime>> {
  files
    .iter()
    .map(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok())
    .collect()
}

/// Loads every source once and reports what failed; the config itself was checked on load.
fn check(loaded: &config::Loaded) -> ExitCode {
  let mut failed = 0;
  for spec in &loaded.config.sources {
    match source::load(loaded, spec, None, 0) {
      Ok(rows) => println!("source {:<16} {} row(s)", spec.name, rows.len()),
      Err(e) => {
        failed += 1;
        println!("source {:<16} FAILED: {}", spec.name, e);
      }
    }
  }
  println!(
    "{}: {} screens, {} widgets, {} actions, {} sources ({} failed)",
    loaded.path.display(),
    loaded.config.screens.len(),
    loaded.config.widgets.len(),
    loaded.config.actions.len(),
    loaded.config.sources.len(),
    failed
  );
  if failed == 0 {
    ExitCode::SUCCESS
  } else {
    ExitCode::from(1)
  }
}

fn close_terminal(terminal: &mut Term) {
  let _ = disable_raw_mode();
  let _ = execute!(
    terminal.backend_mut(),
    DisableMouseCapture,
    DisableBracketedPaste,
    LeaveAlternateScreen
  );
  let _ = terminal.show_cursor();
}

fn interactive(first: config::Loaded, fresh: bool) -> Result<(), String> {
  if first.config.hub.background.as_deref().unwrap_or("auto") == "auto"
    && let Some(light) = terminal_is_light()
  {
    widgets::LIGHT.store(light, std::sync::atomic::Ordering::Relaxed);
  }
  // Shared with the threads that load sources in the background.
  let mut loaded = std::sync::Arc::new(first);
  let mut carry: Option<app::Carry> = None;
  let owner = session::Owner::claim(&app::state_dir(&loaded));
  let mut terminal = open_terminal()?;
  let result = (|| -> Result<(), String> {
    loop {
      let mut app = match carry.take() {
        Some(c) => app::App::from_carry(&loaded, c),
        None => {
          let mut app = app::App::new(&loaded);
          if !owner.owns() {
            app.toast("another hub has this folder's session open: this one keeps none");
          } else if !fresh && let Some(s) = session::Session::load(&app.session_path()) {
            app.restore(s);
          }
          app
        }
      };
      set_mouse(app.mouse_on);
      let watch = loaded.config.hub.reload != Some(false);
      let mut seen = changed_at(&loaded.files);
      let mut saved = String::new();
      let (mut last_save, mut last_look) = (Instant::now(), Instant::now());
      // Sources load in the background, so a slow one never holds up the interface.
      app.store.go_background(loaded.clone());
      let ended = loop {
        app.store.receive();
        app.poll_jobs();
        app.check_alerts();
        terminal.draw(|f| app.draw(f)).map_err(|e| e.to_string())?;
        // Alerts and failures reach the desktop, through the terminal.
        let notes = app.take_notifications();
        if !notes.is_empty() {
          let mut out = std::io::stdout();
          let _ = out.write_all(notes.as_bytes());
          let _ = out.flush();
        }
        // A running terminal widget is redrawn as its program writes.
        let wait = if app.live_terminal() {
          16
        } else if app.store.busy() {
          // Rows arriving from the background are shown soon after.
          50
        } else {
          250
        };
        if event::poll(Duration::from_millis(wait)).map_err(|e| e.to_string())? {
          match event::read().map_err(|e| e.to_string())? {
            Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
            Event::Paste(text) => app.paste(&text),
            Event::Mouse(mouse) => app.mouse(mouse),
            _ => {}
          }
        }
        if std::mem::take(&mut app.mouse_changed) {
          set_mouse(app.mouse_on);
        }
        if let Some(line) = app.on_terminal.take() {
          close_terminal(&mut terminal);
          run_on_terminal(&loaded.base, &line);
          terminal = open_terminal()?;
          set_mouse(app.mouse_on);
          // Cleared without asking the terminal where its cursor is, which some never
          // answer.
          execute!(
            terminal.backend_mut(),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
          )
          .map_err(|e| e.to_string())?;
          app.store.invalidate();
        }
        if owner.owns() && last_save.elapsed() >= Duration::from_secs(2) {
          app.session().save(&app.session_path(), &mut saved);
          last_save = Instant::now();
        }
        let looked = watch && last_look.elapsed() >= Duration::from_secs(1);
        if looked {
          last_look = Instant::now();
        }
        let changed = looked && changed_at(&loaded.files) != seen;
        if changed || std::mem::take(&mut app.reload_asked) {
          seen = changed_at(&loaded.files);
          match config::load(&loaded.path) {
            Ok(new) => break Ended::Reloaded(Box::new(new)),
            Err(problems) => app.config_problems(&problems),
          }
        }
        if app.quit {
          if owner.owns() {
            app.session().save(&app.session_path(), &mut saved);
          }
          break Ended::Quit;
        }
      };
      match ended {
        Ended::Quit => return Ok(()),
        Ended::Reloaded(new) => {
          carry = Some(app.into_carry());
          loaded = std::sync::Arc::new(*new);
        }
      }
    }
  })();
  close_terminal(&mut terminal);
  result
}

fn layout_has(layout: &config::LayoutSpec, widget: &str) -> bool {
  match layout {
    config::LayoutSpec::Widget(w) => w == widget,
    config::LayoutSpec::Split { children, .. } => children.iter().any(|c| layout_has(c, widget)),
  }
}

fn main() -> ExitCode {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let value = |flag: &str| {
    args
      .iter()
      .position(|a| a == flag)
      .and_then(|i| args.get(i + 1))
      .cloned()
  };
  if args.iter().any(|a| a == "-h" || a == "--help") {
    println!(
      "{}\n\nA terminal hub declared in .hub.toml (see xtask/hub/SPEC.md).",
      USAGE
    );
    return ExitCode::SUCCESS;
  }
  let path = match value("--config") {
    Some(p) => PathBuf::from(p),
    None => match config::find(&std::env::current_dir().unwrap_or_default()) {
      Some(p) => p,
      None => {
        eprintln!("hub: no .hub.toml here or in any folder above");
        return ExitCode::from(2);
      }
    },
  };
  let loaded = match config::load(&path) {
    Ok(loaded) => loaded,
    Err(problems) => {
      for p in &problems {
        eprintln!("{}", p);
      }
      eprintln!("hub: {} problem(s) in {}", problems.len(), path.display());
      return ExitCode::from(1);
    }
  };
  if args.iter().any(|a| a == "--check") {
    return check(&loaded);
  }
  if args.iter().any(|a| a == "--no-color") {
    widgets::NO_COLOR.store(true, std::sync::atomic::Ordering::Relaxed);
  }
  // Drawn as the config has it, unless told otherwise.
  let vim = args.iter().any(|a| a == "--vim")
    || (loaded.config.hub.vim == Some(true) && !args.iter().any(|a| a == "--no-vim"));
  if args.iter().any(|a| a == "--keys") {
    print!("{}", app::help_text(&loaded, vim));
    return ExitCode::SUCCESS;
  }
  if let Some(screen) = value("--render") {
    let size = value("--size").unwrap_or_else(|| "100x30".into());
    let Some((w, h)) = size
      .split_once('x')
      .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
    else {
      eprintln!("{}", USAGE);
      return ExitCode::from(2);
    };
    let plain = args.iter().any(|a| a == "--plain");
    return match render(&loaded, &screen, w, h, vim, plain) {
      Ok(text) => {
        print!("{}", text);
        ExitCode::SUCCESS
      }
      Err(e) => {
        eprintln!("hub: {}", e);
        ExitCode::from(1)
      }
    };
  }
  restore_terminal_on_panic();
  match interactive(loaded, args.iter().any(|a| a == "--fresh")) {
    Ok(()) => ExitCode::SUCCESS,
    Err(e) => {
      eprintln!("hub: {}", e);
      ExitCode::from(1)
    }
  }
}

fn open_terminal() -> Result<Term, String> {
  enable_raw_mode().map_err(|e| e.to_string())?;
  let mut out = std::io::stdout();
  execute!(out, EnterAlternateScreen, EnableBracketedPaste).map_err(|e| e.to_string())?;
  Terminal::new(CrosstermBackend::new(out)).map_err(|e| e.to_string())
}

/// One screen drawn as text. A page is drawn for the first row of the widget that opens
/// it.
fn render(
  loaded: &config::Loaded,
  id: &str,
  width: u16,
  height: u16,
  vim: bool,
  plain: bool,
) -> Result<String, String> {
  let screen = loaded
    .screen(id)
    .ok_or_else(|| format!("no screen `{}`", id))?;
  let mut app = app::App::new(loaded);
  app.start_terminals = false;
  // Drawn as asked, whatever a session left in the state folder.
  app.vim = vim;
  let mut terminal = Terminal::new(TestBackend::new(width, height)).map_err(|e| e.to_string())?;
  if screen.page {
    let opener = loaded
      .config
      .widgets
      .iter()
      .find(|w| w.open.as_deref() == Some(id))
      .ok_or_else(|| format!("no widget opens page `{}`", id))?;
    let parent = loaded
      .config
      .screens
      .iter()
      .find(|s| !s.page && layout_has(&s.layout, &opener.id))
      .map(|s| s.id.clone())
      .unwrap_or_else(|| loaded.config.hub.start.clone());
    app.screen = parent;
    terminal.draw(|f| app.draw(f)).map_err(|e| e.to_string())?;
    let row = app
      .selected
      .get(&opener.id)
      .cloned()
      .ok_or_else(|| format!("{} has no rows to open", opener.id))?;
    app.pages.push((id.to_string(), row));
  } else {
    app.screen = id.to_string();
  }
  // Twice: the first draw learns the selections the second shows.
  terminal.draw(|f| app.draw(f)).map_err(|e| e.to_string())?;
  terminal.draw(|f| app.draw(f)).map_err(|e| e.to_string())?;
  if plain {
    return Ok(app.plain());
  }
  let buffer = terminal.backend().buffer().clone();
  let mut out = String::new();
  for y in 0..buffer.area.height {
    let mut line = String::new();
    for x in 0..buffer.area.width {
      line.push_str(buffer[(x, y)].symbol());
    }
    out.push_str(line.trim_end());
    out.push('\n');
  }
  Ok(out)
}

/// On a panic, the terminal is given back as it was found before the message is printed.
fn restore_terminal_on_panic() {
  let default = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    let _ = disable_raw_mode();
    let _ = execute!(
      std::io::stdout(),
      DisableMouseCapture,
      DisableBracketedPaste,
      LeaveAlternateScreen,
      crossterm::cursor::Show
    );
    default(info);
  }));
}

/// Runs a command on the terminal itself, then waits for enter. ctrl+c and ctrl+\\ stop
/// the command, not the hub: the hub ignores them until it ends, the command does not.
fn run_on_terminal(base: &std::path::Path, line: &str) {
  use std::os::unix::process::CommandExt;
  println!("$ {}", line);
  let words = source::words(line);
  if let Some((program, args)) = words.split_first() {
    let ignore = [libc::SIGINT, libc::SIGQUIT];
    // SAFETY: setting a signal's disposition to ignore or default is async-signal-safe and
    // touches no memory; the previous dispositions are restored below.
    let before: Vec<libc::sighandler_t> = ignore
      .iter()
      .map(|s| unsafe { libc::signal(*s, libc::SIG_IGN) })
      .collect();
    let mut command = std::process::Command::new(program);
    command.args(args).current_dir(base);
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
      command.pre_exec(move || {
        for s in ignore {
          libc::signal(s, libc::SIG_DFL);
        }
        Ok(())
      });
    }
    let status = command.status();
    for (s, handler) in ignore.iter().zip(before) {
      // SAFETY: as above.
      unsafe {
        libc::signal(*s, handler);
      }
    }
    match status {
      Ok(status) => println!("\n[{}] press enter to return to the hub", status),
      Err(e) => println!("cannot run {}: {}; press enter to return", program, e),
    }
  }
  let _ = std::io::stdout().flush();
  let mut buffer = String::new();
  let _ = std::io::stdin().read_line(&mut buffer);
}

/// Takes the mouse from the terminal, or gives it back.
fn set_mouse(on: bool) {
  let _ = if on {
    execute!(std::io::stdout(), EnableMouseCapture)
  } else {
    execute!(std::io::stdout(), DisableMouseCapture)
  };
}

/// The interface, until quit. The session is saved as it changes and on quit, and taken
/// up again on start unless `fresh`. When the config's files change (or on `reload`), it
/// is read again and the interface goes on with it, its terminals and actions running.
/// Whether the terminal's background is light, as it answers OSC 11 within a moment;
/// `None` when it does not answer.
#[cfg(unix)]
fn terminal_is_light() -> Option<bool> {
  use std::io::Read;
  use std::os::fd::AsRawFd;
  let mut tty = std::fs::OpenOptions::new()
    .read(true)
    .write(true)
    .open("/dev/tty")
    .ok()?;
  enable_raw_mode().ok()?;
  let answer = (|| {
    tty.write_all(b"\x1b]11;?\x07").ok()?;
    tty.flush().ok()?;
    let mut got = Vec::new();
    let mut buf = [0u8; 64];
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(150) {
      let mut fds = libc::pollfd {
        fd: tty.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
      };
      // SAFETY: one pollfd for a file we hold open.
      let ready = unsafe { libc::poll(&mut fds, 1, 20) };
      if ready > 0 {
        let n = tty.read(&mut buf).ok()?;
        got.extend_from_slice(&buf[..n]);
        if got.contains(&0x07) || got.windows(2).any(|w| w == b"\x1b\\") {
          break;
        }
      }
    }
    Some(String::from_utf8_lossy(&got).into_owned())
  })();
  let _ = disable_raw_mode();
  // `\x1b]11;rgb:RRRR/GGGG/BBBB`: light when bright.
  let rgb = answer?.split("rgb:").nth(1)?.to_string();
  let parts: Vec<f64> = rgb
    .split('/')
    .take(3)
    .filter_map(|p| {
      let hex: String = p.chars().take_while(char::is_ascii_hexdigit).collect();
      let max = 16f64.powi(hex.len() as i32) - 1.0;
      u32::from_str_radix(&hex, 16)
        .ok()
        .map(|v| f64::from(v) / max)
    })
    .collect();
  let [r, g, b] = parts.as_slice() else {
    return None;
  };
  Some(0.2126 * r + 0.7152 * g + 0.0722 * b > 0.5)
}

#[cfg(not(unix))]
fn terminal_is_light() -> Option<bool> {
  None
}
