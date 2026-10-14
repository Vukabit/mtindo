# hub: a terminal hub declared in TOML

`hub` is a terminal interface for a project's development records: runs,
incidents, metrics, logs and commands. It knows nothing about any project.
Everything it shows and every key it answers is declared in one file, found
by walking up from the current folder: `.hub.toml`.

This file is the contract. A key the engine does not know is an error, with
its line; so is a reference to a source, widget, style, screen or action that
does not exist. `hub --check` reads the file, loads every source once and
reports every problem without opening the interface.

```text
hub                       open the interface
hub --check               validate .hub.toml and every source, then exit
hub --render SCREEN       print one screen as text (--size 100x30)
hub --config FILE         use FILE instead of the nearest .hub.toml
```

Paths in the file are relative to the file's folder. Durations are written
`"500ms"`, `"2s"`, `"5m"`, `"1h"`.

## Contents

1. `[hub]`: the program
2. `[theme]` and `[styles]`: colors and named styles
3. `[[source]]`: where data comes from
4. Fields, expressions and templates
5. `[[widget]]`: what shows the data
6. `[[screen]]` and layouts: where widgets go
7. `[bindings]` and `[[action]]`: what keys do
8. `[topbar]`, `[footer]`, `[[alert]]`
9. `[palette]`: running any command, with forms
10. Vim mode
11. The git window: `git.hub.toml`, for any project
12. The session: kept across restarts and reloads
13. The mouse
14. Running it, and reading it without seeing it

## 1. `[hub]`

```toml
[hub]
name = "mto dev"          # shown at the top left
refresh = "1s"            # how often sources are checked for changes
start = "dashboard"       # the first screen's id
min_width = 60            # below this, a notice instead of the interface
min_height = 8            # the same, for rows
narrow_below = 100        # below this, screens use their narrow layout
state = ".dev/hub"        # where hub keeps presets and seen alerts
vim = false               # vim's keys and modes (section 10)
include = ["git.hub.toml"]  # more of the hub, from other files
persist = true            # keep the session across restarts (or a list)
reload = true             # read the config again when its files change
mouse = true              # take the mouse (f3 or :set nomouse gives it back)
color = true              # false: no colors, only bold and reversed
cursor = "hidden"         # "follow": the cursor on the selection
toast = "6s"              # how long a toast stays (a failure's 3 times)
motion = true             # false: nothing moves on its own
notify = "osc"            # a new alert to the desktop: osc, bell, both, none
background = "auto"       # light or dark colors: auto asks the terminal
contrast = false          # the terminal's own colors, strong
```

`include` names files, relative to this one, that are read as part of it:
their sources, widgets, screens, actions and alerts are added after this
file's; their `[theme]`, `[styles]`, `[bindings]` and other tables fill in
only what this file leaves out, so this file always wins. An included file
has no `[hub]` of its own. Its mistakes are reported with its own name and
line.

## 2. `[theme]` and `[styles]`

The theme's `bg`, when it has one, is painted under everything (`text`
takes it unless it has a background of its own). Without color (`[hub]
color = false`, `hub --no-color`, or `NO_COLOR` set to anything, see
no-color.org) no color is drawn: bold and reversed stay, and `selected`
and `tab` are reversed, so the selection still shows.

`[light]`, with the same names, gives colors over `[theme]`'s on a light
terminal: when `[hub] background` is `"light"`, or `"auto"` (the default)
and the terminal says its background is light when the hub asks it (OSC
11) at start. `[hub] contrast = true` (or `:set contrast`, kept for the
next session) draws with the terminal's own colors only, nothing dimmed,
what stands out bold, underlined or reversed, states in its bright colors.

`[theme]` names colors; anything that takes a color takes a name from here,
a `#rrggbb` value or a terminal color name (`red`, `bright-black`, ...).

```toml
[theme]
bg = "#15171b"
fg = "#d7d4cc"
dim = "#747a83"
accent = "#e3a857"
info = "#6fc3c0"
ok = "#8fc47f"
bad = "#e3716b"
warn = "#d8b55c"
frame = "#3a414b"
selected = "#26303b"
```

`[styles]` names styles: a foreground, a background and modifiers. The
engine draws with `text`, `title`, `frame`, `selected`, `dim`, `key`,
`ok`, `bad`, `warn` and `info`; any of them left out has a default:
`selected` reversed (so it shows without color), `title` and `key` bold,
`frame` and `dim` in bright black, `ok` green, `bad` red, `warn` yellow,
`info` cyan.

```toml
[styles]
text = { fg = "fg" }
title = { fg = "accent", bold = true }
frame = { fg = "frame" }
selected = { bg = "selected" }
dim = { fg = "dim" }
key = { fg = "accent", bold = true }
tab = { fg = "bg", bg = "accent", bold = true }
ok = { fg = "ok" }
bad = { fg = "bad" }
```

Keys of a style: `fg`, `bg`, `bold`, `italic`, `underline`, `reverse`.

## 3. `[[source]]`

A source turns files or a command's output into rows. A row is a set of
named fields; a field is a string, a number, a boolean or a list. Every
source has a `name` and a `kind`; some kinds take more keys.

| Kind | Rows | Keys |
|---|---|---|
| `toml` | one per file matched by `glob` | `glob` |
| `tsv` | one per line of each file | `glob`, `header` |
| `lines` | one per line matching `pattern` | `glob` or `run`, `pattern` |
| `command` | the output of `run`, parsed | `run`, `parse` |
| `files` | one per file matched by `glob` | `glob` |
| `git` | what `of` names (below) | `of`, `args` |
| `commands` | one per command in a command tree | `run` |
| `stream` | one per line a running command prints | `run` |
| `http` | what a URL returns, parsed | `url`, `parse` |

