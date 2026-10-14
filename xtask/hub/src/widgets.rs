//! Drawing widgets (SPEC.md section 5): each takes its source's rows, filtered, sorted and
//! limited, and draws them in its rectangle.

use std::collections::{BTreeMap, HashMap};

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
  Axis, Block, BorderType, Borders, Cell, Chart, Dataset, GraphType, Paragraph, Row as TableRow,
  Table,
};
use unicode_width::UnicodeWidthStr;

use crate::config::{Loaded, StyleRef, StyleSpec, WidgetSpec};
use crate::expr::{Row, Scope, Value, parse_time};
use crate::layout;
use crate::source::Store;
use crate::template::{bar, spark, trim};

/// Set when the terminal said its background is light (OSC 11, asked at start).
pub static LIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set by `--no-color`: no colors, whatever the config says.
pub static NO_COLOR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One of a chart's other lines (an annotation, a mark, the pinned point, the
/// crosshair): its points, style and name.
type Rule = (Vec<(f64, f64)>, Style, String);

/// What a widget is drawn against: the page's row, every widget's selected row, the
/// hub's own fields.
pub struct Context<'a> {
  pub loaded: &'a Loaded,
  pub styles: &'a Styles,
  pub page: Option<&'a Row>,
  pub selected: &'a BTreeMap<String, Row>,
  pub vars: &'a Row,
}

impl Context<'_> {
  pub fn scope<'b>(&'b self, row: Option<&'b Row>) -> Scope<'b> {
    Scope {
      row,
      page: self.page,
      selected: Some(self.selected),
      vars: Some(self.vars),
    }
  }

  pub fn render(&self, template: &str, row: Option<&Row>) -> String {
    self.loaded.template(template).render(&self.scope(row))
  }

  pub fn eval(&self, expr: &str, row: Option<&Row>) -> Value {
    self.loaded.expr(expr).eval(&self.scope(row))
  }

  /// A style reference's style for a row: a name, or the first rule that holds.
  pub fn style_for(&self, style: &Option<StyleRef>, row: Option<&Row>) -> Style {
    match style {
      None => self.styles.get("text"),
      Some(StyleRef::Name(n)) => self.styles.get(n),
      Some(StyleRef::Rules(rules)) => rules
        .iter()
        .find(|r| r.when.as_ref().is_none_or(|w| self.eval(w, row).truthy()))
        .map_or(self.styles.get("text"), |r| self.styles.get(&r.style)),
    }
  }
}

/// How a widget is drawn this frame.
pub struct Draw<'a> {
  pub focused: bool,
  /// The pane has the screen to itself.
  pub zoomed: bool,
  pub selected: Option<usize>,
  /// Lines scrolled; the widget sets it back to what it could show.
  pub scroll: std::cell::Cell<usize>,
  /// What the widget showed of what it has: the first and last shown, and how many, set
  /// by those that scroll, for the place shown in the bottom border.
  pub span: std::cell::Cell<Option<(usize, usize, usize)>>,
  pub search: Option<&'a str>,
  /// The column the table is sorted by, and whether descending, when sorted by hand.
  pub sort: Option<(usize, bool)>,
  /// The rows marked for an action on all of them.
  pub marked: &'a [Row],
  /// Where the table drew each column, as (x, width): for a click on its header.
  pub columns: std::cell::RefCell<Vec<(u16, u16)>>,
  /// A log's long lines are broken at its width.
  pub wrap: bool,
  /// Every section of a log open, instead of only those that matter.
  pub folds_open: bool,
  /// A log asked to move to its next (1) or previous (-1) search match, or problem line.
  pub jump: Option<i8>,
  /// The line a log's moves start from, set to where they ended.
  pub at: std::cell::Cell<Option<usize>>,
  /// A chart's window: zoomed in this many halvings, moved back this many half windows.
  pub window: (u32, i32),
  /// Where a chart's crosshair is along x, set to where it ended.
  pub crosshair: std::cell::Cell<Option<f64>>,
  /// The crosshair asked to move to the next (1) or previous (-1) point.
  pub step: Option<i8>,
  /// The column the mouse is over in a chart, to put the crosshair there.
  pub hover: Option<u16>,
  /// A chart's pinned point, its baseline.
  pub pinned: Option<(f64, f64)>,
  /// The point under the crosshair, set when it has one: what pinning takes.
  pub under: std::cell::Cell<Option<(f64, f64)>>,
}

/// How fresh a widget's rows are.
#[derive(Clone, Debug, Default)]
pub struct Fresh {
  /// Why its source's last load failed; the rows are then the ones before.
  pub error: Option<String>,
  /// Its source has not finished loading yet.
  pub waiting: bool,
  /// Its rows are old (its loads fail or take long): how old.
  pub stale: Option<std::time::Duration>,
  /// A reload has been under way for more than a second.
  pub slow: bool,
}

/// Named styles, resolved against the theme.
pub struct Styles(HashMap<String, Style>);

impl Styles {
  /// The styles of the config; with `contrast`, only the terminal's own colors, strong.
  pub fn new(loaded: &Loaded, contrast: bool) -> Styles {
    let mut theme = loaded.config.theme.clone();
    if light(loaded) {
      theme.extend(loaded.config.light.clone());
    }
    let theme = &theme;
    let plain = colorless(loaded);
    let color = |name: &str| -> Option<Color> {
      if plain {
        return None;
      }
      parse_color(theme.get(name).map_or(name, |v| v.as_str()))
    };
    if contrast {
      // The config's own styles keep their colors, on no background, bold; the
      // engine's are the terminal's own.
      let mut out = Styles::contrast(plain);
      for (name, spec) in &loaded.config.styles {
        if !out.0.contains_key(name) {
          let mut style = style_of(spec, &color).add_modifier(Modifier::BOLD);
          style.bg = None;
          out.0.insert(name.clone(), style);
        }
      }
      return out;
    }
    let mut out = HashMap::new();
    for (name, spec) in &loaded.config.styles {
      out.insert(name.clone(), style_of(spec, &color));
    }
    if plain {
      // Without color, what was shown by a background is shown reversed.
      for name in ["selected", "tab"] {
        let style = out.entry(name.to_string()).or_default();
        *style = style.add_modifier(Modifier::REVERSED);
      }
    } else if let Some(bg) = theme.get("bg").and_then(|c| parse_color(c))
      && let Some(text) = out.get_mut("text")
      && text.bg.is_none()
    {
      // The theme's background is painted, not left to the terminal's.
      *text = text.bg(bg);
    }
    Styles(out)
  }

  pub fn get(&self, name: &str) -> Style {
    self.0.get(name).copied().unwrap_or_default()
  }

  /// High contrast: the terminal's own foreground and background, nothing dimmed; what
  /// stands out bold, underlined or reversed; states in the terminal's own bright colors,
  /// bold (or, without color, bold alone).
  fn contrast(plain: bool) -> Styles {
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let hue = |c: Color| if plain { strong } else { strong.fg(c) };
    let mut out = HashMap::new();
    for (name, style) in [
      ("text", Style::default()),
      ("dim", Style::default()),
      ("frame", Style::default()),
      ("title", strong),
      ("key", strong.add_modifier(Modifier::UNDERLINED)),
      ("selected", strong.add_modifier(Modifier::REVERSED)),
      ("tab", strong.add_modifier(Modifier::REVERSED)),
      ("ok", hue(Color::LightGreen)),
      ("bad", hue(Color::LightRed)),
      ("warn", hue(Color::LightYellow)),
      ("info", hue(Color::LightCyan)),
    ] {
      out.insert(name.to_string(), style);
    }
    Styles(out)
  }
}

