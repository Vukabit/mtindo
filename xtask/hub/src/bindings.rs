//! The engine's keys: each of its actions with the keys `[bindings]` gives it, or its
//! defaults, once for vim keys off and once for on. Keys are handled, and help, the footer
//! and `--keys` drawn, from this one table.

use std::collections::BTreeSet;

use crossterm::event::KeyEvent;

use crate::config::{Config, KeyNames};
use crate::keys::{self, Key};

/// The engine's actions, in the order help lists them, with what each does.
pub const ACTIONS: &[(&str, &str)] = &[
  ("up", "up a row"),
  ("down", "down a row"),
  ("half_up", "half a pane up"),
  ("half_down", "half a pane down"),
  ("page_up", "a pane up"),
  ("page_down", "a pane down"),
  ("top", "the first row"),
  ("bottom", "the last row"),
  ("next", "the next pane"),
  ("previous", "the previous pane"),
  ("pane_left", "the pane to the left"),
  ("pane_right", "the pane to the right"),
  ("pane_up", "the pane above"),
  ("pane_down", "the pane below"),
  ("pane_last", "the pane focused before"),
  ("zoom", "the focused pane over the whole screen, or back"),
  ("pane_pick", "number the panes, then go to the one typed"),
  ("screen_next", "the next screen"),
  ("screen_previous", "the previous screen"),
  ("open", "open the selected row's page"),
  ("back", "close the page, or clear the search"),
  ("jump_back", "back to the place left"),
  ("mark_place", "mark where you are, by the letter typed next"),
  (
    "go_place",
    "go to the place marked by the letter typed next",
  ),
  ("jump_forward", "forward again"),
  ("insert", "give a terminal the keys"),
  ("release", "take the keys back from a terminal"),
  (
    "release_prompt",
    "the same, while no full-screen program runs in it",
  ),
  ("tab_new", "a new tab in a terminal"),
  ("tab_close", "close a terminal's tab"),
  ("tab_next", "a terminal's next tab"),
  ("tab_previous", "a terminal's previous tab"),
  ("tab_rename", "rename a terminal's tab"),
  ("search", "search the focused pane"),
  (
    "mark",
    "mark the selected row, for an action on all marked; pin a chart's point",
  ),
  ("mark_all", "mark every row shown, or unmark them all"),
  (
    "yank",
    "copy the focused pane's row or rows: JSON, TSV or Markdown",
  ),
  (
    "sort_next",
    "sort the focused table by its next column; a chart's window later",
  ),
  (
    "sort_reverse",
    "reverse the focused table's sort; a chart's window earlier",
  ),
  ("zoom_in", "a chart's newest half, and so on"),
  ("zoom_out", "a chart's window twice as wide"),
  ("next_match", "a log's next search match, or problem line"),
  (
    "previous_match",
    "a log's previous search match, or problem line",
  ),
  ("wrap", "a log's long lines broken at its width, or not"),
  (
    "fold",
    "every section of a log open, or only those that matter",
  ),
  ("palette", "the palette: run any command"),
  ("alerts", "the alerts"),
  ("refresh", "read every source again"),
  ("cancel", "stop the newest action still running"),
  (
    "undo",
    "take back the last action that can be, after asking",
  ),
  ("reload", "read the config again"),
  ("help", "this help"),
  ("vim", "vim keys on or off"),
  (
    "mouse",
    "the mouse to the hub, or to the terminal to select text",
  ),
  ("quit", "quit"),
];

/// The key typed before a screen's own key: none with vim keys off, `g` with them on (so
/// a count's digits are not taken for screens).
pub const SCREEN_PREFIX: &str = "screen_prefix";

/// Actions bound to the same keys that are not a clash: the first applies only on a
/// terminal, the other everywhere else.
const SHARED: &[(&str, &str)] = &[("open", "insert")];

