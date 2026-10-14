//! The interface: screens and pages, focus and selection, actions, overlays (help,
//! confirmation, alerts, palette and forms), toasts, and the top and bottom bars.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::bindings::{self, Lookup};
use crate::config::{ActionSpec, Loaded};
use crate::expr::{Row, Value};
use crate::keys;
use crate::layout;
use crate::source::{Store, words};
use crate::terminal::Terminal;
use crate::widgets::{self, Context, Draw, Styles};

/// The keys the pane picker numbers panes with, in order.
const PICK_KEYS: &str = "123456789abcdefghijklmnopqrstuvwxyz";

/// The places kept to go back to.
const PLACES_KEPT: usize = 100;

/// How long `s` in the alerts drawer silences an alert's key.
const SNOOZE: Duration = Duration::from_secs(60 * 60);

/// A key and what it does, for the footer and help.
type Hint = (String, String);

/// A place marked by a letter: the screen, the pages over it, the pane and its row.
type Marked = (String, Vec<(String, Row)>, Option<String>, Option<Row>);

/// One of the palette's matches: label, detail, kind, the command's row, or the line a
/// preset or recent run runs.
type PaletteItem = (String, String, &'static str, Option<Row>, Option<String>);

/// A place to go back to: a screen and the pages opened over it.
type Place = (String, Vec<(String, Row)>);

/// A preset of the palette: its name and the command line it runs.
type Preset = (String, String);

/// What a confirmation does on yes.
pub enum Confirmed {
  /// An action, and whether what it asks for was answered.
  Action(Box<ActionSpec>, bool),
  /// A command line from the palette's presets or recent runs.
  Line(String),
  /// Close a terminal widget's tab, stopping its program.
  CloseTab(String, usize),
  /// Run the newest undo.
  Undo,
  Quit,
}

/// One line of help.
pub enum HelpLine {
  Heading(String),
  Blank,
  /// Keys, what they do, and a note.
  Keys(String, String, String),
}

/// What the keys are doing, as vim names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
  Normal,
  /// Typing: into a terminal, a search or a form.
  Insert,
  /// The palette, which vim mode reads as its command line.
  Command,
}

/// What is drawn over the screen.
pub enum Overlay {
  None,
  /// Help, scrolled this many lines.
  Help(usize),
  /// The alerts drawer, the alert chosen.
  Alerts(usize),
  Confirm {
    then: Confirmed,
    text: String,
  },
  Palette {
    query: String,
    at: usize,
  },
  Form(Box<Form>),
  FormHelp(Box<Form>),
  /// Each pane numbered; typing a number goes to that pane.
  Pick,
  /// What to copy of the focused pane: its row as JSON, its rows as TSV or Markdown.
  Yank,
  /// The fields an action asks for, being filled in.
  Ask(Box<Ask>),
  /// A terminal tab's new name, being typed.
  Rename {
    widget: String,
    at: usize,
    text: String,
  },
}

/// One entry of the alerts drawer: a new row an `[[alert]]` matched, or a failure.
#[derive(Clone, Debug)]
pub struct Alert {
  pub text: String,
  pub style: Style,
  /// The page enter opens for it, and its row.
  pub open: Option<(String, Row)>,
  /// An `[[alert]]`'s key (`source:value`): it resolves when that row no longer matches.
  pub key: Option<String>,
  pub since: std::time::SystemTime,
  /// When it stopped being true.
  pub resolved: Option<std::time::SystemTime>,
}

impl Alert {
  /// A failure or a problem, which does not resolve on its own.
  pub fn failure(text: String, style: Style) -> Alert {
    Alert {
      text,
      style,
      open: None,
      key: None,
      since: std::time::SystemTime::now(),
      resolved: None,
    }
  }
}

pub struct App<'a> {
  pub loaded: &'a Loaded,
  pub styles: Styles,
  pub store: Store,
  pub screen: String,
  /// Pages opened, each with the row it was opened for; the last is shown.
  pub pages: Vec<(String, Row)>,
  pub focus: usize,
  pub selection: BTreeMap<String, usize>,
  pub scroll: BTreeMap<String, usize>,
  pub selected: BTreeMap<String, Row>,
  pub overlay: Overlay,
  pub toasts: Vec<(String, Style, Instant)>,
  pub alerts: Vec<Alert>,
  pub seen: HashSet<String>,
  /// Alert keys silenced until a time (`s` in the drawer), kept across restarts.
  snoozed: HashMap<String, std::time::SystemTime>,
  /// Texts waiting to be sent to the desktop (`[hub] notify`).
  notifications: Vec<String>,
  /// How to take back the actions that succeeded and can be, newest last.
  undos: Vec<Undo>,
  /// What was last copied, for tests (the clipboard is the terminal's).
  last_yank: String,
  pub search: Option<String>,
  pub typing_search: bool,
  pub quit: bool,
  /// Set when a `terminal` action must run outside the interface.
  pub on_terminal: Option<String>,
  jobs: Vec<Job>,
  vars: Row,
  /// The widgets of the current screen that take focus, in order.
  focusable: Vec<String>,
  started_alerts: bool,
  /// The terminal widgets' tabs, by widget id; their programs run until the hub ends or
  /// their tab is closed.
  terminals: BTreeMap<String, Tabs>,
  /// The terminal widget that has the keyboard.
  pub captured: Option<String>,
  /// Whether terminal widgets start their programs when shown; off when drawing as text.
  pub start_terminals: bool,
  /// Whether vim's keys and modes are on.
  pub vim: bool,
  /// Keys typed so far of a binding of several (`g g`, `ctrl+w w`).
  typed: Vec<KeyEvent>,
  /// A count typed before a binding, with vim keys on (`5` of `5j`).
  count: String,
  /// Set by back and forward, which must not leave a place of their own.
  jumped: bool,
  /// Each pane's height inside its border, for paging.
  heights: BTreeMap<String, u16>,
  /// The panes that take focus and where they are, as last drawn.
  rects: Vec<(String, Rect)>,
  /// The pane focused before this one on the same screen, for `pane_last`.
  previous_pane: Option<String>,
  /// The pane shown over the whole screen, and the screen or page it is on.
  zoomed: Option<(String, String)>,
  /// A pane to focus when the screen is next drawn (the palette went to one of its rows).
  pending_focus: Option<String>,
  /// The history line each terminal's search is on.
  term_at: HashMap<String, usize>,
  /// Waiting for the letter of a place to mark (true) or to go to (false).
  place_pending: Option<bool>,
  /// Places marked, by letter: the screen, the pages over it, the pane, its row.
  places: HashMap<char, Marked>,
  /// Tables sorted by hand: the column, and whether descending.
  sorts: HashMap<String, (usize, bool)>,
  /// The rows marked in each pane, for an action on all of them.
  marks: HashMap<String, Vec<Row>>,
  /// The rows each selectable pane showed when last drawn.
  shown: HashMap<String, Vec<Row>>,
  /// Logs whose long lines are wrapped.
  wrapped: HashSet<String>,
  /// Logs with every section open.
  folds_open: HashSet<String>,
  /// A move asked of a log, for its next drawing: 1 forward, -1 back.
  log_jump: HashMap<String, i8>,
  /// The line each log's moves start from.
  log_at: HashMap<String, usize>,
  /// Each chart's window: halvings zoomed in, half windows moved back.
  windows: HashMap<String, (u32, i32)>,
  /// Where each chart's crosshair is along x.
  crosshairs: HashMap<String, f64>,
  /// A move of a chart's crosshair, for its next drawing.
  steps: HashMap<String, i8>,
  /// The chart the mouse is over, and the column.
  hover: Option<(String, u16)>,
  /// Each chart's pinned point.
  pins: HashMap<String, (f64, f64)>,
  /// The point under each chart's crosshair when last drawn.
  under: HashMap<String, (f64, f64)>,
  /// The pane focused on each screen or page, by id, so coming back focuses it again.
  focus_of: BTreeMap<String, String>,
  /// The screen or page the focus was last set for.
  focus_place: String,
  /// Rows selected in an earlier session, found again when their widget is drawn.
  pending_rows: BTreeMap<String, Row>,
  /// Terminal tabs of an earlier session, started again when their widget is shown.
  pending_tabs: BTreeMap<String, crate::session::Tabs>,
  /// Set by `reload`: the config is to be read again.
  pub reload_asked: bool,
  /// Where things were drawn, for the mouse.
  hits: Hits,
  /// Whether the hub takes the mouse; and whether that changed, for the terminal to be told.
  pub mouse_on: bool,
  pub mouse_changed: bool,
  /// The last click, to tell a double click: when, and where.
  last_click: Option<(Instant, u16, u16)>,
  /// Where the terminal's cursor goes this frame: where text is typed, if anywhere.
  cursor: Option<(u16, u16)>,
  /// Forms closed before they ran, by command, so opening one again finds it as left.
  drafts: BTreeMap<String, Form>,
  /// The presets as last read, and when their file had changed then.
  presets: Option<(Option<std::time::SystemTime>, Vec<Preset>)>,
  /// Where the keys went before and, after going back, where they went after: as a
  /// browser's back and forward, or vim's jump list.
  behind: Vec<Place>,
  ahead: Vec<Place>,
}

impl<'a> App<'a> {
  pub fn new(loaded: &'a Loaded) -> App<'a> {
    let mut vars = Row::new();
    vars.insert(
      "hub.name".into(),
      Value::Str(loaded.config.hub.name.clone()),
    );
    let mut app = App {
      loaded,
      styles: Styles::new(loaded, loaded.config.hub.contrast == Some(true)),
      store: Store::new(),
      screen: loaded.config.hub.start.clone(),
      pages: Vec::new(),
      focus: 0,
      selection: BTreeMap::new(),
      scroll: BTreeMap::new(),
      selected: BTreeMap::new(),
      overlay: Overlay::None,
      toasts: Vec::new(),
      alerts: Vec::new(),
      seen: HashSet::new(),
      snoozed: HashMap::new(),
      notifications: Vec::new(),
      undos: Vec::new(),
      last_yank: String::new(),
      search: None,
      typing_search: false,
      quit: false,
      on_terminal: None,
      jobs: Vec::new(),
      vars,
      focusable: Vec::new(),
      started_alerts: false,
      terminals: BTreeMap::new(),
      captured: None,
      start_terminals: true,
      vim: loaded.config.hub.vim.unwrap_or(false),
      typed: Vec::new(),
      count: String::new(),
      jumped: false,
      heights: BTreeMap::new(),
      rects: Vec::new(),
      previous_pane: None,
      zoomed: None,
      pending_focus: None,
      place_pending: None,
      term_at: HashMap::new(),
      places: HashMap::new(),
      sorts: HashMap::new(),
      marks: HashMap::new(),
      shown: HashMap::new(),
      wrapped: HashSet::new(),
      folds_open: HashSet::new(),
      log_jump: HashMap::new(),
      log_at: HashMap::new(),
      windows: HashMap::new(),
      crosshairs: HashMap::new(),
      steps: HashMap::new(),
      hover: None,
      pins: HashMap::new(),
      under: HashMap::new(),
      focus_of: BTreeMap::new(),
      focus_place: String::new(),
      pending_rows: BTreeMap::new(),
      pending_tabs: BTreeMap::new(),
      reload_asked: false,
      hits: Hits::default(),
      mouse_on: loaded.config.hub.mouse != Some(false),
      mouse_changed: false,
      last_click: None,
      cursor: None,
      drafts: BTreeMap::new(),
      presets: None,
      behind: Vec::new(),
      ahead: Vec::new(),
    };
    app.load_seen();
    app.load_snoozed();
    app.load_contrast();
    app.load_variables();
    app.load_vim();
    app
  }

  fn state_dir(&self) -> PathBuf {
    state_dir(self.loaded)
  }

  fn load_seen(&mut self) {
    let path = self.state_dir().join("alerts-seen.txt");
    if let Ok(text) = std::fs::read_to_string(&path) {
      self.seen = text.lines().map(str::to_string).collect();
      self.started_alerts = true;
    }
  }

  /// A toggle of vim mode outlives the session; the file holds `on` or `off`.
  fn load_vim(&mut self) {
    if let Ok(text) = std::fs::read_to_string(self.state_dir().join("vim.txt")) {
      self.vim = text.trim() == "on";
    }
  }

  /// Each `[[variable]]`'s value: as chosen last (kept in `[hub] state`), else its
  /// `default`, else its first value.
  fn load_variables(&mut self) {
    let kept: HashMap<String, String> =
      std::fs::read_to_string(self.state_dir().join("variables.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for v in self.loaded.config.variables.clone() {
      let value = kept
        .get(&v.name)
        .cloned()
        .or(v.default.clone())
        .or_else(|| self.variable_values(&v).into_iter().next())
        .unwrap_or_default();
      self
        .vars
        .insert(format!("var.{}", v.name), Value::Str(value));
    }
  }

  /// What a variable may be: its `values`, or the distinct values of `field` in
  /// `source`'s rows, in their order.
  fn variable_values(&mut self, v: &crate::config::VariableSpec) -> Vec<String> {
    if !v.values.is_empty() {
      return v.values.clone();
    }
    let (Some(source), Some(field)) = (&v.source, &v.field) else {
      return Vec::new();
    };
    let mut seen = HashSet::new();
    self
      .store
      .get(self.loaded, source, false)
      .rows
      .iter()
      .filter_map(|r| r.get(field).map(Value::text))
      .filter(|t| seen.insert(t.clone()))
      .collect()
  }

  /// Sets variable `name` (to the value after its own when `value` is `None`); every
  /// source is read again, as their `args` may read it. Whether there was one.
  fn set_variable(&mut self, name: &str, value: Option<&str>) -> bool {
    let Some(v) = self
      .loaded
      .config
      .variables
      .iter()
      .find(|v| v.name == name)
      .cloned()
    else {
      return false;
    };
    let key = format!("var.{}", name);
    let value = match value {
      Some(value) => value.to_string(),
      None => {
        let values = self.variable_values(&v);
        let now = self.vars.get(&key).map(Value::text).unwrap_or_default();
        let at = values.iter().position(|x| *x == now).map_or(0, |i| i + 1);
        values
          .get(at % values.len().max(1))
          .cloned()
          .unwrap_or_default()
      }
    };
    self.vars.insert(key, Value::Str(value.clone()));
    self.store.invalidate();
    let text: String = self
      .loaded
      .config
      .variables
      .iter()
      .map(|v| {
        let value = self
          .vars
          .get(&format!("var.{}", v.name))
          .map(Value::text)
          .unwrap_or_default();
        format!("{}\t{}\n", v.name, value)
      })
      .collect();
    let dir = self.state_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("variables.txt"), text);
    self.toast(&format!("{} = {}", name, value));
    true
  }

  /// High contrast on or off, kept for the next session as vim keys are.
  fn set_contrast(&mut self, on: bool) {
    self.styles = Styles::new(self.loaded, on);
    let dir = self.state_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("contrast.txt"), if on { "on\n" } else { "off\n" });
    self.toast(if on {
      "high contrast on"
    } else {
      "high contrast off"
    });
  }

  fn load_contrast(&mut self) {
    if let Ok(text) = std::fs::read_to_string(self.state_dir().join("contrast.txt")) {
      self.styles = Styles::new(self.loaded, text.trim() == "on");
    }
  }