/// A duration in one unit, as the age of rows says it: `40s`, `3m`, `2h`.
pub fn age_text(age: std::time::Duration) -> String {
  let seconds = age.as_secs();
  match seconds {
    0..60 => format!("{}s", seconds),
    60..3600 => format!("{}m", seconds / 60),
    _ => format!("{}h", seconds / 3600),
  }
}

/// A line with SGR escape sequences (`\x1b[1;31m`) as styles over `base`; other escape
/// sequences left out. Without `colors`, colors are dropped and bold, reverse and the
/// like kept.
pub fn ansi_line(line: &str, base: Style, colors: bool) -> Line<'static> {
  if !line.contains('\x1b') {
    return Line::styled(line.to_string(), base);
  }
  let mut spans: Vec<Span<'static>> = Vec::new();
  let mut style = base;
  let mut text = String::new();
  let mut chars = line.chars().peekable();
  while let Some(c) = chars.next() {
    if c != '\x1b' {
      text.push(c);
      continue;
    }
    match chars.peek() {
      Some('[') => {
        chars.next();
        let mut params = String::new();
        let mut last = ' ';
        for c in chars.by_ref() {
          if ('@'..='~').contains(&c) {
            last = c;
            break;
          }
          params.push(c);
        }
        if last != 'm' {
          continue;
        }
        if !text.is_empty() {
          spans.push(Span::styled(std::mem::take(&mut text), style));
        }
        style = sgr(&params, style, base, colors);
      }
      // An OSC (a title, a link): up to its bell or string end.
      Some(']') => {
        while let Some(c) = chars.next() {
          if c == '\x07' || (c == '\x1b' && chars.peek() == Some(&'\\')) {
            if c == '\x1b' {
              chars.next();
            }
            break;
          }
        }
      }
      _ => {
        chars.next();
      }
    }
  }
  if !text.is_empty() {
    spans.push(Span::styled(text, style));
  }
  Line::from(spans)
}

/// Whether colors are off: `--no-color`, a `NO_COLOR` set to anything (no-color.org), or
/// `[hub] color = false`.
pub fn colorless(loaded: &Loaded) -> bool {
  NO_COLOR.load(std::sync::atomic::Ordering::Relaxed)
    || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
    || loaded.config.hub.color == Some(false)
}

/// Draws a widget; returns its rows (for selection) after drawing.
pub fn draw(
  frame: &mut Frame,
  area: Rect,
  widget: &WidgetSpec,
  store: &mut Store,
  cx: &Context,
  how: &Draw,
) -> Vec<Row> {
  let (mut rows, fresh) = rows(widget, store, cx, how.sort);
  // A search keeps the rows with a field that has it.
  let search = how.search.filter(|q| !q.is_empty());
  let before = rows.len();
  if let Some(q) = search
    && widget.kind != "log"
    && widget.kind != "file"
  {
    rows.retain(|r| matches(r, q));
  }
  let frame_style = if how.focused {
    cx.styles.get("title")
  } else {
    cx.styles.get("frame")
  };
  let mut block = Block::default();
  if widget.border != Some(false) {
    block = block.borders(Borders::ALL).border_style(frame_style);
    // Focus is shown by more than color: a thick border.
    if how.focused {
      block = block.border_type(BorderType::Thick);
    }
  }
  if let Some(title) = &widget.title {
    let first = if widget.row.as_deref() == Some("last") {
      rows.last()
    } else {
      rows.first()
    };
    let text = cx.render(
      title,
      if widget.kind == "text" || widget.kind == "kv" {
        first
      } else {
        None
      },
    );
    let marker = if how.focused { "▶ " } else { "" };
    let zoomed = if how.zoomed { " · zoomed" } else { "" };
    block = block.title(Span::styled(
      format!(" {}{}{} ", marker, text, zoomed),
      cx.styles.get("title"),
    ));
  }
  // The search in force, and how much it kept; a log says how many it found itself.
  if let Some(q) = search.filter(|_| widget.kind != "log") {
    let kept = if matches!(widget.kind.as_str(), "log" | "file") {
      String::new()
    } else {
      format!(" ({} of {})", rows.len(), before)
    };
    block = block.title(Span::styled(
      format!(" /{}{} ", q, kept),
      cx.styles.get("key"),
    ));
  }
  let inner = block.inner(area);
  frame.render_widget(block, area);
  let source = widget.source.as_deref().unwrap_or("");
  // How fresh the rows are, in words in the bottom border: a failed source keeps its
  // rows and says so; old rows say how old; a slow reload says it is under way.
  if widget.border != Some(false) && area.height > 2 {
    let status = if fresh.error.is_some() && !rows.is_empty() {
      let age = fresh
        .stale
        .map(|age| format!("· updated {} ago ", age_text(age)))
        .unwrap_or_default();
      Some((format!(" ⚠ {} failed {}", source, age), "bad"))
    } else if let Some(age) = fresh.stale {
      Some((format!(" updated {} ago ", age_text(age)), "warn"))
    } else if fresh.slow {
      Some((" ⟳ loading ".to_string(), "dim"))
    } else {
      None
    };
    if let Some((text, style)) = status {
      let width = (text.width() as u16).min(area.width.saturating_sub(2));
      frame.render_widget(
        Paragraph::new(Line::styled(text, cx.styles.get(style))),
        Rect::new(area.x + 1, area.y + area.height - 1, width, 1),
      );
    }
  }
  let needs_rows = !matches!(widget.kind.as_str(), "text" | "log" | "file");
  if let Some(e) = &fresh.error
    && (rows.is_empty() || !needs_rows)
  {
    let line = Line::styled(format!("{}: {}", source, e), cx.styles.get("bad"));
    frame.render_widget(Paragraph::new(line), inner);
    return rows;
  }
  if needs_rows && rows.is_empty() {
    let text = if fresh.waiting {
      "loading…".to_string()
    } else {
      widget
        .empty
        .as_ref()
        .map(|e| cx.render(e, None))
        .unwrap_or_else(|| "nothing yet".to_string())
    };
    frame.render_widget(
      Paragraph::new(Line::styled(text, cx.styles.get("dim"))),
      inner,
    );
    return rows;
  }
  match widget.kind.as_str() {
    "table" => table(frame, inner, widget, &rows, cx, how),
    "list" => list(frame, inner, widget, &rows, cx, how),
    "text" => {
      let row = if widget.row.as_deref() == Some("last") {
        rows.last()
      } else {
        rows.first()
      };
      let lines: Vec<Line> = widget
        .lines
        .iter()
        .map(|l| Line::styled(cx.render(l, row), cx.style_for(&widget.style, row)))
        .collect();
      frame.render_widget(Paragraph::new(lines), inner);
    }
    "kv" => {
      let row = if widget.row.as_deref() == Some("last") {
        rows.last()
      } else {
        rows.first()
      };
      let width = widget
        .pairs
        .iter()
        .map(|(k, _)| k.width())
        .max()
        .unwrap_or(0);
      let lines: Vec<Line> = widget
        .pairs
        .iter()
        .map(|(k, v)| {
          Line::from(vec![
            Span::styled(
              format!("{}  ", crate::template::pad(k, width, true)),
              cx.styles.get("dim"),
            ),
            Span::styled(cx.render(v, row), cx.styles.get("text")),
          ])
        })
        .collect();
      frame.render_widget(Paragraph::new(lines), inner);
    }
    "gauges" => gauges(frame, inner, widget, &rows, cx, how),
    "bars" => bars(frame, inner, widget, &rows, cx, how),
    "sparklines" => sparklines(frame, inner, widget, &rows, cx, how),
    "chart" => chart(frame, inner, widget, &rows, store, cx, how),
    "log" => log(frame, inner, widget, store, cx, how),
    "file" => file(frame, inner, widget, store, cx, how),
    _ => {}
  }
  // Where the pane is in what it has, when it cannot show it all: in its bottom border.
  if let Some((first, last, total)) = how.span.get()
    && total > last - first
    && widget.border != Some(false)
    && area.height > 2
  {
    let text = format!(" {}–{} of {} ", first + 1, last, total);
    let width = (text.width() as u16).min(area.width.saturating_sub(2));
    frame.render_widget(
      Paragraph::new(Line::styled(text, frame_style)),
      Rect::new(
        area.x + area.width - 1 - width,
        area.y + area.height - 1,
        width,
        1,
      ),
    );
  }
  rows
}