Every row from files also has `_path` (the file), `_dir` (its folder),
`_rel` and `_reldir` (the same, relative to the `.hub.toml`'s folder),
`_name` (its file name without extension) and `_mtime` (seconds since the
epoch).

A `toml` file's top-level keys become fields; an array of tables becomes a
list of rows, which `len(field)` counts.

A `tsv` file with `header = true` names its columns in its first line;
otherwise `columns = ["a", "b"]` names them.

A `lines` source's `pattern` is a regular expression; its named groups
become fields. A line that does not match is skipped.

A `command` source runs `run` (split like a shell's words, no shell) in the
file's folder, at most every `refresh`, and parses its standard output as
`parse = "lines"` (field `line`), `"tsv"`, `"json"` (an array of objects)
or `"text"` (one row, field `text`).

A `git` source reads the repository the `.hub.toml` is in, in UTC times.
`of` names what it reads (`head` when left out); `limit` caps a log (200):

- `tree`: the changed files of `status` under their folders, folders
  first: each row has `depth`, `name`, `label` (indented, a folder as `▾
  src/`) and `folder`; a folder's `path` is `src/` (git takes it for all in
  it), and it is `staged`, `unstaged`, `untracked` or a `conflict` when a
  file in it is.
- `reflog`: where HEAD has been: `hash`, `short`, `name` (`HEAD@{3}`),
  `subject`, `action` (`commit`, `checkout`...), `when`.
- `worktrees`: `path`, `hash`, `short`, `branch`, `detached`, `bare`,
  `locked`, `prunable`, `main`.
- `bisect`: a bisection under way, a row per step: `verb` (`good`, `bad`,
  `skip`, `first bad`), `hash`, `short`, `subject`; none otherwise.
- `split`: the file in `args`'s changes side by side: `left`, `right`,
  `old`, `new`, `kind` (`context`, `changed`, `removed`, `added`, or a
  `section`, `file`, `hunk`, `note` row).
- `words`: the same file's changes word by word, as `git diff --word-diff`
  marks them in each line (`[-old-]{+new+}`): `text`, `kind`, `old`, `new`,
  `side`.
- `conflicts`: the conflicted file in `args`, a row per line of each
  conflict and two lines around it: `hunk`, `kind` (`ours`, `base`,
  `theirs`, `marker`, `context`), `text`, `line`, `path`, and the whole file
  with that conflict taken one way, for an action to write back:
  `ours_file`, `theirs_file`, `both_file`.
- `head`: one row: `branch`, `commit`, `short`, `upstream`, `ahead`,
  `behind`, `dirty`, `subject`, `when`.
- `status`: one row per changed file: `path`, `from` (a rename's old
  path), `index` and `worktree` (git's status letters, `.` for none),
  `staged`, `unstaged`, `untracked`, `conflict`, `what` (in words),
  `order` (conflicts, staged, both, untracked).
- `log`: the newest `limit` commits (`args`: more `git log` arguments,
  such as `--all`): `hash`, `short`, `parents`, `merge`, `author`,
  `email`, `when`, `refs`, `subject`, `graph`.
- `branches`: local then remote: `name`, `remote`, `head`, `upstream`,
  `ahead`, `behind`, `gone`, `short`, `when`, `subject`.
- `tags`: `name`, `short`, `when`, `subject`.
- `stashes`: `name` (`stash@{0}`), `index`, `hash`, `when`, `subject`.
- `remotes`: `name`, `url`, `push`.
- `changes`: the diff lines of the file in `args`: not staged, then
  staged.
- `commit`: the diff lines of the commit in `args`.
- `blame`: the lines of the file in `args`: `line`, `text`, `hash`,
  `short`, `author`, `when`, `summary`, `first` (the first line of a run
  from one commit), `first_when` (`when` on such a line).

A diff line (`changes`, `commit`) has `side` (`unstaged`, `staged`,
`untracked`, `commit`), `staged`, `file`, `kind` (`section`, `file`,
`note`, `hunk`, `context`, `add`, `remove`), `old` and `new` (its line
numbers), `text` and `hunk` (its hunk's number in the file). Every line of
a hunk has `patch`, that hunk alone as `git apply` takes it; an added or
removed line also has `line_patch`, the patch for it alone. `changes`
starts each side with a `section` row naming it. In `log`, `graph` draws
the lines of history beside each commit: `●` the commit, `◆` a merge, `│`
a line going on, `┘` one ending.

`args`, on any source, is a template read where the source is shown, so a
source can follow a selection: `args = "{sel.git-files.path | quote}"`.
Its words follow `run` (or are what a `git` source reads), and the rows
are kept apart for each value. While it renders to nothing (no row
selected), the source has no rows.

A `stream` source runs `run` and keeps it running while the hub does: each
line it prints is a row as it comes (a JSON object, or with `parse =
"lines"` the field `line`), the newest `limit` (1000) kept; once it ends it
is started again after `refresh`, and what it said on its standard error
is its failure. `hub --render` and `--check` read what it prints in its
first second.

An `http` source fetches `url` (with curl, which must be installed) and
parses it as a `command` source's output; `headers`, a list, are sent with
it, each `$NAME` in them the environment's (`"Authorization: Bearer
$TOKEN"`), so no secret is written in the file.

A `commands` source runs `run`, which prints a command tree as JSON
(below),
and gives one row per runnable command: `path` (`fuzz run`), `about`, and
`args`, the arguments' descriptions, which the palette's forms use.

```json
{"name": "xtask", "about": "...", "args": [...], "commands": [
  {"name": "fuzz", "about": "...", "args": [], "commands": [
    {"name": "run", "about": "...", "args": [
      {"id": "target", "long": null, "short": null, "value": "TARGET",
       "help": "...", "required": true, "multiple": false,
       "takes_value": true, "choices": [], "default": null}]}]}]}
```

Shaping keys, any source:

```toml
[[source]]
name = "metrics"
kind = "tsv"
glob = ".dev/metrics/*.tsv"
header = true
group = { by = "_name", list = ["value", "when"], last = ["value", "unit"] }
computed = { change = "pct_change(value_list)" }
refresh = "5s"
```

- `group`: one row per value of `by`; each field in `list` becomes a list
  field `<name>_list` of every row's value, in order; each in `last` keeps
  the group's last value.
- `computed`: fields from expressions over the row (section 4).
- `refresh`: overrides `[hub] refresh` for this source.
- `page`: with `limit`, how many more rows to read each time a selection
  reaches the last row it gave (a git log 400 commits at a time).

Sources load in the background: a source due to be read again is read on a
thread of its own while the panes go on showing the rows they have, so a
slow command or a large repository never holds up the keys. A pane says how
fresh its rows are in its bottom border, at the left, only when there is
something to say: `⚠ runs failed` when its source's last load failed (the
rows shown are the last good ones; with none, the pane says why), `updated
2m ago` when its rows are older than three of their source's refreshes, and
`⟳ loading` while a reload has taken more than a second. Before a source's
first load ends, its panes say `loading…`. `hub --render` and `hub --check`
wait for every source instead.

## 4. Fields, expressions and templates

An expression reads fields of a row and computes a value:

```text
state == "running" and age(heartbeat) > 30
100 * matched / max(total, 1)
status in ["open", "flaky"]
```

- Values: numbers, `"strings"`, `true`, `false`, lists `[a, b]`, field
  names (`state`, `row.id` for the selected row of a page, `sel.jobs.id`
  for a widget's selected row).
- Operators, loosest first: `or`; `and`; `not`; `==` `!=` `<` `<=` `>`
  `>=` `in` `contains` `starts_with`; `+` `-`; `*` `/` `%`.
- Functions: `len(x)`, `max(a, b)`, `min(a, b)`, `abs(x)`, `now()`,
  `age(time)` (seconds since a UTC time), `secs(a, b)` (seconds from time
  `a` to time `b`, or to now when `b` is empty), `last(list)`,
  `first(list)`, `pct_change(list)` (last against the one before, in %),
  `exists(path)`, `lower(s)`, `upper(s)`, `if(cond, a, b)`.

A template is text with expressions in braces, each with optional formats
after `|`:

```text
"{state}  {command}"
"{100 * mf / f | num:1}%"
"{started | ago}"
"{peak_rss_kb * 1024 | bytes}"
"{value_list | spark:24}"
```

| Format | Gives |
|---|---|
| `num:N` | N decimals |
| `int` | a whole number, with thousands separators |
| `short` | `3.2M`, `45.3k` |
| `pct:N` | `x` (0 to 100) as `x%`, N decimals |
| `bytes` | `1.42 GB`, `640 MB` |
| `duration` | seconds as `1m30s` |
| `ago` | a UTC time as `2m ago` |
| `time`, `date` | a UTC time's `HH:MM:SS`, `YYYY-MM-DD` |
| `spark:N` | a list of numbers as N bars at most |
| `bar:N:MAX` | a number as a bar N wide, full at MAX |
| `pad:N`, `rpad:N` | padded to N columns, left or right |
| `trim:N` | at most N columns, ending in `…` |
| `upper`, `lower` | case |
| `or:TEXT` | TEXT when the value is empty |
| `quote` | one word for a command line, whatever it holds |

`{{` and `}}` are literal braces.

## 5. `[[widget]]`

A widget shows rows of one source. Common keys:

```toml
[[widget]]
id = "jobs"                  # how screens and actions refer to it
type = "table"
title = "Jobs"
source = "runs"
filter = "state != \"\""     # keep rows where this is true
sort = ["-started"]          # by these fields; - for descending
limit = 50
empty = "Nothing recorded in .dev/runs/ yet."
border = true
```

Every widget shows its state without color alone: the focused pane has a
thick border and `▶` before its title; the selected row of a selectable
table or list is marked `▶` (`›` in a pane not focused, so it is seen
there too); a pane with more than it can show says where it is in its
bottom border (`12–29 of 65`), and the wheel scrolls it. `search` (`/`)
keeps the rows of the focused table or list that have the text typed,
ignoring case unless it has a capital: each word must be found, `field:text`
in that field, `-word` nowhere (`state:failed -fuzz`). In a log it finds
instead, keeping every line: those found are underlined, and the log's top
border says how many there are and which one `next_match` is on; the pane's
title says the search and how much it kept (`/exit (4 of 230)`), until
`back` clears it.

Types and their keys:

- `table`: `columns`, a list of `{ header, value, width, align, style }`.
  `value` is a template; `width` is columns, `"30%"`, or `"fill"`; `align`
  is `left`, `right` or `center`; `style` is a style name or a list of
  rules `[{ when = "expr", style = "name" }]`, the first match winning.
  `selectable = true` lets the arrows move a selection, which actions and
  other widgets read (`sel.<id>.<field>`); `open = "page-id"` opens a page
  for the selected row on enter. `sort_next` (`>`) sorts the focused table
  by its next column and `sort_reverse` (`<`) the other way, as a click on
  a column's header does (again, the other way); its header says which with
  `▲` or `▼`, and past the last column the table is as `sort` says again. A
  column whose `value` is one field (`{took | duration}`) sorts by that
  field, so `1.07 GB` comes above `2 MB`; another by the text it shows. The
  selected row stays selected.
  `mark` (space; `v` with vim keys on) marks the selected row, or unmarks
  it, and moves down; `mark_all` (ctrl+a) marks every row shown, or unmarks
  them all; `back` (esc) clears them. A marked row has `●` beside it. An
  action on the pane that runs a command then runs once for each marked
  row, each as if it were the one selected: always after asking (`retry on
  3 marked row(s)?`), skipping the rows its `when` rules out, and clearing
  the marks. Opening a page or copying still takes the selected row.
- `list`: `item`, a template, and `style` as for a column.
- `text`: `lines`, a list of templates, read against the first row (or
  `row = "last"`); without a source, against nothing.
- `kv`: `pairs`, a list of `[label, template]`, one row as above.
- `gauges`: one gauge per row: `label`, `value` (0 to `max`), `max`,
  `target` (a marker), `text` (a template beside the bar), `style` rules.
- `bars`: horizontal bars, one per row: `label`, `value`, `text`.
- `sparklines`: one per row: `label`, `values` (a list field), `text`.
- `chart`: lines over time: `x` (a field holding a time or number), `y`,
  `series` (a field naming each line, optional), `marks` (horizontal
  lines: `[{ value = "expr", label, style }]`), `y_label`, `annotate`
  (vertical lines, one per row of another source: `{ source = "git-log",
  at = "when", text = "{short} {subject}" }`, commits on a time chart). Its
  x axis says where it starts and ends (a time as `2026-10-12 14:05`). A
  chart takes focus: `up` and `down` move a crosshair from point to point,
  as the mouse over it does, and the top of the chart says what it is on:
  its x, each series' value there and the nearest annotation. `mark`
  (space; `v` with vim keys on) pins the point under the crosshair, drawn
  as a line, and the crosshair then says the change from it (`+14.3% vs
  pinned`); again, unpins it. `zoom_in` (`+`) shows the newest half of
  what it shows, `zoom_out` (`-`) twice as much; `sort_reverse` (`<`) moves
  the window back half its width, `sort_next` (`>`) forward, no further
  than the newest.
- `log`: the end of a text file: `path` (a template), `follow` (keep the
  last line in view as it grows; only what was added since is read, and at
  most the file's last 16 MB are kept). Its colors (the escape sequences
  cargo and test runners print) are drawn, without color only as bold and
  the like. `next_match` (`n`) and `previous_match` (`N`) move to the next
  and previous line the search finds, or with no search to the next line
  `level` styles `bad` or `warn`, bringing it to the middle of the pane and
  marking it; `wrap` (alt+z) breaks long lines at the pane's width. `fold`,
  a regular expression, says where sections start (cargo's
  `"^\\s+Running "`): each section is folded to its first line (`▸ 12
  more line(s)`) unless it is the last, or holds a problem line or a line
  the search found; `fold` (`F`) opens them all, and again folds them.
  `level` is a regular expression whose match decides each line's style
  through `styles`, for instance `{ ERROR = "bad", WARN = "warn" }`.
- `file`: a whole text file, scrollable: `path` (a template), `markdown`
  (headings and code blocks styled).
- `terminal`: programs on terminals of their own, drawn in the pane, one per
  tab: `command` (split into words; the user's `$SHELL` when left out), `cwd`
  (relative to `.hub.toml`), `escape` (a key or a list of keys that give the
  keys back, in place of `release`'s). The first tab starts the first time its
  screen is shown; programs run until the hub ends or their tab is closed.
  `insert` (enter) on it gives the tab shown the keys: from then on every key
  and paste goes to its program, as xterm sends them, but those that give the
  keys back (section 7). It answers what a program asks its terminal (what it
  is, where the cursor is, the theme's `fg` and `bg`). Once the program ends,
  `insert` starts it again. `tab_new` starts `command` in a new tab and shows
  it, the keys staying with the hub until `insert`; `tab_next` and
  `tab_previous` go between tabs (with vim keys on, a count goes to that tab:
  `2]`); `tab_close` closes the tab shown, asking first while its program runs,
  and the keys stay with the hub. `tab_rename` names the tab shown; an empty
  name gives it back to the program. `keep = "tmux"` runs each tab's
  program in a tmux session of its own, on the hub's own tmux server (`tmux
  -L hub`): it goes on after the hub quits, the next hub takes it up again
  (as the session keeps the tab), and closing the tab ends it. A terminal
  whose keys are the hub's searches its whole history with `search`,
  `next_match` and `previous_match` scrolling it to each line found, and
  `yank` copies what it shows. With two tabs or more, a strip names them
  by the name given, else the title their program set, else the program's name,
  the one shown reversed. It needs no source.

## 6. `[[screen]]` and layouts

A screen arranges widgets; a page is a screen opened for one row.

```toml
[[screen]]
id = "jobs"
title = "Jobs"
key = "3"                    # switches to it; pages have none
layout = { split = "rows", sizes = ["fill", "8"], children = [
  "jobs", "jobs-log" ] }
narrow = "jobs"              # the layout below [hub] narrow_below

[[screen]]
id = "job"
page = true                  # opened by a table's open = "job"
title = "{row.command | trim:40} · {row.state}"
layout = { split = "columns", sizes = ["40%", "fill"], children = [
  "job-memory", "job-figures" ] }
```

A layout is a widget id, or `{ split, sizes, children }`: `split` is
`rows` or `columns`, `sizes` one per child (columns or rows as a number,
`"N%"`, `"min:N"`, `"max:N"` or `"fill"`), `children` widget ids or more
layouts. A page's widgets read the row it was opened for as `row.<field>`.

A screen's `key` may be left out: it is then gone to by `screen_next` and
`screen_previous`, its tab, or its id in the palette (section 10). A page
has no key.

## 7. `[bindings]` and `[[action]]`

`[bindings]` binds the engine's own actions, once for vim keys off
(`[bindings.default]`) and once for on (`[bindings.vim]`, section 10).
Each action takes a key or a list of keys; keys typed in turn are written
with spaces between (`"g g"`, `"ctrl+w w"`). An action left out keeps its
default:

| action | vim keys off | vim keys on |
|---|---|---|
| `up` | up | k, up, ctrl+y |
| `down` | down | j, down, ctrl+e |
| `half_up` | - | ctrl+u |
| `half_down` | - | ctrl+d |
| `page_up` | pageup | ctrl+b, pageup |
| `page_down` | pagedown | ctrl+f, pagedown |
| `top` | home | gg, home |
| `bottom` | end | G, end |
| `next` | tab | ctrl+w w |
| `previous` | shift+tab | ctrl+w W |
| `pane_left` | shift+left, ctrl+w h | h, ctrl+w h, ctrl+w left |
| `pane_right` | shift+right, ctrl+w l | l, ctrl+w l, ctrl+w right |
| `pane_up` | shift+up, ctrl+w k | ctrl+w k, ctrl+w up |
| `pane_down` | shift+down, ctrl+w j | ctrl+w j, ctrl+w down |
| `pane_last` | ctrl+w p | ctrl+w p |
| `zoom` | ctrl+w z | ctrl+w z, ctrl+w o |
| `pane_pick` | ctrl+w q | ctrl+w q |
| `screen_next` | - | gt |
| `screen_previous` | - | gT |
| `open` | enter | enter, ctrl+] |
| `back` | esc | esc, ctrl+t |
| `jump_back` | alt+left, alt+b | ctrl+o, alt+left |
| `mark_place` | alt+m | alt+m |
| `go_place` | ' | ' |
| `jump_forward` | alt+right, alt+f | tab, alt+right |
| `insert` | enter | i, a, enter |
| `release` | ctrl+\ | ctrl+\ |
| `release_prompt` | esc | esc |
| `tab_new` | ctrl+n | ctrl+n |
| `tab_close` | ctrl+x | ctrl+x |
| `tab_next` | ] | ] |
| `tab_previous` | [ | [ |
| `tab_rename` | R | R |
| `search` | / | / |
| `mark` | space | v |
| `mark_all` | ctrl+a | ctrl+a |
| `yank` | alt+y | alt+y |
| `sort_next` | > | > |
| `sort_reverse` | < | < |
| `zoom_in` | + | + |
| `zoom_out` | - | - |
| `next_match` | n | n |
| `previous_match` | N | N |
| `wrap` | alt+z | alt+z |
| `fold` | F | F |
| `palette` | : | : |
| `alerts` | ! | ! |
| `refresh` | ctrl+r | ctrl+r |
| `cancel` | ctrl+k | ctrl+k |
| `undo` | ctrl+z | ctrl+z |
| `reload` | ctrl+l | ctrl+l |
| `help` | ? | ? |
| `vim` | f2 | f2 |
| `mouse` | f3 | f3 |
| `quit` | q, ctrl+c | q, ZZ, ZQ, ctrl+c |
| `screen_prefix` | - | g |

`screen_prefix` is typed before a screen's own `key`: none with vim keys
off, so `1` goes to the screen keyed `1`; `g` with them on, so `g1` does
(and a digit alone is a count). `insert` and `open` may share keys: on a
terminal widget the keys go to `insert`, elsewhere to `open`. `release` and
`release_prompt` are read only while a terminal has the keys:
`release_prompt`'s keys (esc) work only while no full-screen program runs
in it, so nvim, less or htop keep esc, and `release`'s (ctrl+\) always.

```toml
[bindings.default]
palette = [":", "ctrl+p"]
up = ["up", "k"]
down = ["down", "j"]

[bindings.vim]
screen_prefix = "space"      # space 1 instead of g1
```

After the first keys of a binding of several (`g`, `ctrl+w`), a box at the
bottom right lists the keys that may follow and what each does, until a key
ends or breaks the binding.

The checks refuse an unknown action, a key that is not one, two actions
on the same keys, and keys that can never be typed because a shorter
binding is taken first (`top = "g"` would hide `gt`).

Help (`?`) lists every action with its keys as bound, the screens, the
project's actions and the palette's commands, for the vim setting in use;
`hub --keys` prints the same list, `hub --keys --vim` the one for vim keys
on. With vim keys off the project's actions take a key before the engine;
with them on, the engine's bindings come first, and help notes each
action whose key they take.

Panes that take focus (selectable tables and lists, logs, files and
terminals) are gone to in turn by `next` and `previous`, or by where they
are drawn: `pane_left`, `pane_right`, `pane_up` and `pane_down` go to the
nearest pane that way (the closest edge, then the one overlapping most
across, then the nearest middle). `pane_last` goes back to the pane
focused before on the same screen. `zoom` gives the focused pane the whole
screen (its title says `zoomed`, the footer offers the key back) and takes
it back; going to another pane unzooms first, as tmux does. `pane_pick`
puts a key on each pane, 1 to 9 then a to z in layout order, and typing one
focuses that pane; any other key leaves focus as it was. With vim keys on,
a count before `next` goes to that pane (`3 ctrl+w w`). The focused pane
has a thick border and a ▶ before its title, so focus does not rest on
color.

An engine key that means nothing where it is typed (`tab_rename` with no
terminal focused) is left to the project's action on that key.

`jump_back` and `jump_forward` work as a browser's back and forward: each
screen switched to and each page opened or closed is a place, and back
returns to the place before, forward to the one after, until a new place
is gone to.

`yank` (alt+y) copies the focused table or list, through the terminal (OSC
52): `r` its selected row as JSON, `t` the rows it shows as TSV, `m` as a
Markdown table, each cell as shown. `mark_place` (alt+m) and a letter marks
where the hub is: the screen, its pages, the pane and its row;
`go_place` (`'`) and the letter goes back there, the row selected again
wherever it is now.

`[[variable]]` declares a value every template reads as `var.<name>`, chosen
by its `key` (the next value) or `:let NAME VALUE`, and kept for the next
start; every source is read again when it changes, as their `args` may
read it:

```toml
[[variable]]
name = "target"
values = ["format", "layout", "toml"]   # or source = "runs", field = "target"
default = "format"
key = "T"
```

`[[action]]` declares the rest:

```toml
[[action]]
id = "cancel"
label = "cancel"             # shown in the footer and in help
key = "c"
on = "jobs"                  # a widget id, a screen id, or "global"
when = "sel.jobs.state == \"running\""
confirm = "Cancel {sel.jobs.command}?\nIts records are kept."
run = "cargo xtask runs cancel {sel.jobs.id}"
mode = "wait"                # wait, background, terminal
then = "refresh"             # refresh, back, toast, open:PAGE, screen:ID
toast = "asked {sel.jobs.id} to stop"
```

- `run`: a template, split into words (no shell). Its output goes to the
  log in `[hub] state`.
- `mode`: `wait` runs it with a spinner; `background` starts it and goes
  on; `terminal` leaves the interface, runs it on the terminal, and comes
  back on enter. ctrl+c there stops the command, not the hub.
- `open = "page"` instead of `run` opens a page; `copy = "template"` puts
  text on the clipboard (through the terminal).
- `input`: a template given to `run` on its standard input, in `wait`
  mode: `input = "{sel.git-diff.patch}"` with `run = "git apply --cached
  -"`.
- `ask`: fields filled in before it runs, `[{ name, label, default, lines,
  required }]`; each is `ask.<name>` in its templates. `default` is a
  template; `lines` over 1 makes a field that takes newlines; a `required`
  one may not be empty. Tab goes between fields, ctrl+s runs, enter goes
  to the next field (and runs from the last one-line field), esc gives up.
- `timeout`: how long a `wait` action may run, 30 seconds when left out
  (`"5m"` for a push).
- `before` and `undo`: an action that can be taken back says how. `before`,
  a command template, runs just before it, and what it prints is `before`
  in `undo`, the command that takes it back; `undo_input` is given to that
  on its standard input. Once the action succeeds, `undo` is read (against
  the selection and fields it started with, `before`, and `output`, what it
  printed) and kept; `undo` (ctrl+z) asks, then runs the last kept (the
  last twenty are), and the footer names it. Only `wait` actions have them.

  ```toml
  run = "git restore -- {sel.git-files.path | quote}"
  before = "git diff -- {sel.git-files.path | quote}"
  undo = "git apply --recount -"
  undo_input = "{before}\n"
  ```

- `cancel` (ctrl+k) stops the newest `wait` action still running, with
  everything its command started (it runs in a process group of its own,
  which a timeout stops whole too); the footer offers it while one runs.
  A stopped action says so and is not a failure.
- A `wait` action shows a spinner and its time in the top bar while it
  runs; one that fails says what its command printed on its standard
  error, in a toast and in the alerts drawer. Every action's command and
  what it printed are added to `actions.log` in `[hub] state`, a
  `background` one's as it prints. Quitting while actions run asks first.

The footer lists the keys that do something where the cursor is, then
`help` and `quit` at its right. When they do not all fit, it shows those
that do and then how many more, `… 6 more ?`: help lists every one of them
first, under HERE, before every key of the engine, the screens, all the
actions with their commands and the palette's commands.

## 8. `[topbar]`, `[footer]`, `[[alert]]`

```toml
[topbar]
segments = [
  { text = "{hub.name}", style = "title" },
  { source = "git", text = "{branch} @ {short}" },
  { source = "git", text = "(clean)", when = "not dirty", style = "ok" },
  { source = "runs", count = "state == \"running\"",
    text = "{count} running", when = "count > 0", style = "info" },
]
```

A segment with a `source` reads its last row (a log's or a series' newest);
one with `count` reads the whole source and sets `count` to how many rows
match.

`[footer]` has `extra`, a list of `[key, label]` shown after the actions.
A key written `{action}` shows that action's first key as bound, and
`{screens}` the screens' keys (`1-9`, or `g1-g9` with vim keys on), so the
footer follows `[bindings]`.

```toml
[[alert]]
source = "incidents"
when = "status == \"open\""
key = "id"                   # once per value: an alert is not repeated
text = "new incident: {kind} {target}: {title | trim:40}"
style = "bad"
open = "incident"            # enter on the alert opens this page
```

An alert fires when a row matches `when` with a `key` not seen matching
before: it shows as a toast, reaches the desktop as `[hub] notify` says
(`osc`, the default, sends OSC 9 and OSC 777, which iTerm2, kitty, WezTerm,
foot and Windows Terminal show as notifications; `bell` rings the
terminal's bell; `both`; `none`), and waits in the drawer (`!`), with the
failures of actions (which reach the desktop too) and the problems of a
config that could not be read again. An alert resolves when its row no
longer matches (or is gone): its entry says `✓ resolved 3m ago`, the top
bar counts only those still firing, and should the row match again it fires
again, its entry taking the old one's place. A source that failed resolves
nothing. The keys at the hub's start are taken as seen, once each alert's
source has loaded.

In the drawer the arrows (or `up` and `down`) choose one, enter opens its
page (those that have one end in `›`), `s` snoozes its key for an hour (no
toast and no notification; it still comes to the drawer) or wakes it, `d`
dismisses it and `x` clears them all. What was seen and what is snoozed are
kept in `[hub] state`.

## 9. `[palette]`

```toml
[palette]
source = "commands"          # a commands source
run = "cargo xtask {path} {args}"
background = "cargo xtask {path} {args} --detach"
presets = "presets.toml"     # in [hub] state
recent = { source = "runs", command = "command", limit = 5 }
```

The palette searches every command by its path and description, saved
presets and recent runs: with nothing typed, presets and recent runs come
first; once something is typed, commands do. Enter on a command opens a
form built from its arguments: a field per argument with its type, choices
and default, checked as typed, the field being edited marked `›`; a flag
without a value is a switch shown `[✓] on` or `[ ] off`, flipped by space,
the arrows or `x`, set by `y` and `n`; `f1` (or `ctrl+h`) lists them with
their help; `ctrl+s` saves the form as a preset. The form shows the exact
command it will run; enter runs it in the background, `ctrl+t` on the
terminal. In the palette, `alt+enter` runs a command with its defaults.
Enter on a preset or a recent run shows its command line and runs it only
on `y`. A table or list whose source is the palette's opens the selected
command's form on enter. When the query is a command of the palette's own
(section 10), it is the first line, saying what enter does. Two letters or
more also find rows, in every screen's selectable tables and lists, by
their `id`, `name`, `path`, `hash`, `short`, `signature` or `_name` (`→
0cdf1bf0 · Incidents · INCIDENTS`): enter goes to that screen with the row
selected and its pane focused. The palette opens without `[palette]` too,
for these and its own commands: `:w [FILE]` saves the screen as text (in
`[hub] state` without FILE), `:let NAME VALUE` sets a variable, `:set
contrast` and `:set nocontrast` turn high contrast on and off. A value is
checked by what its name says it is: `N`, `SECONDS`, `COUNT`, `YEAR` and
the like a whole number, `COLSxROWS` and `X.Y.Z` their shapes. Esc keeps a
form as a draft: opening its command again finds it as it was left.

## 10. Vim mode

With `[hub] vim = true`, or after f2 (the `vim` key) or `:set vim` in the
palette, keys work as vim's do: `[bindings.vim]` and its defaults (section
7) replace `[bindings.default]`. The toggle is kept in `[hub] state` for
the next session; `:set novim` or f2 again turns it off. The footer names
the mode:

- NORMAL: moving about. A count may come before the keys: `5j` goes down
  five rows, `3G` to the third row, `2gt` to the second screen. Screens
  are `g` and their key: `g1` to `g9`.
- INSERT: typing into a terminal, a search or a form; esc leaves it.
- COMMAND: the palette. Besides its commands, it takes `q` (`q!`, `qa`,
  `wq`, `x`), `set vim`, `set novim`, `set vim!`, `e` or `reload` (section
  12), a row number, or a screen's id.

esc drops keys typed toward a binding (`g`, `ctrl+w`) and a count.

## 11. The git window: `git.hub.toml`, for any project

`xtask/hub/git.hub.toml` is a git window made only of what this file
describes, for any project to include (`[hub] include`). Its styles are
named `git-*`, which a project's own `[styles]` may redefine.

- Git (key 0): the branch with its upstream, ahead and behind; the changed
  files (git's two status letters, then the path, then what changed in
  words); and the selected file's changes, not staged then staged, each
  side headed.
- History: the log of every branch with its graph, and the selected
  commit's changes.
- Refs: branches, local then remote; tags; stashes; remotes.
- Blame (a page): who last changed each line of a file.

Its actions, by where the cursor is:

- Files: space stages the file, or unstages it when it is staged only; `a`
  stages all, `A` unstages all; `d` discards what is not staged, or
  deletes an untracked file, asking first; `e` edits it in `$EDITOR`; `B`
  opens its blame.
- Changes: space stages the hunk under the cursor, or unstages it on the
  staged side; `S` stages or unstages the one line; `d` discards the hunk,
  asking first.
- Git screen: `c` commits, asking for a summary and a body; `C` commits in
  the editor, with the diff; `M` amends the last commit, asking first; `z`
  stashes everything, asking for a message; `f` fetches, `p` pulls
  (fast-forward only), `P` pushes, asking first.
- History: `y` copies the hash; `o` checks the commit out, asking first;
  `r` reverts it; `x` cherry-picks it; `t` tags it; `u` resets the branch
  to it, keeping the changes staged; `I` rebases interactively from it, on
  the terminal.
- Branches: space switches to the branch, or tracks a remote one; `n`
  makes a branch; `R` renames; `m` merges it in, `r` rebases onto it, `D`
  deletes it, each asking first. Tags: `D` deletes. Stashes: space
  applies, `p` pops, `D` drops. Remotes: `f` fetches.

More of it:

- Files are shown as a tree; space, `d` and the others take a folder whole.
  `V` opens the file's changes side by side, `W` word by word, and on a
  conflicted file `X` its conflicts: `o`, `t` or `b` take the conflict
  under the cursor as ours, theirs or both (each undoable), `O` and `T`
  every one, `a` marks the file resolved.
- History reads 400 more commits each time the selection reaches the last;
  `+` and `-` mark a commit good or bad, which starts a bisection when none
  is under way.
- Reflog: where HEAD has been, with the entry's changes; `u` resets the
  branch to an entry, `o` checks it out (both undoable).
- Worktrees: the working trees, `n` adds one with a new branch, `D`
  removes, `p` prunes; and the bisection's steps, `s` skipping, `R` ending
  it.
- CI (through `gh`, logged in on its own): the branch's pull request and
  the latest runs; `o` opens a run, `l` its failed jobs' log, `w` watches
  it, `r` runs its failed jobs again, `O` opens the pull request.

`undo` (ctrl+z) takes back, after asking, a discard (of a file's changes or
a hunk), a commit, an amend, a revert, a cherry-pick, a reset, a merge, a
rebase, a pull, a switch or checkout (back to where it was), a new or
renamed branch's switch or name, a deleted branch or tag (where it was), a
stash and a dropped stash, as lazygit's `z` does but for discards too.
Staging is not kept, being taken back by the key that did it.

## 12. The session: kept across restarts and reloads

The hub keeps its session in `[hub] state` (`session.json`), saving it
every two seconds while it changes and on quit, through a temporary file
so it is never half written, and takes it up again on start (`hub --fresh`
starts without it). `[hub] persist` keeps all of it (`true`, the default),
none (`false`), or some of its parts:

- `place`: the screen and the pages open over it, the pane focused on each
  screen (coming back to a screen focuses it again, within a session too),
  and the way back and forward.
- `selection`: what was selected in each pane, found again by its `id`,
  `hash`, `path` or `name` (else by all its fields) even when it moved;
  how far panes were scrolled; the search.
- `terminals`: each terminal widget's tabs, their names and which is
  shown, and each one's last screen. Programs do not outlive the hub: each
  tab starts its program again, under its last screen. A tab whose program
  had the whole screen (nvim, less) starts on a blank one.

A session belongs to the first hub opened on its state folder. Another
opened while that one runs (a hub in one of its own terminals, or in a
second window) says so and neither takes up nor saves the session, so the
two never write over each other's.

The config is read again when one of its files (its own or one it
includes) changes, unless `[hub] reload = false`, and on `reload` (ctrl+l;
`:e` or `:reload` in the palette). The hub goes on where it was: its
terminals keep running, its actions keep going. A config with problems is
not taken: the one before goes on, and the alerts drawer lists the
problems.

## 13. The mouse

Unless `[hub] mouse = false`, the hub takes the mouse; `mouse` (f3) or
`:set nomouse` gives it back to the terminal, to select text with it (most
terminals also select with shift held while the hub has it), and `:set
mouse` takes it again.

- A click on a tab goes to its screen; on an open page's chip, back.
- A click on a pane focuses it; on a row of a selectable one, selects it;
  a double click opens it (`open`).
- A click on a footer hint presses its key; on `… N more`, help.
- A click on a terminal's screen gives it the keys (`insert`); on one of
  its tabs, shows it. A click outside a terminal that has the keys takes
  them back.
- The wheel moves the selection of the pane under it, or scrolls it, or a
  terminal's view back into its history (a key brings it back).
- A terminal whose program asked for the mouse (nvim, htop) gets it over
  its screen, as xterm reports it.
- In the palette a click picks a line and a second runs it; in a form a
  click goes to a field and flips a switch; in the pane picker a click
  picks the pane. A click outside help, the alerts or a confirmation
  closes it (a confirmation's answer is then no); outside a form being
  filled in, it does nothing.

## 14. Running it, and reading it without seeing it

```
hub [--config FILE] [--fresh] [--no-color]
hub --check
hub --keys [--vim]
hub --render SCREEN [--size COLSxROWS] [--vim | --no-vim] [--plain]
```

- `--config`: the file, else the nearest `.hub.toml` here or above.
- `--fresh`: start without the session kept (section 12).
- `--no-color`: no colors (section 2).
- `--check`: read the config and every source; say what failed.
- `--keys`: every key, as help lists them, for vim keys off or on.
- `--render`: one screen drawn as text, as the config's vim setting has it
  unless `--vim` or `--no-vim`. `--plain` writes it for a screen reader
  instead: each pane's title, then its rows as text, columns between ` |
  `, the selected row marked `>`, and no drawing characters.

For screen readers and magnifiers, which follow the terminal's cursor: the
cursor is where text is typed (the palette, the search, a form, a field
asked for, a terminal that has the keys), and with `[hub] cursor =
"follow"` on the focused pane's selected row otherwise. Every state is in
text as well as in color, every key is in help and in `--keys`, toasts
last as long as `[hub] toast` says (failures three times as long, and kept
in the drawer), and `[hub] motion = false` stills the spinner.