  fn set_vim(&mut self, on: bool) {
    self.vim = on;
    self.typed.clear();
    self.count.clear();
    let dir = self.state_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("vim.txt"), if on { "on\n" } else { "off\n" });
    self.toast(if on { "vim keys on" } else { "vim keys off" });
  }

  /// Takes the mouse, or leaves it to the terminal, whose own selection of text it
  /// otherwise blocks.
  fn set_mouse(&mut self, on: bool) {
    self.mouse_on = on;
    self.mouse_changed = true;
    self.toast(if on {
      "the hub takes the mouse"
    } else {
      "the mouse is the terminal's: select text with it"
    });
  }

  pub fn toast(&mut self, text: &str) {
    self
      .toasts
      .push((text.to_string(), self.styles.get("text"), Instant::now()));
  }

  /// The mode the keys are in.
  pub fn mode(&self) -> Mode {
    match self.overlay {
      Overlay::Palette { .. } => Mode::Command,
      Overlay::Form(_) | Overlay::Rename { .. } | Overlay::Ask(_) => Mode::Insert,
      _ if self.captured.is_some() || self.typing_search => Mode::Insert,
      _ => Mode::Normal,
    }
  }

  /// Whether a terminal on the screen has the keys or is writing, so the screen must be
  /// redrawn often; an idle one is not.
  pub fn live_terminal(&self) -> bool {
    self.captured.is_some()
      || self
        .focusable
        .iter()
        .any(|w| self.tab(w).is_some_and(|t| t.running() && t.busy()))
  }

  /// Terminal widget `w`'s tab shown.
  fn tab(&self, w: &str) -> Option<&Terminal> {
    self.terminals.get(w).and_then(|t| t.list.get(t.at))
  }

  fn tab_mut(&mut self, w: &str) -> Option<&mut Terminal> {
    self.terminals.get_mut(w).and_then(|t| t.list.get_mut(t.at))
  }

  /// The focused widget, if it is a terminal.
  fn focused_terminal(&self) -> Option<String> {
    self
      .focused_widget()
      .filter(|w| self.loaded.widget(w).is_some_and(|s| s.kind == "terminal"))
      .map(str::to_string)
  }

  /// Starts terminal widget `w`'s program in a new tab after the others, or in place of
  /// tab `at`; shows it. Whether it started.
  fn start_tab(&mut self, w: &str, at: Option<usize>) -> bool {
    self.start_tab_from(w, at, None)
  }

  /// As `start_tab`, drawing `before` (a tab of an earlier session) first.
  fn start_tab_from(
    &mut self,
    w: &str,
    at: Option<usize>,
    before: Option<&crate::session::Tab>,
  ) -> bool {
    let Some(spec) = self.loaded.widget(w) else {
      return false;
    };
    let command = spec.command.clone().unwrap_or_default();
    let cwd = self.loaded.base.join(spec.cwd.as_deref().unwrap_or("."));
    // A program that asks for the colors is told the theme's.
    let theme = |name: &str| {
      self
        .loaded
        .config
        .theme
        .get(name)
        .and_then(|c| crate::terminal::rgb(c))
    };
    let colors = [theme("fg"), theme("bg")];
    // Kept in tmux: its own session, on the hub's own tmux server (`tmux -L hub`), the
    // same one again after a restart. tmux redraws the screen itself, and its own status
    // line is left out.
    let session = (spec.keep.as_deref() == Some("tmux")).then(|| {
      before
        .and_then(|b| b.session.clone())
        .unwrap_or_else(|| tmux_session(&self.loaded.base, w))
    });
    let command = match &session {
      Some(name) => {
        let quote = |t: &str| format!("'{}'", t.replace('\'', "'\"'\"'"));
        let inner = if command.trim().is_empty() {
          String::new()
        } else {
          quote(&command)
        };
        format!(
          "env -u TMUX tmux -L hub new-session -A -s {} {} ; set-option status off",
          quote(name),
          inner
        )
      }
      None => command,
    };
    let tabs = self.terminals.entry(w.to_string()).or_default();
    let (rows, cols) = tabs.size;
    let screen = before
      .filter(|_| session.is_none())
      .map(|b| b.screen.as_str());
    match Terminal::start(&command, &cwd, rows, cols, colors, screen) {
      Ok(mut t) => {
        t.custom = before.and_then(|b| b.name.clone());
        t.session = session;
        match at {
          Some(i) if i < tabs.list.len() => {
            tabs.list[i] = t;
            tabs.at = i;
          }
          _ => {
            tabs.list.push(t);
            tabs.at = tabs.list.len() - 1;
          }
        }
        true
      }
      Err(e) => {
        self.toasts.push((
          format!("{}: cannot start: {}", w, e),
          self.styles.get("bad"),
          Instant::now(),
        ));
        false
      }
    }
  }

  /// Closes terminal widget `w`'s tab `at`, stopping its program (and its tmux session,
  /// when it is kept in one).
  fn close_tab(&mut self, w: &str, at: usize) {
    if let Some(tabs) = self.terminals.get_mut(w)
      && at < tabs.list.len()
    {
      let closed = tabs.list.remove(at);
      if let Some(name) = &closed.session {
        let _ = std::process::Command::new("tmux")
          .args(["-L", "hub", "kill-session", "-t", name])
          .env_remove("TMUX")
          .output();
      }
      tabs.at = tabs.at.min(tabs.list.len().saturating_sub(1));
    }
    if self.captured.as_deref() == Some(w) {
      self.captured = None;
    }
  }

  /// The tab actions, on the focused terminal; whether there was one.
  fn tab_action(&mut self, action: &str, count: Option<i64>) -> bool {
    let Some(w) = self.focused_terminal() else {
      return false;
    };
    let n = self.terminals.get(&w).map_or(0, |t| t.list.len());
    match action {
      // The new tab is shown; the keys stay with the hub until `insert`.
      "tab_new" => {
        self.start_tab(&w, None);
      }
      "tab_close" => {
        let Some(tabs) = self.terminals.get(&w) else {
          return true;
        };
        let at = tabs.at;
        match tabs.list.get(at) {
          Some(t) if t.running() => {
            self.overlay = Overlay::Confirm {
              text: format!(
                "Close tab {} ({})?\nIts program is stopped.",
                at + 1,
                t.name()
              ),
              then: Confirmed::CloseTab(w, at),
            };
          }
          Some(_) => self.close_tab(&w, at),
          None => {}
        }
      }
      "tab_rename" if n > 0 => {
        let at = self.terminals.get(&w).map_or(0, |t| t.at);
        let text = self
          .tab(&w)
          .and_then(|t| t.custom.clone())
          .unwrap_or_default();
        self.overlay = Overlay::Rename {
          widget: w,
          at,
          text,
        };
      }
      "tab_next" | "tab_previous" if n > 0 => {
        let tabs = self.terminals.get_mut(&w).expect("n > 0");
        tabs.at = match (action, count) {
          // As vim's 3gt: the third tab.
          ("tab_next", Some(c)) => ((c - 1).max(0) as usize).min(n - 1),
          ("tab_next", None) => (tabs.at + 1) % n,
          _ => (tabs.at + n - (count.unwrap_or(1) as usize % n)) % n,
        };
      }
      _ => {}
    }
    true
  }

  /// Text pasted: to the terminal that has the keyboard, or the search being typed.
  pub fn paste(&mut self, text: &str) {
    if let Some(t) = self.captured.clone().and_then(|w| self.tab_mut(&w)) {
      t.paste(text);
    } else if self.typing_search {
      self.search.get_or_insert_with(String::new).push_str(text);
    }
  }

  /// Gives the focused terminal widget the keyboard, starting its tab's program again if
  /// it ended, or a first tab if it has none. Whether the focus was a terminal.
  fn capture(&mut self) -> bool {
    let Some(w) = self.focused_terminal() else {
      return false;
    };
    let at = self.terminals.get(&w).map_or(0, |t| t.at);
    let state = self.tab(&w).map(Terminal::running);
    let started = match state {
      None => self.start_tab(&w, None),
      Some(false) => self.start_tab(&w, Some(at)),
      Some(true) => true,
    };
    if started {
      self.captured = Some(w);
    }
    true
  }

  /// Keys while a terminal has the keyboard: all go to it but the keys that give them back
  /// (the terminal's `escape`, else `release`). Keys that start one are held until it is
  /// clear whether they finish it.
  fn terminal_key(&mut self, event: KeyEvent, w: String) {
    let escape = self.loaded.widget(&w).and_then(|s| s.escape_keys());
    let full_screen = self.tab(&w).is_some_and(|t| t.full_screen());
    self.typed.push(event);
    match self
      .loaded
      .bindings
      .release(&self.typed, self.vim, escape.as_deref(), full_screen)
    {
      Lookup::Actions(_) => {
        self.typed.clear();
        self.captured = None;
        return;
      }
      Lookup::Prefix => return,
      _ => {}
    }
    let held = std::mem::take(&mut self.typed);
    let running = self.tab(&w).is_some_and(Terminal::running);
    match self.tab_mut(&w) {
      Some(t) if running => {
        for e in &held {
          t.key(e);
        }
      }
      // The program ended: any key gives the keyboard back.
      _ => self.captured = None,
    }
  }

  /// Runs engine action `action`, `count` times where that means something. Whether it
  /// applied: one that did not leaves the keys to the next action bound to them.
  fn run(&mut self, action: &str, count: Option<i64>) -> bool {
    let n = count.unwrap_or(1);
    let height = self
      .focused_widget()
      .and_then(|w| self.heights.get(w))
      .copied()
      .unwrap_or(10)
      .max(2) as i64;
    // Going to another pane leaves the zoomed one, as tmux does.
    if matches!(
      action,
      "next"
        | "previous"
        | "pane_left"
        | "pane_right"
        | "pane_up"
        | "pane_down"
        | "pane_last"
        | "pane_pick"
    ) {
      self.zoomed = None;
    }
    match action {
      "zoom_in" | "zoom_out" | "up" | "down" | "sort_next" | "sort_reverse" | "mark"
        if self.focused_chart().is_some() =>
      {
        let w = self.focused_chart().unwrap_or_default();
        let (zoom, back) = self.windows.get(&w).copied().unwrap_or((0, 0));
        match action {
          "zoom_in" => {
            self.windows.insert(w, (zoom + 1, back * 2));
          }
          "zoom_out" => {
            self.windows.insert(w, (zoom.saturating_sub(1), back / 2));
          }
          // Later and earlier: half a window at a time, no later than the newest.
          "sort_next" => {
            self.windows.insert(w, (zoom, (back - 1).max(0)));
          }
          "sort_reverse" => {
            let most = 2i32.saturating_pow(zoom + 1) - 2;
            self.windows.insert(w, (zoom, (back + 1).min(most.max(0))));
          }
          "up" => {
            self.steps.insert(w, -1);
          }
          "down" => {
            self.steps.insert(w, 1);
          }
          _ => {
            // Pins the point under the crosshair, or unpins it.
            match (self.pins.contains_key(&w), self.under.get(&w).copied()) {
              (true, _) => {
                self.pins.remove(&w);
              }
              (false, Some(point)) => {
                self.pins.insert(w, point);
              }
              (false, None) => return false,
            }
          }
        }
      }
      "up" => self.move_selection(-n),
      "down" => self.move_selection(n),
      "half_up" => self.move_selection(-n * (height / 2)),
      "half_down" => self.move_selection(n * (height / 2)),
      "page_up" => self.move_selection(-n * height),
      "page_down" => self.move_selection(n * height),
      "top" | "bottom" => {
        if let Some(row) = count {
          self.move_selection(-1_000_000);
          self.move_selection(row - 1);
        } else {
          self.move_selection(if action == "top" {
            -1_000_000
          } else {
            1_000_000
          });
        }
      }
      "zoom_in" | "zoom_out" => return false,
      "next_match" | "previous_match"
        if self.focused_terminal().is_some() && self.search.is_some() =>
      {
        return self.terminal_find(action == "next_match");
      }
      "yank" if self.focused_terminal().is_some() => {
        let Some(w) = self.focused_terminal() else {
          return false;
        };
        let text = self.tab(&w).map(Terminal::shown_text).unwrap_or_default();
        copy_to_clipboard(&text);
        self.last_yank = text.clone();
        self.toast(&format!(
          "copied the {} line(s) shown (OSC 52)",
          text.lines().count()
        ));
      }
      "next_match" | "previous_match" | "wrap" | "fold" => {
        // Only a log has them; elsewhere the key is the project's.
        let Some(w) = self
          .focused_widget()
          .filter(|w| self.loaded.widget(w).is_some_and(|s| s.kind == "log"))
          .map(str::to_string)
        else {
          return false;
        };
        let toggle = |set: &mut HashSet<String>| {
          if !set.remove(&w) {
            set.insert(w.clone());
          }
        };
        match action {
          "wrap" => toggle(&mut self.wrapped),
          "fold" => toggle(&mut self.folds_open),
          _ => {
            self
              .log_jump
              .insert(w.clone(), if action == "next_match" { 1 } else { -1 });
          }
        }
      }
      "mark" | "mark_all" => {
        let Some(w) = self.focused_widget().map(str::to_string) else {
          return false;
        };
        if self
          .loaded
          .widget(&w)
          .is_none_or(|s| s.selectable != Some(true))
        {
          return false;
        }
        let marks = self.marks.entry(w.clone()).or_default();
        if action == "mark_all" {
          let shown = self.shown.get(&w).cloned().unwrap_or_default();
          let all = !shown.is_empty()
            && shown
              .iter()
              .all(|r| marks.iter().any(|m| crate::session::same_row(m, r)));
          *marks = if all { Vec::new() } else { shown };
        } else {
          let Some(row) = self.selected.get(&w).cloned() else {
            return false;
          };
          match marks.iter().position(|m| crate::session::same_row(m, &row)) {
            Some(at) => {
              marks.remove(at);
            }
            None => marks.push(row),
          }
          self.move_selection(1);
        }
      }
      "sort_next" | "sort_reverse" => {
        let Some(w) = self.focused_widget().map(str::to_string) else {
          return false;
        };
        let columns = self
          .loaded
          .widget(&w)
          .filter(|s| s.kind == "table")
          .map_or(0, |s| s.columns.len());
        if columns == 0 {
          return false;
        }
        let next = match (action, self.sorts.get(&w).copied()) {
          ("sort_reverse", Some((c, d))) => Some((c, !d)),
          ("sort_reverse", None) => Some((0, true)),
          // Past the last column, back to the order the config gives.
          (_, Some((c, _))) => (c + 1 < columns).then_some((c + 1, false)),
          (_, None) => Some((0, false)),
        };
        self.sort(&w, next);
      }
      "zoom" => {
        let Some(w) = self.focused_widget().map(str::to_string) else {
          return false;
        };
        let place = self.focus_place.clone();
        self.zoomed = match &self.zoomed {
          Some((p, z)) if *p == place && *z == w => None,
          _ => Some((place, w)),
        };
      }
      "next" => match count {
        // As vim's 3 ctrl+w w: the third pane.
        Some(c) if c >= 1 && (c as usize) <= self.focusable.len() => self.focus = c as usize - 1,
        _ => self.cycle_focus(n),
      },
      "pane_left" | "pane_right" | "pane_up" | "pane_down" => {
        for _ in 0..n {
          if !self.focus_toward(&action[5..]) {
            break;
          }
        }
      }
      "pane_last" => {
        let Some(at) = self
          .previous_pane
          .as_ref()
          .and_then(|p| self.focusable.iter().position(|w| w == p))
        else {
          return false;
        };
        self.focus = at;
      }
      "pane_pick" => {
        if self.focusable.len() < 2 {
          return false;
        }
        self.overlay = Overlay::Pick;
      }
      "previous" => self.cycle_focus(-n),
      "screen_next" => match count {
        // As vim's 3gt: the third screen.
        Some(c) => self.nth_screen(c),
        None => self.cycle_screen(1),
      },
      "screen_previous" => self.cycle_screen(-n),
      "open" => return self.open_selected(),
      "back" => {
        let marked = self
          .focused_widget()
          .is_some_and(|w| self.marks.get(w).is_some_and(|m| !m.is_empty()));
        if marked {
          let w = self.focused_widget().unwrap_or_default().to_string();
          self.marks.remove(&w);
        } else if self.search.is_some() {
          self.search = None;
        } else if self.pages.pop().is_some() {
          self.focus = 0;
        } else {
          return false;
        }
      }
      "jump_back" => self.jump(false),
      "jump_forward" => self.jump(true),
      "insert" => return self.capture(),
      "tab_new" | "tab_close" | "tab_next" | "tab_previous" | "tab_rename" => {
        return self.tab_action(action, count);
      }
      "search" => {
        self.typing_search = true;
        self.search = Some(String::new());
      }
      "palette" => {
        self.overlay = Overlay::Palette {
          query: String::new(),
          at: 0,
        };
      }
      "alerts" => self.overlay = Overlay::Alerts(0),
      "refresh" => self.store.invalidate(),
      "cancel" => self.cancel_job(),
      "mark_place" => {
        self.place_pending = Some(true);
        self.toast("mark this place as… (a letter)");
      }
      "go_place" => {
        if self.places.is_empty() {
          return false;
        }
        self.place_pending = Some(false);
        let names: String = {
          let mut names: Vec<char> = self.places.keys().copied().collect();
          names.sort();
          names.into_iter().collect()
        };
        self.toast(&format!("go to the place marked… ({})", names));
      }
      "yank" => {
        if self
          .focused_widget()
          .and_then(|w| self.loaded.widget(w))
          .is_none_or(|s| !matches!(s.kind.as_str(), "table" | "list"))
        {
          return false;
        }
        self.overlay = Overlay::Yank;
      }
      "undo" => self.ask_undo(),
      "help" => self.overlay = Overlay::Help(0),
      "vim" => self.set_vim(!self.vim),
      "reload" => self.reload_asked = true,
      "mouse" => self.set_mouse(!self.mouse_on),
      "quit" if !self.jobs.is_empty() => {
        let running: Vec<String> = self.jobs.iter().map(|j| j.label.clone()).collect();
        self.overlay = Overlay::Confirm {
          text: format!(
            "{} still running: {}.\nQuit anyway? It goes on without the hub, its output not kept.",
            if running.len() == 1 {
              "An action is"
            } else {
              "Actions are"
            },
            running.join(", ")
          ),
          then: Confirmed::Quit,
        };
      }
      "quit" => self.quit = true,
      _ => return false,
    }
    true
  }

  /// The project's action on `event` where the cursor is; whether there was one.
  fn project_action(&mut self, event: &KeyEvent) -> bool {
    let variable = self
      .loaded
      .config
      .variables
      .iter()
      .find(|v| {
        v.key
          .as_deref()
          .and_then(keys::parse)
          .is_some_and(|k| k.matches(event))
      })
      .map(|v| v.name.clone());
    if let Some(name) = variable {
      return self.set_variable(&name, None);
    }
    for action in self.actions() {
      if keys::parse(&action.key).is_some_and(|k| k.matches(event)) {
        let action = action.clone();
        self.start(&action, false);
        return true;
      }
    }
    false
  }

  /// The `n`th screen with a key, counting from 1.
  fn nth_screen(&mut self, n: i64) {
    let id = self
      .loaded
      .config
      .screens
      .iter()
      .filter(|s| !s.page)
      .nth((n - 1).max(0) as usize)
      .map(|s| s.id.clone());
    if let Some(id) = id {
      self.go(&id);
    }
  }

  /// Focuses the nearest pane in `direction` (`left`, `right`, `up`, `down`) from the
  /// focused one, by where they are drawn: the closest edge, then the most overlap
  /// across, then the nearest middle. Whether there was one.
  fn focus_toward(&mut self, direction: &str) -> bool {
    let Some(here) = self
      .focused_widget()
      .and_then(|w| self.rects.iter().find(|(id, _)| id == w))
      .map(|(_, r)| *r)
    else {
      return false;
    };
    let edges = |x: Rect| {
      (
        x.x as i32,
        x.x as i32 + x.width as i32,
        x.y as i32,
        x.y as i32 + x.height as i32,
      )
    };
    let (hl, hr, ht, hb) = edges(here);
    let mut best: Option<((i32, i32, i32), String)> = None;
    for (id, rect) in &self.rects {
      let (cl, cr, ct, cb) = edges(*rect);
      // The gap to it along the way, and how much the two overlap across it.
      let (gap, overlap, offset) = match direction {
        "left" if cr <= hl => (hl - cr, hb.min(cb) - ht.max(ct), (ct + cb) - (ht + hb)),
        "right" if cl >= hr => (cl - hr, hb.min(cb) - ht.max(ct), (ct + cb) - (ht + hb)),
        "up" if cb <= ht => (ht - cb, hr.min(cr) - hl.max(cl), (cl + cr) - (hl + hr)),
        "down" if ct >= hb => (ct - hb, hr.min(cr) - hl.max(cl), (cl + cr) - (hl + hr)),
        _ => continue,
      };
      let score = (gap, -overlap.max(0), offset.abs());
      if best.as_ref().is_none_or(|(s, _)| score < *s) {
        best = Some((score, id.clone()));
      }
    }
    match best.and_then(|(_, id)| self.focusable.iter().position(|w| *w == id)) {
      Some(at) => {
        self.focus = at;
        true
      }
      None => false,
    }
  }

  fn cycle_focus(&mut self, by: i64) {
    let n = self.focusable.len().max(1) as i64;
    self.focus = (self.focus as i64 + by).rem_euclid(n) as usize;
  }

  /// The next or previous screen with a key, as vim's gt and gT go through tabs.
  fn cycle_screen(&mut self, by: i64) {
    let screens: Vec<String> = self
      .loaded
      .config
      .screens
      .iter()
      .filter(|s| !s.page)
      .map(|s| s.id.clone())
      .collect();
    if screens.is_empty() {
      return;
    }
    let at = screens.iter().position(|s| *s == self.screen).unwrap_or(0) as i64;
    let next = (at + by).rem_euclid(screens.len() as i64) as usize;
    let id = screens[next].clone();
    self.go(&id);
  }

  /// What `line` does as a command of its own (see `ex`), to show before it is run.
  fn ex_meaning(&self, line: &str) -> Option<String> {
    let line = line.trim();
    Some(match line {
      "set vim" => "vim keys on".into(),
      "set novim" => "vim keys off".into(),
      "set vim!" | "set invvim" => "vim keys on or off".into(),
      "reload" | "e" | "e!" | "edit" => "read the config again".into(),
      "w" => "save this screen as text in the state folder".into(),
      _ if line.starts_with("let ") => match line[4..].trim().split_once([' ', '=']) {
        Some((name, value)) => format!("set {} to {}", name.trim(), value.trim()),
        None => return None,
      },
      "set contrast" => "high contrast: the terminal's own colors, strong".into(),
      "set nocontrast" => "the theme's colors".into(),
      _ if line.starts_with("w ") => format!("save this screen as text in {}", line[2..].trim()),
      "set mouse" | "set mouse=a" => "take the mouse".into(),
      "set nomouse" | "set mouse=" => "leave the mouse to the terminal (to select text)".into(),
      _ if !self.vim => return None,
      "q" | "q!" | "qa" | "qa!" | "wq" | "x" | "quit" => "quit the hub".into(),
      _ => {
        if let Ok(n) = line.parse::<i64>() {
          format!("go to row {}", n)
        } else if self.loaded.screen(line).is_some_and(|s| !s.page) {
          format!("go to screen {}", line)
        } else {
          return None;
        }
      }
    })
  }

  /// A palette line read as an ex command: `set vim`, `set novim`, `set vim!` toggle vim
  /// keys; in vim mode, `q`, `qa`, `wq`, `x` quit, a number goes to that row and a
  /// screen's id goes to that screen. Whether `line` was one.
  fn ex(&mut self, line: &str) -> bool {
    let line = line.trim();
    match line {
      "set vim" => self.set_vim(true),
      "set novim" => self.set_vim(false),
      "set vim!" | "set invvim" => self.set_vim(!self.vim),
      "reload" | "e" | "e!" | "edit" => self.reload_asked = true,
      "w" => self.write_screen(None),
      _ if line.starts_with("let ") => {
        let Some((name, value)) = line[4..].trim().split_once([' ', '=']) else {
          return false;
        };
        return self.set_variable(name.trim(), Some(value.trim()));
      }
      "set contrast" => self.set_contrast(true),
      "set nocontrast" => self.set_contrast(false),
      _ if line.starts_with("w ") => self.write_screen(Some(line[2..].trim())),
      "set mouse" | "set mouse=a" => self.set_mouse(true),
      "set nomouse" | "set mouse=" => self.set_mouse(false),
      _ if !self.vim => return false,
      "q" | "q!" | "qa" | "qa!" | "wq" | "x" | "quit" => self.quit = true,
      _ => {
        if let Ok(n) = line.parse::<i64>() {
          self.move_selection(-1_000_000);
          self.move_selection(n - 1);
        } else if self.loaded.screen(line).is_some_and(|s| !s.page) {
          self.go(line);
        } else {
          return false;
        }
      }
    }
    true
  }

  /// Everything to go on with after the config is read again.
  pub fn into_carry(mut self) -> Carry {
    let mut session = self.session_all();
    // The terminals go on running, so nothing of theirs is to be restored.
    session.terminals.clear();
    Carry {
      session,
      terminals: std::mem::take(&mut self.terminals),
      captured: self.captured.take(),
      jobs: std::mem::take(&mut self.jobs),
      toasts: std::mem::take(&mut self.toasts),
      alerts: std::mem::take(&mut self.alerts),
      seen: std::mem::take(&mut self.seen),
      vim: self.vim,
    }
  }

  /// The hub on a config read again, going on from `carry`.
  pub fn from_carry(loaded: &'a Loaded, carry: Carry) -> App<'a> {
    let mut app = App::new(loaded);
    app.restore(carry.session);
    // A terminal widget the new config no longer has is closed.
    app.terminals = carry
      .terminals
      .into_iter()
      .filter(|(w, _)| loaded.widget(w).is_some_and(|s| s.kind == "terminal"))
      .collect();
    app.captured = carry.captured.filter(|w| app.terminals.contains_key(w));
    app.jobs = carry.jobs;
    app.toasts = carry.toasts;
    app.alerts = carry.alerts;
    app.seen = carry.seen;
    app.started_alerts = true;
    app.vim = carry.vim;
    app.toast("read the config again");
    app
  }

  /// Takes the config's problems: the old config goes on, and the drawer lists them.
  pub fn config_problems(&mut self, problems: &[String]) {
    let bad = self.styles.get("bad");
    for p in problems {
      self
        .alerts
        .push(Alert::failure(format!("config: {}", p), bad));
    }
    self.toasts.push((
      format!(
        "the config has {} problem(s); the one before goes on ({} lists them)",
        problems.len(),
        self
          .loaded
          .bindings
          .first("alerts", self.vim)
          .unwrap_or_default()
      ),
      bad,
      Instant::now(),
    ));
  }

  /// The whole session, whatever `[hub] persist` keeps: for a reload.
  fn session_all(&self) -> crate::session::Session {
    let mut out = self.session_with(|_| true);
    out.terminals.clear();
    out
  }

  /// The screen as plain lines, for a screen reader or a pipe: each pane's title, then
  /// what it shows as text, the selected row marked `>`; no drawing characters. Drawn
  /// first, so selections are known.
  pub fn plain(&mut self) -> String {
    let (id, page) = {
      let (id, page) = self.current();
      (id.to_string(), page.cloned())
    };
    let panes: Vec<String> = self.hits.panes.iter().map(|p| p.widget.clone()).collect();
    let mut out = String::new();
    let title = self
      .loaded
      .screen(&id)
      .map(|s| self.context(page.as_ref()).render(&s.title, None))
      .unwrap_or_default();
    out.push_str(&format!("# {}\n", title));
    for w in panes {
      let Some(spec) = self.loaded.widget(&w) else {
        continue;
      };
      let cx = Context {
        loaded: self.loaded,
        styles: &self.styles,
        page: page.as_ref(),
        selected: &self.selected,
        vars: &self.vars,
      };
      let (rows, fresh) = widgets::rows(spec, &mut self.store, &cx, None);
      let error = fresh.error;
      let first = if spec.row.as_deref() == Some("last") {
        rows.last()
      } else {
        rows.first()
      };
      let heading = spec
        .title
        .as_ref()
        .map(|t| {
          cx.render(
            t,
            matches!(spec.kind.as_str(), "text" | "kv")
              .then_some(first)
              .flatten(),
          )
        })
        .unwrap_or_else(|| w.clone());
      out.push_str(&format!("\n## {}\n", heading));
      if let Some(e) = error {
        out.push_str(&format!("{}\n", e));
        continue;
      }
      let selected = self
        .selection
        .get(&w)
        .copied()
        .filter(|_| spec.selectable == Some(true));
      let mark = |i: usize| if selected == Some(i) { "> " } else { "  " };
      match spec.kind.as_str() {
        "table" => {
          let header: Vec<&str> = spec.columns.iter().map(|c| c.header.as_str()).collect();
          out.push_str(&format!("  {}\n", header.join(" | ")));
          for (i, row) in rows.iter().enumerate() {
            let cells: Vec<String> = spec
              .columns
              .iter()
              .map(|c| cx.render(&c.value, Some(row)))
              .collect();
            out.push_str(&format!("{}{}\n", mark(i), cells.join(" | ")));
          }
        }
        "list" => {
          for (i, row) in rows.iter().enumerate() {
            out.push_str(&format!(
              "{}{}\n",
              mark(i),
              cx.render(spec.item.as_deref().unwrap_or(""), Some(row))
            ));
          }
        }
        "text" => {
          for line in &spec.lines {
            out.push_str(&format!("{}\n", cx.render(line, first)));
          }
        }
        "kv" => {
          for (k, v) in &spec.pairs {
            out.push_str(&format!("{}: {}\n", k, cx.render(v, first)));
          }
        }
        "gauges" | "bars" | "sparklines" => {
          for row in &rows {
            let label = cx.render(spec.label.as_deref().unwrap_or(""), Some(row));
            let text = spec
              .text
              .as_ref()
              .map(|t| cx.render(t, Some(row)))
              .unwrap_or_default();
            let value = spec
              .value
              .as_ref()
              .map(|v| cx.eval(v, Some(row)).text())
              .unwrap_or_default();
            // The text beside the bar says it in words when there is one.
            let said = if text.trim().is_empty() { value } else { text };
            out.push_str(&format!("{}: {}\n", label.trim(), said.trim()));
          }
        }
        "chart" => out.push_str(&format!("a chart of {} point(s)\n", rows.len())),
        "log" | "file" => {
          let path = self
            .loaded
            .base
            .join(cx.render(spec.path.as_deref().unwrap_or(""), None));
          let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| format!("{} cannot be read\n", path.display()));
          let lines: Vec<&str> = text.lines().collect();
          let shown = if spec.kind == "log" {
            &lines[lines.len().saturating_sub(20)..]
          } else {
            &lines[..lines.len().min(40)]
          };
          for line in shown {
            out.push_str(&format!("{}\n", line));
          }
        }
        "terminal" => out.push_str(&format!(
          "a terminal running {}\n",
          spec.command.as_deref().unwrap_or("the user's shell")
        )),
        _ => {}
      }
    }
    out
  }

  /// Where the actions' commands and their output are kept.
  fn log_path(&self) -> PathBuf {
    self.state_dir().join("actions.log")
  }

  /// Where the session is kept.
  pub fn session_path(&self) -> PathBuf {
    self.state_dir().join("session.json")
  }

  /// The session as it stands, with the parts `[hub] persist` keeps.
  pub fn session(&self) -> crate::session::Session {
    let hub = &self.loaded.config.hub;
    self.session_with(|part| hub.persists(part))
  }

  fn session_with(&self, keeps: impl Fn(&str) -> bool) -> crate::session::Session {
    use crate::session::{Session, Tab, Tabs as SavedTabs, row_to_json};
    let mut out = Session::new();
    let place = |p: &Place| {
      (
        p.0.clone(),
        p.1
          .iter()
          .map(|(id, row)| (id.clone(), row_to_json(row)))
          .collect(),
      )
    };
    if keeps("place") {
      out.screen = Some(self.screen.clone());
      out.pages = place(&self.place()).1;
      out.focus = self.focus_of.clone();
      out.behind = self.behind.iter().map(place).collect();
      out.ahead = self.ahead.iter().map(place).collect();
    }
    if keeps("selection") {
      out.selected = self
        .selected
        .iter()
        .map(|(w, r)| (w.clone(), row_to_json(r)))
        .collect();
      out.selection = self.selection.clone();
      out.scroll = self.scroll.clone();
      out.search = self.search.clone();
    }
    if keeps("terminals") {
      for (w, tabs) in &self.terminals {
        out.terminals.insert(
          w.clone(),
          SavedTabs {
            at: tabs.at,
            tabs: tabs
              .list
              .iter()
              .map(|t| Tab {
                name: t.custom.clone(),
                session: t.session.clone(),
                screen: t.snapshot(),
              })
              .collect(),
          },
        );
      }
    }
    out
  }

  /// Comes back to `session`: what of it still makes sense with this config.
  pub fn restore(&mut self, session: crate::session::Session) {
    use crate::session::row_from_json;
    let loaded = self.loaded;
    let place = |p: &crate::session::Place| -> Option<Place> {
      loaded.screen(&p.0).filter(|s| !s.page)?;
      let pages: Vec<(String, Row)> = p
        .1
        .iter()
        .filter(|(id, _)| loaded.screen(id).is_some_and(|s| s.page))
        .map(|(id, row)| (id.clone(), row_from_json(row)))
        .collect();
      Some((p.0.clone(), pages))
    };
    if let Some(screen) = &session.screen
      && let Some((screen, pages)) = place(&(screen.clone(), session.pages.clone()))
    {
      self.screen = screen;
      self.pages = pages;
    }
    self.behind = session.behind.iter().filter_map(place).collect();
    self.ahead = session.ahead.iter().filter_map(place).collect();
    self.focus_of = session.focus;
    self.selection = session.selection;
    self.scroll = session.scroll;
    self.search = session.search;
    self.pending_rows = session
      .selected
      .iter()
      .map(|(w, r)| (w.clone(), row_from_json(r)))
      .collect();
    self.pending_tabs = session.terminals;
  }

  fn save_seen(&self) {
    let dir = self.state_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut keys: Vec<&String> = self.seen.iter().collect();
    keys.sort();
    let text: String = keys.iter().map(|k| format!("{}\n", k)).collect();
    let _ = std::fs::write(dir.join("alerts-seen.txt"), text);
  }

  /// The screen or page being shown, and the row a page was opened for.
  fn current(&self) -> (&str, Option<&Row>) {
    match self.pages.last() {
      Some((id, row)) => (id, Some(row)),
      None => (&self.screen, None),
    }
  }

  fn focused_widget(&self) -> Option<&str> {
    self.focusable.get(self.focus).map(String::as_str)
  }

  /// Checks alerts against fresh rows. The first look takes everything present as seen.
  /// A key that matches and was not seen fires: a toast and a desktop notification (none
  /// while it is snoozed) and an entry in the drawer. One that no longer matches resolves:
  /// its entry says so, and if it matches again it fires again.
  pub fn check_alerts(&mut self) {
    let loaded = self.loaded;
    let mut fresh = Vec::new();
    let mut matching: HashSet<String> = HashSet::new();
    let mut ready_sources: HashSet<&str> = HashSet::new();
    // Rows still loading are not yet the source's: until every alert's source has
    // loaded once, nothing is new and nothing is taken as seen.
    let mut ready = true;
    for alert in &loaded.config.alerts {
      let data = self.store.get(loaded, &alert.source, false);
      if data.updated.is_none() && data.error.is_none() {
        ready = false;
        continue;
      }
      // A source that failed keeps its last rows: nothing resolves on its account.
      if data.error.is_none() {
        ready_sources.insert(&alert.source);
      }
      let rows = data.rows.clone();
      for row in rows {
        let scope = crate::expr::Scope {
          row: Some(&row),
          vars: Some(&self.vars),
          ..Default::default()
        };
        if !loaded.expr(&alert.when).eval(&scope).truthy() {
          continue;
        }
        let key = format!(
          "{}:{}",
          alert.source,
          row.get(&alert.key).map(Value::text).unwrap_or_default()
        );
        matching.insert(key.clone());
        if self.seen.insert(key.clone()) && self.started_alerts {
          let text = loaded.template(&alert.text).render(&scope);
          let style = self.styles.get(alert.style.as_deref().unwrap_or("text"));
          fresh.push(Alert {
            text,
            style,
            open: alert.open.clone().map(|p| (p, row.clone())),
            key: Some(key),
            since: std::time::SystemTime::now(),
            resolved: None,
          });
        }
      }
    }
    // What no longer matches resolves.
    let before = self.seen.len();
    self.seen.retain(|k| {
      let source = k.split_once(':').map_or(k.as_str(), |(s, _)| s);
      !ready_sources.contains(source) || matching.contains(k)
    });
    let resolved = self.seen.len() != before;
    let now = std::time::SystemTime::now();
    for alert in self.alerts.iter_mut().filter(|a| a.resolved.is_none()) {
      if let Some(key) = &alert.key {
        let source = key.split_once(':').map_or(key.as_str(), |(s, _)| s);
        if ready_sources.contains(source) && !matching.contains(key) {
          alert.resolved = Some(now);
        }
      }
    }
    if (!self.started_alerts && ready) || !fresh.is_empty() || resolved {
      self.started_alerts |= ready;
      self.save_seen();
    }
    for alert in fresh {
      let snoozed = alert
        .key
        .as_ref()
        .and_then(|k| self.snoozed.get(k))
        .is_some_and(|until| *until > now);
      if !snoozed {
        self
          .toasts
          .push((alert.text.clone(), alert.style, Instant::now()));
        self.notify(&alert.text);
      }
      // The same key fired before: its old entry gives way to this one.
      self
        .alerts
        .retain(|a| a.key.is_none() || a.key != alert.key);
      self.alerts.insert(0, alert);
    }
  }

  /// Sends `text` to the desktop as `[hub] notify` says, when the hub draws next.
  fn notify(&mut self, text: &str) {
    self.notifications.push(text.to_string());
  }

  /// The escape sequences for the notifications waiting: OSC 9 and OSC 777 (which
  /// terminals such as iTerm2, kitty, WezTerm, foot and Windows Terminal turn into desktop
  /// notifications), or a bell, as `[hub] notify` says.
  pub fn take_notifications(&mut self) -> String {
    let how = self.loaded.config.hub.notify.as_deref().unwrap_or("osc");
    let mut out = String::new();
    for text in self.notifications.drain(..) {
      let clean: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
      if matches!(how, "osc" | "both") {
        out.push_str(&format!("\x1b]9;{}\x07", clean));
        out.push_str(&format!(
          "\x1b]777;notify;{};{}\x07",
          self.loaded.config.hub.name.replace(';', ","),
          clean.replace(';', ",")
        ));
      }
      if matches!(how, "bell" | "both") {
        out.push('\x07');
      }
    }
    out
  }

  fn load_snoozed(&mut self) {
    let path = self.state_dir().join("alerts-snoozed.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
      return;
    };
    let now = std::time::SystemTime::now();
    for line in text.lines() {
      if let Some((until, key)) = line.split_once('\t')
        && let Ok(seconds) = until.parse::<u64>()
      {
        let until = std::time::UNIX_EPOCH + Duration::from_secs(seconds);
        if until > now {
          self.snoozed.insert(key.to_string(), until);
        }
      }
    }
  }

  fn save_snoozed(&self) {
    let dir = self.state_dir();
    let _ = std::fs::create_dir_all(&dir);
    let now = std::time::SystemTime::now();
    let text: String = self
      .snoozed
      .iter()
      .filter(|(_, until)| **until > now)
      .map(|(key, until)| {
        let seconds = until
          .duration_since(std::time::UNIX_EPOCH)
          .map_or(0, |d| d.as_secs());
        format!("{}\t{}\n", seconds, key)
      })
      .collect();
    let _ = std::fs::write(dir.join("alerts-snoozed.txt"), text);
  }

  /// The focused pane, when it is a chart.
  fn focused_chart(&self) -> Option<String> {
    self
      .focused_widget()
      .filter(|w| self.loaded.widget(w).is_some_and(|s| s.kind == "chart"))
      .map(str::to_string)
  }

  /// Sorts table `w` by hand, or back as the config sorts it; the selected row stays
  /// selected.
  fn sort(&mut self, w: &str, by: Option<(usize, bool)>) {
    match by {
      Some(by) => self.sorts.insert(w.to_string(), by),
      None => self.sorts.remove(w),
    };
    if let Some(row) = self.selected.get(w).cloned() {
      self.pending_rows.insert(w.to_string(), row);
    }
  }

  /// Starts a command on a thread of its own: `before` first, when given, its output kept
  /// for an undo. The job, for the caller to finish and push.
  fn spawn_wait(
    &mut self,
    label: &str,
    line: &str,
    input: Option<String>,
    limit: Duration,
    before: Option<(String, std::sync::Arc<std::sync::Mutex<String>>)>,
  ) -> Job {
    let (send, done) = channel();
    let base = self.loaded.base.clone();
    let command_line = line.to_string();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = cancel.clone();
    std::thread::spawn(move || {
      if let Some((before_line, kept)) = before {
        match crate::source::run(&base, &words(&before_line), None, &[], limit, &[0]) {
          Ok(output) => *kept.lock().expect("not poisoned") = output.trim_end().to_string(),
          Err(e) => {
            let _ = send.send(Err(format!("before it: {}", e)));
            return;
          }
        }
      }
      let _ = send.send(crate::source::run_cancellable(
        &base,
        &words(&command_line),
        input.as_deref(),
        &[],
        limit,
        &[0],
        Some(&stop),
      ));
    });
    self.toasts.push((
      format!("{}…", label),
      self.styles.get("dim"),
      Instant::now(),
    ));
    Job {
      label: label.to_string(),
      line: line.to_string(),
      started: Instant::now(),
      done,
      pending: None,
      cancel,
      then: None,
      toast: None,
    }
  }

  /// `undo`: asks before taking back the last action that can be.
  fn ask_undo(&mut self) {
    let Some(undo) = self.undos.last() else {
      self.toast("nothing to undo");
      return;
    };
    self.overlay = Overlay::Confirm {
      then: Confirmed::Undo,
      text: format!("Undo {}?\nIt runs: {}", undo.label, undo.line),
    };
  }

  /// Runs the newest undo, as an action of its own that cannot be taken back.
  fn run_undo(&mut self) {
    let Some(undo) = self.undos.pop() else {
      return;
    };
    let label = format!("undo {}", undo.label);
    let job = self.spawn_wait(
      &label,
      &undo.line,
      undo.input,
      Duration::from_secs(60),
      None,
    );
    self.jobs.push(Job {
      then: Some("refresh".into()),
      ..job
    });
  }

  /// Copies the focused table or list: the selected row as JSON (`json`), or the rows it
  /// shows, as it shows them, as TSV (`tsv`) or a Markdown table (`md`).
  fn yank(&mut self, how: &str) {
    let Some(w) = self.focused_widget().map(str::to_string) else {
      return;
    };
    let Some(spec) = self.loaded.widget(&w) else {
      return;
    };
    let page = self.current().1.cloned();
    let text = if how == "json" {
      let Some(row) = self.selected.get(&w) else {
        self.toast("no row is selected");
        return;
      };
      serde_json::to_string_pretty(&crate::session::row_to_json(row)).unwrap_or_default()
    } else {
      let cx = Context {
        loaded: self.loaded,
        styles: &self.styles,
        page: page.as_ref(),
        selected: &self.selected,
        vars: &self.vars,
      };
      // As shown: a selectable pane's rows as last drawn; another's, read now.
      let rows = match self.shown.get(&w) {
        Some(rows) => rows.clone(),
        None => widgets::rows(spec, &mut self.store, &cx, self.sorts.get(&w).copied()).0,
      };
      let (headers, cells): (Vec<String>, Vec<Vec<String>>) = if spec.kind == "table" {
        (
          spec.columns.iter().map(|c| c.header.clone()).collect(),
          rows
            .iter()
            .map(|r| {
              spec
                .columns
                .iter()
                .map(|c| cx.render(&c.value, Some(r)))
                .collect()
            })
            .collect(),
        )
      } else {
        let item = spec.item.clone().unwrap_or_default();
        (
          vec![spec.title.clone().unwrap_or_else(|| w.clone())],
          rows
            .iter()
            .map(|r| vec![cx.render(&item, Some(r))])
            .collect(),
        )
      };
      if how == "tsv" {
        let clean = |c: &str| c.replace(['\t', '\n'], " ");
        let mut out = headers
          .iter()
          .map(|h| clean(h))
          .collect::<Vec<_>>()
          .join("\t");
        for row in &cells {
          out.push('\n');
          out.push_str(&row.iter().map(|c| clean(c)).collect::<Vec<_>>().join("\t"));
        }
        out
      } else {
        let clean = |c: &str| c.replace('|', "\\|").replace('\n', " ");
        let mut out = format!(
          "| {} |\n|{}|",
          headers
            .iter()
            .map(|h| clean(h))
            .collect::<Vec<_>>()
            .join(" | "),
          headers.iter().map(|_| "---").collect::<Vec<_>>().join("|")
        );
        for row in &cells {
          out.push_str(&format!(
            "\n| {} |",
            row.iter().map(|c| clean(c)).collect::<Vec<_>>().join(" | ")
          ));
        }
        out
      }
    };
    copy_to_clipboard(&text);
    self.last_yank = text.clone();
    let what = match how {
      "json" => "the row as JSON".to_string(),
      "tsv" => format!("{} line(s) as TSV", text.lines().count()),
      _ => format!("{} line(s) as Markdown", text.lines().count()),
    };
    self.toast(&format!(
      "copied {} (OSC 52, which some terminals ignore)",
      what
    ));
  }

  /// `:w [FILE]`: the screen as plain text, to FILE (from the config's folder) or to a new
  /// file in `[hub] state`.
  fn write_screen(&mut self, file: Option<&str>) {
    let text = self.plain();
    let path = match file {
      Some(f) => self.loaded.base.join(f),
      None => {
        let stamp = crate::template::now_stamp().replace([':', '-'], "");
        self
          .state_dir()
          .join("screens")
          .join(format!("{}-{}.txt", self.current().0, stamp))
      }
    };
    let written = path
      .parent()
      .map_or(Ok(()), std::fs::create_dir_all)
      .and_then(|_| std::fs::write(&path, text));
    match written {
      Ok(()) => self.toast(&format!("saved {}", path.display())),
      Err(e) => self.toast(&format!("cannot write {}: {}", path.display(), e)),
    }
  }

  /// In the focused terminal's history, the next (or previous) line with the search,
  /// scrolled to; whether there was one.
  fn terminal_find(&mut self, forward: bool) -> bool {
    let (Some(w), Some(query)) = (self.focused_terminal(), self.search.clone()) else {
      return false;
    };
    let Some(lines) = self.tab(&w).map(Terminal::history) else {
      return false;
    };
    let found: Vec<usize> = (0..lines.len())
      .filter(|&i| widgets::has(&lines[i], &query))
      .collect();
    if found.is_empty() {
      self.toast(&format!("no line has {}", query));
      return true;
    }
    let from = self.term_at.get(&w).copied();
    let next = match (forward, from) {
      (true, Some(f)) => found.iter().copied().find(|&i| i > f).unwrap_or(found[0]),
      (false, Some(f)) => found
        .iter()
        .rev()
        .copied()
        .find(|&i| i < f)
        .unwrap_or(found[found.len() - 1]),
      // From the newest, as a shell's history search goes.
      (_, None) => found[found.len() - 1],
    };
    self.term_at.insert(w.clone(), next);
    if let Some(t) = self.tab_mut(&w) {
      // A line or two of what came before it, above it.
      t.show_line(next.saturating_sub(2));
    }
    let which = found.iter().position(|&i| i == next).unwrap_or(0) + 1;
    self.toast(&format!(
      "/{}: {} of {}, line {}: {}",
      query,
      which,
      found.len(),
      next + 1,
      lines[next].trim()
    ));
    true
  }

  /// Marks where the hub is as place `name`: the screen, its pages, the focused pane and
  /// its selected row.
  fn set_place(&mut self, name: char) {
    let pane = self.focused_widget().map(str::to_string);
    let row = pane.as_ref().and_then(|w| self.selected.get(w).cloned());
    self
      .places
      .insert(name, (self.screen.clone(), self.pages.clone(), pane, row));
    self.toast(&format!("marked {}", name));
  }

  /// Goes back to place `name`, its row selected again wherever it is now.
  fn go_place(&mut self, name: char) {
    let Some((screen, pages, pane, row)) = self.places.get(&name).cloned() else {
      self.toast(&format!("no place marked {}", name));
      return;
    };
    self.go(&screen);
    self.pages = pages;
    if let Some(w) = pane {
      if let Some(row) = row {
        self.selection.entry(w.clone()).or_insert(0);
        self.pending_rows.insert(w.clone(), row);
      }
      self.pending_focus = Some(w);
    }
  }

  /// `cancel`: stops the newest action still running, and whatever its command started.
  fn cancel_job(&mut self) {
    let newest = self
      .jobs
      .iter()
      .rev()
      .find(|j| !j.cancel.load(std::sync::atomic::Ordering::Relaxed));
    let text = match newest {
      Some(job) => {
        job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        format!("stopping {}…", job.label)
      }
      None => "no action is running".to_string(),
    };
    self.toast(&text);
  }

  /// Finished commands: their toasts and what follows them.
  pub fn poll_jobs(&mut self) {
    // A terminal whose program ended gives the keys back at once.
    if let Some(w) = self.captured.clone()
      && !self.tab(&w).is_some_and(Terminal::running)
    {
      self.captured = None;
      self.toast("the program ended: the keys are the hub's again");
    }
    let mut done = Vec::new();
    let mut undone: Vec<(Pending, String)> = Vec::new();
    let log = self.log_path();
    self.jobs.retain(|job| match job.done.try_recv() {
      Ok(result) => {
        // Every action's command and what it printed, kept.
        if let Some(mut f) = log_file(
          &log,
          &format!(
            "{} ({:.1}s): {}",
            job.label,
            job.started.elapsed().as_secs_f64(),
            job.line
          ),
        ) {
          use std::io::Write;
          let _ = match &result {
            Ok(out) => write!(f, "{}", out),
            Err(e) => writeln!(f, "failed: {}", e),
          };
        }
        let cancelled = job.cancel.load(std::sync::atomic::Ordering::Relaxed);
        if let (Ok(output), Some(pending)) = (&result, &job.pending) {
          undone.push((pending_copy(pending), output.clone()));
        }
        done.push((
          job.label.clone(),
          result,
          job.then.clone(),
          job.toast.clone(),
          cancelled,
        ));
        false
      }
      Err(_) => true,
    });
    // What the actions that succeeded need to be taken back, read against what they
    // started with.
    for (pending, output) in undone {
      let mut vars = pending.vars.clone();
      let before = pending.before.lock().expect("not poisoned").clone();
      vars.insert("before".into(), Value::Str(before));
      vars.insert("output".into(), Value::Str(output.trim_end().to_string()));
      let cx = Context {
        loaded: self.loaded,
        styles: &self.styles,
        page: pending.page.as_ref(),
        selected: &pending.selected,
        vars: &vars,
      };
      let action = &pending.action;
      if let Some(undo) = &action.undo {
        let line = cx.render(undo, None);
        let input = action.undo_input.as_ref().map(|t| cx.render(t, None));
        self.undos.push(Undo {
          label: action.label.clone(),
          line,
          input,
        });
        // The last twenty.
        let excess = self.undos.len().saturating_sub(20);
        self.undos.drain(..excess);
      }
    }
    for (label, result, then, toast, cancelled) in done {
      // Stopped as asked: no failure.
      if cancelled && result.is_err() {
        let text = format!("{}: stopped", label);
        self
          .toasts
          .push((text, self.styles.get("dim"), Instant::now()));
        continue;
      }
      // A failure stays in the alerts drawer after its toast.
      if let Err(e) = &result {
        let bad = self.styles.get("bad");
        let text = format!("{} failed: {}", label, e);
        self.notify(&text);
        self.alerts.push(Alert::failure(text, bad));
      }
      match result {
        Ok(_) => {
          let text = toast.unwrap_or_else(|| format!("{}: done", label));
          self
            .toasts
            .push((text, self.styles.get("ok"), Instant::now()));
          self.after(then.as_deref());
        }
        Err(e) => self.toasts.push((
          format!("{}: {}", label, e),
          self.styles.get("bad"),
          Instant::now(),
        )),
      }
    }
    // A failure is left longer to be read; it is in the alerts drawer too.
    let bad = self.styles.get("bad");
    let motion = self
      .loaded
      .config
      .hub
      .toast
      .as_deref()
      .and_then(crate::source::duration);
    let base = motion.unwrap_or(Duration::from_secs(6));
    self
      .toasts
      .retain(|(_, style, at)| at.elapsed() < if *style == bad { base * 3 } else { base });
  }

  fn after(&mut self, then: Option<&str>) {
    match then {
      Some("refresh") | None => self.store.invalidate(),
      Some("back") => {
        self.pages.pop();
        self.store.invalidate();
      }
      Some(t) if t.starts_with("open:") => {
        let row = self
          .focused_widget()
          .and_then(|w| self.selected.get(w))
          .cloned()
          .unwrap_or_default();
        self.pages.push((t[5..].to_string(), row));
        self.focus = 0;
      }
      Some(t) if t.starts_with("screen:") => self.go(&t[7..]),
      _ => {}
    }
  }

  fn go(&mut self, screen: &str) {
    self.screen = screen.to_string();
    self.pages.clear();
    self.focus = 0;
    self.search = None;
  }

  /// The actions that apply where the cursor is.
  fn actions(&self) -> Vec<&'a ActionSpec> {
    let (screen, page) = self.current();
    let focused = self.focused_widget();
    let cx = self.context(page);
    self
      .loaded
      .config
      .actions
      .iter()
      .filter(|a| a.on == "global" || a.on == screen || Some(a.on.as_str()) == focused)
      .filter(|a| a.when.as_ref().is_none_or(|w| cx.eval(w, None).truthy()))
      .collect()
  }

  fn context<'b>(&'b self, page: Option<&'b Row>) -> Context<'b> {
    Context {
      loaded: self.loaded,
      styles: &self.styles,
      page,
      selected: &self.selected,
      vars: &self.vars,
    }
  }

  /// Starts an action: asks first if it says so.
  fn start(&mut self, action: &ActionSpec, confirmed: bool) {
    self.begin(action, confirmed, false);
  }

  /// Starts an action: its fields asked for first, unless `answered`; then confirmed,
  /// unless `confirmed`; then run.
  fn begin(&mut self, action: &ActionSpec, confirmed: bool, answered: bool) {
    let page = self.current().1.cloned();
    if !answered && !action.ask.is_empty() {
      let cx = self.context(page.as_ref());
      let values = action
        .ask
        .iter()
        .map(|f| {
          f.default
            .as_ref()
            .map(|d| cx.render(d, None))
            .unwrap_or_default()
        })
        .collect();
      self.overlay = Overlay::Ask(Box::new(Ask {
        action: action.clone(),
        values,
        at: 0,
        problem: None,
      }));
      return;
    }
    // A command run on its pane's marked rows: once for each, always asked first.
    let marked = self
      .marks
      .get(&action.on)
      .filter(|m| !m.is_empty() && action.run.is_some())
      .cloned();
    if let Some(rows) = marked {
      if !confirmed {
        let saved = self.selected.insert(action.on.clone(), rows[0].clone());
        let detail = action
          .confirm
          .as_ref()
          .map(|c| {
            format!(
              "\nThe first: {}",
              self.context(page.as_ref()).render(c, None)
            )
          })
          .unwrap_or_default();
        self.restore_selected(&action.on, saved);
        self.overlay = Overlay::Confirm {
          then: Confirmed::Action(Box::new(action.clone()), answered),
          text: format!(
            "{} on {} marked row(s)?{}",
            action.label,
            rows.len(),
            detail
          ),
        };
        return;
      }
      let saved = self.selected.get(&action.on).cloned();
      let mut skipped = 0;
      for row in rows {
        self.selected.insert(action.on.clone(), row);
        let cx = self.context(page.as_ref());
        if action
          .when
          .as_ref()
          .is_some_and(|w| !cx.eval(w, None).truthy())
        {
          skipped += 1;
          continue;
        }
        self.run_one(action, page.as_ref());
      }
      self.restore_selected(&action.on, saved);
      self.marks.remove(&action.on);
      if skipped > 0 {
        self.toast(&format!(
          "{}: {} marked row(s) skipped, where it does not apply",
          action.label, skipped
        ));
      }
      return;
    }
    if let (Some(confirm), false) = (&action.confirm, confirmed) {
      let text = self.context(page.as_ref()).render(confirm, None);
      self.overlay = Overlay::Confirm {
        then: Confirmed::Action(Box::new(action.clone()), answered),
        text,
      };
      return;
    }
    self.run_one(action, page.as_ref());
  }

  /// Puts back the row selected in `w` before a batch, or none.
  fn restore_selected(&mut self, w: &str, saved: Option<Row>) {
    match saved {
      Some(row) => self.selected.insert(w.to_string(), row),
      None => self.selected.remove(w),
    };
  }

  /// Runs an action once, as the selection stands: opens its page, copies its text, or
  /// runs its command.
  fn run_one(&mut self, action: &ActionSpec, page: Option<&Row>) {
    let cx = self.context(page);
    if let Some(open) = &action.open {
      let row = self
        .focused_widget()
        .and_then(|w| self.selected.get(w))
        .cloned()
        .unwrap_or_default();
      self.pages.push((open.clone(), row));
      self.focus = 0;
      return;
    }
    if let Some(copy) = &action.copy {
      let text = cx.render(copy, None);
      copy_to_clipboard(&text);
      self.toasts.push((
        format!(
          "sent to the clipboard (OSC 52, which some terminals ignore): {}",
          text
        ),
        self.styles.get("ok"),
        Instant::now(),
      ));
      return;
    }
    let Some(run) = &action.run else {
      return;
    };
    let line = cx.render(run, None);
    let toast = action.toast.as_ref().map(|t| cx.render(t, None));
    match action.mode.as_deref().unwrap_or("wait") {
      "terminal" => self.on_terminal = Some(line),
      "background" => {
        match spawn_background(&self.loaded.base, &line, &self.log_path()) {
          Ok(()) => {
            let text = toast.unwrap_or_else(|| format!("started: {}", line));
            self
              .toasts
              .push((text, self.styles.get("ok"), Instant::now()));
          }
          Err(e) => self
            .toasts
            .push((e, self.styles.get("bad"), Instant::now())),
        }
        self.after(action.then.as_deref());
      }
      _ => {
        let input = action.input.as_ref().map(|t| cx.render(t, None));
        let limit = action
          .timeout
          .as_deref()
          .and_then(crate::source::duration)
          .unwrap_or(Duration::from_secs(30));
        let before_line = action.before.as_ref().map(|t| cx.render(t, None));
        let pending = action.undo.as_ref().map(|_| Pending {
          action: Box::new(action.clone()),
          selected: self.selected.clone(),
          page: page.cloned(),
          vars: self.vars.clone(),
          before: Default::default(),
        });
        let before = pending.as_ref().map(|p| p.before.clone());
        let job = self.spawn_wait(&action.label, &line, input, limit, before_line.zip(before));
        self.jobs.push(Job {
          pending,
          then: action.then.clone(),
          toast,
          ..job
        });
        self.toasts.push((
          format!("{}…", action.label),
          self.styles.get("dim"),
          Instant::now(),
        ));
      }
    }
  }

  /// Handles the mouse: a click focuses, selects, switches or presses what is under it,
  /// twice opens; the wheel scrolls what is under it. A terminal with the keys whose
  /// program asked for the mouse gets it over its screen.
  pub fn mouse(&mut self, event: MouseEvent) {
    if !self.mouse_on {
      return;
    }
    let (x, y) = (event.column, event.row);
    let at = |r: &Rect| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
    let wheel = match event.kind {
      MouseEventKind::ScrollUp => -3,
      MouseEventKind::ScrollDown => 3,
      _ => 0,
    };
    let click = matches!(event.kind, MouseEventKind::Down(MouseButton::Left));
    // The mouse over a chart puts its crosshair there.
    if matches!(event.kind, MouseEventKind::Moved) && self.captured.is_none() {
      let over = self
        .pane_at(x, y)
        .filter(|w| self.loaded.widget(w).is_some_and(|s| s.kind == "chart"));
      self.hover = over.map(|w| (w, x));
      return;
    }
    if let Some(w) = self.captured.clone() {
      let over = self
        .hits
        .term_screens
        .iter()
        .find(|(r, id)| *id == w && at(r))
        .map(|(r, _)| *r);
      match over {
        Some(r) => {
          if let Some(t) = self.tab_mut(&w) {
            if t.wants_mouse() {
              t.mouse(&event, x - r.x, y - r.y);
            } else if wheel != 0 {
              t.scroll_back(-wheel);
            }
          }
          return;
        }
        // A click elsewhere gives the keys back, then does what it does.
        None if click => self.captured = None,
        None => return,
      }
    }
    if wheel != 0 {
      return self.wheel(x, y, wheel);
    }
    if !click {
      return;
    }
    let double = self
      .last_click
      .is_some_and(|(t, cx, cy)| t.elapsed() < Duration::from_millis(400) && cx == x && cy == y);
    self.last_click = Some((Instant::now(), x, y));
    let item = self.hits.items.iter().find(|(r, _)| at(r)).map(|(_, i)| *i);
    let inside = self.hits.popup.is_some_and(|r| at(&r));
    match std::mem::replace(&mut self.overlay, Overlay::None) {
      Overlay::None => {}
      Overlay::Palette { query, at: _ } if item.is_some() => {
        let item = item.unwrap_or(0);
        if double {
          return self.palette_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            query,
            item,
          );
        }
        self.overlay = Overlay::Palette { query, at: item };
        return;
      }
      Overlay::Form(mut form) => {
        if let Some(i) = item.filter(|i| *i < form.fields.len()) {
          form.at = i;
          // A click on a switch flips it.
          if !form.fields[i].takes_value {
            form.fields[i].on = !form.fields[i].on;
          }
        }
        self.overlay = Overlay::Form(form);
        return;
      }
      Overlay::Pick => {
        if let Some(i) = self
          .pane_at(x, y)
          .and_then(|w| self.focusable.iter().position(|f| *f == w))
        {
          self.focus = i;
        }
        return;
      }
      // Forms being filled in are not lost to a stray click.
      overlay @ (Overlay::Ask(_) | Overlay::Rename { .. }) => {
        self.overlay = overlay;
        return;
      }
      Overlay::FormHelp(form) => {
        self.overlay = Overlay::Form(form);
        return;
      }
      // Inside the others nothing is clicked; outside closes them (a confirmation's
      // answer is then no).
      overlay => {
        if inside {
          self.overlay = overlay;
        }
        return;
      }
    }
    if self.hits.page_chip.is_some_and(|r| at(&r)) {
      self.run("back", None);
      return;
    }
    if let Some(id) = self
      .hits
      .tabs
      .iter()
      .find(|(r, _)| at(r))
      .map(|(_, id)| id.clone())
    {
      self.go(&id);
      return;
    }
    if let Some(key) = self
      .hits
      .footer
      .iter()
      .find(|(r, _)| at(r))
      .map(|(_, k)| k.clone())
    {
      // A hint's key, pressed: the first of keys typed in turn.
      if let Some(k) = key.split_whitespace().next().and_then(keys::parse) {
        self.key(KeyEvent::new(k.code, k.modifiers));
      }
      return;
    }
    if let Some((w, i)) = self
      .hits
      .term_tabs
      .iter()
      .find(|(r, ..)| at(r))
      .map(|(_, w, i)| (w.clone(), *i))
    {
      if let Some(f) = self.focusable.iter().position(|f| *f == w) {
        self.focus = f;
      }
      if let Some(tabs) = self.terminals.get_mut(&w) {
        tabs.at = i.min(tabs.list.len().saturating_sub(1));
      }
      return;
    }
    let Some(w) = self.pane_at(x, y) else {
      return;
    };
    if let Some(f) = self.focusable.iter().position(|f| *f == w) {
      self.focus = f;
    }
    let kind = self
      .loaded
      .widget(&w)
      .map(|s| s.kind.clone())
      .unwrap_or_default();
    if kind == "terminal" {
      // A click on a terminal's screen gives it the keys.
      if self
        .hits
        .term_screens
        .iter()
        .any(|(r, id)| *id == w && at(r))
      {
        self.run("insert", None);
      }
      return;
    }
    // A click on a table's header sorts by that column; again, the other way.
    let header = self
      .hits
      .panes
      .iter()
      .find(|p| p.widget == w && kind == "table" && y + 1 == p.data_top)
      .and_then(|p| {
        p.columns
          .iter()
          .position(|(cx, width)| x >= *cx && x < cx + width)
      });
    if let Some(column) = header {
      let by = match self.sorts.get(&w) {
        Some((c, d)) if *c == column => (column, !d),
        _ => (column, false),
      };
      self.sort(&w, Some(by));
      return;
    }
    let row = self
      .hits
      .panes
      .iter()
      .find(|p| p.widget == w)
      .and_then(|p| {
        (y >= p.data_top)
          .then(|| p.start + (y - p.data_top) as usize)
          .filter(|r| *r < p.rows)
      });
    if let Some(row) = row
      && self
        .loaded
        .widget(&w)
        .is_some_and(|s| s.selectable == Some(true))
    {
      self.selection.insert(w.clone(), row);
      // A double click on the row opens it.
      if double {
        self.run("open", None);
      }
    }
  }

  /// The pane at (`x`, `y`), as last drawn.
  fn pane_at(&self, x: u16, y: u16) -> Option<String> {
    self
      .hits
      .panes
      .iter()
      .find(|p| {
        x >= p.area.x
          && x < p.area.x + p.area.width
          && y >= p.area.y
          && y < p.area.y + p.area.height
      })
      .map(|p| p.widget.clone())
  }

  /// The wheel, `by` rows, over (`x`, `y`): help or the palette when open, else the pane
  /// under it (a terminal's history, when its program did not ask for the mouse).
  fn wheel(&mut self, x: u16, y: u16, by: i64) {
    match &mut self.overlay {
      Overlay::Help(at) => {
        *at = (*at as i64 + by).max(0) as usize;
        return;
      }
      Overlay::Palette { at, .. } => {
        *at = (*at as i64 + by.signum()).max(0) as usize;
        return;
      }
      Overlay::None => {}
      _ => return,
    }
    let Some(w) = self.pane_at(x, y) else {
      return;
    };
    if self.loaded.widget(&w).is_some_and(|s| s.kind == "terminal") {
      if let Some(t) = self.tab_mut(&w) {
        if t.wants_mouse() {
          // Over a program that takes the mouse but without its keys: its own scrolling.
          let r = self
            .hits
            .term_screens
            .iter()
            .find(|(_, id)| *id == w)
            .map(|(r, _)| *r);
          if let (Some(r), Some(t)) = (r, self.tab_mut(&w)) {
            let kind = if by < 0 {
              MouseEventKind::ScrollUp
            } else {
              MouseEventKind::ScrollDown
            };
            let event = MouseEvent {
              kind,
              column: x,
              row: y,
              modifiers: KeyModifiers::NONE,
            };
            t.mouse(&event, x.saturating_sub(r.x), y.saturating_sub(r.y));
          }
        } else {
          t.scroll_back(-by);
        }
      }
      return;
    }
    self.move_in(w, by);
  }

  /// Handles one key, remembering the place it left.
  pub fn key(&mut self, event: KeyEvent) {
    let before = self.place();
    let pane = self.focused_widget().map(str::to_string);
    self.jumped = false;
    self.handle_key(event);
    let after = self.place();
    let now = self.focused_widget().map(str::to_string);
    if after == before && now != pane && pane.is_some() {
      self.previous_pane = pane;
    } else if after != before {
      self.previous_pane = None;
    }
    if after != before && !self.jumped {
      self.behind.push(before);
      if self.behind.len() > PLACES_KEPT {
        self.behind.remove(0);
      }
      self.ahead.clear();
    }
  }

  fn place(&self) -> Place {
    (self.screen.clone(), self.pages.clone())
  }

  /// Goes back to the last place left, or forward again.
  fn jump(&mut self, forward: bool) {
    let here = self.place();
    let (from, to) = if forward {
      (&mut self.ahead, &mut self.behind)
    } else {
      (&mut self.behind, &mut self.ahead)
    };
    let Some(place) = from.pop() else {
      return;
    };
    self.jumped = true;
    to.push(here);
    (self.screen, self.pages) = place;
    self.focus = 0;
    self.search = None;
  }

  fn handle_key(&mut self, event: KeyEvent) {
    if let Some(w) = self.captured.clone()
      && matches!(self.overlay, Overlay::None)
    {
      return self.terminal_key(event, w);
    }
    // After `mark_place` or `go_place`, a letter names the place.
    if let Some(set) = self.place_pending.take() {
      match event.code {
        KeyCode::Char(c) if c.is_ascii_alphanumeric() => {
          if set {
            self.set_place(c);
          } else {
            self.go_place(c);
          }
        }
        _ => self.toast("no place named"),
      }
      return;
    }
    match std::mem::replace(&mut self.overlay, Overlay::None) {
      Overlay::None => {}
      // Only y says yes: enter, a key pressed out of habit, does not.
      Overlay::Confirm { then, text } => {
        match (event.code, then) {
          (KeyCode::Char('y' | 'Y'), Confirmed::Action(action, answered)) => {
            self.begin(&action, true, answered)
          }
          (KeyCode::Char('y' | 'Y'), Confirmed::Line(line)) => self.run_line(&line, true),
          (KeyCode::Char('y' | 'Y'), Confirmed::Undo) => self.run_undo(),
          (KeyCode::Char('y' | 'Y'), Confirmed::CloseTab(w, at)) => self.close_tab(&w, at),
          (KeyCode::Char('y' | 'Y'), Confirmed::Quit) => self.quit = true,
          (KeyCode::Char('n' | 'N') | KeyCode::Esc | KeyCode::Enter, _) => {}
          (_, then) => self.overlay = Overlay::Confirm { then, text },
        }
        return;
      }
      Overlay::Palette { query, at } => return self.palette_key(event, query, at),
      Overlay::Form(form) => return self.form_key(event, *form),
      Overlay::FormHelp(form) => {
        self.overlay = Overlay::Form(form);
        return;
      }
      Overlay::Ask(mut ask) => {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let n = ask.action.ask.len();
        let lines = ask.action.ask[ask.at].lines.unwrap_or(1);
        let submit = (ctrl && event.code == KeyCode::Char('s'))
          || (event.code == KeyCode::Enter && lines <= 1 && ask.at + 1 == n);
        if submit {
          let missing: Vec<&str> = ask
            .action
            .ask
            .iter()
            .zip(&ask.values)
            .filter(|(f, v)| f.required == Some(true) && v.trim().is_empty())
            .map(|(f, _)| f.label.as_str())
            .collect();
          if missing.is_empty() {
            for (f, v) in ask.action.ask.iter().zip(&ask.values) {
              self
                .vars
                .insert(format!("ask.{}", f.name), Value::Str(v.clone()));
            }
            let action = ask.action.clone();
            self.begin(&action, false, true);
          } else {
            ask.problem = Some(format!("{} cannot be empty", missing.join(", ")));
            self.overlay = Overlay::Ask(ask);
          }
          return;
        }
        match event.code {
          KeyCode::Esc => return,
          KeyCode::Tab => ask.at = (ask.at + 1) % n,
          KeyCode::BackTab => ask.at = (ask.at + n - 1) % n,
          KeyCode::Enter if lines > 1 => ask.values[ask.at].push('\n'),
          KeyCode::Enter => ask.at = (ask.at + 1) % n,
          KeyCode::Backspace => {
            ask.values[ask.at].pop();
          }
          KeyCode::Char('u') if ctrl => ask.values[ask.at].clear(),
          KeyCode::Char(c) if !ctrl => ask.values[ask.at].push(c),
          _ => {}
        }
        ask.problem = None;
        self.overlay = Overlay::Ask(ask);
        return;
      }
      Overlay::Yank => {
        match event.code {
          KeyCode::Char('r') => self.yank("json"),
          KeyCode::Char('t') => self.yank("tsv"),
          KeyCode::Char('m') => self.yank("md"),
          _ => {}
        }
        return;
      }
      // A pane's number focuses it; any other key leaves the panes as they were.
      Overlay::Pick => {
        if let KeyCode::Char(c) = event.code
          && let Some(at) = PICK_KEYS.find(c.to_ascii_lowercase())
          && at < self.focusable.len()
        {
          self.focus = at;
        }
        return;
      }
      Overlay::Rename {
        widget,
        at,
        mut text,
      } => {
        match event.code {
          KeyCode::Esc => return,
          KeyCode::Enter => {
            let name = text.trim().to_string();
            if let Some(t) = self
              .terminals
              .get_mut(&widget)
              .and_then(|t| t.list.get_mut(at))
            {
              // Empty gives the name back to the program.
              t.custom = (!name.is_empty()).then_some(name);
            }
            return;
          }
          KeyCode::Backspace => {
            text.pop();
          }
          KeyCode::Char('u') if event.modifiers.contains(KeyModifiers::CONTROL) => text.clear(),
          KeyCode::Char(c) => text.push(c),
          _ => {}
        }
        self.overlay = Overlay::Rename { widget, at, text };
        return;
      }
      // The drawer: up and down choose an alert, enter opens its page, d dismisses it, x
      // clears them all; any other key closes it.
      Overlay::Alerts(at) => {
        let n = self.alerts.len();
        let action = match self.loaded.bindings.lookup(&[event], self.vim) {
          Lookup::Actions(a) => a[0],
          _ => "",
        };
        match (event.code, action) {
          (_, "down") => self.overlay = Overlay::Alerts((at + 1).min(n.saturating_sub(1))),
          (_, "up") => self.overlay = Overlay::Alerts(at.saturating_sub(1)),
          (KeyCode::Enter, _) => {
            if let Some((page, row)) = self.alerts.get(at).and_then(|a| a.open.clone()) {
              self.pages.push((page, row));
              self.focus = 0;
            }
          }
          (KeyCode::Char('d'), _) if at < n => {
            self.alerts.remove(at);
            self.overlay = Overlay::Alerts(at.min(n.saturating_sub(2)));
          }
          (KeyCode::Char('x'), _) => self.alerts.clear(),
          (KeyCode::Char('s'), _) => {
            if let Some(key) = self.alerts.get(at).and_then(|a| a.key.clone()) {
              let now = std::time::SystemTime::now();
              if self.snoozed.get(&key).is_some_and(|until| *until > now) {
                self.snoozed.remove(&key);
              } else {
                self.snoozed.insert(key, now + SNOOZE);
              }
              self.save_snoozed();
            }
          }
          _ => {}
        }
        return;
      }
      // Help moves as panes do, by the bindings (j, gg, ctrl+d, pagedown...) and space; any
      // other key closes it.
      Overlay::Help(at) => {
        self.typed.push(event);
        let step: i64 = match self.loaded.bindings.lookup(&self.typed, self.vim) {
          Lookup::Prefix => {
            self.overlay = Overlay::Help(at);
            return;
          }
          Lookup::Actions(actions) => match actions[0] {
            "down" => 1,
            "up" => -1,
            "half_down" | "page_down" => 10,
            "half_up" | "page_up" => -10,
            "top" => -1_000_000,
            "bottom" => 1_000_000,
            _ => 0,
          },
          _ => 0,
        };
        self.typed.clear();
        let step = if event.code == KeyCode::Char(' ') {
          10
        } else {
          step
        };
        if step != 0 {
          self.overlay = Overlay::Help((at as i64 + step).max(0) as usize);
        }
        return;
      }
    }
    if self.typing_search {
      match event.code {
        KeyCode::Enter => self.typing_search = false,
        KeyCode::Esc => {
          self.typing_search = false;
          self.search = None;
        }
        KeyCode::Backspace => {
          if let Some(s) = self.search.as_mut() {
            s.pop();
          }
        }
        KeyCode::Char(c) => self.search.get_or_insert_with(String::new).push(c),
        _ => {}
      }
      return;
    }
    // esc drops keys typed toward a binding, and a count.
    if event.code == KeyCode::Esc && !(self.typed.is_empty() && self.count.is_empty()) {
      self.typed.clear();
      self.count.clear();
      return;
    }
    let fresh = self.typed.is_empty() && self.count.is_empty();
    // With vim keys off the project's actions come first; with them on, the bindings.
    if !self.vim && fresh && self.project_action(&event) {
      return;
    }
    if self.vim
      && self.typed.is_empty()
      && (event.modifiers - KeyModifiers::SHIFT).is_empty()
      && let KeyCode::Char(c) = event.code
      && c.is_ascii_digit()
      && (c != '0' || !self.count.is_empty())
    {
      self.count.push(c);
      return;
    }
    self.typed.push(event);
    match self.loaded.bindings.lookup(&self.typed, self.vim) {
      Lookup::Prefix => {}
      Lookup::Actions(actions) => {
        let count = std::mem::take(&mut self.count).parse().ok();
        self.typed.clear();
        let applied = actions.into_iter().any(|a| self.run(a, count));
        // An engine key that means nothing here (R with no terminal) is the project's.
        if !applied && self.vim && fresh {
          self.project_action(&event);
        }
      }
      Lookup::Screen(id) => {
        self.typed.clear();
        self.count.clear();
        self.go(&id);
      }
      Lookup::None => {
        self.typed.clear();
        self.count.clear();
        if self.vim && fresh {
          self.project_action(&event);
        }
      }
    }
  }

  fn move_selection(&mut self, by: i64) {
    if let Some(w) = self.focused_widget().map(str::to_string) {
      self.move_in(w, by);
    }
  }

  /// Moves widget `w`'s selection, or scrolls it, `by` rows.
  fn move_in(&mut self, w: String, by: i64) {
    {
      // A followed log scrolls back from its end: up goes further back.
      let by = match self.loaded.widget(&w) {
        Some(s) if s.kind == "log" && s.follow != Some(false) => -by,
        _ => by,
      };
      // A pane with a selection scrolls only to keep it in view (the drawing does that);
      // one without, a log or a file, scrolls itself.
      if self
        .loaded
        .widget(&w)
        .is_some_and(|s| s.selectable == Some(true))
      {
        let entry = self.selection.entry(w).or_insert(0);
        *entry = (*entry as i64 + by).max(0) as usize;
      } else {
        let scroll = self.scroll.entry(w).or_insert(0);
        *scroll = (*scroll as i64 + by).max(0) as usize;
      }
    }
  }

  /// Opens the selected row's page, or the form of a palette command; whether there was
  /// one.
  fn open_selected(&mut self) -> bool {
    let Some(w) = self.focused_widget() else {
      return false;
    };
    // A row of the palette's commands opens that command's form.
    let palette_source = self
      .loaded
      .config
      .palette
      .as_ref()
      .map(|p| p.source.clone());
    if self.loaded.widget(w).and_then(|w| w.source.clone()) == palette_source
      && let Some(row) = self.selected.get(w)
    {
      let form = self.form_for(&row.clone());
      self.overlay = Overlay::Form(Box::new(form));
      return true;
    }
    let Some(open) = self.loaded.widget(w).and_then(|w| w.open.clone()) else {
      return false;
    };
    match self.selected.get(w).cloned() {
      Some(row) => {
        self.pages.push((open, row));
        self.focus = 0;
        true
      }
      None => false,
    }
  }

  /// The palette's matches: (label, detail, kind, command row or preset line).
  fn palette_items(&mut self, query: &str) -> Vec<PaletteItem> {
    let palette = self.loaded.config.palette.clone();
    let mut items = Vec::new();
    // What the query means as a command of its own (`set vim`, a screen), first, so
    // enter never does something the list does not show.
    if let Some(meaning) = self.ex_meaning(query) {
      items.push((format!(":{}", query.trim()), meaning, "command", None, None));
    }
    let mut saved = Vec::new();
    let rows = self.rows_matching(query);
    let Some(palette) = palette else {
      items.extend(rows);
      return items;
    };
    // The presets file is read again only when it changed.
    let path = self
      .state_dir()
      .join(palette.presets.as_deref().unwrap_or("presets.toml"));
    let changed = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    if self.presets.as_ref().is_none_or(|(at, _)| *at != changed) {
      self.presets = Some((changed, read_presets(&path)));
    }
    let presets = self
      .presets
      .as_ref()
      .map(|(_, p)| p.clone())
      .unwrap_or_default();
    for preset in presets {
      saved.push((
        format!("★ {}", preset.0),
        preset.1.clone(),
        "preset",
        None,
        Some(preset.1),
      ));
    }
    if let Some(recent) = &palette.recent {
      let rows = self
        .store
        .get(self.loaded, &recent.source, false)
        .rows
        .clone();
      let mut seen = HashSet::new();
      for row in rows.iter().rev() {
        let line = match row.get(&recent.command) {
          Some(Value::List(words)) => words.iter().map(Value::text).collect::<Vec<_>>().join(" "),
          Some(v) => v.text(),
          None => continue,
        };
        if seen.insert(line.clone()) && seen.len() <= recent.limit.unwrap_or(5) {
          saved.push((
            format!("↺ {}", line),
            "ran before".into(),
            "recent",
            None,
            Some(line),
          ));
        }
      }
    }
    let mut commands = Vec::new();
    for row in self
      .store
      .get(self.loaded, &palette.source, false)
      .rows
      .clone()
    {
      let path = row.get("path").map(Value::text).unwrap_or_default();
      let about = row.get("about").map(Value::text).unwrap_or_default();
      commands.push((path, about, "command", Some(row), None));
    }
    // Typing looks for a command: commands first. With nothing typed, what was saved or
    // ran before comes first, to run again.
    if query.trim().is_empty() {
      items.extend(saved);
      items.extend(commands);
    } else {
      items.extend(commands);
      items.extend(rows);
      items.extend(saved);
    }
    let query = query.to_lowercase();
    items.retain(|(label, detail, ..)| {
      let hay = format!("{} {}", label, detail).to_lowercase();
      let mut chars = hay.chars();
      query.chars().all(|q| chars.any(|c| c == q))
    });
    items
  }

  /// Rows whose name for themselves (`id`, `name`, `path`, `hash`, `short`, `signature`,
  /// `_name`) has `query`, from the selectable panes of every screen, for the palette to go
  /// to: at most thirty, each with the screen and pane it is in. Panes whose source follows
  /// a selection are left out.
  fn rows_matching(&mut self, query: &str) -> Vec<PaletteItem> {
    let query = query.trim().to_lowercase();
    if query.chars().count() < 2 {
      return Vec::new();
    }
    let mut out = Vec::new();
    let screens: Vec<(String, String, Vec<String>)> = self
      .loaded
      .config
      .screens
      .iter()
      .filter(|s| !s.page)
      .map(|s| {
        let mut widgets = Vec::new();
        layout_widgets(&s.layout, &mut widgets);
        (s.id.clone(), s.title.clone(), widgets)
      })
      .collect();
    for (screen, title, widgets) in screens {
      for w in widgets {
        let Some(spec) = self.loaded.widget(&w) else {
          continue;
        };
        let Some(source) = spec
          .source
          .clone()
          .filter(|_| spec.selectable == Some(true))
        else {
          continue;
        };
        if self.loaded.source(&source).is_none_or(|s| s.args.is_some()) {
          continue;
        }
        let pane = spec.title.clone().unwrap_or_else(|| w.clone());
        let pane = self.context(None).render(&pane, None);
        for row in self.store.get(self.loaded, &source, false).rows.clone() {
          let name = ["id", "name", "path", "hash", "short", "signature", "_name"]
            .iter()
            .filter_map(|k| row.get(*k).map(Value::text))
            .find(|v| v.to_lowercase().contains(&query));
          if let Some(name) = name {
            out.push((
              format!("→ {}", name),
              format!("{} · {}", title, pane),
              "row",
              Some(row),
              Some(format!("{}\u{1f}{}", screen, w)),
            ));
            if out.len() >= 30 {
              return out;
            }
          }
        }
      }
    }
    out
  }

  fn palette_key(&mut self, event: KeyEvent, mut query: String, mut at: usize) {
    let items = self.palette_items(&query);
    match event.code {
      KeyCode::Esc => return,
      KeyCode::Up => at = at.saturating_sub(1),
      KeyCode::Down => at = (at + 1).min(items.len().saturating_sub(1)),
      KeyCode::Backspace => {
        query.pop();
        at = 0;
      }
      KeyCode::Enter => {
        let defaults = event.modifiers.contains(KeyModifiers::CONTROL)
          || event.modifiers.contains(KeyModifiers::ALT);
        if let Some((_, _, kind, row, line)) = items.get(at).cloned() {
          match (kind, row, line) {
            ("command", Some(row), _) => {
              let form = self.form_for(&row);
              if defaults {
                self.run_form(&form, false);
              } else {
                self.overlay = Overlay::Form(Box::new(form));
              }
            }
            ("command", None, None) => {
              self.ex(&query);
            }
            ("row", Some(row), Some(target)) => {
              if let Some((screen, widget)) = target.split_once('\u{1f}') {
                self.go(screen);
                self.pending_focus = Some(widget.to_string());
                self.selection.entry(widget.to_string()).or_insert(0);
                self.pending_rows.insert(widget.to_string(), row);
              }
            }
            // A line saved or run before runs only once confirmed, shown as it is.
            (_, _, Some(line)) => {
              self.overlay = Overlay::Confirm {
                text: format!("Run this again?\n\n$ {}", line),
                then: Confirmed::Line(line),
              };
            }
            _ => {}
          }
        }
        return;
      }
      KeyCode::Char(c) => {
        query.push(c);
        at = 0;
      }
      _ => {}
    }
    self.overlay = Overlay::Palette { query, at };
  }

  fn form_key(&mut self, event: KeyEvent, mut form: Form) {
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    match event.code {
      // Closed, it is kept as a draft for the next time.
      KeyCode::Esc => {
        self.drafts.insert(form.path.clone(), form);
        return;
      }
      // f1 too: on some terminals ctrl+h is backspace.
      KeyCode::F(1) => {
        self.overlay = Overlay::FormHelp(Box::new(form));
        return;
      }
      KeyCode::Char('h') if ctrl => {
        self.overlay = Overlay::FormHelp(Box::new(form));
        return;
      }
      KeyCode::Char('s') if ctrl => {
        let line = form_line(
          self
            .loaded
            .config
            .palette
            .as_ref()
            .map_or("{path} {args}", |p| p.run.as_str()),
          &form,
        );
        self.save_preset(&form.path, &line);
      }
      KeyCode::Char('t') if ctrl => {
        if form_problems(&form).is_empty() {
          self.run_form(&form, true);
          return;
        }
      }
      KeyCode::Enter => {
        if form_problems(&form).is_empty() {
          self.run_form(&form, false);
          return;
        }
      }
      KeyCode::Tab | KeyCode::Down => form.at = (form.at + 1) % form.fields.len().max(1),
      KeyCode::BackTab | KeyCode::Up => {
        let n = form.fields.len().max(1);
        form.at = (form.at + n - 1) % n;
      }
      _ => {
        if let Some(field) = form.fields.get_mut(form.at) {
          edit_field(field, &event);
        }
      }
    }
    self.overlay = Overlay::Form(Box::new(form));
  }

  fn save_preset(&mut self, name: &str, line: &str) {
    let Some(palette) = &self.loaded.config.palette else {
      return;
    };
    let path = self
      .state_dir()
      .join(palette.presets.as_deref().unwrap_or("presets.toml"));
    let _ = std::fs::create_dir_all(self.state_dir());
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    text.push_str(&format!(
      "[[preset]]\nname = {:?}\nrun = {:?}\n\n",
      name, line
    ));
    let _ = std::fs::write(&path, text);
    self.toasts.push((
      format!("saved preset: {}", line),
      self.styles.get("ok"),
      Instant::now(),
    ));
  }

  /// A command's form: the draft left of it, else a new one.
  fn form_for(&mut self, row: &Row) -> Form {
    let path = row.get("path").map(Value::text).unwrap_or_default();
    self.drafts.remove(&path).unwrap_or_else(|| form_of(row))
  }

  fn run_form(&mut self, form: &Form, on_terminal: bool) {
    let Some(palette) = self.loaded.config.palette.clone() else {
      return;
    };
    if on_terminal {
      self.on_terminal = Some(form_line(&palette.run, form));
    } else {
      let template = palette.background.as_deref().unwrap_or(&palette.run);
      let line = form_line(template, form);
      self.run_line(&line, false);
    }
  }

  fn run_line(&mut self, line: &str, recent: bool) {
    let line = if recent {
      self
        .loaded
        .config
        .palette
        .as_ref()
        .and_then(|p| p.background.as_ref())
        .map(|b| {
          b.replace("{path} {args}", line)
            .replace("{path}", line)
            .replace("{args}", "")
        })
        .unwrap_or_else(|| line.to_string())
    } else {
      line.to_string()
    };
    match spawn_background(&self.loaded.base, &line, &self.log_path()) {
      Ok(()) => self.toasts.push((
        format!("started: {}", line),
        self.styles.get("ok"),
        Instant::now(),
      )),
      Err(e) => self
        .toasts
        .push((e, self.styles.get("bad"), Instant::now())),
    }
    self.store.invalidate();
  }

  /// Draws a frame.
  /// The keys that may follow those typed so far, and what each does, in a box at the
  /// bottom right of the body, as helix and which-key show them.
  fn draw_which_key(&self, frame: &mut Frame, body: Rect) {
    use unicode_width::UnicodeWidthStr;
    let next = self.loaded.bindings.continuations(&self.typed, self.vim);
    if next.is_empty() || body.height < 4 {
      return;
    }
    let keys = next.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
    let rows = next.len().min(body.height as usize - 2);
    let lines: Vec<Line> = next
      .iter()
      .take(rows)
      .map(|(k, what)| {
        Line::from(vec![
          Span::styled(format!("{:<w$}  ", k, w = keys), self.styles.get("key")),
          Span::styled(what.clone(), self.styles.get("text")),
        ])
      })
      .collect();
    let width = lines
      .iter()
      .map(|l| l.width() as u16)
      .max()
      .unwrap_or(0)
      .saturating_add(2)
      .min(body.width);
    let height = rows as u16 + 2;
    let typed: Vec<String> = self.typed.iter().map(keys::name).collect();
    let area = Rect::new(
      body.x + body.width - width,
      body.y + body.height - height,
      width,
      height,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
      Paragraph::new(lines).block(
        Block::default()
          .borders(Borders::ALL)
          .border_style(self.styles.get("frame"))
          .title(Span::styled(
            format!(" {} … ", crate::bindings::shown(&typed.join(" "))),
            self.styles.get("title"),
          )),
      ),
      area,
    );
  }

  pub fn draw(&mut self, frame: &mut Frame) {
    let area = frame.area();
    let base = Block::default().style(self.styles.get("text"));
    frame.render_widget(base, area);
    let min = self.loaded.config.hub.min_width.unwrap_or(40);
    let min_height = self.loaded.config.hub.min_height.unwrap_or(8);
    if area.width < min || area.height < min_height {
      let quit = self
        .loaded
        .bindings
        .first("quit", self.vim)
        .unwrap_or_default();
      let text = format!(
        "{} needs at least {} columns and {} rows; this terminal has {} by {}. {} quits.",
        self.loaded.config.hub.name, min, min_height, area.width, area.height, quit
      );
      frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
      return;
    }
    let [top, tabs, body, footer] = Layout::vertical([
      Constraint::Length(1),
      Constraint::Length(1),
      Constraint::Fill(1),
      Constraint::Length(1),
    ])
    .areas(area);
    self.hits = Hits::default();
    self.cursor = None;
    self.draw_topbar(frame, top);
    self.draw_tabs(frame, tabs);
    self.draw_body(frame, body);
    self.draw_footer(frame, footer);
    if !self.typed.is_empty() && matches!(self.overlay, Overlay::None) {
      self.draw_which_key(frame, body);
    }
    // Toasts under an overlay, so they never hide what it asks.
    self.draw_toasts(frame, area);
    self.draw_overlay(frame, area);
    // The terminal's own cursor where something is typed, for screen readers and
    // magnifiers, which follow it; hidden otherwise.
    if let Some((x, y)) = self.cursor.take() {
      frame.set_cursor_position((x, y));
    }
  }

  fn draw_topbar(&mut self, frame: &mut Frame, area: Rect) {
    use unicode_width::UnicodeWidthStr;
    let mut spans = Vec::new();
    let segments = self.loaded.config.topbar.segments.clone();
    for seg in &segments {
      let mut vars = self.vars.clone();
      let rows = match &seg.source {
        Some(s) => {
          let data = self.store.get(self.loaded, s, false);
          // Not loaded yet: the segment would say what its rows do not.
          if data.updated.is_none() {
            continue;
          }
          data.rows.clone()
        }
        None => Vec::new(),
      };
      // The newest row: the last a log or a series appended.
      let row = rows.last().cloned();
      if let Some(count) = &seg.count {
        let n = rows
          .iter()
          .filter(|r| {
            self
              .loaded
              .expr(count)
              .eval(&crate::expr::Scope {
                row: Some(r),
                ..Default::default()
              })
              .truthy()
          })
          .count();
        vars.insert("count".into(), Value::Num(n as f64));
      }
      let scope = crate::expr::Scope {
        row: if seg.count.is_some() {
          None
        } else {
          row.as_ref()
        },
        vars: Some(&vars),
        ..Default::default()
      };
      if seg
        .when
        .as_ref()
        .is_some_and(|w| !self.loaded.expr(w).eval(&scope).truthy())
      {
        continue;
      }
      let text = self.loaded.template(&seg.text).render(&scope);
      if !spans.is_empty() {
        spans.push(Span::styled(" · ", self.styles.get("dim")));
      }
      spans.push(Span::styled(
        text,
        seg
          .style
          .as_deref()
          .map_or(self.styles.get("text"), |s| self.styles.get(s)),
      ));
    }
    // The engine's own state stays in view at the right; the project's segments give
    // way, cut with `…`.
    let mut right: Vec<Span> = Vec::new();
    if let Some(job) = self.jobs.first() {
      const SPIN: [char; 4] = ['◐', '◓', '◑', '◒'];
      let elapsed = job.started.elapsed();
      // Still, with `[hub] motion = false`; the time still counts.
      let frame_at = if self.loaded.config.hub.motion == Some(false) {
        0
      } else {
        (elapsed.as_millis() / 250 % 4) as usize
      };
      let more = if self.jobs.len() > 1 {
        format!(" +{}", self.jobs.len() - 1)
      } else {
        String::new()
      };
      right.push(Span::styled(
        format!(
          " {} {} {}{} ",
          SPIN[frame_at],
          job.label,
          crate::template::duration(elapsed.as_secs_f64()),
          more
        ),
        self.styles.get("info"),
      ));
    }
    let firing = self.alerts.iter().filter(|a| a.resolved.is_none()).count();
    if firing > 0 {
      right.push(Span::styled(
        format!(" ⚠ {} alert(s) ", firing),
        self.styles.get("warn"),
      ));
    }
    let right_width: usize = right.iter().map(|s| s.content.width()).sum();
    let room = (area.width as usize).saturating_sub(right_width);
    let mut left: Vec<Span> = Vec::new();
    let mut used = 0;
    for span in spans {
      let w = span.content.width();
      if used + w > room {
        let cut = crate::widgets::fit(&span.content, room.saturating_sub(used));
        used += cut.width();
        left.push(Span::styled(cut, span.style));
        break;
      }
      used += w;
      left.push(span);
    }
    left.push(Span::raw(" ".repeat(room.saturating_sub(used))));
    left.extend(right);
    frame.render_widget(Paragraph::new(Line::from(left)), area);
  }

  /// The tab bar: an open page's chip first (a click on it goes back), then the screens,
  /// scrolled so the one shown is always in view, `…` marking those cut off.
  fn draw_tabs(&mut self, frame: &mut Frame, area: Rect) {
    use unicode_width::UnicodeWidthStr;
    let mut spans = Vec::new();
    let mut x = area.x;
    if let Some((id, row)) = self.pages.last() {
      let title = self
        .loaded
        .screen(id)
        .map(|s| self.context(Some(row)).render(&s.title, None))
        .unwrap_or_default();
      let back = self
        .loaded
        .bindings
        .first("back", self.vim)
        .unwrap_or_default();
      let chip = crate::widgets::fit(
        &format!(" ‹ {} · {} ", back, title),
        (area.width / 2).max(10) as usize,
      );
      let w = chip.width() as u16;
      self.hits.page_chip = Some(Rect::new(x, area.y, w, 1));
      x += w + 1;
      spans.push(Span::styled(chip, self.styles.get("tab")));
      spans.push(Span::raw(" "));
    }
    let screens: Vec<(String, String)> = self
      .loaded
      .config
      .screens
      .iter()
      .filter(|s| !s.page)
      .map(|s| {
        let key = self
          .loaded
          .bindings
          .screens(self.vim)
          .into_iter()
          .find(|(id, _)| *id == s.id)
          .map(|(_, k)| k)
          .unwrap_or_default();
        (
          s.id.clone(),
          format!(" {} {} ", key, s.title).replace("  ", " "),
        )
      })
      .collect();
    let here = screens
      .iter()
      .position(|(id, _)| *id == self.screen)
      .unwrap_or(0);
    let room = (area.x + area.width).saturating_sub(x) as usize;
    // From the first screen, or from as far left as keeps the one shown in view.
    let mut first = 0;
    let width_of = |range: std::ops::Range<usize>| -> usize {
      screens[range].iter().map(|(_, l)| l.width()).sum()
    };
    while first < here && width_of(first..here + 1) + 2 > room {
      first += 1;
    }
    if first > 0 {
      spans.push(Span::styled("…", self.styles.get("dim")));
      x += 1;
    }
    for (i, (id, label)) in screens.iter().enumerate().skip(first) {
      let w = label.width() as u16;
      if (x + w) as usize > (area.x + area.width) as usize {
        spans.push(Span::styled("…", self.styles.get("dim")));
        break;
      }
      let style = if i == here && self.pages.is_empty() {
        self.styles.get("tab")
      } else if i == here {
        self.styles.get("title")
      } else {
        self.styles.get("dim")
      };
      self
        .hits
        .tabs
        .push((Rect::new(x, area.y, w, 1), id.clone()));
      x += w;
      spans.push(Span::styled(label.clone(), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
  }

  fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
    let (id, page) = {
      let (id, page) = self.current();
      (id.to_string(), page.cloned())
    };
    let Some(screen) = self.loaded.screen(&id) else {
      return;
    };
    let narrow = self
      .loaded
      .config
      .hub
      .narrow_below
      .is_some_and(|n| area.width < n);
    let layout = match (&screen.narrow, narrow) {
      (Some(n), true) => n,
      _ => &screen.layout,
    };
    let mut placed = Vec::new();
    layout::place(layout, area, &mut placed);
    self.focusable = placed
      .iter()
      .filter(|(w, _)| {
        self.loaded.widget(w).is_some_and(|w| {
          w.selectable == Some(true)
            || matches!(w.kind.as_str(), "log" | "file" | "terminal" | "chart")
        })
      })
      .map(|(w, _)| w.clone())
      .collect();
    self.rects = placed
      .iter()
      .filter(|(w, _)| self.focusable.contains(w))
      .cloned()
      .collect();
    // Arriving on a screen or page focuses the pane focused there before.
    if self.focus_place != id {
      self.focus = self
        .focus_of
        .get(&id)
        .and_then(|w| self.focusable.iter().position(|f| f == w))
        .unwrap_or(0);
      self.focus_place = id.clone();
    }
    if let Some(w) = self.pending_focus.take()
      && let Some(at) = self.focusable.iter().position(|f| *f == w)
    {
      self.focus = at;
    }
    if self.focus >= self.focusable.len() {
      self.focus = 0;
    }
    if let Some(w) = self.focusable.get(self.focus) {
      self.focus_of.insert(id.clone(), w.clone());
    }
    // A zoomed pane has the screen to itself.
    if let Some((place, w)) = &self.zoomed
      && *place == id
      && placed.iter().any(|(p, _)| p == w)
    {
      placed = vec![(w.clone(), area)];
      self.rects = placed.clone();
    }
    let zoomed = self
      .zoomed
      .as_ref()
      .filter(|(p, _)| *p == id)
      .map(|(_, w)| w.clone());
    let focused = self.focused_widget().map(str::to_string);
    for (w, rect) in placed {
      let Some(spec) = self.loaded.widget(&w) else {
        continue;
      };
      let is_focused = focused.as_deref() == Some(w.as_str());
      let border = if spec.border == Some(false) { 0 } else { 2 };
      self
        .heights
        .insert(w.clone(), rect.height.saturating_sub(border));
      if spec.kind == "terminal" {
        self.hits.panes.push(PaneHit {
          widget: w.clone(),
          area: rect,
          data_top: rect.y,
          start: 0,
          rows: 0,
          columns: Vec::new(),
        });
        self.draw_terminal(frame, rect, &w, is_focused);
        continue;
      }
      let selected = (spec.selectable == Some(true)).then(|| *self.selection.get(&w).unwrap_or(&0));
      let how = Draw {
        focused: is_focused,
        zoomed: zoomed.as_deref() == Some(w.as_str()),
        selected,
        scroll: std::cell::Cell::new(*self.scroll.get(&w).unwrap_or(&0)),
        span: std::cell::Cell::new(None),
        search: if is_focused {
          self.search.as_deref()
        } else {
          None
        },
        sort: self.sorts.get(&w).copied(),
        marked: self.marks.get(&w).map_or(&[], Vec::as_slice),
        columns: std::cell::RefCell::new(Vec::new()),
        wrap: self.wrapped.contains(&w),
        folds_open: self.folds_open.contains(&w),
        jump: self.log_jump.remove(&w),
        at: std::cell::Cell::new(self.log_at.get(&w).copied()),
        window: self.windows.get(&w).copied().unwrap_or((0, 0)),
        crosshair: std::cell::Cell::new(self.crosshairs.get(&w).copied()),
        step: self.steps.remove(&w),
        hover: self
          .hover
          .as_ref()
          .filter(|(h, _)| *h == w)
          .map(|(_, x)| *x),
        pinned: self.pins.get(&w).copied(),
        under: std::cell::Cell::new(None),
      };
      let cx = Context {
        loaded: self.loaded,
        styles: &self.styles,
        page: page.as_ref(),
        selected: &self.selected,
        vars: &self.vars,
      };
      let rows = widgets::draw(frame, rect, spec, &mut self.store, &cx, &how);
      self.scroll.insert(w.clone(), how.scroll.get());
      if let Some(at) = how.at.get() {
        self.log_at.insert(w.clone(), at);
      }
      if spec.kind == "chart" {
        match how.crosshair.get() {
          Some(x) => self.crosshairs.insert(w.clone(), x),
          None => self.crosshairs.remove(&w),
        };
        match how.under.get() {
          Some(p) => self.under.insert(w.clone(), p),
          None => self.under.remove(&w),
        };
      }
      let inset = u16::from(spec.border != Some(false));
      let header = u16::from(spec.kind == "table");
      self.hits.panes.push(PaneHit {
        widget: w.clone(),
        area: rect,
        data_top: rect.y + inset + header,
        start: how.scroll.get(),
        rows: rows.len(),
        columns: how.columns.take(),
      });
      if spec.selectable == Some(true) {
        self.shown.insert(w.clone(), rows.clone());
      }
      // At the last row of a source that pages: more of it.
      if let (Some(source), Some(sel)) = (&spec.source, selected)
        && sel + 1 >= rows.len()
        && !rows.is_empty()
      {
        let cx = Context {
          loaded: self.loaded,
          styles: &self.styles,
          page: page.as_ref(),
          selected: &self.selected,
          vars: &self.vars,
        };
        let args = widgets::source_args(&cx, source);
        if self.store.grow(self.loaded, source, args.as_deref()) {
          let dim = self.styles.get("dim");
          self.toasts.push(("more rows…".into(), dim, Instant::now()));
        }
      }
      // With `[hub] cursor = "follow"`, the cursor rests on the focused pane's selection.
      if is_focused
        && self.loaded.config.hub.cursor.as_deref() == Some("follow")
        && let Some(sel) = selected
        && sel >= how.scroll.get()
      {
        let y = rect.y + inset + header + (sel - how.scroll.get()) as u16;
        if y < rect.y + rect.height - inset {
          self.cursor = Some((rect.x + inset, y));
        }
      }
      // Recorded at once, so panes drawn after it this frame follow it; a table that
      // emptied has nothing selected.
      // A row selected in an earlier session is found again, wherever it is now.
      let mut selected = selected;
      if selected.is_some()
        && !rows.is_empty()
        && let Some(saved) = self.pending_rows.remove(&w)
        && let Some(at) = rows
          .iter()
          .position(|r| crate::session::same_row(r, &saved))
      {
        selected = Some(at);
      }
      if let Some(s) = selected {
        let clamped = s.min(rows.len().saturating_sub(1));
        self.selection.insert(w.clone(), clamped);
        match rows.get(clamped) {
          Some(row) => self.selected.insert(w.clone(), row.clone()),
          None => self.selected.remove(&w),
        };
      }
    }
  }

  /// A terminal widget: its tab's program's screen, the first started the first time it
  /// is shown, and a strip of its tabs when it has more than one.
  fn draw_terminal(&mut self, frame: &mut Frame, area: Rect, w: &str, focused: bool) {
    let Some(spec) = self.loaded.widget(w) else {
      return;
    };
    let captured = self.captured.as_deref() == Some(w);
    let mut block = Block::default();
    if spec.border != Some(false) {
      let style = if captured {
        self.styles.get("key")
      } else if focused {
        self.styles.get("title")
      } else {
        self.styles.get("frame")
      };
      block = block.borders(Borders::ALL).border_style(style);
      if focused || captured {
        block = block.border_type(ratatui::widgets::BorderType::Thick);
      }
    }
    let inner = block.inner(area);
    let first_show = !self.terminals.contains_key(w);
    // The area under the strip of tabs, when there are two or more.
    let under = |count: usize| {
      if count > 1 {
        Rect {
          y: inner.y + 1,
          height: inner.height.saturating_sub(1),
          ..inner
        }
      } else {
        inner
      }
    };
    let coming = self
      .pending_tabs
      .get(w)
      .filter(|_| first_show)
      .map_or(1, |t| t.tabs.len());
    let first_area = under(self.terminals.get(w).map_or(coming, |t| t.list.len()));
    self.terminals.entry(w.to_string()).or_default().size = (first_area.height, first_area.width);
    if self.start_terminals && first_show {
      // The tabs of an earlier session, each with its last screen; else one new tab.
      match self.pending_tabs.remove(w) {
        Some(saved) if !saved.tabs.is_empty() => {
          for tab in &saved.tabs {
            self.start_tab_from(w, None, Some(tab));
          }
          if let Some(tabs) = self.terminals.get_mut(w) {
            tabs.at = saved.at.min(tabs.list.len().saturating_sub(1));
          }
        }
        _ => {
          if !self.start_tab(w, None) {
            self.start_terminals = false;
          }
        }
      }
    }
    let count = self.terminals.get(w).map_or(0, |t| t.list.len());
    let strip = count > 1;
    let screen_area = under(count);
    let command = spec.command.clone().unwrap_or_default();
    let key = |action: &str| {
      self
        .loaded
        .bindings
        .first(action, self.vim)
        .unwrap_or_default()
    };
    let (insert, new) = (key("insert"), key("tab_new"));
    let escape = self.release_shown(w);
    let at = self.terminals.get(w).map_or(0, |t| t.at);
    let never = first_show || !self.start_terminals;
    let state = match self.tab_mut(w) {
      None if never => "not started".to_string(),
      None => format!("no tabs · {} starts one", insert),
      Some(t) => match t.has_exited() {
        Some(how) => format!("{} · {} starts it again", how, insert),
        None if t.scrolled_back() > 0 => format!(
          "{} lines back in its history · a key or the wheel comes back",
          t.scrolled_back()
        ),
        None if captured => format!("{} gives the keys back", escape),
        None => format!("{} to type · {} new tab", insert, new),
      },
    };
    let title = spec.title.clone().unwrap_or_else(|| "terminal".to_string());
    let title = self.context(None).render(&title, None);
    let marker = if focused || captured { "▶ " } else { "" };
    block = block.title(Line::from(vec![
      Span::styled(format!(" {}{} ", marker, title), self.styles.get("title")),
      Span::styled(format!("{} ", state), self.styles.get("dim")),
    ]));
    frame.render_widget(block, area);
    self.hits.term_screens.push((screen_area, w.to_string()));
    if strip && let Some(tabs) = self.terminals.get(w) {
      let mut spans = Vec::new();
      let mut x = inner.x;
      for (i, t) in tabs.list.iter().enumerate() {
        let name = format!(
          " {} {}{} ",
          i + 1,
          crate::widgets::fit(&t.name(), 20),
          if t.running() { "" } else { " (exited)" }
        );
        // The tab shown is marked by more than color: reversed and bold.
        let style = if i == at {
          self
            .styles
            .get("title")
            .add_modifier(ratatui::style::Modifier::REVERSED | ratatui::style::Modifier::BOLD)
        } else {
          self.styles.get("dim")
        };
        let width = unicode_width::UnicodeWidthStr::width(name.as_str()) as u16;
        self
          .hits
          .term_tabs
          .push((Rect::new(x, inner.y, width, 1), w.to_string(), i));
        x += width + 1;
        spans.push(Span::styled(name, style));
        spans.push(Span::raw(" "));
      }
      frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect { height: 1, ..inner },
      );
    }
    let (rows, cols) = (screen_area.height, screen_area.width);
    if let Some(tabs) = self.terminals.get_mut(w) {
      for t in &mut tabs.list {
        t.resize(rows, cols);
      }
    }
    let mut cursor = None;
    match self.tab(w) {
      Some(t) => {
        if let Ok(parser) = t.parser.lock() {
          // The real cursor, where the program has it, rather than a drawn one.
          let screen = tui_term::widget::PseudoTerminal::new(parser.screen())
            .cursor(tui_term::widget::Cursor::default().visibility(false));
          frame.render_widget(screen, screen_area);
          let (row, col) = parser.screen().cursor_position();
          if captured && !parser.screen().hide_cursor() && parser.screen().scrollback() == 0 {
            cursor = Some((screen_area.x + col, screen_area.y + row));
          }
        }
      }
      None => {
        let shown = if command.is_empty() {
          "$SHELL"
        } else {
          &command
        };
        let text = if first_show || !self.start_terminals {
          format!("{} runs here when this screen is shown", shown)
        } else {
          format!("{} starts {} here", insert, shown)
        };
        frame.render_widget(
          Paragraph::new(Line::styled(text, self.styles.get("dim"))),
          screen_area,
        );
      }
    }
    if cursor.is_some() {
      self.cursor = cursor;
    }
  }

  /// The keys that give terminal widget `w`'s keys back, as shown.
  /// With no full-screen program running, `release_prompt`'s keys work too.
  fn release_shown(&self, w: &str) -> String {
    let mut keys: Vec<String> = match self.loaded.widget(w).and_then(|s| s.escape_keys()) {
      Some(keys) => keys.iter().map(|k| bindings::shown(k)).collect(),
      None => vec![self.loaded.bindings.shown("release", self.vim)],
    };
    if !self.tab(w).is_some_and(|t| t.full_screen()) {
      keys.push(self.loaded.bindings.shown("release_prompt", self.vim));
    }
    keys.retain(|k| !k.is_empty());
    keys.join(", ")
  }

  /// A footer key: `{action}` is that action's first key, `{screens}` the screens' keys.
  fn footer_key(&self, key: &str) -> String {
    match key.strip_prefix('{').and_then(|k| k.strip_suffix('}')) {
      Some("screens") => self.loaded.bindings.screens_hint(self.vim),
      Some(action) => self
        .loaded
        .bindings
        .first(action, self.vim)
        .unwrap_or_default(),
      None => key.to_string(),
    }
  }

  fn draw_footer(&mut self, frame: &mut Frame, area: Rect) {
    let mut spans = Vec::new();
    if self.vim {
      let (name, style) = match self.mode() {
        Mode::Normal => ("NORMAL", "title"),
        Mode::Insert => ("INSERT", "key"),
        Mode::Command => ("COMMAND", "key"),
      };
      spans.push(Span::styled(
        format!(" {} ", name),
        self
          .styles
          .get(style)
          .add_modifier(ratatui::style::Modifier::REVERSED),
      ));
      spans.push(Span::raw(" "));
    }
    if !self.typed.is_empty() || !self.count.is_empty() {
      let typed: Vec<String> = self.typed.iter().map(keys::name).collect();
      spans.push(Span::styled(
        format!(
          "{} ",
          bindings::shown(&format!("{} {}", self.count, typed.join(" ")))
        ),
        self.styles.get("dim"),
      ));
    }
    let first = |action: &str| {
      self
        .loaded
        .bindings
        .first(action, self.vim)
        .unwrap_or_default()
    };
    // Hints that fit, then how many more help lists; help and quit stay at the right.
    let (hints, pinned): (Vec<Hint>, Vec<Hint>) = if let Some(keys) = self.overlay_keys() {
      (keys, Vec::new())
    } else if let Some(w) = &self.captured {
      (
        vec![
          (self.release_shown(w), "back to the hub".into()),
          (String::new(), "every other key goes to the terminal".into()),
        ],
        Vec::new(),
      )
    } else if self.typing_search {
      (
        vec![
          (
            "/".into(),
            format!("{}▏", self.search.clone().unwrap_or_default()),
          ),
          ("enter".into(), "keep".into()),
          ("esc".into(), "clear".into()),
        ],
        Vec::new(),
      )
    } else {
      (
        self.here(),
        vec![
          (first("help"), "help".into()),
          (first("quit"), "quit".into()),
        ],
      )
    };
    let width = |hints: &[Hint]| -> usize {
      use unicode_width::UnicodeWidthStr;
      hints
        .iter()
        .map(|(k, l)| k.width() + 1 + l.width() + 2)
        .sum()
    };
    let used: usize = {
      use unicode_width::UnicodeWidthStr;
      spans.iter().map(|s| s.content.width()).sum()
    };
    let room = (area.width as usize).saturating_sub(used + width(&pinned));
    let mut shown = hints.len();
    let help = first("help");
    let more = |n: usize| (format!("… {} more", n), help.clone());
    if width(&hints) > room {
      shown = 0;
      while shown < hints.len() {
        let rest = hints.len() - shown - 1;
        let tail = if rest > 0 { width(&[more(rest)]) } else { 0 };
        if width(&hints[..=shown]) + tail > room {
          break;
        }
        shown += 1;
      }
    }
    let hint = |key: &str, label: &str, spans: &mut Vec<Span>| {
      spans.push(Span::styled(key.to_string(), self.styles.get("key")));
      spans.push(Span::styled(
        format!(" {}  ", label),
        self.styles.get("dim"),
      ));
    };
    // Each hint is where a click presses its key.
    let mut x = area.x + used as u16;
    let mut clicks: Vec<(Rect, String)> = Vec::new();
    let mut place = |key: &str, label: &str, x: &mut u16| {
      let w = width(&[(key.to_string(), label.to_string())]) as u16;
      clicks.push((
        Rect::new(*x, area.y, w.saturating_sub(2), 1),
        key.to_string(),
      ));
      *x += w;
    };
    for (key, label) in &hints[..shown] {
      hint(key, label, &mut spans);
      place(key, label, &mut x);
    }
    if shown < hints.len() {
      let label = format!("… {} more ", hints.len() - shown);
      spans.push(Span::styled(label.clone(), self.styles.get("dim")));
      spans.push(Span::styled(help.clone(), self.styles.get("key")));
      spans.push(Span::raw("  "));
      place(&help, &label, &mut x);
    }
    let mut right = Vec::new();
    for (key, label) in &pinned {
      hint(key, label, &mut right);
    }
    let left_width: usize = {
      use unicode_width::UnicodeWidthStr;
      spans.iter().map(|s| s.content.width()).sum()
    };
    let pad = (area.width as usize).saturating_sub(left_width + width(&pinned));
    spans.push(Span::raw(" ".repeat(pad)));
    x = area.x + (left_width + pad) as u16;
    for (key, label) in &pinned {
      place(key, label, &mut x);
    }
    spans.extend(right);
    self.hits.footer = clicks;
    if self.typing_search {
      // After `/ ` and the search typed.
      let typed = self.search.as_deref().unwrap_or("");
      self.cursor = Some((
        area.x + used as u16 + 2 + unicode_width::UnicodeWidthStr::width(typed) as u16,
        area.y,
      ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
  }

  /// The keys of the overlay open, for the footer: it says what they do there, not what
  /// they do on the screen under it.
  fn overlay_keys(&self) -> Option<Vec<Hint>> {
    let h = |pairs: &[(&str, &str)]| -> Vec<Hint> {
      pairs
        .iter()
        .map(|(k, l)| (k.to_string(), l.to_string()))
        .collect()
    };
    Some(match &self.overlay {
      Overlay::None => return None,
      Overlay::Help(_) => h(&[("↑↓ gg G", "scroll"), ("any other key", "close")]),
      Overlay::Alerts(_) => h(&[
        ("↑↓", "choose"),
        ("enter", "open"),
        ("d", "dismiss"),
        ("x", "clear all"),
        ("esc", "close"),
      ]),
      Overlay::Confirm { .. } => h(&[("y", "yes"), ("n enter esc", "no")]),
      Overlay::Palette { .. } => h(&[
        ("↑↓", "choose"),
        ("enter", "open"),
        ("alt+enter", "run with defaults"),
        ("esc", "close"),
      ]),
      Overlay::Form(_) => h(&[
        ("tab", "next field"),
        ("enter", "run"),
        ("ctrl+t", "run on the terminal"),
        ("ctrl+s", "save as a preset"),
        ("f1", "flags"),
        ("esc", "close, keeping it"),
      ]),
      Overlay::FormHelp(_) => h(&[("any key", "back to the form")]),
      Overlay::Ask(_) => h(&[("tab", "next field"), ("ctrl+s", "run"), ("esc", "give up")]),
      Overlay::Rename { .. } => h(&[("enter", "keep"), ("esc", "give up")]),
      Overlay::Pick => h(&[("1-9 a-z", "that pane"), ("any other key", "stay")]),
      Overlay::Yank => h(&[
        ("r", "row as JSON"),
        ("t", "rows as TSV"),
        ("m", "rows as Markdown"),
      ]),
    })
  }

  /// The keys that do something where the cursor is: a terminal's, the project's actions
  /// that apply, the footer's extra hints. The footer shows them; help lists them first.
  fn here(&self) -> Vec<Hint> {
    let first = |action: &str| {
      self
        .loaded
        .bindings
        .first(action, self.vim)
        .unwrap_or_default()
    };
    let mut out = Vec::new();
    // What just happened comes first: stopping it, or taking it back.
    if !self.jobs.is_empty() {
      out.push((first("cancel"), "stop".to_string()));
    }
    if let Some(undo) = self.undos.last() {
      out.push((first("undo"), format!("undo {}", undo.label)));
    }
    if let Some(w) = self.focused_terminal() {
      out.push((first("insert"), "type".to_string()));
      out.push((first("tab_new"), "new tab".to_string()));
      if self.terminals.get(&w).is_some_and(|t| t.list.len() > 1) {
        out.push((
          format!("{} {}", first("tab_previous"), first("tab_next")),
          "tabs".to_string(),
        ));
        out.push((first("tab_close"), "close tab".to_string()));
        out.push((first("tab_rename"), "rename tab".to_string()));
      }
    }
    if let Some(spec) = self
      .focused_widget()
      .and_then(|w| self.loaded.widget(w))
      .filter(|s| s.kind == "log")
    {
      let what = if self.search.is_some() {
        "matches"
      } else {
        "problems"
      };
      out.push((
        format!("{} {}", first("next_match"), first("previous_match")),
        what.to_string(),
      ));
      out.push((first("wrap"), "wrap".to_string()));
      if spec.fold.is_some() {
        out.push((first("fold"), "folds".to_string()));
      }
    }
    for action in self.actions() {
      out.push((action.key.clone(), action.label.clone()));
    }

    if self
      .zoomed
      .as_ref()
      .is_some_and(|(p, _)| *p == self.focus_place)
    {
      out.push((first("zoom"), "unzoom".to_string()));
    }
    for v in &self.loaded.config.variables {
      if let Some(key) = &v.key {
        let value = self
          .vars
          .get(&format!("var.{}", v.name))
          .map(Value::text)
          .unwrap_or_default();
        out.push((key.clone(), format!("{} {}", v.name, value)));
      }
    }
    for (key, label) in &self.loaded.config.footer.extra {
      out.push((self.footer_key(key), label.clone()));
    }
    out
  }

  fn draw_overlay(&mut self, frame: &mut Frame, area: Rect) {
    let mut overlay = std::mem::replace(&mut self.overlay, Overlay::None);
    match &mut overlay {
      Overlay::None => {}
      Overlay::Yank => {
        let line = |key: &str, what: &str| {
          Line::from(vec![
            Span::styled(format!("{}  ", key), self.styles.get("key")),
            Span::styled(what.to_string(), self.styles.get("text")),
          ])
        };
        let lines = vec![
          line("r", "the selected row, as JSON"),
          line("t", "the rows shown, as TSV"),
          line("m", "the rows shown, as a Markdown table"),
        ];
        let rect = popup(
          frame,
          area,
          "copy · any other key leaves",
          lines,
          &self.styles,
          50,
          7,
        );
        self.hits.popup = Some(rect);
      }
      // Each pane gets its key in a box at its middle, as tmux's display-panes.
      Overlay::Pick => {
        let style = self
          .styles
          .get("title")
          .add_modifier(ratatui::style::Modifier::REVERSED | ratatui::style::Modifier::BOLD);
        for ((_, rect), key) in self.rects.iter().zip(PICK_KEYS.chars()) {
          let label = Rect {
            x: rect.x + rect.width.saturating_sub(5) / 2,
            y: rect.y + rect.height.saturating_sub(3) / 2,
            width: 5.min(rect.width),
            height: 3.min(rect.height),
          };
          frame.render_widget(Clear, label);
          frame.render_widget(
            Paragraph::new(vec![
              Line::default(),
              Line::from(format!("  {}  ", key)),
              Line::default(),
            ])
            .style(style),
            label,
          );
        }
      }
      Overlay::Ask(ask) => {
        let mut lines = Vec::new();
        let mut typing_at: Option<(u16, u16)> = None;
        for (i, (f, v)) in ask.action.ask.iter().zip(&ask.values).enumerate() {
          let here = i == ask.at;
          // The field being typed in is marked by more than color.
          let marker = if here { "› " } else { "  " };
          let label = format!(
            "{}{}{}",
            marker,
            f.label,
            if f.required == Some(true) { " *" } else { "" }
          );
          lines.push(Line::styled(
            label,
            if here {
              self.styles.get("title")
            } else {
              self.styles.get("dim")
            },
          ));
          let shown: Vec<&str> = v.split('\n').collect();
          let rows = f.lines.unwrap_or(1).max(1) as usize;
          for (j, text) in shown.iter().enumerate().take(rows.max(shown.len())) {
            let cursor = if here && j + 1 == shown.len() {
              typing_at = Some((
                lines.len() as u16,
                4 + unicode_width::UnicodeWidthStr::width(*text) as u16,
              ));
              "▏"
            } else {
              ""
            };
            lines.push(Line::styled(
              format!("    {}{}", text, cursor),
              self.styles.get("text"),
            ));
          }
          for _ in shown.len()..rows {
            lines.push(Line::default());
          }
        }
        if let Some(problem) = &ask.problem {
          lines.push(Line::styled(problem.clone(), self.styles.get("bad")));
        }
        lines.push(Line::default());
        lines.push(Line::styled(
          "tab next field · ctrl+s run · enter next (newline in a long field) · esc",
          self.styles.get("dim"),
        ));
        let title = ask.action.label.clone();
        let rect = popup_with(frame, area, &title, lines, &self.styles, (70, 50), false);
        self.hits.popup = Some(rect);
        if let Some((line, x)) = typing_at {
          self.cursor = Some((rect.x + 1 + x, rect.y + 1 + line));
        }
      }
      Overlay::Rename { at, text, .. } => {
        let lines = vec![
          Line::from(vec![
            Span::styled(format!("tab {}: ", *at + 1), self.styles.get("dim")),
            Span::styled(format!("{}▏", text), self.styles.get("text")),
          ]),
          Line::default(),
          Line::styled(
            "enter keeps it · empty gives the name back to the program · esc",
            self.styles.get("dim"),
          ),
        ];
        let rect = popup_with(
          frame,
          area,
          "rename tab",
          lines,
          &self.styles,
          (50, 20),
          false,
        );
        self.hits.popup = Some(rect);
        let before = format!("tab {}: {}", *at + 1, text);
        self.cursor = Some((
          rect.x + 1 + unicode_width::UnicodeWidthStr::width(before.as_str()) as u16,
          rect.y + 1,
        ));
      }
      Overlay::Help(at) => {
        let lines = help(self.loaded, self.vim, &self.here());
        let width = keys_width(&lines);
        // Scrolled no further than its last line at the bottom.
        *at = (*at).min(
          lines
            .len()
            .saturating_sub(area.height.saturating_sub(2) as usize),
        );
        let lines: Vec<Line> = lines
          .into_iter()
          .skip(*at)
          .map(|line| match line {
            HelpLine::Heading(h) => Line::styled(h, self.styles.get("title")),
            HelpLine::Blank => Line::default(),
            HelpLine::Keys(keys, what, note) => Line::from(vec![
              Span::styled(format!("  {:<width$}", keys), self.styles.get("key")),
              Span::styled(what, self.styles.get("text")),
              Span::styled(note, self.styles.get("dim")),
            ]),
          })
          .collect();
        let rect = popup_with(
          frame,
          area,
          "help · ↑↓ scroll · any other key closes",
          lines,
          &self.styles,
          (90, 80),
          false,
        );
        self.hits.popup = Some(rect);
      }
      Overlay::Alerts(at) => {
        let lines: Vec<Line> = if self.alerts.is_empty() {
          vec![Line::styled("no alerts", self.styles.get("dim"))]
        } else {
          self
            .alerts
            .iter()
            .enumerate()
            .map(|(i, alert)| {
              let mark = if i == *at { "▶ " } else { "  " };
              let page = if alert.open.is_some() { " ›" } else { "" };
              let ago =
                |t: std::time::SystemTime| widgets::age_text(t.elapsed().unwrap_or_default());
              let snoozed = alert
                .key
                .as_ref()
                .and_then(|k| self.snoozed.get(k))
                .filter(|until| **until > std::time::SystemTime::now());
              let state = match (alert.resolved, snoozed) {
                (Some(at), _) => format!(" · ✓ resolved {} ago", ago(at)),
                (None, Some(until)) => format!(
                  " · snoozed for {}",
                  widgets::age_text(
                    until
                      .duration_since(std::time::SystemTime::now())
                      .unwrap_or_default()
                  )
                ),
                (None, None) => format!(" · {} ago", ago(alert.since)),
              };
              let style = if alert.resolved.is_some() {
                self.styles.get("dim")
              } else {
                alert.style
              };
              Line::styled(format!("{}{}{}{}", mark, alert.text, page, state), style)
            })
            .collect()
        };
        let rect = popup(
          frame,
          area,
          "alerts · ↑↓ choose · enter open (›) · s snooze 1h · d dismiss · x clear all · esc",
          lines,
          &self.styles,
          70,
          50,
        );
        self.hits.popup = Some(rect);
      }
      Overlay::Confirm { text, .. } => {
        let mut lines: Vec<Line> = text
          .lines()
          .map(|l| Line::styled(l.to_string(), self.styles.get("text")))
          .collect();
        lines.push(Line::default());
        lines.push(Line::from(vec![
          Span::styled("y", self.styles.get("key")),
          Span::styled(" yes   ", self.styles.get("text")),
          Span::styled("n", self.styles.get("key")),
          Span::styled(", enter or esc no", self.styles.get("text")),
        ]));
        popup(frame, area, "confirm [y/N]", lines, &self.styles, 60, 30);
      }
      Overlay::Palette { query, at } => {
        let items = self.palette_items(query);
        let mut lines = vec![Line::from(vec![
          Span::styled("› ", self.styles.get("key")),
          Span::styled(format!("{}▏", query), self.styles.get("text")),
        ])];
        let height = (area.height as usize * 7 / 10).saturating_sub(4);
        let start = at.saturating_sub(height.saturating_sub(1));
        for (i, (label, detail, kind, ..)) in items.iter().enumerate().skip(start).take(height) {
          let style = if i == *at {
            self.styles.get("selected")
          } else {
            Style::default()
          };
          lines.push(Line::from(vec![
            Span::styled(
              format!(" {:<34}", trim_to(label, 34)),
              self.styles.get("text").patch(style),
            ),
            Span::styled(format!("{:<8}", kind), self.styles.get("dim").patch(style)),
            Span::styled(trim_to(detail, 60), self.styles.get("dim").patch(style)),
          ]));
        }
        let shown = lines.len() - 1;
        let rect = popup_with(
          frame,
          area,
          "run · enter configure · alt+enter run with defaults · esc",
          lines,
          &self.styles,
          (96, 70),
          false,
        );
        self.hits.popup = Some(rect);
        self.cursor = Some((
          rect.x + 3 + unicode_width::UnicodeWidthStr::width(query.as_str()) as u16,
          rect.y + 1,
        ));
        for k in 0..shown {
          self.hits.items.push((
            Rect::new(rect.x + 1, rect.y + 2 + k as u16, rect.width - 2, 1),
            start + k,
          ));
        }
      }
      Overlay::Form(form) => {
        let mut lines = form_lines(
          form,
          &self.styles,
          self
            .loaded
            .config
            .palette
            .as_ref()
            .map(|p| p.run.as_str())
            .unwrap_or("{path} {args}"),
        );
        let fields = form.fields.len();
        // Too many fields for the screen: those around the one being edited, the
        // description above and the command below kept.
        let room = area.height.saturating_sub(2) as usize;
        let (skip, fields) = if lines.len() > room {
          let excess = (lines.len() - room).min(fields.saturating_sub(1));
          let shown = fields - excess;
          let skip = form.at.saturating_sub(shown - 1).min(excess);
          lines.drain(2 + skip + shown..2 + fields);
          lines.drain(2..2 + skip);
          (skip, shown)
        } else {
          (0, fields)
        };
        let rect = popup_with(
          frame,
          area,
          &format!(
            "{} · tab next · enter run · ctrl+t on terminal · ctrl+s save · ctrl+h or f1 flags · esc",
            form.path
          ),
          lines,
          &self.styles,
          (90, 70),
          false,
        );
        self.hits.popup = Some(rect);
        if let Some(f) = form.fields.get(form.at).filter(|f| f.takes_value) {
          // After the field's text: the mark, its name in 20, then `[ `.
          let x =
            rect.x + 1 + 1 + 20 + 2 + unicode_width::UnicodeWidthStr::width(f.text.as_str()) as u16;
          self.cursor = Some((
            x.min(rect.x + rect.width - 2),
            rect.y + 3 + (form.at - skip) as u16,
          ));
        }
        // Under the description and a blank line, a line per field.
        for i in 0..fields {
          self.hits.items.push((
            Rect::new(rect.x + 1, rect.y + 3 + i as u16, rect.width - 2, 1),
            i + skip,
          ));
        }
      }
      Overlay::FormHelp(form) => {
        let mut lines = vec![
          Line::styled(form.about.clone(), self.styles.get("text")),
          Line::default(),
        ];
        for f in &form.fields {
          let name = f
            .long
            .as_ref()
            .map(|l| format!("--{}", l))
            .unwrap_or_else(|| f.value_name.clone());
          let mut help = f.help.clone();
          if !f.choices.is_empty() {
            help.push_str(&format!(" (one of: {})", f.choices.join(", ")));
          }
          if let Some(d) = &f.default {
            help.push_str(&format!(" [default {}]", d));
          }
          lines.push(Line::from(vec![
            Span::styled(format!("  {:<22}", name), self.styles.get("key")),
            Span::styled(help, self.styles.get("text")),
          ]));
        }
        popup(
          frame,
          area,
          &format!("{} · flags · any key back", form.path),
          lines,
          &self.styles,
          90,
          70,
        );
      }
    }
    self.overlay = overlay;
  }

  /// The newest toasts, bottom right above the footer, each wrapped to at most four lines.
  fn draw_toasts(&self, frame: &mut Frame, area: Rect) {
    use unicode_width::UnicodeWidthStr;
    let mut bottom = area.height.saturating_sub(1);
    for (text, style, _) in self.toasts.iter().rev().take(3) {
      let room = area.width.saturating_sub(4).clamp(10, 70) as usize;
      let width = (text.width().min(room) as u16 + 4).min(area.width.saturating_sub(2));
      let inner = width.saturating_sub(4).max(1) as usize;
      let lines = text.width().div_ceil(inner).clamp(1, 4) as u16;
      let height = lines + 2;
      if bottom < height + 2 {
        break;
      }
      let rect = Rect::new(
        area.width.saturating_sub(width + 1),
        bottom - height,
        width,
        height,
      );
      bottom -= height;
      frame.render_widget(Clear, rect);
      let block = Block::default()
        .borders(Borders::ALL)
        .border_style(self.styles.get("frame"));
      frame.render_widget(
        Paragraph::new(Line::styled(text.clone(), *style))
          .block(block)
          .wrap(Wrap { trim: true }),
        rect,
      );
    }
  }
}

/// An action's fields being filled in: their values, the field being typed in, and why
/// it cannot run yet.
pub struct Ask {
  action: ActionSpec,
  values: Vec<String>,
  at: usize,
  problem: Option<String>,
}

/// What goes on across a reload of the config: the session, and what is running.
pub struct Carry {
  session: crate::session::Session,
  terminals: BTreeMap<String, Tabs>,
  captured: Option<String>,
  jobs: Vec<Job>,
  toasts: Vec<(String, Style, Instant)>,
  alerts: Vec<Alert>,
  seen: HashSet<String>,
  vim: bool,
}

#[derive(Clone)]
pub struct Field {
  pub long: Option<String>,
  pub short: Option<String>,
  pub value_name: String,
  pub help: String,
  pub required: bool,
  pub takes_value: bool,
  pub multiple: bool,
  pub choices: Vec<String>,
  pub default: Option<String>,
  pub text: String,
  pub on: bool,
}

/// A command's arguments, being filled in.
#[derive(Clone)]
pub struct Form {
  pub path: String,
  pub about: String,
  pub fields: Vec<Field>,
  pub at: usize,
}

/// Where things were drawn in the last frame, for the mouse.
#[derive(Default)]
struct Hits {
  /// A tab of the tab bar and its screen.
  tabs: Vec<(Rect, String)>,
  /// The page's chip in the tab bar: a click goes back.
  page_chip: Option<Rect>,
  panes: Vec<PaneHit>,
  /// A footer hint and its key.
  footer: Vec<(Rect, String)>,
  /// A terminal's tab: its widget and number.
  term_tabs: Vec<(Rect, String, usize)>,
  /// A terminal widget's screen.
  term_screens: Vec<(Rect, String)>,
  /// The overlay's box, and its lines that can be clicked: a palette item, a form field.
  popup: Option<Rect>,
  items: Vec<(Rect, usize)>,
}

struct Job {
  label: String,
  line: String,
  started: Instant,
  done: Receiver<Result<String, String>>,
  /// Kept for `undo` once it succeeds.
  pending: Option<Pending>,
  /// Set to stop it (`cancel`).
  cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
  then: Option<String>,
  toast: Option<String>,
}

/// A pane as drawn: where, where its first row of data is, and which row that is.
struct PaneHit {
  widget: String,
  area: Rect,
  data_top: u16,
  start: usize,
  rows: usize,
  /// A table's columns, as (x, width), for a click on its header.
  columns: Vec<(u16, u16)>,
}

/// What an action that can be taken back keeps until it ends: its `undo` and the
/// selection, page and fields it started with.
struct Pending {
  action: Box<ActionSpec>,
  selected: BTreeMap<String, Row>,
  page: Option<Row>,
  vars: Row,
  /// What `before` printed, set by the job's thread.
  before: std::sync::Arc<std::sync::Mutex<String>>,
}

/// A terminal widget's programs, one per tab.
#[derive(Default)]
struct Tabs {
  list: Vec<Terminal>,
  /// The tab shown.
  at: usize,
  /// The rows and columns a program gets: the pane's, less the tab strip.
  size: (u16, u16),
}

/// How to take back an action that succeeded.
#[derive(Clone, Debug)]
struct Undo {
  label: String,
  line: String,
  input: Option<String>,
}

/// The command line a form gives, in `template` (`{path}`, `{args}`). Values equal to their
/// default are left out.
pub fn form_line(template: &str, form: &Form) -> String {
  let quote = |s: &str| {
    if s.is_empty() || s.contains(char::is_whitespace) {
      format!("\"{}\"", s.replace('"', "\\\""))
    } else {
      s.to_string()
    }
  };
  let mut args: Vec<String> = Vec::new();
  for f in &form.fields {
    let flag = f
      .long
      .as_ref()
      .map(|l| format!("--{}", l))
      .or_else(|| f.short.as_ref().map(|s| format!("-{}", s)));
    if !f.takes_value {
      if f.on
        && let Some(flag) = flag
      {
        args.push(flag);
      }
      continue;
    }
    if f.text.is_empty() || f.default.as_deref() == Some(f.text.as_str()) {
      continue;
    }
    let values: Vec<String> = if f.multiple {
      f.text.split_whitespace().map(quote).collect()
    } else {
      vec![quote(&f.text)]
    };
    for value in values {
      match &flag {
        Some(flag) => args.push(format!("{} {}", flag, value)),
        None => args.push(value),
      }
    }
  }
  template
    .replace("{path}", &form.path)
    .replace("{args}", &args.join(" "))
    .trim()
    .to_string()
}

/// A command row's arguments as a form.
pub fn form_of(row: &Row) -> Form {
  let text = |r: &Row, k: &str| match r.get(k) {
    Some(Value::Null) | None => None,
    Some(v) => Some(v.text()),
  };
  let flag = |r: &Row, k: &str| r.get(k).is_some_and(Value::truthy);
  let fields = match row.get("args") {
    Some(Value::Rows(args)) => args
      .iter()
      .filter(|a| text(a, "id").as_deref() != Some("help"))
      .map(|a| {
        let default = text(a, "default");
        Field {
          long: text(a, "long"),
          short: text(a, "short"),
          value_name: text(a, "value").unwrap_or_default(),
          help: text(a, "help").unwrap_or_default(),
          required: flag(a, "required"),
          takes_value: flag(a, "takes_value"),
          multiple: flag(a, "multiple"),
          choices: match a.get("choices") {
            Some(Value::List(c)) => c.iter().map(Value::text).collect(),
            _ => Vec::new(),
          },
          text: default.clone().unwrap_or_default(),
          default,
          on: false,
        }
      })
      .collect(),
    _ => Vec::new(),
  };
  Form {
    path: row.get("path").map(Value::text).unwrap_or_default(),
    about: row.get("about").map(Value::text).unwrap_or_default(),
    fields,
    at: 0,
  }
}

/// What is wrong with a form's values: a required one empty, a value not among its choices.
pub fn form_problems(form: &Form) -> Vec<String> {
  let mut problems = Vec::new();
  for f in &form.fields {
    let name = f.long.clone().unwrap_or_else(|| f.value_name.clone());
    if f.takes_value && f.required && f.text.trim().is_empty() {
      problems.push(format!("{} is required", name));
    }
    if f.takes_value && !f.choices.is_empty() && !f.text.is_empty() && !f.choices.contains(&f.text)
    {
      problems.push(format!("{} must be one of {}", name, f.choices.join(", ")));
    }
    // What the value's name says it is, checked as typed.
    let text = f.text.trim();
    if f.takes_value && !text.is_empty() {
      let all_digits = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit());
      let shape = match f.value_name.as_str() {
        "N" | "NUM" | "COUNT" | "SECONDS" | "MS" | "YEAR" | "JOBS" | "THREADS" => {
          (!all_digits(text)).then_some("a whole number")
        }
        "COLSxROWS" => (!text
          .split_once('x')
          .is_some_and(|(a, b)| all_digits(a) && all_digits(b)))
        .then_some("COLSxROWS, as 100x30"),
        "X.Y.Z" => (text.split('.').count() != 3 || !text.split('.').all(all_digits))
          .then_some("X.Y.Z, as 1.2.3"),
        _ => None,
      };
      if let Some(shape) = shape {
        problems.push(format!("{} must be {}", name, shape));
      }
    }
  }
  problems
}

/// Help, for vim keys off or on: every engine action with its keys, the screens, the
/// project's actions and the palette's commands. `--keys` prints the same lines.
pub fn help(loaded: &Loaded, vim: bool, here: &[(String, String)]) -> Vec<HelpLine> {
  let b = &loaded.bindings;
  let keys = |k: &str, what: &str, note: &str| HelpLine::Keys(k.into(), what.into(), note.into());
  let mut out = Vec::new();
  // What can be done where the cursor is, first: all the footer had no room for.
  if !here.is_empty() {
    out.push(HelpLine::Heading("HERE".into()));
    for (k, what) in here {
      out.push(keys(k, what, ""));
    }
    out.push(HelpLine::Blank);
  }
  out.push(HelpLine::Heading(format!(
    "KEYS · vim keys {}",
    if vim { "on" } else { "off" }
  )));
  if vim {
    out.push(keys("N", "a count, typed before the keys: 5j, 3G, 2gt", ""));
  }
  for (action, what) in bindings::ACTIONS {
    let what = match bindings::with_count(action) {
      Some(more) if vim => format!("{}{}", what, more),
      _ => what.to_string(),
    };
    let shown = b.shown(action, vim);
    if shown.is_empty() {
      out.push(keys("-", &what, " (unbound)"));
    } else {
      out.push(keys(&shown, &what, ""));
    }
  }
  out.push(HelpLine::Blank);
  out.push(HelpLine::Heading("SCREENS".into()));
  for (id, k) in b.screens(vim) {
    out.push(keys(&k, &id, ""));
  }
  if !loaded.config.actions.is_empty() {
    out.push(HelpLine::Blank);
    out.push(HelpLine::Heading("ACTIONS".into()));
  }
  for a in &loaded.config.actions {
    let what = a
      .run
      .clone()
      .or_else(|| a.open.clone().map(|o| format!("open {}", o)))
      .or_else(|| a.copy.as_ref().map(|_| "copy".to_string()))
      .unwrap_or_default();
    let event = keys::parse(&a.key).map(|k| KeyEvent::new(k.code, k.modifiers));
    let taken = event.and_then(|e| match b.lookup(&[e], vim) {
      Lookup::Actions(found) => Some(found.join(", ")),
      Lookup::Screen(id) => Some(format!("screen {}", id)),
      Lookup::Prefix => Some("a longer binding".to_string()),
      Lookup::None => None,
    });
    let note = match taken {
      Some(t) if vim => format!(" (vim keys give it to {})", t),
      Some(t) => format!(" (over {} where it applies)", t),
      None => String::new(),
    };
    out.push(keys(
      &a.key,
      &format!("{} · {} · {}", a.label, a.on, what),
      &note,
    ));
  }
  out.push(HelpLine::Blank);
  out.push(HelpLine::Heading("PALETTE COMMANDS".into()));
  out.push(keys(
    ":set vim",
    "vim keys on (:set novim off, :set vim! toggles)",
    "",
  ));
  out.push(keys(":reload", "read the config again (also :e)", ""));
  out.push(keys(":w [FILE]", "save this screen as text", ""));
  out.push(keys(":let NAME VALUE", "set a [[variable]]", ""));
  out.push(keys(
    ":set contrast",
    "high contrast (:set nocontrast the theme's colors)",
    "",
  ));
  if vim {
    out.push(keys(":q", "quit (also :q! :qa :wq :x)", ""));
    out.push(keys(":N", "row N", ""));
    out.push(keys(":<screen id>", "that screen", ""));
  }
  out
}

/// Help as text, for `--keys`.
pub fn help_text(loaded: &Loaded, vim: bool) -> String {
  let mut out = String::new();
  let lines = help(loaded, vim, &[]);
  let width = keys_width(&lines);
  for line in lines {
    match line {
      HelpLine::Heading(h) => out.push_str(&h),
      HelpLine::Blank => {}
      HelpLine::Keys(k, what, note) => {
        out.push_str(format!("  {:<width$}{}{}", k, what, note).trim_end())
      }
    }
    out.push('\n');
  }
  out
}

/// The folder the hub keeps its state in: `[hub] state`, from the config's folder.
pub fn state_dir(loaded: &Loaded) -> PathBuf {
  loaded
    .base
    .join(loaded.config.hub.state.as_deref().unwrap_or(".hub"))
}

/// Puts text on the clipboard through the terminal (OSC 52).
fn copy_to_clipboard(text: &str) {
  use std::io::Write;
  const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let bytes = text.as_bytes();
  let mut encoded = String::new();
  for chunk in bytes.chunks(3) {
    let n = (chunk[0] as u32) << 16
      | (*chunk.get(1).unwrap_or(&0) as u32) << 8
      | *chunk.get(2).unwrap_or(&0) as u32;
    for i in 0..4 {
      if i <= chunk.len() {
        encoded.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
      } else {
        encoded.push('=');
      }
    }
  }
  let mut out = std::io::stdout();
  let _ = write!(out, "\x1b]52;c;{}\x07", encoded);
  let _ = out.flush();
}

fn edit_field(field: &mut Field, event: &KeyEvent) {
  // A flag without a value is a switch: space, the arrows or x flip it, y and n set it.
  if !field.takes_value {
    match event.code {
      KeyCode::Char(' ' | 'x' | 'X') | KeyCode::Left | KeyCode::Right => field.on = !field.on,
      KeyCode::Char('y' | 'Y') => field.on = true,
      KeyCode::Char('n' | 'N') => field.on = false,
      _ => {}
    }
    return;
  }
  if !field.choices.is_empty()
    && matches!(
      event.code,
      KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
    )
  {
    let at = field.choices.iter().position(|c| *c == field.text);
    let n = field.choices.len();
    let next = match (at, event.code) {
      (Some(i), KeyCode::Left) => (i + n - 1) % n,
      (Some(i), _) => (i + 1) % n,
      (None, _) => 0,
    };
    field.text = field.choices[next].clone();
    return;
  }
  match event.code {
    KeyCode::Backspace => {
      field.text.pop();
    }
    KeyCode::Char(c) => field.text.push(c),
    _ => {}
  }
}

fn form_lines(form: &Form, styles: &Styles, template: &str) -> Vec<Line<'static>> {
  let mut lines = vec![
    Line::styled(form.about.clone(), styles.get("dim")),
    Line::default(),
  ];
  for (i, f) in form.fields.iter().enumerate() {
    let name = f
      .long
      .as_ref()
      .map(|l| format!("--{}", l))
      .unwrap_or_else(|| f.value_name.clone());
    let value = if f.takes_value {
      format!("[ {:<24} ]", f.text)
    } else {
      format!("{:<28}", if f.on { "[✓] on" } else { "[ ] off" })
    };
    let here = i == form.at;
    let style = if here {
      styles.get("selected")
    } else {
      Style::default()
    };
    let mut note = f.help.clone();
    if !f.choices.is_empty() {
      note = format!("{} (←→ {})", note, f.choices.join("|"));
    }
    if f.required {
      note = format!("required · {}", note);
    }
    // The field being edited is marked by more than its background.
    let marker = if here { "›" } else { " " };
    lines.push(Line::from(vec![
      Span::styled(
        format!("{}{:<20}", marker, name),
        styles.get("key").patch(style),
      ),
      Span::styled(format!("{} ", value), styles.get("text").patch(style)),
      Span::styled(trim_to(&note, 60), styles.get("dim")),
    ]));
  }
  lines.push(Line::default());
  if let Some(f) = form.fields.get(form.at) {
    let how = if !f.takes_value {
      "space, ←→ or x switch it · y on · n off"
    } else if !f.choices.is_empty() {
      "space or ←→ choose"
    } else {
      "type its value · backspace"
    };
    lines.push(Line::styled(
      format!(" {} · tab next field · enter runs", how),
      styles.get("dim"),
    ));
  }
  for problem in form_problems(form) {
    lines.push(Line::styled(format!(" ✗ {}", problem), styles.get("bad")));
  }
  lines.push(Line::from(vec![
    Span::styled(" $ ", styles.get("dim")),
    Span::styled(form_line(template, form), styles.get("text")),
  ]));
  lines
}

/// The width of help's keys column: the widest keys and two spaces.
fn keys_width(lines: &[HelpLine]) -> usize {
  lines
    .iter()
    .map(|l| match l {
      HelpLine::Keys(k, _, _) => unicode_width::UnicodeWidthStr::width(k.as_str()) + 2,
      _ => 0,
    })
    .max()
    .unwrap_or(0)
}

/// The widgets of a layout, in order.
fn layout_widgets(layout: &crate::config::LayoutSpec, out: &mut Vec<String>) {
  match layout {
    crate::config::LayoutSpec::Widget(w) => out.push(w.clone()),
    crate::config::LayoutSpec::Split { children, .. } => {
      for child in children {
        layout_widgets(child, out);
      }
    }
  }
}

/// The actions' log, opened to add to, with a heading line for what follows.
fn log_file(path: &std::path::Path, heading: &str) -> Option<std::fs::File> {
  use std::io::Write;
  if let Some(dir) = path.parent() {
    let _ = std::fs::create_dir_all(dir);
  }
  let mut file = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(path)
    .ok()?;
  let _ = writeln!(file, "\n── {} · {}", crate::template::now_stamp(), heading);
  Some(file)
}

/// A running command started by an action.
/// A copy of what a finished job kept for its undo.
fn pending_copy(pending: &Pending) -> Pending {
  Pending {
    action: pending.action.clone(),
    selected: pending.selected.clone(),
    page: pending.page.clone(),
    vars: pending.vars.clone(),
    before: pending.before.clone(),
  }
}

/// A bordered box over the screen, `w`% by `h`% of it.
fn popup(
  frame: &mut Frame,
  area: Rect,
  title: &str,
  lines: Vec<Line>,
  styles: &Styles,
  w: u16,
  h: u16,
) -> Rect {
  popup_with(frame, area, title, lines, styles, (w, h), true)
}

/// A box over the screen, `size` (percent wide, percent high) at least; its lines
/// wrapped, or cut where they end (so each line is on a row of its own, as clicks need).
fn popup_with(
  frame: &mut Frame,
  area: Rect,
  title: &str,
  lines: Vec<Line>,
  styles: &Styles,
  size: (u16, u16),
  wrap: bool,
) -> Rect {
  let (w, h) = size;
  let width = (area.width * w / 100).max(30).min(area.width);
  let height = (lines.len() as u16 + 2)
    .max(area.height * h / 100)
    .min(area.height);
  let rect = Rect::new(
    (area.width - width) / 2,
    (area.height - height) / 2,
    width,
    height,
  );
  frame.render_widget(Clear, rect);
  let block = Block::default()
    .borders(Borders::ALL)
    .border_style(styles.get("title"))
    .title(Span::styled(
      crate::widgets::fit(&format!(" {} ", title), width.saturating_sub(2) as usize),
      styles.get("title"),
    ))
    .style(styles.get("text"));
  let mut paragraph = Paragraph::new(lines).block(block);
  if wrap {
    paragraph = paragraph.wrap(Wrap { trim: false });
  }
  frame.render_widget(paragraph, rect);
  rect
}

/// Presets saved by the palette: (name, command line).
fn read_presets(path: &std::path::Path) -> Vec<(String, String)> {
  #[derive(serde::Deserialize)]
  struct Presets {
    #[serde(default)]
    preset: Vec<Preset>,
  }
  #[derive(serde::Deserialize)]
  struct Preset {
    name: String,
    run: String,
  }
  std::fs::read_to_string(path)
    .ok()
    .and_then(|t| toml::from_str::<Presets>(&t).ok())
    .map(|p| p.preset.into_iter().map(|p| (p.name, p.run)).collect())
    .unwrap_or_default()
}

/// Starts a command line on its own, not waited for; its output is discarded (a
/// project's commands keep their own records).
/// Starts `line` in `base` on its own, its output added to the log at `log`.
fn spawn_background(
  base: &std::path::Path,
  line: &str,
  log: &std::path::Path,
) -> Result<(), String> {
  let words = words(line);
  let (program, args) = words.split_first().ok_or("an empty command")?;
  let out = log_file(log, &format!("started in the background: {}", line));
  let mut command = std::process::Command::new(program);
  command
    .args(args)
    .current_dir(base)
    .stdin(std::process::Stdio::null());
  match out.and_then(|f| f.try_clone().ok().map(|g| (f, g))) {
    Some((f, g)) => {
      command.stdout(f).stderr(g);
    }
    None => {
      command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    }
  }
  #[cfg(unix)]
  {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
  }
  command
    .spawn()
    .map(|_| ())
    .map_err(|e| format!("cannot start {}: {}", program, e))
}

/// A new tmux session's name for a tab of terminal widget `w`: the project's folder, the
/// widget and a number of its own, so no two hubs or tabs share one.
fn tmux_session(base: &std::path::Path, w: &str) -> String {
  let project = base
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_default();
  let stamp = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_millis());
  let clean = |t: &str| t.replace(['.', ':', ' '], "-");
  format!(
    "hub-{}-{}-{}",
    clean(&project),
    clean(w),
    stamp % 1_000_000_000
  )
}