/// What keys typed so far come to.
#[derive(Debug, PartialEq)]
pub enum Lookup {
  /// The actions they are the keys of, in `ACTIONS` order.
  Actions(Vec<&'static str>),
  /// A screen's keys: its id.
  Screen(String),
  /// The start of some binding: wait for more.
  Prefix,
  None,
}

/// Keys typed in turn, as written: `"g g"`, `"ctrl+w w"`.
#[derive(Clone, Debug)]
pub struct Binding {
  pub text: String,
  pub keys: Vec<Key>,
}

#[derive(Clone, Debug, Default)]
pub struct Bindings {
  tables: [Table; 2],
}

impl Bindings {
  /// The table for `config`, whose bindings have been checked.
  pub fn new(config: &Config) -> Bindings {
    let mut out = Bindings::default();
    for (vim, given) in [
      (false, &config.bindings.default),
      (true, &config.bindings.vim),
    ] {
      let table = &mut out.tables[vim as usize];
      for (action, _) in ACTIONS {
        let bound = names(given.get(*action), action, vim)
          .into_iter()
          .filter_map(|text| parse(&text).map(|keys| Binding { text, keys }))
          .collect();
        table.actions.push((action, bound));
      }
      table.prefix = names(given.get(SCREEN_PREFIX), SCREEN_PREFIX, vim)
        .first()
        .cloned()
        .unwrap_or_default();
      for s in config.screens.iter().filter(|s| !s.page) {
        let Some(key) = &s.key else { continue };
        let text = format!("{} {}", table.prefix, key).trim().to_string();
        if let Some(keys) = parse(&text) {
          table.screens.push((s.id.clone(), Binding { text, keys }));
        }
      }
    }
    out
  }

  /// The keys of `action`.
  pub fn keys(&self, action: &str, vim: bool) -> &[Binding] {
    self.tables[vim as usize]
      .actions
      .iter()
      .find(|(a, _)| *a == action)
      .map_or(&[], |(_, b)| b.as_slice())
  }

  /// The keys of `action` as help shows them, `, ` between.
  pub fn shown(&self, action: &str, vim: bool) -> String {
    let keys: Vec<String> = self
      .keys(action, vim)
      .iter()
      .map(|b| shown(&b.text))
      .collect();
    keys.join(", ")
  }

  /// The first key of `action` as shown, for a hint.
  pub fn first(&self, action: &str, vim: bool) -> Option<String> {
    self.keys(action, vim).first().map(|b| shown(&b.text))
  }

  /// Each screen with a key, and its keys as shown.
  pub fn screens(&self, vim: bool) -> Vec<(String, String)> {
    self.tables[vim as usize]
      .screens
      .iter()
      .map(|(id, b)| (id.clone(), shown(&b.text)))
      .collect()
  }

  /// The screens' keys in one hint, runs of digits as ranges: `1-9 0`, `g1-g9 g0`.
  pub fn screens_hint(&self, vim: bool) -> String {
    let table = &self.tables[vim as usize];
    let prefix = shown(&table.prefix);
    let keys: Vec<String> = table
      .screens
      .iter()
      .filter_map(|(_, b)| b.text.split_whitespace().last().map(str::to_string))
      .collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < keys.len() {
      let mut j = i;
      let digit = |k: &str| k.parse::<u32>().ok().filter(|_| k.len() == 1);
      while j + 1 < keys.len()
        && matches!((digit(&keys[j]), digit(&keys[j + 1])), (Some(a), Some(b)) if b == a + 1)
      {
        j += 1;
      }
      if j >= i + 2 {
        out.push(format!("{p}{}-{p}{}", keys[i], keys[j], p = prefix));
      } else {
        for k in &keys[i..=j] {
          out.push(format!("{}{}", prefix, k));
        }
      }
      i = j + 1;
    }
    out.join(" ")
  }

  /// What may follow `typed`, the start of longer bindings: each next key (or keys) as
  /// shown, with what it does, for the popup that offers them.
  pub fn continuations(&self, typed: &[KeyEvent], vim: bool) -> Vec<(String, String)> {
    let table = &self.tables[vim as usize];
    let rest = |b: &Binding| {
      let words: Vec<&str> = b.text.split_whitespace().skip(typed.len()).collect();
      shown(&words.join(" "))
    };
    let longer = |b: &&Binding| b.keys.len() > typed.len() && starts(&b.keys, typed);
    let mut out: Vec<(String, String)> = Vec::new();
    for (action, bindings) in &table.actions {
      let what = ACTIONS
        .iter()
        .find(|(a, _)| a == action)
        .map_or("", |(_, w)| *w);
      for binding in bindings.iter().filter(longer) {
        out.push((rest(binding), what.to_string()));
      }
    }
    for (screen, binding) in &table.screens {
      if longer(&binding) {
        out.push((rest(binding), format!("screen {}", screen)));
      }
    }
    out
  }