/// Text cut to `width` columns, its end given as `…`: what is shown is what the text
/// starts with, whichever way it is aligned.
pub fn fit(text: &str, width: usize) -> String {
  use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
  if text.width() <= width {
    return text.to_string();
  }
  let mut out = String::new();
  let mut used = 0;
  for c in text.chars() {
    let w = c.width().unwrap_or(0);
    if used + w + 1 > width {
      break;
    }
    out.push(c);
    used += w;
  }
  if width > 0 {
    out.push('…');
  }
  out
}

/// Whether `text` has `query` in it: ignoring case unless the query has a capital.
pub fn has(text: &str, query: &str) -> bool {
  if query.chars().any(char::is_uppercase) {
    text.contains(query)
  } else {
    text.to_lowercase().contains(&query.to_lowercase())
  }
}

/// Whether the light colors apply: `[hub] background`, else what the terminal said.
pub fn light(loaded: &Loaded) -> bool {
  match loaded.config.hub.background.as_deref() {
    Some("light") => true,
    Some("dark") => false,
    _ => LIGHT.load(std::sync::atomic::Ordering::Relaxed),
  }
}

/// Whether `row` has what a search asks: each word of `query` somewhere in its fields,
/// `field:text` in that field, `-word` nowhere; each ignoring case unless it has a
/// capital.
pub fn matches(row: &Row, query: &str) -> bool {
  query.split_whitespace().all(|word| {
    let (negated, word) = match word.strip_prefix('-') {
      Some(rest) if !rest.is_empty() => (true, rest),
      _ => (false, word),
    };
    let found = match word.split_once(':') {
      Some((field, text)) if row.contains_key(field) => {
        row.get(field).is_some_and(|v| has(&v.text(), text))
      }
      _ => row.values().any(|v| has(&v.text(), word)),
    };
    found != negated
  })
}

pub fn parse_color(text: &str) -> Option<Color> {
  if let Some(hex) = text.strip_prefix('#') {
    let n = u32::from_str_radix(hex, 16).ok()?;
    return Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
  }
  Some(match text {
    "black" => Color::Black,
    "red" => Color::Red,
    "green" => Color::Green,
    "yellow" => Color::Yellow,
    "blue" => Color::Blue,
    "magenta" => Color::Magenta,
    "cyan" => Color::Cyan,
    "gray" => Color::Gray,
    "white" => Color::White,
    "dark-gray" | "bright-black" => Color::DarkGray,
    "bright-red" => Color::LightRed,
    "bright-green" => Color::LightGreen,
    "bright-yellow" => Color::LightYellow,
    "bright-blue" => Color::LightBlue,
    "bright-magenta" => Color::LightMagenta,
    "bright-cyan" => Color::LightCyan,
    "reset" => Color::Reset,
    _ => return None,
  })
}

/// A widget's rows: its source's, filtered, sorted and limited, then sorted by column
/// `sort` (descending when its flag is set) when one is given.
pub fn rows(
  widget: &WidgetSpec,
  store: &mut Store,
  cx: &Context,
  sort: Option<(usize, bool)>,
) -> (Vec<Row>, Fresh) {
  let Some(source) = &widget.source else {
    return (Vec::new(), Fresh::default());
  };
  let args = source_args(cx, source);
  let data = store.get_for(cx.loaded, source, args.as_deref(), false);
  let fresh = Fresh {
    error: data.error.clone(),
    waiting: data.updated.is_none() && data.error.is_none(),
    stale: data
      .stale()
      .then(|| data.updated.map(|at| at.elapsed()))
      .flatten(),
    slow: data
      .loading
      .is_some_and(|at| at.elapsed() > std::time::Duration::from_secs(1)),
  };
  let mut rows: Vec<Row> = data
    .rows
    .iter()
    .filter(|r| {
      widget
        .filter
        .as_ref()
        .is_none_or(|f| cx.eval(f, Some(r)).truthy())
    })
    .cloned()
    .collect();
  for key in widget.sort.iter().rev() {
    let (field, descending) = match key.strip_prefix('-') {
      Some(f) => (f, true),
      None => (key.as_str(), false),
    };
    rows.sort_by(|a, b| {
      let (x, y) = (
        a.get(field).cloned().unwrap_or(Value::Null),
        b.get(field).cloned().unwrap_or(Value::Null),
      );
      let ordering = match (x.num(), y.num()) {
        (Some(p), Some(q)) => p.partial_cmp(&q).unwrap_or(std::cmp::Ordering::Equal),
        _ => x.text().cmp(&y.text()),
      };
      if descending {
        ordering.reverse()
      } else {
        ordering
      }
    });
  }
  if let Some(limit) = widget.limit {
    rows.truncate(limit);
  }
  if let Some((column, descending)) = sort
    && let Some(spec) = widget.columns.get(column)
  {
    // By the field the column shows, when it shows one alone: 1.07 GB above 2 MB.
    let field = cx
      .loaded
      .template(&spec.value)
      .sole_field()
      .map(str::to_string);
    let key = |row: &Row| match &field {
      Some(f) => row.get(f).cloned().unwrap_or(Value::Null),
      None => Value::Str(cx.render(&spec.value, Some(row))),
    };
    rows.sort_by(|a, b| {
      let (x, y) = (key(a), key(b));
      let ordering = match (x.num(), y.num()) {
        (Some(p), Some(q)) => p.partial_cmp(&q).unwrap_or(std::cmp::Ordering::Equal),
        _ => x.text().cmp(&y.text()),
      };
      if descending {
        ordering.reverse()
      } else {
        ordering
      }
    });
  }
  (rows, fresh)
}

/// A source's `args` as they read where it is shown.
pub fn source_args(cx: &Context, source: &str) -> Option<String> {
  cx.loaded
    .source(source)
    .and_then(|s| s.args.as_ref())
    .map(|a| cx.render(a, None))
}

/// `line` without its escape sequences.
pub fn strip_ansi(line: &str) -> String {
  if !line.contains('\x1b') {
    return line.to_string();
  }
  ansi_line(line, Style::default(), false)
    .spans
    .iter()
    .map(|s| s.content.as_ref())
    .collect()
}

/// Text marked for alignment by the table, cut to its column: right-aligned text is
/// padded by the cell.
fn aligned(text: &str, align: Option<&str>, width: u16) -> Line<'static> {
  let line = Line::from(fit(text, width as usize));
  match align {
    Some("right") => line.right_aligned(),
    Some("center") => line.centered(),
    _ => line,
  }
}