fn trim_to(text: &str, width: usize) -> String {
  crate::template::trim(text, width)
}

#[cfg(test)]
mod tests {
  use ratatui::Terminal;
  use ratatui::backend::TestBackend;

  use super::*;

  fn chord(app: &mut App, terminal: &mut Terminal<TestBackend>, code: KeyCode, m: KeyModifiers) {
    app.key(KeyEvent::new(code, m));
    terminal.draw(|f| app.draw(f)).unwrap();
  }

  fn click(app: &mut App, terminal: &mut Terminal<TestBackend>, x: u16, y: u16) {
    app.mouse(MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column: x,
      row: y,
      modifiers: KeyModifiers::NONE,
    });
    terminal.draw(|f| app.draw(f)).unwrap();
  }

  fn field(long: &str, takes_value: bool, default: Option<&str>) -> Field {
    Field {
      long: Some(long.into()),
      short: None,
      value_name: long.to_uppercase(),
      help: String::new(),
      required: false,
      takes_value,
      multiple: false,
      choices: Vec::new(),
      default: default.map(str::to_string),
      text: default.unwrap_or("").to_string(),
      on: false,
    }
  }

  /// Waits for the actions running to end, then draws.
  fn finish(app: &mut App, terminal: &mut Terminal<TestBackend>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !app.jobs.is_empty() {
      assert!(Instant::now() < deadline, "the action did not finish");
      std::thread::sleep(Duration::from_millis(20));
      app.poll_jobs();
    }
    terminal.draw(|f| app.draw(f)).unwrap();
  }

  /// The fixture, keeping its state in a folder of its own.
  fn fixture(name: &str) -> Loaded {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/.hub.toml");
    let mut loaded = crate::config::load(&path).unwrap();
    let state = std::env::temp_dir().join(format!("hub-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    loaded.config.hub.state = Some(state.to_string_lossy().into_owned());
    loaded
  }

  /// A repository with f.txt changed in two places, and the git window open on it.
  fn git_window(name: &str) -> (crate::git::tests::Repo, Loaded) {
    let repo = crate::git::tests::Repo::new(name);
    let lines: Vec<String> = (1..=30).map(|i| format!("line {}", i)).collect();
    repo.write("f.txt", &(lines.join("\n") + "\n"));
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "base"]);
    let mut changed = lines.clone();
    changed[1] = "line 2 changed".into();
    changed[25] = "line 26 changed".into();
    repo.write("f.txt", &(changed.join("\n") + "\n"));
    std::fs::write(repo.0.join(".git/info/exclude"), ".hub.toml\n").unwrap();
    let git_toml = concat!(env!("CARGO_MANIFEST_DIR"), "/git.hub.toml");
    let state = repo.0.join(".git/hub-state");
    repo.write(
      ".hub.toml",
      &format!(
        "[hub]\nname = \"t\"\nstart = \"git\"\nstate = \"{}\"\ninclude = [\"{}\"]\n",
        state.display(),
        git_toml
      ),
    );
    let loaded = crate::config::load(&repo.0.join(".hub.toml")).unwrap();
    (repo, loaded)
  }

  fn press(app: &mut App, terminal: &mut Terminal<TestBackend>, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
    terminal.draw(|f| app.draw(f)).unwrap();
  }

  fn screen_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
      .map(|y| {
        (0..buffer.area.width)
          .map(|x| buffer[(x, y)].symbol())
          .collect::<String>()
      })
      .collect::<Vec<_>>()
      .join("\n")
  }

  /// A chart takes focus: up and down move a crosshair from point to point, which says
  /// its time, value and nearest annotation; `v` (space) pins a point, and the crosshair
  /// then says the change from it; `+` and `-` zoom, `<` and `>` move the window; the
  /// mouse over it moves the crosshair.
  #[test]
  fn test_a_chart_zooms_moves_and_reads_its_points() {
    let dir = std::env::temp_dir().join(format!("hub-chart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
      dir.join(".hub.toml"),
      format!(
        "[hub]\nname = \"t\"\nstart = \"s\"\nstate = \"{}\"\n\
         [[source]]\nname = \"points\"\nkind = \"tsv\"\nglob = \"points.tsv\"\nheader = true\n\
         [[source]]\nname = \"commits\"\nkind = \"tsv\"\nglob = \"commits.tsv\"\nheader = true\n\
         [[widget]]\nid = \"c\"\ntype = \"chart\"\ntitle = \"SPEED\"\nsource = \"points\"\n\
         x = \"when\"\ny = \"value\"\n\
         annotate = {{ source = \"commits\", at = \"when\", text = \"{{short}}\" }}\n\
         [[screen]]\nid = \"s\"\ntitle = \"S\"\nkey = \"1\"\nlayout = \"c\"\n",
        dir.join("state").display()
      ),
    )
    .unwrap();
    let mut points = String::from("when\tvalue\n");
    for day in 1..=8 {
      points.push_str(&format!("2026-10-0{}T12:00:00Z\t{}\n", day, day * 10));
    }
    std::fs::write(dir.join("points.tsv"), points).unwrap();
    std::fs::write(
      dir.join("commits.tsv"),
      "when\tshort\n2026-10-07T09:00:00Z\tabc1234\n",
    )
    .unwrap();
    let loaded = crate::config::load(&dir.join(".hub.toml")).unwrap();
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let text = screen_text(&terminal);
    assert!(
      text.contains("2026-10-01 12:00") && text.contains("2026-10-08 12:00"),
      "{}",
      text
    );
    press(&mut app, &mut terminal, KeyCode::Down);
    assert!(
      screen_text(&terminal).contains("2026-10-08 12:00 · 80"),
      "{}",
      screen_text(&terminal)
    );
    press(&mut app, &mut terminal, KeyCode::Up);
    // The point of the 7th, and the commit nearest it.
    assert!(
      screen_text(&terminal).contains("2026-10-07 12:00 · 70 · abc1234"),
      "{}",
      screen_text(&terminal)
    );
    press(&mut app, &mut terminal, KeyCode::Char(' '));
    assert_eq!(app.pins["c"].1, 70.0);
    press(&mut app, &mut terminal, KeyCode::Down);
    assert!(
      screen_text(&terminal).contains("80 (+14.3% vs pinned)"),
      "{}",
      screen_text(&terminal)
    );
    // Zoomed in twice: the newest quarter; moved back, an earlier one.
    press(&mut app, &mut terminal, KeyCode::Char('+'));
    press(&mut app, &mut terminal, KeyCode::Char('+'));
    let text = screen_text(&terminal);
    assert!(
      !text.contains("2026-10-01 12:00") && text.contains("2026-10-08 12:00"),
      "{}",
      text
    );
    press(&mut app, &mut terminal, KeyCode::Char('<'));
    assert!(
      !screen_text(&terminal).contains("2026-10-08 12:00"),
      "{}",
      screen_text(&terminal)
    );
    press(&mut app, &mut terminal, KeyCode::Char('>'));
    press(&mut app, &mut terminal, KeyCode::Char('-'));
    press(&mut app, &mut terminal, KeyCode::Char('-'));
    assert!(screen_text(&terminal).contains("2026-10-01 12:00"));
    // The mouse at the chart's left edge puts the crosshair on the first point.
    app.mouse(MouseEvent {
      kind: MouseEventKind::Moved,
      column: 6,
      row: 8,
      modifiers: KeyModifiers::NONE,
    });
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(
      screen_text(&terminal).contains("2026-10-01 12:00 · 10"),
      "{}",
      screen_text(&terminal)
    );
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn test_a_form_gives_its_command_line() {
    let mut target = field("target", true, None);
    target.long = None;
    target.required = true;
    let mut form = Form {
      path: "fuzz run".into(),
      about: String::new(),
      fields: vec![
        target,
        field("time", true, Some("60")),
        field("detach", false, None),
      ],
      at: 0,
    };
    assert_eq!(form_problems(&form), ["TARGET is required"]);
    form.fields[0].text = "format".into();
    form.fields[1].text = "300".into();
    form.fields[2].on = true;
    assert!(form_problems(&form).is_empty());
    assert_eq!(
      form_line("cargo xtask {path} {args}", &form),
      "cargo xtask fuzz run format --time 300 --detach"
    );
    form.fields[1].text = "60".into();
    assert_eq!(
      form_line("x {path} {args}", &form),
      "x fuzz run format --detach"
    );
  }

  /// A footer with more hints than room shows those that fit, then how many more help
  /// lists, and keeps help and quit at its right; help lists them all first.
  #[test]
  fn test_a_full_footer_says_how_many_more() {
    let loaded = fixture("footer");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(44, 20)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let footer = screen_text(&terminal).lines().last().unwrap().to_string();
    assert!(footer.contains("more ?"), "{}", footer);
    assert!(footer.trim_end().ends_with("? help  q quit"), "{}", footer);
    let mut wide = Terminal::new(TestBackend::new(120, 20)).unwrap();
    wide.draw(|f| app.draw(f)).unwrap();
    let footer = screen_text(&wide).lines().last().unwrap().to_string();
    assert!(!footer.contains("more"), "{}", footer);
    press(&mut app, &mut terminal, KeyCode::Char('?'));
    let shown = screen_text(&terminal);
    assert!(
      shown.contains("HERE") && shown.contains("wipe"),
      "{}",
      shown
    );
  }

  /// A log draws its colors, folds the sections that do not matter, moves between
  /// problem lines (or search matches, which it finds without leaving the rest), and
  /// wraps when asked.
  #[test]
  fn test_a_log_folds_finds_moves_and_wraps() {
    let dir = std::env::temp_dir().join(format!("hub-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
      dir.join(".hub.toml"),
      format!(
        "[hub]\nname = \"t\"\nstart = \"s\"\nstate = \"{}\"\n\
         [[widget]]\nid = \"out\"\ntype = \"log\"\ntitle = \"OUT\"\npath = \"run.log\"\n\
         follow = false\nfold = \"^\\\\s+Running \"\nlevel = \"error|FAILED\"\n\
         styles = {{ error = \"bad\", FAILED = \"bad\" }}\n\
         [[screen]]\nid = \"s\"\ntitle = \"S\"\nkey = \"1\"\nlayout = \"out\"\n",
        dir.join("state").display()
      ),
    )
    .unwrap();
    std::fs::write(
      dir.join("run.log"),
      "   \x1b[1;32mCompiling\x1b[0m a\n     Running tests/one.rs\ntest one ... ok\n\
       test two ... ok\n     Running tests/two.rs\ntest x ... FAILED\nerror: boom\n\
       \x20    Running tests/three.rs\ntest y ... ok and a line long enough that it wraps at forty-six\n",
    )
    .unwrap();
    let loaded = crate::config::load(&dir.join(".hub.toml")).unwrap();
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(48, 16)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let text = screen_text(&terminal);
    // Colors drawn, not shown as codes.
    assert!(
      text.contains("Compiling a") && !text.contains("[1;32m"),
      "{}",
      text
    );
    // The first section folds; the one with a failure and the last stay open.
    assert!(text.contains("tests/one.rs  ▸ 2 more line(s)"), "{}", text);
    assert!(!text.contains("test one"), "{}", text);
    assert!(
      text.contains("test x ... FAILED") && text.contains("test y"),
      "{}",
      text
    );
    assert!(text.contains("n N problems"), "{}", text);
    press(&mut app, &mut terminal, KeyCode::Char('n'));
    assert_eq!(app.log_at["out"], 5);
    press(&mut app, &mut terminal, KeyCode::Char('n'));
    assert_eq!(app.log_at["out"], 6);
    press(&mut app, &mut terminal, KeyCode::Char('n'));
    assert_eq!(app.log_at["out"], 5, "round again");
    // A search finds without leaving lines out, opening the fold a match is in.
    app.search = Some("ok".into());
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('n'));
    assert_eq!(app.log_at["out"], 8);
    let text = screen_text(&terminal);
    assert!(
      text.contains("test one ... ok") && text.contains("FAILED"),
      "{}",
      text
    );
    assert!(text.contains("/ok 3 of 3"), "{}", text);
    app.search = None;
    // F opens every section; alt+z wraps the long line.
    press(&mut app, &mut terminal, KeyCode::Char('F'));
    assert!(screen_text(&terminal).contains("test two ... ok"));
    assert!(!screen_text(&terminal).contains("forty-six"));
    app.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(
      screen_text(&terminal).contains("forty-six"),
      "{}",
      screen_text(&terminal)
    );
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// A form with more fields than the screen has rows shows those around the one being
  /// edited, with its command line still in view; a number field is checked as typed.
  #[test]
  fn test_a_long_form_keeps_its_field_and_command_in_view() {
    let loaded = fixture("long-form");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 14)).unwrap();
    let mut fields: Vec<Field> = (0..20)
      .map(|i| field(&format!("f{}", i), true, None))
      .collect();
    fields[15].value_name = "N".into();
    app.overlay = Overlay::Form(Box::new(Form {
      path: "many".into(),
      about: "Many flags.".into(),
      fields,
      at: 15,
    }));
    terminal.draw(|f| app.draw(f)).unwrap();
    let shown = screen_text(&terminal);
    assert!(shown.contains("›--f15"), "{}", shown);
    assert!(!shown.contains("--f0 "), "{}", shown);
    assert!(shown.contains("$ "), "{}", shown);
    typed(&mut app, &mut terminal, "4x");
    assert!(
      screen_text(&terminal).contains("f15 must be a whole number"),
      "{}",
      screen_text(&terminal)
    );
    // A click lands on the field drawn there.
    let (rect, i) = app.hits.items[0];
    click(&mut app, &mut terminal, rect.x + 2, rect.y);
    let Overlay::Form(form) = &app.overlay else {
      panic!("no form");
    };
    assert_eq!(form.at, i);
    assert!(i > 0);
  }

  /// alt+m and a letter marks the place; ' and the letter goes back to it, its row
  /// selected.
  #[test]
  fn test_a_marked_place_is_gone_back_to() {
    let loaded = fixture("places");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Down);
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    app.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT));
    press(&mut app, &mut terminal, KeyCode::Char('q'));
    assert!(!app.quit, "the letter names the place, not quit");
    press(&mut app, &mut terminal, KeyCode::Up);
    press(&mut app, &mut terminal, KeyCode::Char('3'));
    assert_eq!(app.screen, "grid");
    press(&mut app, &mut terminal, KeyCode::Char('\''));
    press(&mut app, &mut terminal, KeyCode::Char('q'));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(app.screen, "main");
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
  }

  /// A session saved and taken up again comes back to the same place: screen, page,
  /// focus per screen, selection (found again by identity), search, the way back.
  #[test]
  fn test_a_session_comes_back() {
    let loaded = fixture("session");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('3'));
    chord(&mut app, &mut terminal, KeyCode::Right, KeyModifiers::SHIFT);
    assert_eq!(app.focused_widget(), Some("file-b"));
    press(&mut app, &mut terminal, KeyCode::Char('1'));
    press(&mut app, &mut terminal, KeyCode::Down);
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert_eq!(app.pages.len(), 1);
    let mut session = app.session();
    // As if the rows had moved since: the index is wrong, the row is still found.
    session.selection.insert("runs".into(), 0);
    let mut back = App::new(&loaded);
    back.restore(crate::session::Session::load_from_str(
      &serde_json::to_string(&session).unwrap(),
    ));
    terminal.draw(|f| back.draw(f)).unwrap();
    assert_eq!(back.screen, "main");
    assert_eq!(back.pages.len(), 1);
    assert!(screen_text(&terminal).contains("RUN a"));
    press(&mut back, &mut terminal, KeyCode::Esc);
    terminal.draw(|f| back.draw(f)).unwrap();
    assert_eq!(back.selected["runs"]["id"], Value::Str("a".into()));
    // The grid comes back with its focus where it was.
    press(&mut back, &mut terminal, KeyCode::Char('3'));
    assert_eq!(back.focused_widget(), Some("file-b"));
    // And the way back goes through the places of the session before.
    chord(&mut back, &mut terminal, KeyCode::Left, KeyModifiers::ALT);
    assert_eq!(back.screen, "main");
  }

  /// alt+y copies the focused table: its row as JSON, its rows as TSV or Markdown, as
  /// shown; `:w FILE` saves the screen as text.
  #[test]
  fn test_a_table_copies_and_a_screen_saves() {
    let loaded = fixture("yank");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let alt_y = |app: &mut App, terminal: &mut Terminal<TestBackend>| {
      app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT));
      terminal.draw(|f| app.draw(f)).unwrap();
    };
    alt_y(&mut app, &mut terminal);
    assert!(screen_text(&terminal).contains("as a Markdown table"));
    press(&mut app, &mut terminal, KeyCode::Char('m'));
    assert_eq!(
      app.last_yank,
      "| STATE | COMMAND | TOOK | PEAK |\n|---|---|---|---|\n\
       | failed | test | 5.5s | 1.07 GB |\n| passed | build --release | 1m30s | 2 MB |"
    );
    alt_y(&mut app, &mut terminal);
    press(&mut app, &mut terminal, KeyCode::Char('t'));
    assert!(
      app
        .last_yank
        .starts_with("STATE\tCOMMAND\tTOOK\tPEAK\nfailed\ttest\t5.5s")
    );
    alt_y(&mut app, &mut terminal);
    press(&mut app, &mut terminal, KeyCode::Char('r'));
    assert!(app.last_yank.contains("\"id\": \"b\""), "{}", app.last_yank);
    let out = std::env::temp_dir().join(format!("hub-screen-{}.txt", std::process::id()));
    assert!(app.ex(&format!("w {}", out.display())));
    let saved = std::fs::read_to_string(&out).unwrap();
    assert!(
      saved.contains("RUNS") && saved.contains("build --release"),
      "{}",
      saved
    );
    let _ = std::fs::remove_file(&out);
  }

  /// In a terminal with the keys the hub's, a search finds lines in its whole history,
  /// n and N moving between them as the terminal scrolls to each; alt+y copies what it
  /// shows.
  #[test]
  fn test_a_terminal_history_is_searched_and_copied() {
    let mut loaded = fixture("termsearch");
    let shell = loaded
      .config
      .widgets
      .iter_mut()
      .find(|w| w.id == "shell")
      .unwrap();
    shell.command =
      Some("sh -c 'i=1; while [ $i -le 200 ]; do echo line $i; i=$((i+1)); done; sleep 30'".into());
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('2'));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !screen_text(&terminal).contains("line 200") {
      assert!(Instant::now() < deadline, "{}", screen_text(&terminal));
      std::thread::sleep(Duration::from_millis(50));
      terminal.draw(|f| app.draw(f)).unwrap();
    }
    assert!(app.tab("shell").unwrap().history().len() >= 200);
    app.search = Some("line 42".into());
    press(&mut app, &mut terminal, KeyCode::Char('n'));
    let text = screen_text(&terminal);
    assert!(
      text.contains("line 42") && !text.contains("line 200"),
      "{}",
      text
    );
    assert!(
      app
        .toasts
        .iter()
        .any(|(t, ..)| t.contains("1 of 1, line 42"))
    );
    app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT));
    assert!(app.last_yank.contains("line 42"), "{}", app.last_yank);
  }

  /// A terminal kept in tmux outlives the hub: a hub started again with the session
  /// takes the same program up again; closing the tab ends it.
  #[test]
  fn test_a_terminal_kept_in_tmux_outlives_the_hub() {
    if std::process::Command::new("tmux")
      .arg("-V")
      .output()
      .is_err()
    {
      return;
    }
    let mut loaded = fixture("tmux");
    let shell = loaded
      .config
      .widgets
      .iter_mut()
      .find(|w| w.id == "shell")
      .unwrap();
    shell.keep = Some("tmux".into());
    let wait_for = |app: &mut App, terminal: &mut Terminal<TestBackend>, text: &str| {
      let deadline = Instant::now() + Duration::from_secs(10);
      while !screen_text(terminal).contains(text) {
        assert!(
          Instant::now() < deadline,
          "no {}:\n{}",
          text,
          screen_text(terminal)
        );
        std::thread::sleep(Duration::from_millis(50));
        terminal.draw(|f| app.draw(f)).unwrap();
      }
    };
    // tmux has started once it has the whole screen.
    let started = |app: &mut App, terminal: &mut Terminal<TestBackend>| {
      let deadline = Instant::now() + Duration::from_secs(10);
      while !app.tab("shell").is_some_and(|t| t.full_screen()) {
        assert!(Instant::now() < deadline, "tmux did not start");
        std::thread::sleep(Duration::from_millis(50));
        terminal.draw(|f| app.draw(f)).unwrap();
      }
    };
    let session = {
      let mut app = App::new(&loaded);
      let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
      press(&mut app, &mut terminal, KeyCode::Char('2'));
      press(&mut app, &mut terminal, KeyCode::Enter);
      started(&mut app, &mut terminal);
      typed(
        &mut app,
        &mut terminal,
        "KEPT=kept-$((6*7)); echo ready-$((40+2))",
      );
      press(&mut app, &mut terminal, KeyCode::Enter);
      wait_for(&mut app, &mut terminal, "ready-42");
      app.session()
    };
    // The hub is gone; its program is not.
    let name = session.terminals["shell"].tabs[0].session.clone().unwrap();
    let listed = std::process::Command::new("tmux")
      .args(["-L", "hub", "has-session", "-t", &name])
      .env_remove("TMUX")
      .status()
      .unwrap();
    assert!(listed.success());
    let mut app = App::new(&loaded);
    app.restore(session);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('2'));
    press(&mut app, &mut terminal, KeyCode::Enter);
    started(&mut app, &mut terminal);
    typed(&mut app, &mut terminal, "echo $KEPT");
    press(&mut app, &mut terminal, KeyCode::Enter);
    wait_for(&mut app, &mut terminal, "kept-42");
    // Closing the tab ends the session.
    app.close_tab("shell", 0);
    let gone = std::process::Command::new("tmux")
      .args(["-L", "hub", "has-session", "-t", &name])
      .env_remove("TMUX")
      .status()
      .unwrap();
    assert!(!gone.success());
  }

  /// A terminal's tab takes the name given it, over the program's; empty gives it back.
  #[test]
  fn test_a_terminal_tab_is_renamed() {
    let loaded = fixture("rename");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('2'));
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('n'),
      KeyModifiers::CONTROL,
    );
    press(&mut app, &mut terminal, KeyCode::Char('R'));
    assert_eq!(app.mode(), Mode::Insert);
    typed(&mut app, &mut terminal, "build");
    assert!(screen_text(&terminal).contains("tab 2: build"));
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert!(screen_text(&terminal).contains(" 2 build "));
    press(&mut app, &mut terminal, KeyCode::Char('R'));
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('u'),
      KeyModifiers::CONTROL,
    );
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert!(screen_text(&terminal).contains(" 2 sh "));
  }

  /// A terminal widget runs its program, takes every key once given the keyboard, and
  /// gives it back on ctrl+\, or on esc while no full-screen program runs.
  #[test]
  fn test_a_terminal_widget_takes_the_keys_and_gives_them_back() {
    let loaded = fixture("terminal");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('2'));
    assert!(screen_text(&terminal).contains("enter to type"));
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert_eq!(app.captured.as_deref(), Some("shell"));
    assert_eq!(app.mode(), Mode::Insert);
    // q is the shell's now, not quit.
    typed(&mut app, &mut terminal, "echo q-$((6*7))");
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert!(!app.quit);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !screen_text(&terminal).contains("q-42") {
      assert!(Instant::now() < deadline, "{}", screen_text(&terminal));
      std::thread::sleep(Duration::from_millis(50));
      terminal.draw(|f| app.draw(f)).unwrap();
    }
    assert!(app.live_terminal());
    // ctrl+\ gives the keys back, as terminals send it (ctrl+4).
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('4'),
      KeyModifiers::CONTROL,
    );
    assert_eq!(app.captured, None);
    // At a prompt, esc does too.
    press(&mut app, &mut terminal, KeyCode::Enter);
    press(&mut app, &mut terminal, KeyCode::Esc);
    assert_eq!(app.captured, None);
    // A full-screen program keeps esc; ctrl+\ still gives the keys back.
    press(&mut app, &mut terminal, KeyCode::Enter);
    typed(&mut app, &mut terminal, "printf '\\033[?1049h'; read x");
    press(&mut app, &mut terminal, KeyCode::Enter);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.tab("shell").is_some_and(|t| t.full_screen()) {
      assert!(Instant::now() < deadline, "{}", screen_text(&terminal));
      std::thread::sleep(Duration::from_millis(50));
      terminal.draw(|f| app.draw(f)).unwrap();
    }
    assert!(screen_text(&terminal).contains("ctrl+\\ gives the keys back"));
    press(&mut app, &mut terminal, KeyCode::Esc);
    assert_eq!(app.captured.as_deref(), Some("shell"));
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('\\'),
      KeyModifiers::CONTROL,
    );
    assert_eq!(app.captured, None);
    // Tabs: a new one is shown but leaves the keys with the hub; ] and [ go between
    // them; closing a running one asks first, and leaves the keys with the hub too.
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('n'),
      KeyModifiers::CONTROL,
    );
    assert_eq!(app.captured, None);
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.terminals["shell"].list.len(), 2);
    assert_eq!(app.terminals["shell"].at, 1);
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert_eq!(app.captured.as_deref(), Some("shell"));
    typed(
      &mut app,
      &mut terminal,
      "printf '\\033]2;named\\007'; echo second-$((1+1))",
    );
    press(&mut app, &mut terminal, KeyCode::Enter);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !screen_text(&terminal).contains("second-2") {
      assert!(Instant::now() < deadline, "{}", screen_text(&terminal));
      std::thread::sleep(Duration::from_millis(50));
      terminal.draw(|f| app.draw(f)).unwrap();
    }
    assert!(screen_text(&terminal).contains(" 1 sh "));
    // A tab is named by the title its program sets.
    assert!(screen_text(&terminal).contains(" 2 named "));
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('\\'),
      KeyModifiers::CONTROL,
    );
    press(&mut app, &mut terminal, KeyCode::Char(']'));
    assert_eq!(app.terminals["shell"].at, 0);
    assert!(!screen_text(&terminal).contains("second-2"));
    press(&mut app, &mut terminal, KeyCode::Char('['));
    assert_eq!(app.terminals["shell"].at, 1);
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('x'),
      KeyModifiers::CONTROL,
    );
    assert!(screen_text(&terminal).contains("Close tab 2 (named)?"));
    press(&mut app, &mut terminal, KeyCode::Char('y'));
    assert_eq!(app.terminals["shell"].list.len(), 1);
    assert!(!screen_text(&terminal).contains(" 1 sh "));
    app.vim = true;
    press(&mut app, &mut terminal, KeyCode::Char('i'));
    assert!(app.captured.is_some());
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('\\'),
      KeyModifiers::CONTROL,
    );
    assert_eq!(app.captured, None);
    assert_eq!(app.mode(), Mode::Normal);
  }

  /// A `[[variable]]` is read by every template as `var.<name>`; its key goes to its next
  /// value, `:let` sets one, and the choice is kept for the next start.
  #[test]
  fn test_a_variable_is_chosen_and_kept() {
    let dir = std::env::temp_dir().join(format!("hub-var-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
      dir.join(".hub.toml"),
      format!(
        "[hub]\nname = \"t\"\nstart = \"s\"\nstate = \"{}\"\n\
         [[variable]]\nname = \"target\"\nvalues = [\"format\", \"layout\", \"toml\"]\nkey = \"T\"\n\
         [[widget]]\nid = \"w\"\ntype = \"text\"\nlines = [\"fuzzing {{var.target}}\"]\n\
         [[screen]]\nid = \"s\"\ntitle = \"S\"\nkey = \"1\"\nlayout = \"w\"\n",
        dir.join("state").display()
      ),
    )
    .unwrap();
    let loaded = crate::config::load(&dir.join(".hub.toml")).unwrap();
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(screen_text(&terminal).contains("fuzzing format"));
    assert!(screen_text(&terminal).contains("T target format"));
    press(&mut app, &mut terminal, KeyCode::Char('T'));
    assert!(screen_text(&terminal).contains("fuzzing layout"));
    assert!(app.ex("let target toml"));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(screen_text(&terminal).contains("fuzzing toml"));
    // Kept for the next start.
    let mut again = App::new(&loaded);
    terminal.draw(|f| again.draw(f)).unwrap();
    assert!(screen_text(&terminal).contains("fuzzing toml"));
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// An alert fires once for a new matching row, with a desktop notification; resolves
  /// when the row stops matching; fires again when it matches again; and while snoozed
  /// goes to the drawer quietly.
  #[test]
  fn test_alerts_fire_resolve_fire_again_and_snooze() {
    let dir = std::env::temp_dir().join(format!("hub-alerts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
      dir.join(".hub.toml"),
      format!(
        "[hub]\nname = \"t\"\nstart = \"s\"\nstate = \"{}\"\n\
         [[source]]\nname = \"items\"\nkind = \"tsv\"\nglob = \"items.tsv\"\nheader = true\n\
         [[widget]]\nid = \"w\"\ntype = \"list\"\nsource = \"items\"\nitem = \"{{id}}\"\n\
         [[screen]]\nid = \"s\"\ntitle = \"S\"\nkey = \"1\"\nlayout = \"w\"\n\
         [[alert]]\nsource = \"items\"\nwhen = \"status == \\\"open\\\"\"\nkey = \"id\"\n\
         text = \"{{id}} opened\"\n",
        dir.join("state").display()
      ),
    )
    .unwrap();
    let items = |rows: &[(&str, &str)]| {
      let mut text = String::from("id\tstatus\n");
      for (id, status) in rows {
        text.push_str(&format!("{}\t{}\n", id, status));
      }
      std::fs::write(dir.join("items.tsv"), text).unwrap();
    };
    let loaded = crate::config::load(&dir.join(".hub.toml")).unwrap();
    let mut app = App::new(&loaded);
    let check = |app: &mut App| {
      app.store.invalidate();
      app.check_alerts();
    };
    items(&[("a", "open")]);
    check(&mut app);
    // What was there at the start is taken as seen.
    assert!(app.alerts.is_empty());
    items(&[("a", "open"), ("b", "open")]);
    check(&mut app);
    assert_eq!(app.alerts.len(), 1);
    assert_eq!(app.alerts[0].text, "b opened");
    let sent = app.take_notifications();
    assert!(sent.contains("\x1b]9;b opened\x07"), "{:?}", sent);
    assert!(
      sent.contains("\x1b]777;notify;t;b opened\x07"),
      "{:?}",
      sent
    );
    items(&[("a", "open"), ("b", "closed")]);
    check(&mut app);
    assert!(app.alerts[0].resolved.is_some());
    items(&[("a", "open"), ("b", "open")]);
    check(&mut app);
    assert_eq!(app.alerts.len(), 1, "the old entry gives way");
    assert!(app.alerts[0].resolved.is_none());
    assert!(!app.take_notifications().is_empty());
    // Snoozed, it fires quietly.
    app
      .snoozed
      .insert("items:b".into(), std::time::SystemTime::now() + SNOOZE);
    items(&[("a", "open"), ("b", "closed")]);
    check(&mut app);
    items(&[("a", "open"), ("b", "open")]);
    let toasts = app.toasts.len();
    check(&mut app);
    assert!(app.take_notifications().is_empty());
    assert_eq!(app.toasts.len(), toasts);
    assert!(app.alerts[0].resolved.is_none());
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// Space marks rows and `ctrl+a` all of them; an action that runs a command on a pane
  /// with marked rows runs once for each, after asking; esc clears the marks.
  #[test]
  fn test_an_action_runs_on_every_marked_row() {
    let mut loaded = fixture("marks");
    let out = std::env::temp_dir().join(format!("hub-marks-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let run = format!("sh -c 'echo {{sel.runs.id}} >> {}'", out.display());
    let mut action = loaded.config.actions[0].clone();
    action.id = "note".into();
    action.label = "note".into();
    action.key = "n".into();
    action.on = "runs".into();
    action.when = None;
    action.confirm = None;
    action.run = Some(run.clone());
    action.then = None;
    loaded
      .templates
      .insert(run.clone(), crate::template::Template::parse(&run).unwrap());
    loaded.config.actions.push(action);
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char(' '));
    assert_eq!(app.marks["runs"].len(), 1);
    assert!(screen_text(&terminal).contains("●"));
    // esc clears them.
    press(&mut app, &mut terminal, KeyCode::Esc);
    assert!(!app.marks.contains_key("runs"));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(app.marks["runs"].len(), 2);
    press(&mut app, &mut terminal, KeyCode::Char('n'));
    assert!(
      screen_text(&terminal).contains("note on 2 marked row(s)?"),
      "{}",
      screen_text(&terminal)
    );
    press(&mut app, &mut terminal, KeyCode::Char('y'));
    finish(&mut app, &mut terminal);
    let mut ids: Vec<String> = std::fs::read_to_string(&out)
      .unwrap()
      .lines()
      .map(str::to_string)
      .collect();
    ids.sort();
    assert_eq!(ids, ["a", "b"]);
    assert!(!app.marks.contains_key("runs"));
    // The selection is as it was before the batch: marking moved it down to a.
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    let _ = std::fs::remove_file(&out);
  }

  /// Back and forward go through the places left, as a browser's do; vim mode adds
  /// ctrl+o and tab, as vim's jump list.
  #[test]
  fn test_back_and_forward_retrace_screens_and_pages() {
    let loaded = fixture("jumps");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert_eq!(app.pages.len(), 1);
    press(&mut app, &mut terminal, KeyCode::Char('2'));
    assert_eq!((app.screen.as_str(), app.pages.len()), ("shell", 0));
    chord(&mut app, &mut terminal, KeyCode::Left, KeyModifiers::ALT);
    assert_eq!((app.screen.as_str(), app.pages.len()), ("main", 1));
    chord(&mut app, &mut terminal, KeyCode::Left, KeyModifiers::ALT);
    assert_eq!((app.screen.as_str(), app.pages.len()), ("main", 0));
    // Nothing further back.
    chord(&mut app, &mut terminal, KeyCode::Left, KeyModifiers::ALT);
    assert_eq!(app.pages.len(), 0);
    chord(&mut app, &mut terminal, KeyCode::Right, KeyModifiers::ALT);
    assert_eq!(app.pages.len(), 1);
    app.vim = true;
    press(&mut app, &mut terminal, KeyCode::Tab);
    assert_eq!(app.screen, "shell");
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('o'),
      KeyModifiers::CONTROL,
    );
    assert_eq!((app.screen.as_str(), app.pages.len()), ("main", 1));
    // A new place ends the way forward.
    press(&mut app, &mut terminal, KeyCode::Esc);
    press(&mut app, &mut terminal, KeyCode::Tab);
    assert_eq!((app.screen.as_str(), app.pages.len()), ("main", 0));
  }

  /// With sources loading in the background, a pane says `loading…` until its rows
  /// arrive, and alerts wait for their source: the rows already there when the hub starts
  /// are taken as seen, not reported as new.
  #[test]
  fn test_background_loading_shows_loading_and_does_not_flood_alerts() {
    let mut loaded = fixture("background");
    loaded.config.alerts.push(crate::config::AlertSpec {
      source: "runs".into(),
      when: "true".into(),
      key: "id".into(),
      text: "run {id}".into(),
      style: None,
      open: None,
    });
    loaded
      .exprs
      .insert("true".into(), crate::expr::Expr::parse("true").unwrap());
    loaded.templates.insert(
      "run {id}".into(),
      crate::template::Template::parse("run {id}").unwrap(),
    );
    let loaded = std::sync::Arc::new(loaded);
    let mut app = App::new(&loaded);
    app.store.go_background(loaded.clone());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    app.check_alerts();
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(
      screen_text(&terminal).contains("loading…"),
      "{}",
      screen_text(&terminal)
    );
    assert!(!app.started_alerts);
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.store.busy() {
      assert!(Instant::now() < deadline, "the loads never ended");
      std::thread::sleep(Duration::from_millis(10));
      app.store.receive();
    }
    app.check_alerts();
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(app.started_alerts);
    assert!(app.alerts.is_empty(), "{:?}", app.alerts.len());
    assert!(!app.toasts.iter().any(|(t, ..)| t.starts_with("run ")));
    let text = screen_text(&terminal);
    assert!(
      !text.contains("loading…") && text.contains("failed  test"),
      "{}",
      text
    );
  }

  /// ctrl+k stops the newest action still running, with what its command started; it is
  /// not a failure. The footer offers it while one runs.
  #[cfg(unix)]
  #[test]
  fn test_cancel_stops_a_running_action() {
    let mut loaded = fixture("cancel");
    let mut action = loaded.config.actions[0].clone();
    action.id = "long".into();
    action.label = "long".into();
    action.key = "z".into();
    action.on = "global".into();
    action.when = None;
    action.confirm = None;
    action.run = Some("sh -c 'sleep 30 & wait'".into());
    action.then = None;
    loaded.templates.insert(
      "sh -c 'sleep 30 & wait'".into(),
      crate::template::Template::parse("sh -c 'sleep 30 & wait'").unwrap(),
    );
    loaded.config.actions.push(action);
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('z'));
    assert_eq!(app.jobs.len(), 1);
    assert!(
      screen_text(&terminal).contains("ctrl+k stop"),
      "{}",
      screen_text(&terminal)
    );
    let started = Instant::now();
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    finish(&mut app, &mut terminal);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(app.jobs.is_empty());
    assert!(app.toasts.iter().any(|(t, ..)| t == "long: stopped"));
    assert!(!app.alerts.iter().any(|a| a.text.contains("long failed")));
    // With nothing running, it says so.
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert!(app.toasts.iter().any(|(t, ..)| t == "no action is running"));
  }

  #[test]
  fn test_choices_cycle_and_are_checked() {
    let mut f = field("sanitizer", true, Some("address"));
    f.choices = vec!["address".into(), "none".into()];
    edit_field(&mut f, &KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(f.text, "none");
    edit_field(&mut f, &KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(f.text, "address");
    f.text = "thread".into();
    let form = Form {
      path: String::new(),
      about: String::new(),
      fields: vec![f],
      at: 0,
    };
    assert_eq!(
      form_problems(&form),
      ["sanitizer must be one of address, none"]
    );
  }

  /// An action that fails is said in a toast, kept in the alerts drawer with what its
  /// command printed, and written with its output to the actions' log; quitting while one
  /// runs asks first.
  #[test]
  fn test_failures_are_kept_and_quitting_asks_while_actions_run() {
    let mut loaded = fixture("feedback");
    let mut add = |id: &str, key: &str, run: &str| {
      let mut action = loaded.config.actions[0].clone();
      action.id = id.into();
      action.label = id.into();
      action.key = key.into();
      action.on = "global".into();
      action.when = None;
      action.confirm = None;
      action.run = Some(run.into());
      action.then = None;
      loaded
        .templates
        .insert(run.into(), crate::template::Template::parse(run).unwrap());
      loaded.config.actions.push(action);
    };
    add("fails", "z", "sh -c 'echo no-such-thing >&2; exit 3'");
    add("waits", "Z", "sleep 3");
    let log =
      std::path::PathBuf::from(loaded.config.hub.state.clone().unwrap()).join("actions.log");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('z'));
    finish(&mut app, &mut terminal);
    assert!(
      app
        .alerts
        .iter()
        .any(|a| a.text.contains("fails failed") && a.text.contains("no-such-thing"))
    );
    let kept = std::fs::read_to_string(&log).unwrap();
    assert!(
      kept.contains("fails (") && kept.contains("no-such-thing"),
      "{}",
      kept
    );
    press(&mut app, &mut terminal, KeyCode::Char('Z'));
    press(&mut app, &mut terminal, KeyCode::Char('q'));
    assert!(!app.quit);
    assert!(screen_text(&terminal).contains("An action is still running: waits."));
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert!(!app.quit);
    press(&mut app, &mut terminal, KeyCode::Char('q'));
    press(&mut app, &mut terminal, KeyCode::Char('y'));
    assert!(app.quit);
  }

  /// Keys drive the fixture hub: selection, pages, help, the palette and its form, and a
  /// confirmation.
  #[test]
  fn test_keys_drive_screens_pages_overlays_and_forms() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/.hub.toml");
    let loaded = crate::config::load(&path).unwrap();
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(app.selected["runs"]["id"], Value::Str("b".into()));
    press(&mut app, &mut terminal, KeyCode::Down);
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    // Moving the selection where all the rows fit does not scroll them.
    assert!(screen_text(&terminal).contains("failed  test"));
    assert_eq!(app.scroll.get("runs").copied().unwrap_or(0), 0);
    press(&mut app, &mut terminal, KeyCode::Up);
    // The retry action only shows for a failed run, which b is.
    assert!(screen_text(&terminal).contains("r retry"));
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert_eq!(app.pages.last().map(|p| p.0.as_str()), Some("run"));
    assert!(screen_text(&terminal).contains("RUN b"));
    press(&mut app, &mut terminal, KeyCode::Esc);
    assert!(app.pages.is_empty());
    press(&mut app, &mut terminal, KeyCode::Char('?'));
    assert!(screen_text(&terminal).contains("KEYS · vim keys off"));
    // Help scrolls; its last lines come into view.
    for _ in 0..6 {
      press(&mut app, &mut terminal, KeyCode::PageDown);
    }
    assert!(screen_text(&terminal).contains("ACTIONS"));
    assert!(matches!(app.overlay, Overlay::Help(_)));
    // To its end and back by the bindings.
    press(&mut app, &mut terminal, KeyCode::End);
    assert!(screen_text(&terminal).contains("PALETTE COMMANDS"));
    press(&mut app, &mut terminal, KeyCode::Home);
    assert!(screen_text(&terminal).contains("HERE"));
    press(&mut app, &mut terminal, KeyCode::Esc);
    // The palette: search, then the form of the command found.
    press(&mut app, &mut terminal, KeyCode::Char(':'));
    for c in "buil".chars() {
      press(&mut app, &mut terminal, KeyCode::Char(c));
    }
    assert!(screen_text(&terminal).contains("Build it"));
    // The terminal's cursor is where the next letter goes, for screen readers.
    let rect = app.hits.popup.unwrap();
    let at = terminal.get_cursor_position().unwrap();
    assert_eq!((at.x, at.y), (rect.x + 3 + 4, rect.y + 1));
    press(&mut app, &mut terminal, KeyCode::Enter);
    let Overlay::Form(form) = &app.overlay else {
      panic!("no form");
    };
    assert_eq!(form.path, "build");
    assert!(screen_text(&terminal).contains("TARGET is required"));
    // A switch says on or off; x flips it, n sets it off, y on.
    assert!(screen_text(&terminal).contains("›--release"));
    assert!(screen_text(&terminal).contains("[ ] off"));
    press(&mut app, &mut terminal, KeyCode::Char('x'));
    assert!(screen_text(&terminal).contains("[✓] on"));
    press(&mut app, &mut terminal, KeyCode::Char('n'));
    assert!(screen_text(&terminal).contains("[ ] off"));
    press(&mut app, &mut terminal, KeyCode::Char('y'));
    press(&mut app, &mut terminal, KeyCode::Tab);
    for c in "core".chars() {
      press(&mut app, &mut terminal, KeyCode::Char(c));
    }
    let Overlay::Form(form) = &app.overlay else {
      panic!("no form");
    };
    assert_eq!(
      form_line("tool {path} {args}", form),
      "tool build --release core"
    );
    assert!(screen_text(&terminal).contains("$ tool build --release core"));
    // Closed, the form is kept: opened again, it is as it was left.
    press(&mut app, &mut terminal, KeyCode::Esc);
    press(&mut app, &mut terminal, KeyCode::Char(':'));
    typed(&mut app, &mut terminal, "buil");
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert!(screen_text(&terminal).contains("$ tool build --release core"));
    assert!(screen_text(&terminal).contains("tab next field"));
    press(&mut app, &mut terminal, KeyCode::Esc);
    // A confirmation: n leaves it, nothing runs.
    press(&mut app, &mut terminal, KeyCode::Char('w'));
    assert!(screen_text(&terminal).contains("Wipe b?"));
    // Enter is not yes.
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert!(matches!(app.overlay, Overlay::None));
    press(&mut app, &mut terminal, KeyCode::Char('w'));
    press(&mut app, &mut terminal, KeyCode::Char('n'));
    assert!(matches!(app.overlay, Overlay::None));
    press(&mut app, &mut terminal, KeyCode::Char('q'));
    assert!(app.quit);
  }

  /// Panes are gone to by direction, by the one focused before, and by number; focus
  /// shows as a thick border and a marker, not only a color.
  #[test]
  fn test_panes_are_reached_by_direction_last_and_number() {
    let loaded = fixture("panes");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('3'));
    let focused = |app: &App| app.focused_widget().unwrap_or("").to_string();
    assert_eq!(focused(&app), "runs");
    assert!(screen_text(&terminal).contains("┏ ▶ RUNS"));
    let shift = KeyModifiers::SHIFT;
    chord(&mut app, &mut terminal, KeyCode::Right, shift);
    // B is beside RUNS; C is lower down.
    assert_eq!(focused(&app), "file-b");
    chord(&mut app, &mut terminal, KeyCode::Down, shift);
    assert_eq!(focused(&app), "file-c");
    chord(&mut app, &mut terminal, KeyCode::Left, shift);
    // A overlaps C across more than RUNS does.
    assert_eq!(focused(&app), "file-a");
    chord(&mut app, &mut terminal, KeyCode::Left, shift);
    assert_eq!(focused(&app), "file-a");
    chord(&mut app, &mut terminal, KeyCode::Up, shift);
    assert_eq!(focused(&app), "runs");
    // ctrl+w p: the pane before.
    let ctrl = KeyModifiers::CONTROL;
    chord(&mut app, &mut terminal, KeyCode::Char('w'), ctrl);
    press(&mut app, &mut terminal, KeyCode::Char('p'));
    assert_eq!(focused(&app), "file-a");
    // ctrl+w q numbers the panes; 3 is file-b, in layout order.
    chord(&mut app, &mut terminal, KeyCode::Char('w'), ctrl);
    press(&mut app, &mut terminal, KeyCode::Char('q'));
    assert!(matches!(app.overlay, Overlay::Pick));
    assert!(screen_text(&terminal).contains("  3  "));
    press(&mut app, &mut terminal, KeyCode::Char('3'));
    assert_eq!(focused(&app), "file-b");
    // With vim keys on: h and l, and 4 ctrl+w w for the fourth pane.
    app.vim = true;
    press(&mut app, &mut terminal, KeyCode::Char('h'));
    assert_eq!(focused(&app), "runs");
    press(&mut app, &mut terminal, KeyCode::Char('4'));
    chord(&mut app, &mut terminal, KeyCode::Char('w'), ctrl);
    press(&mut app, &mut terminal, KeyCode::Char('w'));
    assert_eq!(focused(&app), "file-c");
  }

  /// A search keeps the rows that have it, ignoring case unless it has a capital, and the
  /// pane's title says it and how much it kept.
  #[test]
  fn test_search_filters_tables_and_says_so() {
    let loaded = fixture("search");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('/'));
    typed(&mut app, &mut terminal, "BUILD");
    assert!(screen_text(&terminal).contains("/BUILD (0 of 2)"));
    for _ in 0..5 {
      press(&mut app, &mut terminal, KeyCode::Backspace);
    }
    typed(&mut app, &mut terminal, "build");
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert!(screen_text(&terminal).contains("/build (1 of 2)"));
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    press(&mut app, &mut terminal, KeyCode::Esc);
    assert!(!screen_text(&terminal).contains("/build"));
  }

  /// `>` and `<` sort a table by its columns (a column of one field by that field's
  /// value), as a click on a header does; the selected row stays selected. A search
  /// takes `field:text` and `-word`.
  #[test]
  fn test_tables_sort_by_column_and_filter_by_field() {
    let loaded = fixture("sort");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let order =
      |app: &App| -> Vec<String> { app.shown["runs"].iter().map(|r| r["id"].text()).collect() };
    assert_eq!(order(&app), ["b", "a"]);
    press(&mut app, &mut terminal, KeyCode::Char('>'));
    assert!(screen_text(&terminal).contains("STATE ▲"));
    press(&mut app, &mut terminal, KeyCode::Char('>'));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(order(&app), ["a", "b"]);
    // b was selected and still is.
    assert_eq!(app.selected["runs"]["id"], Value::Str("b".into()));
    press(&mut app, &mut terminal, KeyCode::Char('>'));
    // TOOK sorts by the seconds, not the text: 5.5s before 1m30s.
    assert_eq!(order(&app), ["b", "a"]);
    press(&mut app, &mut terminal, KeyCode::Char('<'));
    assert!(screen_text(&terminal).contains("TOOK ▼"));
    assert_eq!(order(&app), ["a", "b"]);
    // A click on the COMMAND header sorts by it; again, the other way.
    let (x, y) = {
      let pane = app.hits.panes.iter().find(|p| p.widget == "runs").unwrap();
      (pane.columns[1].0 + 1, pane.data_top - 1)
    };
    click(&mut app, &mut terminal, x, y);
    assert!(screen_text(&terminal).contains("COMMAND ▲"));
    click(&mut app, &mut terminal, x, y);
    assert!(screen_text(&terminal).contains("COMMAND ▼"));
    assert_eq!(order(&app), ["b", "a"]);
    // A search by field, and one leaving a word out.
    app.search = Some("state:passed".into());
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(order(&app), ["a"]);
    app.search = Some("-failed".into());
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(order(&app), ["a"]);
  }

  /// A terminal's tabs come back after a restart with their names and their last screen;
  /// across a reload of the config they go on running.
  #[test]
  fn test_terminal_tabs_come_back_and_go_on_across_a_reload() {
    let loaded = fixture("tabs-back");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('2'));
    press(&mut app, &mut terminal, KeyCode::Enter);
    typed(&mut app, &mut terminal, "echo kept-$((20+1))");
    press(&mut app, &mut terminal, KeyCode::Enter);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !screen_text(&terminal).contains("kept-21") {
      assert!(Instant::now() < deadline, "{}", screen_text(&terminal));
      std::thread::sleep(Duration::from_millis(50));
      terminal.draw(|f| app.draw(f)).unwrap();
    }
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('\\'),
      KeyModifiers::CONTROL,
    );
    press(&mut app, &mut terminal, KeyCode::Char('R'));
    typed(&mut app, &mut terminal, "work");
    press(&mut app, &mut terminal, KeyCode::Enter);
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('n'),
      KeyModifiers::CONTROL,
    );
    // A reload: the same programs go on.
    let parser = std::sync::Arc::clone(&app.terminals["shell"].list[0].parser);
    let carry = app.into_carry();
    let mut again = App::from_carry(&loaded, carry);
    terminal.draw(|f| again.draw(f)).unwrap();
    assert!(std::sync::Arc::ptr_eq(
      &parser,
      &again.terminals["shell"].list[0].parser
    ));
    assert_eq!(again.screen, "shell");
    // A restart: new programs, under the screens they had and with their names.
    let session = again.session();
    drop(again);
    let mut back = App::new(&loaded);
    back.restore(session);
    terminal.draw(|f| back.draw(f)).unwrap();
    assert_eq!(back.terminals["shell"].list.len(), 2);
    assert_eq!(back.terminals["shell"].at, 1);
    assert!(screen_text(&terminal).contains(" 1 work "));
    press(&mut back, &mut terminal, KeyCode::Char('['));
    assert!(screen_text(&terminal).contains("kept-21"));
  }

  /// A conflicted file opens on its conflicts; one is taken as ours, as theirs or both,
  /// each hunk alone, and the file is marked resolved.
  #[test]
  fn test_the_git_window_resolves_a_conflict_hunk_by_hunk() {
    let (repo, loaded) = git_window("conflict");
    repo.git(&["stash", "-q"]);
    repo.write("g.txt", "a\nb\nc\nd\ne\nf\ng\nh\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "g"]);
    repo.git(&["switch", "-qc", "other"]);
    repo.write("g.txt", "a\nB-theirs\nc\nd\ne\nf\nG-theirs\nh\n");
    repo.git(&["commit", "-qam", "theirs"]);
    repo.git(&["switch", "-q", "main"]);
    repo.write("g.txt", "a\nB-ours\nc\nd\ne\nf\nG-ours\nh\n");
    repo.git(&["commit", "-qam", "ours"]);
    let merged = std::process::Command::new("git")
      .args([
        "-c",
        "user.name=a",
        "-c",
        "user.email=a@b",
        "merge",
        "other",
      ])
      .current_dir(&repo.0)
      .output()
      .unwrap();
    assert!(!merged.status.success());
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let down_to =
      |app: &mut App, terminal: &mut Terminal<TestBackend>, w: &str, f: &str, v: &str| {
        press(app, terminal, KeyCode::Home);
        for _ in 0..60 {
          if app
            .selected
            .get(w)
            .is_some_and(|r| r.get(f).map(Value::text).as_deref() == Some(v))
          {
            return;
          }
          press(app, terminal, KeyCode::Down);
        }
        panic!("no {} {} in {}:\n{}", f, v, w, screen_text(terminal));
      };
    down_to(&mut app, &mut terminal, "git-files", "path", "g.txt");
    press(&mut app, &mut terminal, KeyCode::Char('X'));
    assert!(
      screen_text(&terminal).contains("Conflicts in g.txt"),
      "{}",
      screen_text(&terminal)
    );
    // The first conflict, ours.
    down_to(
      &mut app,
      &mut terminal,
      "git-conflict-lines",
      "kind",
      "ours",
    );
    press(&mut app, &mut terminal, KeyCode::Char('o'));
    finish(&mut app, &mut terminal);
    let text = std::fs::read_to_string(repo.0.join("g.txt")).unwrap();
    assert!(
      text.starts_with("a\nB-ours\nc\n") && text.contains("<<<<<<<"),
      "{}",
      text
    );
    // Taken back, then the other way.
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('z'),
      KeyModifiers::CONTROL,
    );
    press(&mut app, &mut terminal, KeyCode::Char('y'));
    finish(&mut app, &mut terminal);
    assert!(
      std::fs::read_to_string(repo.0.join("g.txt"))
        .unwrap()
        .matches("<<<<<<<")
        .count()
        == 2
    );
    app.store.invalidate();
    terminal.draw(|f| app.draw(f)).unwrap();
    down_to(
      &mut app,
      &mut terminal,
      "git-conflict-lines",
      "kind",
      "theirs",
    );
    press(&mut app, &mut terminal, KeyCode::Char('t'));
    finish(&mut app, &mut terminal);
    app.store.invalidate();
    terminal.draw(|f| app.draw(f)).unwrap();
    down_to(
      &mut app,
      &mut terminal,
      "git-conflict-lines",
      "kind",
      "ours",
    );
    press(&mut app, &mut terminal, KeyCode::Char('b'));
    finish(&mut app, &mut terminal);
    let text = std::fs::read_to_string(repo.0.join("g.txt")).unwrap();
    assert_eq!(text, "a\nB-theirs\nc\nd\ne\nf\nG-ours\nG-theirs\nh\n");
    press(&mut app, &mut terminal, KeyCode::Char('a'));
    finish(&mut app, &mut terminal);
    assert!(
      repo
        .git(&["diff", "--name-only", "--diff-filter=U"])
        .is_empty()
    );
  }

  /// The git window on a real repository: space on a hunk stages that hunk alone, and c
  /// commits it with the message asked for.
  #[test]
  fn test_the_git_window_stages_a_hunk_and_commits() {
    let (repo, loaded) = git_window("window");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(app.selected["git-files"]["path"].text(), "f.txt");
    assert!(screen_text(&terminal).contains("── Not staged ──"));
    // To the changes, down to the first hunk's header, and stage it.
    press(&mut app, &mut terminal, KeyCode::Tab);
    assert_eq!(app.focused_widget(), Some("git-diff"));
    while app.selected["git-diff"]["kind"].text() != "hunk" {
      press(&mut app, &mut terminal, KeyCode::Down);
    }
    assert!(screen_text(&terminal).contains("space stage hunk"));
    press(&mut app, &mut terminal, KeyCode::Char(' '));
    finish(&mut app, &mut terminal);
    let staged = repo.git(&["diff", "--cached"]);
    assert!(
      staged.contains("+line 2 changed") && !staged.contains("line 26"),
      "{}",
      staged
    );
    assert!(screen_text(&terminal).contains("── Staged ──"));
    // Commit it, the message asked for.
    press(&mut app, &mut terminal, KeyCode::Char('c'));
    assert!(screen_text(&terminal).contains("Summary *"));
    // Empty, it may not run.
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('s'),
      KeyModifiers::CONTROL,
    );
    assert!(screen_text(&terminal).contains("Summary cannot be empty"));
    typed(&mut app, &mut terminal, "Change line two");
    press(&mut app, &mut terminal, KeyCode::Tab);
    typed(&mut app, &mut terminal, "Only the first hunk.");
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('s'),
      KeyModifiers::CONTROL,
    );
    finish(&mut app, &mut terminal);
    let message = repo.git(&["log", "-1", "--format=%B"]);
    assert_eq!(message.trim(), "Change line two\n\nOnly the first hunk.");
    let left = repo.git(&["diff"]);
    assert!(left.contains("+line 26 changed") && !left.contains("line 2 changed"));
  }

  /// One line is staged alone; its hunk is unstaged again; a branch is made through the
  /// form asked for and left by switching back.
  #[test]
  fn test_the_git_window_stages_lines_and_switches_branches() {
    let (repo, loaded) = git_window("lines");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Tab);
    while app.selected["git-diff"]["text"].text() != "+line 26 changed" {
      press(&mut app, &mut terminal, KeyCode::Down);
    }
    press(&mut app, &mut terminal, KeyCode::Char('S'));
    finish(&mut app, &mut terminal);
    let staged = repo.git(&["diff", "--cached"]);
    assert!(
      staged.contains("+line 26 changed") && !staged.contains("-line 26\n"),
      "{}",
      staged
    );
    // The removed line was kept, so the staged file has both.
    assert!(
      repo
        .git(&["show", ":f.txt"])
        .contains("line 26\nline 26 changed\n")
    );
    // Unstage that hunk: on the staged side, space.
    while !(app.selected["git-diff"]["side"].text() == "staged"
      && app.selected["git-diff"]["kind"].text() == "hunk")
    {
      press(&mut app, &mut terminal, KeyCode::Down);
    }
    press(&mut app, &mut terminal, KeyCode::Char(' '));
    finish(&mut app, &mut terminal);
    assert_eq!(repo.git(&["diff", "--cached"]), "");
    // Refs: a new branch, then back to main.
    app.go("git-refs");
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('n'));
    typed(&mut app, &mut terminal, "topic");
    press(&mut app, &mut terminal, KeyCode::Enter);
    finish(&mut app, &mut terminal);
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "topic");
    while app.selected["git-branch-list"]["name"].text() != "main" {
      press(&mut app, &mut terminal, KeyCode::Down);
    }
    press(&mut app, &mut terminal, KeyCode::Char(' '));
    finish(&mut app, &mut terminal);
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
  }

  /// Discarding a file's changes, deleting a branch and committing can each be taken
  /// back with ctrl+z, after asking: the changes, the branch where it was, the commit's
  /// changes staged again.
  #[test]
  fn test_the_git_window_undoes_discards_deletes_and_commits() {
    let (repo, loaded) = git_window("undo");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let undo = |app: &mut App, terminal: &mut Terminal<TestBackend>| {
      chord(app, terminal, KeyCode::Char('z'), KeyModifiers::CONTROL);
      assert!(
        screen_text(terminal).contains("Undo "),
        "{}",
        screen_text(terminal)
      );
      press(app, terminal, KeyCode::Char('y'));
      finish(app, terminal);
    };
    // Discard f.txt's changes, then bring them back.
    let changes = repo.git(&["diff"]);
    press(&mut app, &mut terminal, KeyCode::Char('d'));
    press(&mut app, &mut terminal, KeyCode::Char('y'));
    finish(&mut app, &mut terminal);
    assert_eq!(repo.git(&["diff"]), "");
    assert!(
      screen_text(&terminal).contains("undo discard"),
      "{}",
      screen_text(&terminal)
    );
    undo(&mut app, &mut terminal);
    assert_eq!(repo.git(&["diff"]), changes);
    // Delete a branch, then make it again where it was.
    repo.git(&["branch", "topic"]);
    let at = repo.git(&["rev-parse", "topic"]);
    app.go("git-refs");
    app.store.invalidate();
    terminal.draw(|f| app.draw(f)).unwrap();
    while app.selected["git-branch-list"]["name"].text() != "topic" {
      press(&mut app, &mut terminal, KeyCode::Down);
    }
    press(&mut app, &mut terminal, KeyCode::Char('D'));
    press(&mut app, &mut terminal, KeyCode::Char('y'));
    finish(&mut app, &mut terminal);
    assert!(repo.git(&["branch", "--list", "topic"]).is_empty());
    undo(&mut app, &mut terminal);
    assert_eq!(repo.git(&["rev-parse", "topic"]), at);
    // Commit everything, then take the commit back: its changes staged again.
    repo.git(&["add", "."]);
    let head = repo.git(&["rev-parse", "HEAD"]);
    app.go("git");
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char('c'));
    typed(&mut app, &mut terminal, "Both lines");
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('s'),
      KeyModifiers::CONTROL,
    );
    finish(&mut app, &mut terminal);
    assert_ne!(repo.git(&["rev-parse", "HEAD"]), head);
    undo(&mut app, &mut terminal);
    assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
    assert!(repo.git(&["diff", "--cached"]).contains("+line 26 changed"));
    // Nothing left to take back.
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('z'),
      KeyModifiers::CONTROL,
    );
    assert!(app.toasts.iter().any(|(t, ..)| t == "nothing to undo"));
  }

  /// The mouse: tabs switch screens, a click focuses a pane and selects a row, a double
  /// click opens it, the page's chip goes back, the wheel moves, a footer hint does what
  /// it says, a click on a terminal gives it the keys, the palette and forms take clicks.
  #[test]
  fn test_the_mouse_does_what_is_under_it() {
    let loaded = fixture("mouse");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let tab = |app: &App, id: &str| app.hits.tabs.iter().find(|(_, t)| t == id).unwrap().0;
    let grid = tab(&app, "grid");
    click(&mut app, &mut terminal, grid.x + 1, grid.y);
    assert_eq!(app.screen, "grid");
    let pane = |app: &App, w: &str| app.hits.panes.iter().find(|p| p.widget == w).unwrap().area;
    let c = pane(&app, "file-c");
    click(&mut app, &mut terminal, c.x + 3, c.y + 3);
    assert_eq!(app.focused_widget(), Some("file-c"));
    let main = tab(&app, "main");
    click(&mut app, &mut terminal, main.x + 1, main.y);
    // The second row of runs: a click selects it, a second click there opens it.
    let top = app
      .hits
      .panes
      .iter()
      .find(|p| p.widget == "runs")
      .unwrap()
      .data_top;
    click(&mut app, &mut terminal, 5, top + 1);
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    click(&mut app, &mut terminal, 5, top + 1);
    assert_eq!(app.pages.len(), 1);
    let chip = app.hits.page_chip.unwrap();
    click(&mut app, &mut terminal, chip.x + 1, chip.y);
    assert!(app.pages.is_empty());
    wheel_at(&mut app, &mut terminal, 5, top, false);
    assert_eq!(app.selected["runs"]["id"], Value::Str("b".into()));
    // A footer hint: w asks to wipe; a click outside the question answers no.
    let wipe = app.hits.footer.iter().find(|(_, k)| k == "w").unwrap().0;
    click(&mut app, &mut terminal, wipe.x, wipe.y);
    assert!(screen_text(&terminal).contains("Wipe b?"));
    click(&mut app, &mut terminal, 0, 0);
    assert!(matches!(app.overlay, Overlay::None));
    // The palette: a click picks a command, a second opens its form, where a click on a
    // switch flips it.
    press(&mut app, &mut terminal, KeyCode::Char(':'));
    let (first, _) = app.hits.items[0];
    click(&mut app, &mut terminal, first.x + 2, first.y);
    click(&mut app, &mut terminal, first.x + 2, first.y);
    let Overlay::Form(form) = &app.overlay else {
      panic!("no form");
    };
    assert!(!form.fields[0].on);
    let (field, _) = app.hits.items[0];
    click(&mut app, &mut terminal, field.x + 2, field.y);
    let Overlay::Form(form) = &app.overlay else {
      panic!("no form");
    };
    assert!(form.fields[0].on);
    press(&mut app, &mut terminal, KeyCode::Esc);
    // A terminal: a click on its screen gives it the keys; one elsewhere takes them back.
    let shell = tab(&app, "shell");
    click(&mut app, &mut terminal, shell.x + 1, shell.y);
    let (screen, _) = app.hits.term_screens[0].clone();
    click(&mut app, &mut terminal, screen.x + 2, screen.y + 2);
    assert_eq!(app.captured.as_deref(), Some("shell"));
    click(&mut app, &mut terminal, main.x + 1, main.y);
    assert_eq!(app.captured, None);
    assert_eq!(app.screen, "main");
    // Off, the mouse does nothing.
    press(&mut app, &mut terminal, KeyCode::F(3));
    assert!(!app.mouse_on && app.mouse_changed);
    click(&mut app, &mut terminal, grid.x + 1, grid.y);
    assert_eq!(app.screen, "main");
  }

  /// Typing part of a row's id in the palette offers that row, wherever it is; enter goes
  /// to its screen with it selected, its pane focused.
  #[test]
  fn test_the_palette_goes_to_a_row_anywhere() {
    let dir = std::env::temp_dir().join(format!("hub-goto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
      dir.join("items.tsv"),
      "id\tnote\nalpha\tone\nbeta\ttwo\ngamma\tthree\n",
    )
    .unwrap();
    std::fs::write(
      dir.join(".hub.toml"),
      format!(
        "[hub]\nname = \"t\"\nstart = \"home\"\nstate = \"{}\"\n\
         [[source]]\nname = \"items\"\nkind = \"tsv\"\nglob = \"items.tsv\"\nheader = true\n\
         [[widget]]\nid = \"hello\"\ntype = \"text\"\nlines = [\"home\"]\n\
         [[widget]]\nid = \"notes\"\ntype = \"text\"\nlines = [\"notes\"]\n\
         [[widget]]\nid = \"list\"\ntype = \"list\"\ntitle = \"ITEMS\"\nsource = \"items\"\n\
         selectable = true\nitem = \"{{id}}\"\n\
         [[screen]]\nid = \"home\"\ntitle = \"Home\"\nkey = \"1\"\nlayout = \"hello\"\n\
         [[screen]]\nid = \"stuff\"\ntitle = \"Stuff\"\nkey = \"2\"\n\
         layout = {{ split = \"rows\", sizes = [3, \"fill\"], children = [\"notes\", \"list\"] }}\n",
        dir.join("state").display()
      ),
    )
    .unwrap();
    let loaded = crate::config::load(&dir.join(".hub.toml")).unwrap();
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    press(&mut app, &mut terminal, KeyCode::Char(':'));
    typed(&mut app, &mut terminal, "gam");
    assert!(
      screen_text(&terminal).contains("→ gamma"),
      "{}",
      screen_text(&terminal)
    );
    assert!(screen_text(&terminal).contains("Stuff · ITEMS"));
    press(&mut app, &mut terminal, KeyCode::Enter);
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(app.screen, "stuff");
    assert_eq!(app.focused_widget(), Some("list"));
    assert_eq!(app.selected["list"]["id"], Value::Str("gamma".into()));
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// Vim mode: counts and motions, tabs, the command line, and the toggle outliving the
  /// session.
  #[test]
  fn test_vim_keys_move_switch_and_quit() {
    let loaded = fixture("vim");
    let mut app = App::new(&loaded);
    assert!(!app.vim);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    // Off, j is nobody's key, and :q is not a command.
    press(&mut app, &mut terminal, KeyCode::Char('j'));
    assert_eq!(app.selected["runs"]["id"], Value::Str("b".into()));
    press(&mut app, &mut terminal, KeyCode::Char(':'));
    typed(&mut app, &mut terminal, "set vim");
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert!(app.vim);
    assert!(screen_text(&terminal).contains("NORMAL"));
    press(&mut app, &mut terminal, KeyCode::Char('G'));
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    typed(&mut app, &mut terminal, "gg");
    assert_eq!(app.selected["runs"]["id"], Value::Str("b".into()));
    typed(&mut app, &mut terminal, "2G");
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    press(&mut app, &mut terminal, KeyCode::Char('k'));
    assert_eq!(app.selected["runs"]["id"], Value::Str("b".into()));
    typed(&mut app, &mut terminal, "5j");
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    // ctrl+] opens the row's page and ctrl+t closes it, as vim's tags.
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char(']'),
      KeyModifiers::CONTROL,
    );
    assert_eq!(app.pages.len(), 1);
    chord(
      &mut app,
      &mut terminal,
      KeyCode::Char('t'),
      KeyModifiers::CONTROL,
    );
    assert!(app.pages.is_empty());
    // esc drops a command half typed: this G goes to the last row, not the fifth.
    typed(&mut app, &mut terminal, "gg5");
    press(&mut app, &mut terminal, KeyCode::Esc);
    press(&mut app, &mut terminal, KeyCode::Char('G'));
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    typed(&mut app, &mut terminal, "g");
    press(&mut app, &mut terminal, KeyCode::Esc);
    press(&mut app, &mut terminal, KeyCode::Char('g'));
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    assert!(screen_text(&terminal).contains("NORMAL  g"));
    press(&mut app, &mut terminal, KeyCode::Esc);
    typed(&mut app, &mut terminal, "gt");
    assert_eq!(app.screen, "shell");
    typed(&mut app, &mut terminal, "gT");
    assert_eq!(app.screen, "main");
    // Screens' keys follow g; a digit alone is a count, not a screen.
    typed(&mut app, &mut terminal, "2");
    assert_eq!(app.screen, "main");
    press(&mut app, &mut terminal, KeyCode::Esc);
    typed(&mut app, &mut terminal, "g2");
    assert_eq!(app.screen, "shell");
    typed(&mut app, &mut terminal, "1gt");
    assert_eq!(app.screen, "main");
    press(&mut app, &mut terminal, KeyCode::Char(':'));
    assert!(screen_text(&terminal).contains("COMMAND"));
    typed(&mut app, &mut terminal, "shell");
    // What enter will do is the first line of the list.
    assert!(screen_text(&terminal).contains("go to screen shell"));
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert_eq!(app.screen, "shell");
    // The toggle is kept for the next session.
    assert!(App::new(&loaded).vim);
    press(&mut app, &mut terminal, KeyCode::Char(':'));
    typed(&mut app, &mut terminal, "q");
    press(&mut app, &mut terminal, KeyCode::Enter);
    assert!(app.quit);
    press(&mut app, &mut terminal, KeyCode::F(2));
    assert!(!app.vim && !App::new(&loaded).vim);
  }

  /// Without color nothing is drawn in a color, and the selection is still seen:
  /// reversed, and marked.
  #[test]
  fn test_without_color_the_selection_still_shows() {
    let mut loaded = fixture("no-color");
    loaded.config.hub.color = Some(false);
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    for cell in buffer.content() {
      assert_eq!(cell.fg, ratatui::style::Color::Reset, "{:?}", cell);
      assert_eq!(cell.bg, ratatui::style::Color::Reset, "{:?}", cell);
    }
    let y = app.hits.panes[0].data_top;
    assert_eq!(buffer[(0, y)].symbol(), "┃");
    assert_eq!(buffer[(1, y)].symbol(), "▶");
    assert!(
      buffer[(5, y)]
        .modifier
        .contains(ratatui::style::Modifier::REVERSED)
    );
  }

  /// `ctrl+w z` gives the focused pane the whole screen and takes it back; going to
  /// another pane unzooms. Typing `ctrl+w` shows what may follow it.
  #[test]
  fn test_zoom_and_the_keys_that_may_follow() {
    let loaded = fixture("zoom");
    let mut app = App::new(&loaded);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(screen_text(&terminal).contains("SERIES"));
    let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
    app.key(ctrl_w);
    terminal.draw(|f| app.draw(f)).unwrap();
    let text = screen_text(&terminal);
    assert!(text.contains("ctrl+w …"), "{}", text);
    assert!(
      text.contains("z  the focused pane over the whole screen"),
      "{}",
      text
    );
    press(&mut app, &mut terminal, KeyCode::Char('z'));
    let text = screen_text(&terminal);
    assert!(text.contains("RUNS · zoomed"), "{}", text);
    assert!(!text.contains("SERIES"), "{}", text);
    assert!(text.contains("unzoom"), "{}", text);
    // The rows fill the screen and stay selectable.
    press(&mut app, &mut terminal, KeyCode::Down);
    assert_eq!(app.selected["runs"]["id"], Value::Str("a".into()));
    app.key(ctrl_w);
    press(&mut app, &mut terminal, KeyCode::Char('z'));
    assert!(screen_text(&terminal).contains("SERIES"));
    app.key(ctrl_w);
    press(&mut app, &mut terminal, KeyCode::Char('z'));
    assert!(!screen_text(&terminal).contains("SERIES"));
    press(&mut app, &mut terminal, KeyCode::Tab);
    assert!(screen_text(&terminal).contains("SERIES"));
    assert!(app.zoomed.is_none());
  }

  fn typed(app: &mut App, terminal: &mut Terminal<TestBackend>, text: &str) {
    for c in text.chars() {
      press(app, terminal, KeyCode::Char(c));
    }
  }

  fn wheel_at(app: &mut App, terminal: &mut Terminal<TestBackend>, x: u16, y: u16, down: bool) {
    app.mouse(MouseEvent {
      kind: if down {
        MouseEventKind::ScrollDown
      } else {
        MouseEventKind::ScrollUp
      },
      column: x,
      row: y,
      modifiers: KeyModifiers::NONE,
    });
    terminal.draw(|f| app.draw(f)).unwrap();
  }
}