  /// What `typed` comes to. A whole binding wins over a longer one it starts.
  pub fn lookup(&self, typed: &[KeyEvent], vim: bool) -> Lookup {
    let table = &self.tables[vim as usize];
    let whole = |b: &Binding| b.keys.len() == typed.len() && starts(&b.keys, typed);
    let found: Vec<&'static str> = table
      .actions
      .iter()
      .filter(|(_, bs)| bs.iter().any(whole))
      .map(|(a, _)| *a)
      .collect();
    if !found.is_empty() {
      return Lookup::Actions(found);
    }
    if let Some((id, _)) = table.screens.iter().find(|(_, b)| whole(b)) {
      return Lookup::Screen(id.clone());
    }
    let longer = |b: &Binding| b.keys.len() > typed.len() && starts(&b.keys, typed);
    if table.actions.iter().any(|(_, bs)| bs.iter().any(longer))
      || table.screens.iter().any(|(_, b)| longer(b))
    {
      return Lookup::Prefix;
    }
    Lookup::None
  }

  /// What `typed` comes to among the keys that take a terminal's keys back: `release`'s,
  /// or the terminal's own `escape` keys instead; and `release_prompt`'s unless a
  /// full-screen program runs, which keeps them (esc is nvim's).
  pub fn release(
    &self,
    typed: &[KeyEvent],
    vim: bool,
    escape: Option<&[String]>,
    full_screen: bool,
  ) -> Lookup {
    let mut keys: Vec<Binding> = match escape {
      Some(names) => names
        .iter()
        .filter_map(|t| {
          parse(t).map(|keys| Binding {
            text: t.clone(),
            keys,
          })
        })
        .collect(),
      None => self.keys("release", vim).to_vec(),
    };
    if !full_screen {
      keys.extend(self.keys("release_prompt", vim).iter().cloned());
    }
    if keys
      .iter()
      .any(|b| b.keys.len() == typed.len() && starts(&b.keys, typed))
    {
      Lookup::Actions(vec!["release"])
    } else if keys
      .iter()
      .any(|b| b.keys.len() > typed.len() && starts(&b.keys, typed))
    {
      Lookup::Prefix
    } else {
      Lookup::None
    }
  }
}

/// One half of the table: vim keys off, or on.
#[derive(Clone, Debug, Default)]
struct Table {
  actions: Vec<(&'static str, Vec<Binding>)>,
  /// Each screen with a key, and the keys that go to it.
  screens: Vec<(String, Binding)>,
  prefix: String,
}

/// Problems with `[bindings]`: unknown actions, keys that are not keys, and keys that
/// can never be reached because another binding takes them first.
pub fn check(config: &Config) -> Vec<String> {
  let mut problems = Vec::new();
  for (half, given) in [
    ("default", &config.bindings.default),
    ("vim", &config.bindings.vim),
  ] {
    for (action, keys) in given {
      let what = format!("[bindings.{}] {}", half, action);
      if action != SCREEN_PREFIX && !ACTIONS.iter().any(|(a, _)| a == action) {
        let all: Vec<&str> = ACTIONS.iter().map(|(a, _)| *a).collect();
        problems.push(format!(
          "{}: the engine has no such action (one of {}, {})",
          what,
          all.join(", "),
          SCREEN_PREFIX
        ));
        continue;
      }
      for text in names(Some(keys), action, false) {
        if parse(&text).is_none() && !(action == SCREEN_PREFIX && text.is_empty()) {
          problems.push(format!("{}: `{}` is not a key", what, text));
        }
      }
    }
  }
  if !problems.is_empty() {
    return problems;
  }
  let table = Bindings::new(config);
  for vim in [false, true] {
    let half = if vim { "vim" } else { "default" };
    // Every binding but those read only while a terminal has the keys.
    let mut all: Vec<(String, &Binding)> = Vec::new();
    for (action, bs) in &table.tables[vim as usize].actions {
      if !action.starts_with("release") {
        all.extend(bs.iter().map(|b| (action.to_string(), b)));
      }
    }
    for (id, b) in &table.tables[vim as usize].screens {
      all.push((format!("screen {}", id), b));
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (i, (a, x)) in all.iter().enumerate() {
      for (b, y) in &all[i + 1..] {
        if a == b || SHARED.contains(&(a.as_str(), b.as_str())) {
          continue;
        }
        let (short, long, s, l) = if x.keys.len() <= y.keys.len() {
          (x, y, a, b)
        } else {
          (y, x, b, a)
        };
        let reaches = long.keys.len() >= short.keys.len()
          && short
            .keys
            .iter()
            .zip(&long.keys)
            .all(|(p, q)| p.code == q.code && p.modifiers == q.modifiers);
        if !reaches {
          continue;
        }
        let problem = if short.keys.len() == long.keys.len() {
          format!(
            "[bindings.{}] `{}` is bound to both {} and {}",
            half,
            shown(&short.text),
            s,
            l
          )
        } else {
          format!(
            "[bindings.{}] {} `{}` is never reached: `{}` ({}) is taken first",
            half,
            l,
            shown(&long.text),
            shown(&short.text),
            s
          )
        };
        if seen.insert(problem.clone()) {
          problems.push(problem);
        }
      }
    }
  }
  problems
}

/// An action's keys when `[bindings]` leaves it out.
pub fn defaults(action: &str, vim: bool) -> &'static [&'static str] {
  if vim {
    return match action {
      "up" => &["k", "up", "ctrl+y"],
      "down" => &["j", "down", "ctrl+e"],
      "half_up" => &["ctrl+u"],
      "half_down" => &["ctrl+d"],
      "page_up" => &["ctrl+b", "pageup"],
      "page_down" => &["ctrl+f", "pagedown"],
      "top" => &["g g", "home"],
      "bottom" => &["G", "end"],
      "next" => &["ctrl+w w"],
      "previous" => &["ctrl+w W"],
      "pane_left" => &["h", "ctrl+w h", "ctrl+w left"],
      "pane_right" => &["l", "ctrl+w l", "ctrl+w right"],
      "pane_up" => &["ctrl+w k", "ctrl+w up"],
      "pane_down" => &["ctrl+w j", "ctrl+w down"],
      "pane_last" => &["ctrl+w p"],
      "zoom" => &["ctrl+w z", "ctrl+w o"],
      "pane_pick" => &["ctrl+w q"],
      "screen_next" => &["g t"],
      "screen_previous" => &["g T"],
      "open" => &["enter", "ctrl+]"],
      "back" => &["esc", "ctrl+t"],
      "jump_back" => &["ctrl+o", "alt+left"],
      "jump_forward" => &["tab", "alt+right"],
      "mark_place" => &["alt+m"],
      "go_place" => &["'"],
      "insert" => &["i", "a", "enter"],
      "release" => &["ctrl+\\"],
      "release_prompt" => &["esc"],
      "tab_new" => &["ctrl+n"],
      "tab_close" => &["ctrl+x"],
      "tab_next" => &["]"],
      "tab_previous" => &["["],
      "tab_rename" => &["R"],
      "search" => &["/"],
      "mark" => &["v"],
      "mark_all" => &["ctrl+a"],
      "yank" => &["alt+y"],
      "sort_next" => &[">"],
      "sort_reverse" => &["<"],
      "zoom_in" => &["+"],
      "zoom_out" => &["-"],
      "next_match" => &["n"],
      "previous_match" => &["N"],
      "wrap" => &["alt+z"],
      "fold" => &["F"],
      "palette" => &[":"],
      "alerts" => &["!"],
      "refresh" => &["ctrl+r"],
      "cancel" => &["ctrl+k"],
      "undo" => &["ctrl+z"],
      "reload" => &["ctrl+l"],
      "help" => &["?"],
      "vim" => &["f2"],
      "mouse" => &["f3"],
      "quit" => &["q", "Z Z", "Z Q", "ctrl+c"],
      SCREEN_PREFIX => &["g"],
      _ => &[],
    };
  }
  match action {
    "up" => &["up"],
    "down" => &["down"],
    "page_up" => &["pageup"],
    "page_down" => &["pagedown"],
    "top" => &["home"],
    "bottom" => &["end"],
    "next" => &["tab"],
    "previous" => &["shift+tab"],
    "pane_left" => &["shift+left", "ctrl+w h"],
    "pane_right" => &["shift+right", "ctrl+w l"],
    "pane_up" => &["shift+up", "ctrl+w k"],
    "pane_down" => &["shift+down", "ctrl+w j"],
    "pane_last" => &["ctrl+w p"],
    "zoom" => &["ctrl+w z"],
    "pane_pick" => &["ctrl+w q"],
    "open" => &["enter"],
    "back" => &["esc"],
    "jump_back" => &["alt+left", "alt+b"],
    "jump_forward" => &["alt+right", "alt+f"],
    "mark_place" => &["alt+m"],
    "go_place" => &["'"],
    "insert" => &["enter"],
    "release" => &["ctrl+\\"],
    "release_prompt" => &["esc"],
    "tab_new" => &["ctrl+n"],
    "tab_close" => &["ctrl+x"],
    "tab_next" => &["]"],
    "tab_previous" => &["["],
    "tab_rename" => &["R"],
    "search" => &["/"],
    "mark" => &["space"],
    "mark_all" => &["ctrl+a"],
    "yank" => &["alt+y"],
    "sort_next" => &[">"],
    "sort_reverse" => &["<"],
    "zoom_in" => &["+"],
    "zoom_out" => &["-"],
    "next_match" => &["n"],
    "previous_match" => &["N"],
    "wrap" => &["alt+z"],
    "fold" => &["F"],
    "palette" => &[":"],
    "alerts" => &["!"],
    "refresh" => &["ctrl+r"],
    "cancel" => &["ctrl+k"],
    "undo" => &["ctrl+z"],
    "reload" => &["ctrl+l"],
    "help" => &["?"],
    "vim" => &["f2"],
    "mouse" => &["f3"],
    "quit" => &["q", "ctrl+c"],
    _ => &[],
  }
}

/// Reads keys typed in turn, separated by spaces.
pub fn parse(text: &str) -> Option<Vec<Key>> {
  let keys: Option<Vec<Key>> = text.split_whitespace().map(keys::parse).collect();
  keys.filter(|k| !k.is_empty())
}

/// A sequence as help shows it: letters typed in turn run together (`gg`, `ZZ`); other
/// keys keep their spaces (`ctrl+w w`).
pub fn shown(text: &str) -> String {
  let mut out = String::new();
  let mut last_single = false;
  for word in text.split_whitespace() {
    let single = word.chars().count() == 1;
    if !out.is_empty() && !(single && last_single) {
      out.push(' ');
    }
    out.push_str(word);
    last_single = single;
  }
  out
}

/// What a count before an action's keys does, with vim keys on.
pub fn with_count(action: &str) -> Option<&'static str> {
  match action {
    "top" | "bottom" => Some("; with a count, that row"),
    "screen_next" => Some("; with a count, that screen"),
    "tab_next" => Some("; with a count, that tab"),
    "next" => Some("; with a count, that pane"),
    "up" | "down" | "half_up" | "half_down" | "page_up" | "page_down" | "previous"
    | "screen_previous" => Some("; with a count, that many"),
    _ => None,
  }
}

fn names(keys: Option<&KeyNames>, action: &str, vim: bool) -> Vec<String> {
  match keys {
    Some(KeyNames::One(k)) => vec![k.clone()],
    Some(KeyNames::Many(ks)) => ks.clone(),
    None => defaults(action, vim)
      .iter()
      .map(|k| k.to_string())
      .collect(),
  }
}

fn starts(keys: &[Key], typed: &[KeyEvent]) -> bool {
  typed.len() <= keys.len() && typed.iter().zip(keys).all(|(e, k)| k.matches(e))
}

#[cfg(test)]
mod tests {
  use crossterm::event::{KeyCode, KeyModifiers};