fn bars(
  frame: &mut Frame,
  area: Rect,
  widget: &WidgetSpec,
  rows: &[Row],
  cx: &Context,
  how: &Draw,
) {
  let labels: Vec<String> = rows
    .iter()
    .map(|r| cx.render(widget.label.as_deref().unwrap_or(""), Some(r)))
    .collect();
  let texts: Vec<String> = rows
    .iter()
    .map(|r| {
      widget
        .text
        .as_ref()
        .map(|t| cx.render(t, Some(r)))
        .unwrap_or_default()
    })
    .collect();
  let values: Vec<f64> = rows
    .iter()
    .map(|r| number(cx, &widget.value, r, 0.0))
    .collect();
  let max = values.iter().copied().fold(0.0, f64::max);
  let label_width = labels
    .iter()
    .map(|l| l.width())
    .max()
    .unwrap_or(0)
    .min(area.width as usize / 3);
  let text_width = texts.iter().map(|t| t.width()).max().unwrap_or(0);
  let bar_width = (area.width as usize)
    .saturating_sub(label_width + text_width + 3)
    .max(4);
  let shown = window(
    rows.len(),
    area.height as usize,
    how.selected,
    how.scroll.get(),
  );
  how.scroll.set(shown.start);
  let lines: Vec<Line> = shown
    .clone()
    .map(|i| {
      let mut style = cx.style_for(&widget.style, Some(&rows[i]));
      if how.focused && how.selected == Some(i) {
        style = style.patch(cx.styles.get("selected"));
      }
      Line::from(vec![
        Span::styled(
          format!(
            "{} ",
            crate::template::pad(&trim(&labels[i], label_width), label_width, true)
          ),
          cx.styles.get("text"),
        ),
        Span::styled(
          format!("{:<1$}", bar(values[i], max, bar_width), bar_width),
          style,
        ),
        Span::styled(format!(" {}", texts[i]), cx.styles.get("dim")),
      ])
    })
    .collect();
  frame.render_widget(Paragraph::new(lines), area);
}

fn chart(
  frame: &mut Frame,
  area: Rect,
  widget: &WidgetSpec,
  rows: &[Row],
  store: &mut Store,
  cx: &Context,
  how: &Draw,
) {
  let mut series: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
  // Whether x is a time, to say it as one.
  let mut times = false;
  for row in rows {
    let raw = widget.x.as_ref().map(|e| cx.eval(e, Some(row)));
    let x = raw.as_ref().and_then(coordinate);
    times |= raw.as_ref().is_some_and(|v| v.num().is_none());
    let y = widget.y.as_ref().and_then(|e| cx.eval(e, Some(row)).num());
    let name = widget
      .series
      .as_ref()
      .map(|s| cx.eval(s, Some(row)).text())
      .unwrap_or_default();
    if let (Some(x), Some(y)) = (x, y) {
      series.entry(name).or_default().push((x, y));
    }
  }
  for points in series.values_mut() {
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
  }
  let all: Vec<(f64, f64)> = series.values().flatten().copied().collect();
  if all.is_empty() {
    frame.render_widget(
      Paragraph::new(Line::styled("no points", cx.styles.get("dim"))),
      area,
    );
    return;
  }
  let (mut x0, mut x1) = (f64::MAX, f64::MIN);
  for (x, _) in &all {
    x0 = x0.min(*x);
    x1 = x1.max(*x);
  }
  if (x1 - x0).abs() < f64::EPSILON {
    x1 = x0 + 1.0;
  }
  // The window: the newest part when zoomed in, earlier when moved back.
  let (zoom, back) = how.window;
  let width = (x1 - x0) / 2f64.powi(zoom as i32);
  let right = (x1 - f64::from(back.max(0)) * width / 2.0).max(x0 + width);
  let (wx0, wx1) = (right - width, right);
  let inside = |x: f64| x >= wx0 - width * 1e-9 && x <= wx1 + width * 1e-9;
  let shown: BTreeMap<String, Vec<(f64, f64)>> = series
    .iter()
    .map(|(name, points)| {
      (
        name.clone(),
        points.iter().copied().filter(|(x, _)| inside(*x)).collect(),
      )
    })
    .collect();
  let visible: Vec<(f64, f64)> = shown.values().flatten().copied().collect();
  let (mut y0, mut y1) = (f64::MAX, f64::MIN);
  for (_, y) in &visible {
    y0 = y0.min(*y);
    y1 = y1.max(*y);
  }
  if visible.is_empty() {
    (y0, y1) = (0.0, 1.0);
  }
  let marks: Vec<(f64, String, Style)> = widget
    .marks
    .iter()
    .filter_map(|m| {
      let v = cx.eval(&m.value, rows.last()).num()?;
      Some((
        v,
        m.label.clone().unwrap_or_default(),
        m.style
          .as_deref()
          .map_or(cx.styles.get("dim"), |s| cx.styles.get(s)),
      ))
    })
    .collect();
  for (v, _, _) in &marks {
    y0 = y0.min(*v);
    y1 = y1.max(*v);
  }
  if let Some((_, v)) = how.pinned {
    y0 = y0.min(v);
    y1 = y1.max(v);
  }
  if (y1 - y0).abs() < f64::EPSILON {
    y0 -= 1.0;
    y1 += 1.0;
  }
  // Rows of another source as vertical lines: commits on a time chart.
  let annotations: Vec<(f64, String)> = widget
    .annotate
    .as_ref()
    .map(|a| {
      let args = cx
        .loaded
        .source(&a.source)
        .and_then(|s| s.args.as_ref())
        .map(|t| cx.render(t, None));
      let rows = store
        .get_for(cx.loaded, &a.source, args.as_deref(), false)
        .rows
        .clone();
      rows
        .iter()
        .filter_map(|r| {
          let x = coordinate(&cx.eval(&a.at, Some(r)))?;
          inside(x).then(|| (x, cx.render(&a.text, Some(r))))
        })
        .collect()
    })
    .unwrap_or_default();
  // Where the graph is drawn: right of the y axis's labels and line.
  let label = |v: f64| crate::template::short(v);
  let label_width = [y0, (y0 + y1) / 2.0, y1]
    .iter()
    .map(|v| label(*v).width())
    .max()
    .unwrap_or(0) as u16;
  let graph_left = area.x + label_width + 1;
  let graph_width = area.width.saturating_sub(label_width + 1).max(1);
  // The crosshair: where the mouse is, or moved point by point; on the nearest point.
  let mut xs: Vec<f64> = visible.iter().map(|(x, _)| *x).collect();
  xs.sort_by(f64::total_cmp);
  xs.dedup();
  let mut crosshair = how.crosshair.get().filter(|x| inside(*x));
  if let Some(column) = how.hover
    && column >= graph_left
  {
    let at = f64::from(column - graph_left) / f64::from(graph_width.saturating_sub(1).max(1));
    crosshair = Some(wx0 + at.min(1.0) * (wx1 - wx0));
  }
  if let Some(step) = how.step
    && !xs.is_empty()
  {
    crosshair = Some(match (crosshair, step > 0) {
      (Some(c), true) => xs
        .iter()
        .copied()
        .find(|x| *x > c)
        .unwrap_or(xs[xs.len() - 1]),
      (Some(c), false) => xs.iter().rev().copied().find(|x| *x < c).unwrap_or(xs[0]),
      (None, _) => xs[xs.len() - 1],
    });
  }
  let crosshair = crosshair.and_then(|c| {
    xs.iter()
      .copied()
      .min_by(|a, b| (a - c).abs().total_cmp(&(b - c).abs()))
  });
  how.crosshair.set(crosshair);
  let say_x = |x: f64| {
    if times {
      let full = crate::template::utc(x as i64).replace('T', " ");
      full.get(..16).unwrap_or(&full).to_string()
    } else {
      crate::template::short(x)
    }
  };
  let palette = ["info", "ok", "warn", "accent", "bad"];
  let mut lines: Vec<Rule> = Vec::new();
  for (x, _) in &annotations {
    let style = widget
      .annotate
      .as_ref()
      .and_then(|a| a.style.as_deref())
      .map_or(cx.styles.get("dim"), |s| cx.styles.get(s));
    lines.push((vec![(*x, y0), (*x, y1)], style, String::new()));
  }
  for (v, name, style) in &marks {
    lines.push((vec![(wx0, *v), (wx1, *v)], *style, name.clone()));
  }
  if let Some((_, v)) = how.pinned {
    lines.push((
      vec![(wx0, v), (wx1, v)],
      cx.styles.get("warn"),
      "pinned".into(),
    ));
  }
  if let Some(c) = crosshair {
    lines.push((vec![(c, y0), (c, y1)], cx.styles.get("key"), String::new()));
  }
  let mut datasets: Vec<Dataset> = shown
    .iter()
    .enumerate()
    .map(|(i, (name, points))| {
      Dataset::default()
        .name(name.clone())
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(cx.styles.get(palette[i % palette.len()]))
        .data(points)
    })
    .collect();
  for (points, style, name) in &lines {
    let mut set = Dataset::default()
      .marker(Marker::Dot)
      .graph_type(GraphType::Line)
      .style(*style)
      .data(points);
    if !name.is_empty() {
      set = set.name(name.clone());
    }
    datasets.push(set);
  }
  let dim = |text: String| Span::styled(text, cx.styles.get("dim"));
  let chart = Chart::new(datasets)
    .x_axis(
      Axis::default()
        .bounds([wx0, wx1])
        .labels(vec![dim(say_x(wx0)), dim(say_x(wx1))])
        .style(cx.styles.get("frame")),
    )
    .y_axis(
      Axis::default()
        .bounds([y0, y1])
        .labels(vec![
          dim(label(y0)),
          dim(label((y0 + y1) / 2.0)),
          dim(label(y1)),
        ])
        .title(widget.y_label.clone().unwrap_or_default())
        .style(cx.styles.get("frame")),
    );
  frame.render_widget(chart, area);
  // What the crosshair is on, at the top: its x, each series' value there, the change
  // from the pinned point, and the nearest annotation.
  let Some(c) = crosshair else {
    how.under.set(None);
    return;
  };
  let mut said = vec![say_x(c)];
  let mut under = None;
  for (name, points) in &shown {
    if let Some(&(_, y)) = points.iter().find(|(x, _)| *x == c) {
      under.get_or_insert((c, y));
      let change = how
        .pinned
        .filter(|(_, p)| p.abs() > f64::EPSILON)
        .map(|(_, p)| format!(" ({:+.1}% vs pinned)", 100.0 * (y - p) / p.abs()))
        .unwrap_or_default();
      let name = if name.is_empty() {
        String::new()
      } else {
        format!("{} ", name)
      };
      said.push(format!("{}{}{}", name, crate::template::short(y), change));
    }
  }
  how.under.set(under);
  if let Some((_, text)) = annotations
    .iter()
    .min_by(|a, b| (a.0 - c).abs().total_cmp(&(b.0 - c).abs()))
  {
    said.push(text.clone());
  }
  let text = format!(" {} ", said.join(" · "));
  let w = (text.width() as u16).min(area.width.saturating_sub(graph_left - area.x));
  frame.render_widget(
    Paragraph::new(Line::styled(fit(&text, w as usize), cx.styles.get("key"))),
    Rect::new(graph_left, area.y, w, 1),
  );
}

