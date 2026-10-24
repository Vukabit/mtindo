# Configuration

`mto` reads `rustfmt.toml` as the standard formatter does, so an existing one
works unchanged, and its own settings from `mto.toml`. How `mto` behaves for
you on every project is set in a separate user config.

## Where settings come from

For each folder it formats, the settings come from, lowest precedence first:

1. The defaults, for the style edition in effect.
2. The nearest `.rustfmt.toml` or `rustfmt.toml`, found as the standard
   formatter finds it: the folder, each parent folder, the home folder, then
   the formatter's folder in the user's configuration folder. Only that one
   file is used.
3. Every `.mto.toml` or `mto.toml` from the outermost folder down; a nearer
   file wins key by key, and a file with `root = true` stops the search
   upwards. An `mto.toml` may also set the formatting settings, and wins over
   `rustfmt.toml` with a warning when they disagree.
4. The package edition in the nearest `Cargo.toml`, as `cargo fmt` passes it.
5. The command line: `--config`, `--edition`, `--style-edition`.

`--config-path FILE` replaces steps 2 and 3 with that one file.

Unknown keys are warnings. A value of the wrong type or outside the legal
values stops the run with exit code `2`, naming the file, line and column.
With `format = false`, the formatting settings change nothing except the
style edition.

## Seeing what applies

```bash
mto config path            # the files in effect, history and the user config
mto config print           # every value, with where it came from
mto config print --minimal # only what differs from the defaults
mto config options         # every setting a file may hold, by file
mto config options --toml  # the same, as a commented file
```

Every setting, the formatting settings and mto's own, is documented with its
legal values and defaults in the
[settings reference](../generated/config/README.md), which also has a JSON
Schema of `mto.toml` for editors, and in the
[manual](../generated/manual.md) (`mto --man`).

## mto.toml

`mto init` writes an `mto.toml` with layout on and mto's other settings at
their defaults, each with a comment; without the comments:

```toml
layout = true
format = true
blank_lines_between_blocks = 1
group_impls = true
ignore_dirs = [".git", "target", "node_modules", "vendor", "dist", ".venv"]
impl_method_order = "written"
import_grouping = true
import_sort = "style_edition"
name_sort = "version"
sort_alphabetically = true
sort_imports = true
test_module = "last"
trait_impl_order = []
trait_impl_sort = "name"
type_order = ["ModDecl", "UseDecl", "MacroDecl", "MacroDef", "ConstDecl",
    "StaticDecl", "TypeAlias", "TraitDecl", "EnumDecl", "StructDecl", "Union",
    "ImplBlock", "ExternBlock", "FnDecl", "Unknown"]
visibility_order = ["Pub", "PubCrate", "PubIn", "PubSuper", "Private"]
```

`reorder_impl_items` is honored inside impl blocks, exactly as the standard
formatter applies it. `tab_width` was renamed `tab_spaces`, the standard
name.

## The user config

How `mto` behaves for you, on every project, is set in
`~/.config/mtindo/config.toml` (`$XDG_CONFIG_HOME` is honored;
`%APPDATA%\mtindo\config.toml` on Windows; `MTINDO_CONFIG` names another
file). It is created on the first run, with every setting commented out at
its default, except when `CI` is set. It holds no layout or formatting
settings, and project files cannot hold its settings.

```toml
[history]
enabled = true      # record every run that writes files
keep_days = 30      # 0: no limit; the newest run is always kept
keep_runs = 100
max_size = "256MB"  # bytes, or KB, MB, GB

[output]
color = "auto"      # always, never; --color, NO_COLOR, FORCE_COLOR win

[report]
save = true         # keep bug reports for `mto report`; false: print them

[safety]
allow_dirty = false # with history off, overwrite uncommitted changes
sync = "batch"      # or "full": flush each file to disk as it is written

[doctor]
enabled = false     # mto doctor on/off; the build guard
command = ["cargo", "check", "--workspace", "--all-targets", "--keep-going"]
timeout = 600       # seconds a check may take; 0: no limit
max_checks = 64     # checks one narrowing may run
```

`mto config print --user` shows each value and where it came from. History
and bug reports are described in [Safety and history](safety.md), the
`[doctor]` section in [The build guard](build-guard.md).