  use super::*;

  fn config(extra: &str) -> Config {
    toml::from_str(&format!(
      "[hub]\nname = \"t\"\nstart = \"a\"\n\n[[screen]]\nid = \"a\"\ntitle = \"A\"\nkey = \
       \"1\"\nlayout = \"w\"\n\n[[screen]]\nid = \"b\"\ntitle = \"B\"\nkey = \"2\"\nlayout = \
       \"w\"\n\n{}",
      extra
    ))
    .unwrap()
  }

  /// The defaults as SPEC.md lists them.
  fn defaults_table() -> String {
    let cell = |action: &str, vim: bool| {
      let keys: Vec<String> = defaults(action, vim).iter().map(|k| shown(k)).collect();
      if keys.is_empty() {
        "-".to_string()
      } else {
        keys.join(", ")
      }
    };
    let mut out = String::from("| action | vim keys off | vim keys on |\n|---|---|---|\n");
    for (action, _) in ACTIONS.iter().chain(&[(SCREEN_PREFIX, "")]) {
      out.push_str(&format!(
        "| `{}` | {} | {} |\n",
        action,
        cell(action, false),
        cell(action, true)
      ));
    }
    out
  }

  fn ev(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
  }

  #[test]
  fn test_bindings_replace_defaults_and_clashes_are_problems() {
    let c = config(
      "[bindings.default]\ndown = [\"j\", \"down\"]\n\n[bindings.vim]\nscreen_prefix = \"space\"\n",
    );
    assert!(check(&c).is_empty());
    let b = Bindings::new(&c);
    assert_eq!(b.shown("down", false), "j, down");
    assert_eq!(
      b.lookup(&[ev(' '), ev('2')], true),
      Lookup::Screen("b".into())
    );
    let c = config("[bindings.default]\nup = \"x\"\ndown = \"x\"\nfly = \"f\"\n");
    assert!(check(&c)[0].contains("no such action"));
    let c = config("[bindings.default]\nup = \"x\"\ndown = \"x\"\nrefresh = \"1\"\n");
    let problems = check(&c);
    assert!(
      problems
        .iter()
        .any(|p| p.contains("`x` is bound to both up and down")),
      "{:?}",
      problems
    );
    assert!(
      problems
        .iter()
        .any(|p| p.contains("both refresh and screen a")),
      "{:?}",
      problems
    );
    let c = config("[bindings.vim]\ntop = \"g\"\n");
    assert!(
      check(&c)
        .iter()
        .any(|p| p.contains("is never reached: `g` (top) is taken first"))
    );
  }