/// A field as an x coordinate: a number, or a UTC time as seconds.
fn coordinate(value: &Value) -> Option<f64> {
  value.num().or_else(|| parse_time(&value.text()))
}

fn file(
  frame: &mut Frame,
  area: Rect,
  widget: &WidgetSpec,
  store: &mut Store,
  cx: &Context,
  how: &Draw,
) {
  let path = cx
    .loaded
    .base
    .join(cx.render(widget.path.as_deref().unwrap_or(""), None));
  let unreadable = format!("{} cannot be read", path.display());
  // Read again only when it changed.
  let text = store.tail(&path).unwrap_or(&unreadable);
  let mut in_code = false;
  let scroll = how
    .scroll
    .get()
    .min(text.lines().count().saturating_sub(area.height as usize));
  how.scroll.set(scroll);
  let total = text.lines().count();
  how.span.set(Some((
    scroll,
    (scroll + area.height as usize).min(total),
    total,
  )));
  let lines: Vec<Line> = text
    .lines()
    .skip(scroll)
    .take(area.height as usize)
    .map(|line| {
      let line = untabbed(line);
      if widget.markdown != Some(true) {
        return Line::styled(line, cx.styles.get("text"));
      }
      if line.starts_with("```") {
        in_code = !in_code;
        return Line::styled(line, cx.styles.get("dim"));
      }
      let style = if in_code {
        cx.styles.get("dim")
      } else if line.starts_with('#') {
        cx.styles.get("title")
      } else {
        cx.styles.get("text")
      };
      Line::styled(line, style)
    })
    .collect();
  frame.render_widget(Paragraph::new(lines), area);
}

fn gauges(
  frame: &mut Frame,
  area: Rect,
  widget: &WidgetSpec,
  rows: &[Row],
  cx: &Context,
  how: &Draw,
) {
  let labels: Vec<String> = rows
    .iter()
    .map(|r| cx.render(widget.label.as_deref().unwrap_or(""), Some(r)))
    .collect();
  let texts: Vec<String> = rows
    .iter()
    .map(|r| {
      widget
        .text
        .as_ref()
        .map(|t| cx.render(t, Some(r)))
        .unwrap_or_default()
    })
    .collect();
  let label_width = labels.iter().map(|l| l.width()).max().unwrap_or(0);
  let text_width = texts.iter().map(|t| t.width()).max().unwrap_or(0);
  let bar_width = (area.width as usize)
    .saturating_sub(label_width + text_width + 3)
    .max(4);
  // More rows than room scroll, the place shown in the border.
  let shown = window(rows.len(), area.height as usize, None, how.scroll.get());
  how.scroll.set(shown.start);
  how.span.set(Some((shown.start, shown.end, rows.len())));
  let lines: Vec<Line> = rows
    .iter()
    .enumerate()
    .skip(shown.start)
    .take(shown.len())
    .map(|(i, row)| {
      let max = number(cx, &widget.max, row, 100.0);
      let value = number(cx, &widget.value, row, 0.0);
      let mut cells: Vec<char> = format!("{:<1$}", bar(value, max, bar_width), bar_width)
        .chars()
        .collect();
      if let Some(target) = widget
        .target
        .as_ref()
        .and_then(|t| cx.eval(t, Some(row)).num())
      {
        let at = ((target / max.max(f64::EPSILON)).clamp(0.0, 1.0) * bar_width as f64) as usize;
        if at < cells.len() && cells[at] == ' ' {
          cells[at] = '┊';
        }
      }
      Line::from(vec![
        Span::styled(
          format!("{} ", crate::template::pad(&labels[i], label_width, true)),
          cx.styles.get("text"),
        ),
        Span::styled(
          cells.into_iter().collect::<String>(),
          cx.style_for(&widget.style, Some(row)),
        ),
        Span::styled(format!(" {}", texts[i]), cx.styles.get("text")),
      ])
    })
    .collect();
  frame.render_widget(Paragraph::new(lines), area);
}

fn is_marked(row: &Row, how: &Draw) -> bool {
  how.marked.iter().any(|m| crate::session::same_row(m, row))
}

fn list(
  frame: &mut Frame,
  area: Rect,
  widget: &WidgetSpec,
  rows: &[Row],
  cx: &Context,
  how: &Draw,
) {
  let item = widget.item.as_deref().unwrap_or("");
  let shown = window(
    rows.len(),
    area.height as usize,
    how.selected,
    how.scroll.get(),
  );
  how.scroll.set(shown.start);
  how.span.set(Some((shown.start, shown.end, rows.len())));
  let lines: Vec<Line> = rows[shown.clone()]
    .iter()
    .enumerate()
    .map(|(i, row)| {
      let mut style = cx.style_for(&widget.style, Some(row));
      let here = how.selected == Some(shown.start + i);
      if how.focused && here {
        style = style.patch(cx.styles.get("selected"));
      }
      let text = cx.render(item, Some(row));
      match how.selected {
        Some(_) => Line::from(vec![
          marker(here, how.focused, is_marked(row, how), cx),
          Span::styled(text, style),
        ]),
        None => Line::styled(text, style),
      }
    })
    .collect();
  frame.render_widget(Paragraph::new(lines), area);
}

fn log(
  frame: &mut Frame,
  area: Rect,
  widget: &WidgetSpec,
  store: &mut Store,
  cx: &Context,
  how: &Draw,
) {
  let path = cx
    .loaded
    .base
    .join(cx.render(widget.path.as_deref().unwrap_or(""), None));
  // Only what was added since the last frame is read.
  let text = store.tail(&path).unwrap_or("");
  if text.is_empty() {
    let message = widget
      .empty
      .as_ref()
      .map(|e| cx.render(e, None))
      .unwrap_or_else(|| format!("{} is empty or missing", path.display()));
    frame.render_widget(
      Paragraph::new(Line::styled(message, cx.styles.get("dim"))),
      area,
    );
    return;
  }
  let level = widget
    .level
    .as_ref()
    .and_then(|l| regex::Regex::new(l).ok());
  let fold = widget.fold.as_ref().and_then(|f| regex::Regex::new(f).ok());
  let colors = !colorless(cx.loaded);
  let raw: Vec<&str> = text.lines().collect();
  // Each line as read, its colors kept apart from its text.
  let plain: Vec<String> = raw.iter().map(|l| strip_ansi(l)).collect();
  let style_of = |i: usize| {
    level
      .as_ref()
      .and_then(|re| re.find(&plain[i]))
      .and_then(|m| widget.styles.get(m.as_str()))
      .map(String::as_str)
  };
  let problem = |i: usize| matches!(style_of(i), Some("bad" | "warn"));
  let search = how.search.filter(|q| !q.is_empty());
  let found = |i: usize| search.is_some_and(|q| has(&plain[i], q));
  // Sections, when the widget says where they start: each folds to its first line,
  // unless every one is open, it is the last, or it holds a problem or a match.
  let mut visible: Vec<(usize, usize)> = Vec::new();
  match &fold {
    Some(re) => {
      let starts: Vec<usize> = (0..raw.len()).filter(|&i| re.is_match(&plain[i])).collect();
      let mut at = 0;
      if starts.first() != Some(&0) {
        let end = starts.first().copied().unwrap_or(raw.len());
        visible.extend((0..end).map(|i| (i, 0)));
        at = end;
      }
      for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(raw.len());
        let last = k + 1 == starts.len();
        let matters = (start..end).any(|i| problem(i) || found(i));
        if how.folds_open || last || matters || end - start == 1 {
          visible.extend((start..end).map(|i| (i, 0)));
        } else {
          visible.push((start, end - start - 1));
        }
        at = end;
      }
      let _ = at;
    }
    None => visible.extend((0..raw.len()).map(|i| (i, 0))),
  }
  // A move to the next or previous match (or problem line, with no search).
  let targets: Vec<usize> = visible
    .iter()
    .map(|(i, _)| *i)
    .filter(|&i| {
      if search.is_some() {
        found(i)
      } else {
        problem(i)
      }
    })
    .collect();
  if let Some(direction) = how.jump
    && !targets.is_empty()
  {
    let from = how.at.get();
    let next = match (direction > 0, from) {
      (true, Some(f)) => targets
        .iter()
        .copied()
        .find(|&t| t > f)
        .unwrap_or(targets[0]),
      (false, Some(f)) => targets
        .iter()
        .rev()
        .copied()
        .find(|&t| t < f)
        .unwrap_or(targets[targets.len() - 1]),
      (true, None) => targets[0],
      (false, None) => targets[targets.len() - 1],
    };
    how.at.set(Some(next));
  }
  let at = how.at.get();
  // The lines as drawn: wrapped at the width when asked.
  let width = area.width.max(1) as usize;
  let mut rows: Vec<(usize, Line)> = Vec::new();
  for &(i, hidden) in &visible {
    let base = style_of(i).map_or(cx.styles.get("text"), |s| cx.styles.get(s));
    let mut line = ansi_line(&untabbed(raw[i]), base, colors);
    if hidden > 0 {
      line.spans.push(Span::styled(
        format!("  ▸ {} more line(s)", hidden),
        cx.styles.get("dim"),
      ));
    }
    if found(i) {
      line = line.patch_style(Style::default().add_modifier(Modifier::UNDERLINED));
    }
    if at == Some(i) {
      line = line.patch_style(cx.styles.get("selected"));
    }
    if how.wrap {
      for piece in wrap_line(line, width) {
        rows.push((i, piece));
      }
    } else {
      rows.push((i, line));
    }
  }
  let height = area.height as usize;
  let follow = widget.follow != Some(false);
  // A move brings its line to the middle of the pane.
  if how.jump.is_some()
    && let Some(target) = at
    && let Some(row) = rows.iter().position(|(i, _)| *i == target)
  {
    let top = row.saturating_sub(height / 2);
    let scroll = if follow {
      rows.len().saturating_sub(top + height)
    } else {
      top
    };
    how.scroll.set(scroll);
  }
  // A followed log counts its scroll back from the end.
  let scroll = how.scroll.get().min(rows.len().saturating_sub(height));
  how.scroll.set(scroll);
  let end = if follow {
    rows.len() - scroll
  } else {
    (scroll + height).min(rows.len())
  };
  let start = end.saturating_sub(height);
  how.span.set(Some((start, end, rows.len())));
  let shown: Vec<Line> = rows.drain(start..end).map(|(_, l)| l).collect();
  frame.render_widget(Paragraph::new(shown), area);
  // How many matches, and which: on the top border's right.
  if let Some(q) = search
    && area.y > 0
  {
    let all: Vec<usize> = (0..raw.len()).filter(|&i| found(i)).collect();
    let which = at
      .and_then(|a| all.iter().position(|&i| i == a))
      .map_or(String::new(), |p| format!("{} of ", p + 1));
    let text = format!(" /{} {}{} ", q, which, all.len());
    let w = (text.width() as u16).min(area.width);
    frame.render_widget(
      Paragraph::new(Line::styled(text, cx.styles.get("key"))),
      Rect::new(area.x + area.width - w, area.y - 1, w, 1),
    );
  }
}