  #[test]
  fn test_keys_typed_find_their_action_or_wait() {
    let b = Bindings::new(&config(""));
    assert_eq!(b.lookup(&[ev('1')], false), Lookup::Screen("a".into()));
    assert_eq!(b.lookup(&[ev('1')], true), Lookup::None);
    assert_eq!(b.lookup(&[ev('g')], true), Lookup::Prefix);
    assert_eq!(
      b.lookup(&[ev('g'), ev('2')], true),
      Lookup::Screen("b".into())
    );
    assert_eq!(
      b.lookup(&[ev('g'), ev('g')], true),
      Lookup::Actions(vec!["top"])
    );
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
      b.lookup(&[enter], false),
      Lookup::Actions(vec!["open", "insert"])
    );
    assert_eq!(b.screens_hint(false), "1 2");
    assert_eq!(b.shown("top", true), "gg, home");
  }

  #[test]
  fn test_sequences_parse_and_show() {
    assert_eq!(parse("g g").unwrap().len(), 2);
    assert!(parse("").is_none() && parse("g nonsense").is_none());
    assert_eq!(shown("g g"), "gg");
    assert_eq!(shown("ctrl+w w"), "ctrl+w w");
    assert_eq!(shown("ctrl+\\ ctrl+n"), "ctrl+\\ ctrl+n");
    assert_eq!(shown("g 1"), "g1");
  }

  /// The defaults clash nowhere, and every action has keys with vim on.
  #[test]
  fn test_the_defaults_are_whole_and_clash_nowhere() {
    let c = config("");
    assert_eq!(check(&c), Vec::<String>::new());
    for (action, _) in ACTIONS {
      assert!(
        !defaults(action, true).is_empty(),
        "{} has no vim keys",
        action
      );
    }
  }

  /// SPEC.md lists every default as the code has it.
  #[test]
  fn test_the_spec_lists_the_defaults() {
    let spec = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/SPEC.md")).unwrap();
    let table = defaults_table();
    assert!(
      spec.contains(&table),
      "SPEC.md section 7 should hold:\n{}",
      table
    );
  }
}