/// The gutter of a row: `▶` on the selected row of the focused pane (`›` elsewhere), then
/// `●` when it is marked.
fn marker(here: bool, focused: bool, marked: bool, cx: &Context) -> Span<'static> {
  let dot = if marked { "●" } else { " " };
  match (here, focused) {
    (true, true) => Span::styled(format!("▶{}", dot), cx.styles.get("selected")),
    (true, false) => Span::styled(format!("›{}", dot), cx.styles.get("dim")),
    _ if marked => Span::styled(format!(" {}", dot), cx.styles.get("key")),
    _ => Span::raw("  "),
  }
}

fn number(cx: &Context, expr: &Option<String>, row: &Row, default: f64) -> f64 {
  expr
    .as_ref()
    .and_then(|e| cx.eval(e, Some(row)).num())
    .unwrap_or(default)
}

/// `style` after the SGR parameters `params` (`1;31`), `base` being what a reset gives.
fn sgr(params: &str, mut style: Style, base: Style, colors: bool) -> Style {
  let codes: Vec<u16> = if params.is_empty() {
    vec![0]
  } else {
    params
      .split([';', ':'])
      .map(|p| p.parse().unwrap_or(0))
      .collect()
  };
  let named = |n: u16| match n {
    0 => Color::Black,
    1 => Color::Red,
    2 => Color::Green,
    3 => Color::Yellow,
    4 => Color::Blue,
    5 => Color::Magenta,
    6 => Color::Cyan,
    _ => Color::Gray,
  };
  let bright = |n: u16| match n {
    0 => Color::DarkGray,
    1 => Color::LightRed,
    2 => Color::LightGreen,
    3 => Color::LightYellow,
    4 => Color::LightBlue,
    5 => Color::LightMagenta,
    6 => Color::LightCyan,
    _ => Color::White,
  };
  let mut i = 0;
  while i < codes.len() {
    let code = codes[i];
    let mut color: Option<(bool, Color)> = None;
    match code {
      0 => style = base,
      1 => style = style.add_modifier(Modifier::BOLD),
      2 => style = style.add_modifier(Modifier::DIM),
      3 => style = style.add_modifier(Modifier::ITALIC),
      4 => style = style.add_modifier(Modifier::UNDERLINED),
      7 => style = style.add_modifier(Modifier::REVERSED),
      22 => style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
      23 => style = style.remove_modifier(Modifier::ITALIC),
      24 => style = style.remove_modifier(Modifier::UNDERLINED),
      27 => style = style.remove_modifier(Modifier::REVERSED),
      30..=37 => color = Some((true, named(code - 30))),
      90..=97 => color = Some((true, bright(code - 90))),
      40..=47 => color = Some((false, named(code - 40))),
      100..=107 => color = Some((false, bright(code - 100))),
      39 if colors => style.fg = base.fg,
      49 if colors => style.bg = base.bg,
      38 | 48 => {
        let fg = code == 38;
        match codes.get(i + 1) {
          Some(5) => {
            if let Some(&n) = codes.get(i + 2) {
              color = Some((fg, Color::Indexed(n as u8)));
            }
            i += 2;
          }
          Some(2) => {
            if let (Some(&r), Some(&g), Some(&b)) =
              (codes.get(i + 2), codes.get(i + 3), codes.get(i + 4))
            {
              color = Some((fg, Color::Rgb(r as u8, g as u8, b as u8)));
            }
            i += 4;
          }
          _ => {}
        }
      }
      _ => {}
    }
    if let Some((fg, c)) = color
      && colors
    {
      if fg {
        style.fg = Some(c);
      } else {
        style.bg = Some(c);
      }
    }
    i += 1;
  }
  style
}

fn sparklines(
  frame: &mut Frame,
  area: Rect,
  widget: &WidgetSpec,
  rows: &[Row],
  cx: &Context,
  how: &Draw,
) {
  let labels: Vec<String> = rows
    .iter()
    .map(|r| cx.render(widget.label.as_deref().unwrap_or(""), Some(r)))
    .collect();
  let label_width = labels
    .iter()
    .map(|l| l.width())
    .max()
    .unwrap_or(0)
    .min(area.width as usize / 2);
  let shown = window(rows.len(), area.height as usize, None, how.scroll.get());
  how.scroll.set(shown.start);
  how.span.set(Some((shown.start, shown.end, rows.len())));
  let lines: Vec<Line> = rows
    .iter()
    .enumerate()
    .skip(shown.start)
    .take(shown.len())
    .map(|(i, row)| {
      let values: Vec<f64> = match widget.values.as_ref().map(|v| cx.eval(v, Some(row))) {
        Some(Value::List(items)) => items.iter().filter_map(Value::num).collect(),
        _ => Vec::new(),
      };
      let text = widget
        .text
        .as_ref()
        .map(|t| cx.render(t, Some(row)))
        .unwrap_or_default();
      let room = (area.width as usize)
        .saturating_sub(label_width + text.width() + 3)
        .max(4);
      Line::from(vec![
        Span::styled(
          format!(
            "{} ",
            crate::template::pad(&trim(&labels[i], label_width), label_width, true)
          ),
          cx.styles.get("text"),
        ),
        Span::styled(
          spark(&values[values.len().saturating_sub(room)..]),
          cx.style_for(&widget.style, Some(row)),
        ),
        Span::styled(format!(" {}", text), cx.styles.get("dim")),
      ])
    })
    .collect();
  frame.render_widget(Paragraph::new(lines), area);
}

fn style_of(spec: &StyleSpec, color: &dyn Fn(&str) -> Option<Color>) -> Style {
  let mut style = Style::default();
  if let Some(fg) = spec.fg.as_deref().and_then(color) {
    style = style.fg(fg);
  }
  if let Some(bg) = spec.bg.as_deref().and_then(color) {
    style = style.bg(bg);
  }
  for (on, modifier) in [
    (spec.bold, Modifier::BOLD),
    (spec.italic, Modifier::ITALIC),
    (spec.underline, Modifier::UNDERLINED),
    (spec.reverse, Modifier::REVERSED),
  ] {
    if on == Some(true) {
      style = style.add_modifier(modifier);
    }
  }
  style
}

fn table(
  frame: &mut Frame,
  area: Rect,
  widget: &WidgetSpec,
  rows: &[Row],
  cx: &Context,
  how: &Draw,
) {
  let widths: Vec<Constraint> = widget
    .columns
    .iter()
    .map(|c| {
      c.width
        .as_ref()
        .and_then(layout::constraint)
        .unwrap_or(Constraint::Fill(1))
    })
    .collect();
  // A selectable table keeps two columns at its left for the selected row's mark.
  let gutter = if how.selected.is_some() { 2 } else { 0 };
  let full = area;
  let area = Rect {
    x: area.x + gutter,
    width: area.width.saturating_sub(gutter),
    ..area
  };
  // The columns as the table will lay them out, so each cell is cut to its own.
  let placed = Layout::horizontal(widths.clone())
    .flex(Flex::Start)
    .spacing(1)
    .split(area);
  let columns: Vec<u16> = placed.iter().map(|r| r.width).collect();
  *how.columns.borrow_mut() = placed.iter().map(|r| (r.x, r.width)).collect();
  let header = TableRow::new(widget.columns.iter().zip(&columns).enumerate().map(
    |(at, (c, w))| {
      // The column sorted by says which way.
      let header = match how.sort {
        Some((column, descending)) if column == at => {
          format!("{} {}", c.header, if descending { "▼" } else { "▲" })
        }
        _ => c.header.clone(),
      };
      Cell::from(aligned(&header, c.align.as_deref(), *w))
    },
  ))
  .style(cx.styles.get("dim"));
  let height = area.height.saturating_sub(1) as usize;
  let shown = window(rows.len(), height, how.selected, how.scroll.get());
  how.scroll.set(shown.start);
  let body: Vec<TableRow> = rows[shown.clone()]
    .iter()
    .enumerate()
    .map(|(i, row)| {
      let cells = widget.columns.iter().zip(&columns).map(|(c, w)| {
        let text = cx.render(&c.value, Some(row));
        Cell::from(aligned(&text, c.align.as_deref(), *w).style(cx.style_for(&c.style, Some(row))))
      });
      let mut r = TableRow::new(cells);
      if how.focused && how.selected == Some(shown.start + i) {
        r = r.style(cx.styles.get("selected"));
      }
      r
    })
    .collect();
  frame.render_widget(
    Table::new(body, widths).header(header).column_spacing(1),
    area,
  );
  how.span.set(Some((shown.start, shown.end, rows.len())));
  if gutter > 0 {
    for at in shown.clone() {
      let here = how.selected == Some(at);
      let marked = is_marked(&rows[at], how);
      if here || marked {
        let y = full.y + 1 + (at - shown.start) as u16;
        frame.render_widget(
          Paragraph::new(Line::from(marker(here, how.focused, marked, cx))),
          Rect::new(full.x, y, gutter, 1),
        );
      }
    }
  }
}

/// A line with its tabs as spaces to the next multiple of 8: a terminal cell cannot hold
/// a tab.
fn untabbed(line: &str) -> String {
  let mut out = String::new();
  for c in line.chars() {
    if c == '\t' {
      let column = out.width();
      out.push_str(&" ".repeat(8 - column % 8));
    } else {
      out.push(c);
    }
  }
  out
}

/// The rows a list of `height` shows, keeping the selection in view.
fn window(
  len: usize,
  height: usize,
  selected: Option<usize>,
  scroll: usize,
) -> std::ops::Range<usize> {
  // A selection moved past the end stands on the last row.
  let start = match selected.map(|s| s.min(len.saturating_sub(1))) {
    Some(s) if height > 0 => s
      .saturating_sub(height.saturating_sub(1))
      .max(scroll.min(s).min(len.saturating_sub(height))),
    _ => scroll.min(len.saturating_sub(height)),
  };
  start..len.min(start + height)
}

/// `line` cut into lines at most `width` columns wide, each piece keeping its style.
fn wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
  if line.width() <= width {
    return vec![line];
  }
  let mut out = Vec::new();
  let mut current: Vec<Span<'static>> = Vec::new();
  let mut used = 0;
  for span in line.spans {
    let mut piece = String::new();
    for c in span.content.chars() {
      let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
      if used + w > width && used > 0 {
        if !piece.is_empty() {
          current.push(Span::styled(std::mem::take(&mut piece), span.style));
        }
        out.push(Line::from(std::mem::take(&mut current)).style(line.style));
        used = 0;
      }
      piece.push(c);
      used += w;
    }
    if !piece.is_empty() {
      current.push(Span::styled(piece, span.style));
    }
  }
  if !current.is_empty() {
    out.push(Line::from(current).style(line.style));
  }
  out
}

#[cfg(test)]
mod tests {

  use super::*;

  fn fixture() -> Loaded {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/.hub.toml");
    crate::config::load(&path).unwrap()
  }

  #[test]
  fn test_a_long_line_wraps_keeping_its_styles() {
    let line = Line::from(vec![
      Span::styled("abcd", Style::default().fg(Color::Red)),
      Span::raw("efgh"),
    ]);
    let pieces = wrap_line(line, 3);
    let texts: Vec<String> = pieces.iter().map(|l| l.to_string()).collect();
    assert_eq!(texts, ["abc", "def", "gh"]);
    assert_eq!(pieces[1].spans[0].style.fg, Some(Color::Red));
    assert_eq!(pieces[1].spans[1].style.fg, None);
  }

  #[test]
  fn test_colors_and_windows() {
    assert_eq!(parse_color("#e3a857"), Some(Color::Rgb(0xe3, 0xa8, 0x57)));
    assert_eq!(parse_color("bright-red"), Some(Color::LightRed));
    assert_eq!(parse_color("nope"), None);
    assert_eq!(window(100, 10, Some(0), 0), 0..10);
    assert_eq!(window(100, 10, Some(25), 0), 16..26);
    assert_eq!(window(5, 10, None, 0), 0..5);
    // A cell too long keeps its start: 13m21s ago is not shown as 3m21s ago.
    assert_eq!(fit("13m21s ago", 9), "13m21s a…");
    assert_eq!(fit("13m21s ago", 10), "13m21s ago");
    assert_eq!(fit("日本語テキスト", 7), "日本語…");
    assert_eq!(window(2, 10, Some(1_000_000), 1_000_000), 0..2);
    assert_eq!(window(30, 10, None, 1_000_000), 20..30);
    assert_eq!(untabbed("b\tlog"), "b       log");
  }

  #[test]
  fn test_escape_sequences_become_styles() {
    let line = ansi_line(
      "a \x1b[1;31mred\x1b[0m b\x1b]0;title\x07",
      Style::default(),
      true,
    );
    assert_eq!(line.spans.len(), 3);
    assert_eq!(line.spans[1].content, "red");
    assert_eq!(line.spans[1].style.fg, Some(Color::Red));
    assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(line.spans[2].content, " b");
    let gray = ansi_line("\x1b[38;5;244mx\x1b[38;2;1;2;3my", Style::default(), true);
    assert_eq!(gray.spans[0].style.fg, Some(Color::Indexed(244)));
    assert_eq!(gray.spans[1].style.fg, Some(Color::Rgb(1, 2, 3)));
    // Without color, bold stays.
    let plain = ansi_line("\x1b[1;31mred", Style::default(), false);
    assert_eq!(plain.spans[0].style.fg, None);
    assert!(plain.spans[0].style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(strip_ansi("\x1b[2K\x1b[32mok\x1b[0m"), "ok");
  }

  #[test]
  fn test_light_colors_and_high_contrast() {
    let mut loaded = fixture();
    assert_eq!(
      Styles::new(&loaded, false).get("text").fg,
      Some(Color::Rgb(0xdd, 0xdd, 0xdd))
    );
    loaded.config.light.insert("fg".into(), "#111111".into());
    loaded.config.hub.background = Some("light".into());
    assert_eq!(
      Styles::new(&loaded, false).get("text").fg,
      Some(Color::Rgb(0x11, 0x11, 0x11))
    );
    loaded.config.hub.background = Some("dark".into());
    assert_eq!(
      Styles::new(&loaded, false).get("text").fg,
      Some(Color::Rgb(0xdd, 0xdd, 0xdd))
    );
    // High contrast: the terminal's own colors, nothing dimmed, the selection reversed.
    let strong = Styles::new(&loaded, true);
    assert_eq!(strong.get("text"), Style::default());
    assert_eq!(strong.get("dim"), Style::default());
    assert!(
      strong
        .get("selected")
        .add_modifier
        .contains(Modifier::REVERSED)
    );
    assert_eq!(strong.get("bad").fg, Some(Color::LightRed));
  }
}
