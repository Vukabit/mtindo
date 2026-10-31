---
title: mto settings reference
generated_by: cargo xtask docs
standard_formatter:
  stable:
    version: "1.10.0"
    released: "2026-10-01"
  nightly:
    version: "1.11.0-nightly"
    released: "2026-10-10"
keys:
  total: 120
  mto: 18
  formatting: 90
  user: 12
in_mto:
  supported: 36
  partial: 27
  missing: 41
  not_planned: 4
---

# mto settings reference

Every key mto reads from a configuration file: its type, legal values, defaults,
and how far mto honors it. These pages are generated from mto's settings table,
the one `mto --man` and `mto config print` read, by `cargo xtask docs`; change
the table, not the pages.

## The files

*   [`mto.toml`](mto.toml.md) or `.mto.toml`: mto's own settings, which only
    these files may hold. They may hold the formatting settings too.
*   [`rustfmt.toml`](rustfmt.toml.md) or `.rustfmt.toml`: the formatting
    settings, as the standard formatter reads them.
*   [`config.toml`](config.toml.md): the per-user settings, how mto behaves for
    one person on every project.
*   [`mto.schema.json`](mto.schema.json): a JSON Schema of `mto.toml`, for
    editors.

## How mto finds settings

mto reads the settings for each folder it formats from, lowest precedence first:

1. The defaults on these pages, for the style edition in effect.
2. The nearest `.rustfmt.toml` or `rustfmt.toml`, found as the standard
   formatter finds it: in the folder, then each parent folder, then the home
   folder, then the standard formatter's folder in the user's configuration
   folder. Only that one file is used.
3. Every `.mto.toml` or `mto.toml` from the outermost folder down to the folder
   itself; a nearer file wins key by key. The search upwards stops at a file
   that sets `root = true`. An `mto.toml` may also set the formatting settings,
   and wins over `rustfmt.toml`, with a warning when the two disagree.
4. The edition of the package in the nearest `Cargo.toml` (inherited from the
   workspace when it says `edition.workspace = true`), as `cargo fmt` passes it.
5. The command line: `--config key=value`, `--edition` and `--style-edition`.

`--config-path FILE` replaces steps 2 and 3 with that one file, which may hold
both mto's settings and the formatting settings. `mto config path` lists the
files in effect and `mto config print` shows every value with where it came
from.

The style edition is resolved as the standard formatter resolves it:
`style_edition` if set, else `version` (`One` is 2015, `Two` is 2024), else
`edition`, else 2015. It decides the defaults of the settings that change
between style editions.

An unknown key, or a key in the wrong file, is a warning and is ignored. A value
of the wrong type, or outside the legal values, is an error (exit code 2) naming
the file, line and column. Values are matched without regard to case, as the
standard formatter matches them.

## Turning mto on

Layout and formatting are off by default, so mto changes nothing in a project
until an `mto.toml` turns them on; `mto init` writes one with layout on. Three
keys of `mto.toml` decide where and what mto does:

*   `layout = true`: Rearrange the items of each file. While it is off, mto's
    other settings but `format` and `ignore_dirs` have no effect, and setting
    one is a warning.
*   `format = true`: Format code in the Rust style. It does not apply where
    `disable_all_formatting` is set.
*   `root = true`: this file is the outermost `mto.toml` read; the search
    upwards for more stops here. Only `mto.toml` and `.mto.toml` may hold it, as
    `true` or `false`.

## The per-user file

The user configuration holds how mto behaves for you, never layout or
formatting. It is `$MTINDO_CONFIG` when set (the file must exist), else
`$XDG_CONFIG_HOME/mtindo/config.toml`, else `~/.config/mtindo/config.toml` on
macOS and Linux and `%APPDATA%\mtindo\config.toml` on Windows. Project files
cannot set user settings, and the user configuration cannot set project ones.

Its keys are written in sections: `history.enabled` is `enabled` under
`[history]`.

## Editor support

`mto.schema.json` describes `mto.toml` in JSON Schema (draft 2020-12): each
key's type, legal values, default and description, and `root`. TOML editor
tooling that reads JSON Schema can check and complete `mto.toml` with it. Some
tools read a comment on the first line naming the schema by its path or URL; a
project keeping a copy beside its `mto.toml` would start the file:

```toml
#:schema ./mto.schema.json
layout = true
```

mto matches values without regard to case. The schema lists each value as
`mto config print` writes it, so an editor may flag another spelling that mto
accepts.

## Every key

The last column says how far mto honors the key: supported, partial, missing or
not planned; the key's entry says which part. The per-user keys are mto's own
behavior, so the column does not apply to them.

<table>
<tr><th>Key</th><th>File</th><th>Summary</th><th>In mto</th></tr>
<tr>
<td><a href="rustfmt.toml.md#array_width">
<code>array_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum width of an array literal before falling back to vertical
    formatting.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#attr_fn_like_width">
<code>attr_fn_like_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum width of the args of a function-like attributes before falling back
    to vertical formatting.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#binop_separator">
<code>binop_separator</code></a></td>
<td>rustfmt.toml</td>
<td>Where to put a binary operator when a binary expression goes multiline.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="mto.toml.md#blank_lines_between_blocks">
<code>blank_lines_between_blocks</code></a></td>
<td>mto.toml</td>
<td>Blank lines between items</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#blank_lines_lower_bound">
<code>blank_lines_lower_bound</code></a></td>
<td>rustfmt.toml</td>
<td>Minimum number of blank lines which must be put between items.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#blank_lines_upper_bound">
<code>blank_lines_upper_bound</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum number of blank lines which can be put between items.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#brace_style">
<code>brace_style</code></a></td>
<td>rustfmt.toml</td>
<td>Brace style for items.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#chain_width">
<code>chain_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum length of a chain to fit on a single line.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#color">
<code>color</code></a></td>
<td>rustfmt.toml</td>
<td>What Color option to use when none is supplied: Always, Never, Auto.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#combine_control_expr">
<code>combine_control_expr</code></a></td>
<td>rustfmt.toml</td>
<td>Combine control expressions with function calls.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#comment_width">
<code>comment_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum length of comments. No effect unless wrap_comments = true.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#condense_wildcard_suffixes">
<code>condense_wildcard_suffixes</code></a></td>
<td>rustfmt.toml</td>
<td>Replace strings of _ wildcards by a single .. in tuple patterns.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#control_brace_style">
<code>control_brace_style</code></a></td>
<td>rustfmt.toml</td>
<td>Brace style for control flow constructs.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#disable_all_formatting">
<code>disable_all_formatting</code></a></td>
<td>rustfmt.toml</td>
<td>Don't reformat anything.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#doc_comment_code_block_small_heuristics">
<code>doc_comment_code_block_small_heuristics</code></a></td>
<td>rustfmt.toml</td>
<td>Value for use_small_heuristics for code blocks in doc comments. No effect
    unless format_code_in_doc_comments = true.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#doc_comment_code_block_width">
<code>doc_comment_code_block_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum width for code snippets in doc comments. No effect unless
    format_code_in_doc_comments = true.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="config.toml.md#doctorcommand">
<code>doctor.command</code></a></td>
<td>config.toml</td>
<td>The command the build guard runs, as its words</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="config.toml.md#doctorenabled">
<code>doctor.enabled</code></a></td>
<td>config.toml</td>
<td>Check that the project still builds after each run (it runs
    <code>command</code>)</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="config.toml.md#doctormax_checks">
<code>doctor.max_checks</code></a></td>
<td>config.toml</td>
<td>The most times one narrowing runs the command</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="config.toml.md#doctortimeout">
<code>doctor.timeout</code></a></td>
<td>config.toml</td>
<td>Seconds the command may run before it counts as failed</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#edition">
<code>edition</code></a></td>
<td>rustfmt.toml</td>
<td>The edition of the parser (RFC 2052)</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#emit_mode">
<code>emit_mode</code></a></td>
<td>rustfmt.toml</td>
<td>What emit Mode to use when none is supplied.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#empty_item_single_line">
<code>empty_item_single_line</code></a></td>
<td>rustfmt.toml</td>
<td>Put empty-body functions and impls on a single line.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#enum_discrim_align_threshold">
<code>enum_discrim_align_threshold</code></a></td>
<td>rustfmt.toml</td>
<td>Align enum variants discrims, if their diffs fit within threshold.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#error_on_line_overflow">
<code>error_on_line_overflow</code></a></td>
<td>rustfmt.toml</td>
<td>Error if unable to get all lines within max_width.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#error_on_unformatted">
<code>error_on_unformatted</code></a></td>
<td>rustfmt.toml</td>
<td>Error if unable to get comments or string literals within max_width, or they
    are left with trailing whitespaces.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#file_lines">
<code>file_lines</code></a></td>
<td>rustfmt.toml</td>
<td>Lines to format; this is not supported in rustfmt.toml, and can only be
    specified via the --file-lines option.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#float_literal_trailing_zero">
<code>float_literal_trailing_zero</code></a></td>
<td>rustfmt.toml</td>
<td>Add or remove trailing zero in floating-point literals.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#fn_args_layout">
<code>fn_args_layout</code></a></td>
<td>rustfmt.toml</td>
<td>(deprecated: use fn_params_layout instead)</td>
<td>not planned</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#fn_call_width">
<code>fn_call_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum width of the args of a function call before falling back to vertical
    formatting.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#fn_params_layout">
<code>fn_params_layout</code></a></td>
<td>rustfmt.toml</td>
<td>Control the layout of parameters in function signatures.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#fn_single_line">
<code>fn_single_line</code></a></td>
<td>rustfmt.toml</td>
<td>Put single-expression functions on a single line.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#force_explicit_abi">
<code>force_explicit_abi</code></a></td>
<td>rustfmt.toml</td>
<td>Always print the abi for extern items.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#force_multiline_blocks">
<code>force_multiline_blocks</code></a></td>
<td>rustfmt.toml</td>
<td>Force multiline closure bodies and match arms to be wrapped in a block.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="mto.toml.md#format">
<code>format</code></a></td>
<td>mto.toml</td>
<td>Format code in the Rust style</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#format_code_in_doc_comments">
<code>format_code_in_doc_comments</code></a></td>
<td>rustfmt.toml</td>
<td>Format the code snippet in doc comments.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#format_generated_files">
<code>format_generated_files</code></a></td>
<td>rustfmt.toml</td>
<td>Format generated files.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#format_macro_bodies">
<code>format_macro_bodies</code></a></td>
<td>rustfmt.toml</td>
<td>Format the bodies of declarative macro definitions.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#format_macro_matchers">
<code>format_macro_matchers</code></a></td>
<td>rustfmt.toml</td>
<td>Format the metavariable matching patterns in macros.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#format_strings">
<code>format_strings</code></a></td>
<td>rustfmt.toml</td>
<td>Format string literals where necessary.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#generated_marker_line_search_limit">
<code>generated_marker_line_search_limit</code></a></td>
<td>rustfmt.toml</td>
<td>Number of lines to check for a <code>@generated</code> marker when
    <code>format_generated_files</code> is enabled.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#generated_markers">
<code>generated_markers</code></a></td>
<td>mto.toml</td>
<td>Texts that mark a file as generated</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#group_impls">
<code>group_impls</code></a></td>
<td>mto.toml</td>
<td>Place each impl block right after the type it is for</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#group_imports">
<code>group_imports</code></a></td>
<td>rustfmt.toml</td>
<td>Controls the strategy for how imports are grouped together.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#hard_tabs">
<code>hard_tabs</code></a></td>
<td>rustfmt.toml</td>
<td>Use tab characters for indentation, spaces for alignment.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#hex_literal_case">
<code>hex_literal_case</code></a></td>
<td>rustfmt.toml</td>
<td>Format hexadecimal integer literals.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#hide_parse_errors">
<code>hide_parse_errors</code></a></td>
<td>rustfmt.toml</td>
<td>Hide errors from the parser.</td>
<td>not planned</td>
</tr>
<tr>
<td><a href="config.toml.md#historyenabled">
<code>history.enabled</code></a></td>
<td>config.toml</td>
<td>Record every run that writes files, so <code>mto restore</code> can undo
    it</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="config.toml.md#historykeep_days">
<code>history.keep_days</code></a></td>
<td>config.toml</td>
<td>Days a run is kept (0: no limit)</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="config.toml.md#historykeep_runs">
<code>history.keep_runs</code></a></td>
<td>config.toml</td>
<td>Runs kept (0: no limit)</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="config.toml.md#historymax_size">
<code>history.max_size</code></a></td>
<td>config.toml</td>
<td>The most room history may take (0: no limit)</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#ignore">
<code>ignore</code></a></td>
<td>rustfmt.toml</td>
<td>Skip formatting the specified files and directories.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="mto.toml.md#ignore_dirs">
<code>ignore_dirs</code></a></td>
<td>mto.toml</td>
<td>Folders mto never enters, by name or by path</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#impl_method_order">
<code>impl_method_order</code></a></td>
<td>mto.toml</td>
<td>The order of functions in trait impls</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#import_grouping">
<code>import_grouping</code></a></td>
<td>mto.toml</td>
<td>Group imports into std, external crates and local paths</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#import_sort">
<code>import_sort</code></a></td>
<td>mto.toml</td>
<td>The style edition imports are sorted by</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#imports_granularity">
<code>imports_granularity</code></a></td>
<td>rustfmt.toml</td>
<td>Merge or split imports to the provided granularity.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#imports_indent">
<code>imports_indent</code></a></td>
<td>rustfmt.toml</td>
<td>Indent of imports.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#imports_layout">
<code>imports_layout</code></a></td>
<td>rustfmt.toml</td>
<td>Item layout inside a import block.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#indent_style">
<code>indent_style</code></a></td>
<td>rustfmt.toml</td>
<td>How do we indent expressions or items.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#inline_attribute_width">
<code>inline_attribute_width</code></a></td>
<td>rustfmt.toml</td>
<td>Write an item and its attribute on the same line if their combined width is
    below a threshold.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="mto.toml.md#layout">
<code>layout</code></a></td>
<td>mto.toml</td>
<td>Rearrange the items of each file</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#layout_generated_files">
<code>layout_generated_files</code></a></td>
<td>mto.toml</td>
<td>Lay out generated files too</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#make_backup">
<code>make_backup</code></a></td>
<td>rustfmt.toml</td>
<td>Backup changed files.</td>
<td>not planned</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#match_arm_blocks">
<code>match_arm_blocks</code></a></td>
<td>rustfmt.toml</td>
<td>Wrap the body of arms in blocks when it does not fit on the same line with
    the pattern of arms.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#match_arm_indent">
<code>match_arm_indent</code></a></td>
<td>rustfmt.toml</td>
<td>Determines whether match arms are indented.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#match_arm_leading_pipes">
<code>match_arm_leading_pipes</code></a></td>
<td>rustfmt.toml</td>
<td>Determines whether leading pipes are emitted on match arms.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#match_block_trailing_comma">
<code>match_block_trailing_comma</code></a></td>
<td>rustfmt.toml</td>
<td>Put a trailing comma after a block based match arm (non-block arms are not
    affected)</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#max_width">
<code>max_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum width of each line.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#merge_derives">
<code>merge_derives</code></a></td>
<td>rustfmt.toml</td>
<td>Merge multiple <code>#[derive(...)]</code> into a single one.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#merge_imports">
<code>merge_imports</code></a></td>
<td>rustfmt.toml</td>
<td>(deprecated: use imports_granularity instead)</td>
<td>not planned</td>
</tr>
<tr>
<td><a href="mto.toml.md#name_sort">
<code>name_sort</code></a></td>
<td>mto.toml</td>
<td>How items of one kind are sorted by name</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#newline_style">
<code>newline_style</code></a></td>
<td>rustfmt.toml</td>
<td>Unix or Windows line endings.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#normalize_comments">
<code>normalize_comments</code></a></td>
<td>rustfmt.toml</td>
<td>Convert /* */ comments to // comments where possible.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#normalize_doc_attributes">
<code>normalize_doc_attributes</code></a></td>
<td>rustfmt.toml</td>
<td>Normalize doc attributes as doc comments.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="config.toml.md#outputcolor">
<code>output.color</code></a></td>
<td>config.toml</td>
<td>Whether to color the output</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#overflow_delimited_expr">
<code>overflow_delimited_expr</code></a></td>
<td>rustfmt.toml</td>
<td>Allow trailing bracket/brace delimited expressions to overflow.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#print_misformatted_file_names">
<code>print_misformatted_file_names</code></a></td>
<td>rustfmt.toml</td>
<td>Prints the names of mismatched files that were formatted. Prints the names
    of files that would be formatted when used with <code>--check</code>
    mode.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#remove_nested_parens">
<code>remove_nested_parens</code></a></td>
<td>rustfmt.toml</td>
<td>Remove nested parens.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#reorder_impl_items">
<code>reorder_impl_items</code></a></td>
<td>rustfmt.toml</td>
<td>Reorder impl items.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#reorder_imports">
<code>reorder_imports</code></a></td>
<td>rustfmt.toml</td>
<td>Reorder import and extern crate statements alphabetically.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#reorder_modules">
<code>reorder_modules</code></a></td>
<td>rustfmt.toml</td>
<td>Reorder module statements alphabetically in group.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="config.toml.md#reportsave">
<code>report.save</code></a></td>
<td>config.toml</td>
<td>Keep a report of each bug mto finds in itself, for <code>mto
    report</code></td>
<td>n/a</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#required_version">
<code>required_version</code></a></td>
<td>rustfmt.toml</td>
<td>Require a specific version of the standard formatter.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="config.toml.md#safetyallow_dirty">
<code>safety.allow_dirty</code></a></td>
<td>config.toml</td>
<td>With history off, let mto overwrite uncommitted changes</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="config.toml.md#safetysync">
<code>safety.sync</code></a></td>
<td>config.toml</td>
<td>How much each write makes last at once</td>
<td>n/a</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#short_array_element_width_threshold">
<code>short_array_element_width_threshold</code></a></td>
<td>rustfmt.toml</td>
<td>Width threshold for an array element to be considered short.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#show_parse_errors">
<code>show_parse_errors</code></a></td>
<td>rustfmt.toml</td>
<td>Show errors from the parser (unstable)</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#single_line_if_else_max_width">
<code>single_line_if_else_max_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum line length for single line if-else expressions. A value of zero
    means always break if-else expressions.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#single_line_let_else_max_width">
<code>single_line_let_else_max_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum line length for single line let-else statements. A value of zero
    means always format the divergent <code>else</code> block over multiple
    lines.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#skip_children">
<code>skip_children</code></a></td>
<td>rustfmt.toml</td>
<td>Don't reformat out of line modules.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#skip_macro_invocations">
<code>skip_macro_invocations</code></a></td>
<td>rustfmt.toml</td>
<td>Skip formatting the bodies of macros invoked with the following names.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#sort_alphabetically">
<code>sort_alphabetically</code></a></td>
<td>mto.toml</td>
<td>Sort items of the same kind by name</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#sort_imports">
<code>sort_imports</code></a></td>
<td>mto.toml</td>
<td>Sort imports by path</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#space_after_colon">
<code>space_after_colon</code></a></td>
<td>rustfmt.toml</td>
<td>Leave a space after the colon.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#space_before_colon">
<code>space_before_colon</code></a></td>
<td>rustfmt.toml</td>
<td>Leave a space before the colon.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#spaces_around_ranges">
<code>spaces_around_ranges</code></a></td>
<td>rustfmt.toml</td>
<td>Put spaces around the .. and ..= range operators.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#struct_field_align_threshold">
<code>struct_field_align_threshold</code></a></td>
<td>rustfmt.toml</td>
<td>Align struct fields if their diffs fits within threshold.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#struct_lit_single_line">
<code>struct_lit_single_line</code></a></td>
<td>rustfmt.toml</td>
<td>Put small struct literals on a single line.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#struct_lit_width">
<code>struct_lit_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum width in the body of a struct lit before falling back to vertical
    formatting.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#struct_variant_width">
<code>struct_variant_width</code></a></td>
<td>rustfmt.toml</td>
<td>Maximum width in the body of a struct variant before falling back to
    vertical formatting.</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#style_edition">
<code>style_edition</code></a></td>
<td>rustfmt.toml</td>
<td>The edition of the Style Guide (RFC 3338)</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#tab_spaces">
<code>tab_spaces</code></a></td>
<td>rustfmt.toml</td>
<td>Number of spaces per tab.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="mto.toml.md#test_module">
<code>test_module</code></a></td>
<td>mto.toml</td>
<td>Where <code>#[cfg(test)]</code> modules go</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#trailing_comma">
<code>trailing_comma</code></a></td>
<td>rustfmt.toml</td>
<td>How to handle trailing commas for lists.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#trailing_semicolon">
<code>trailing_semicolon</code></a></td>
<td>rustfmt.toml</td>
<td>Add trailing semicolon after break, continue and return.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="mto.toml.md#trait_impl_order">
<code>trait_impl_order</code></a></td>
<td>mto.toml</td>
<td>Traits whose impls come first under a type</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#trait_impl_sort">
<code>trait_impl_sort</code></a></td>
<td>mto.toml</td>
<td>How trait impls under a type are ordered</td>
<td>supported</td>
</tr>
<tr>
<td><a href="mto.toml.md#type_order">
<code>type_order</code></a></td>
<td>mto.toml</td>
<td>The order of item kinds in a file</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#type_punctuation_density">
<code>type_punctuation_density</code></a></td>
<td>rustfmt.toml</td>
<td>Determines if '+' or '=' are wrapped in spaces in the punctuation of
    types.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#unstable_features">
<code>unstable_features</code></a></td>
<td>rustfmt.toml</td>
<td>Enables unstable features. Only available on nightly channel.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#use_field_init_shorthand">
<code>use_field_init_shorthand</code></a></td>
<td>rustfmt.toml</td>
<td>Use field initialization shorthand if possible.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#use_small_heuristics">
<code>use_small_heuristics</code></a></td>
<td>rustfmt.toml</td>
<td>Whether to use different formatting for items and expressions if they
    satisfy a heuristic notion of 'small'.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#use_try_shorthand">
<code>use_try_shorthand</code></a></td>
<td>rustfmt.toml</td>
<td>Replace uses of the try! macro by the ? shorthand.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#verbose">
<code>verbose</code></a></td>
<td>rustfmt.toml</td>
<td>How much to information to emit to the user.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#version">
<code>version</code></a></td>
<td>rustfmt.toml</td>
<td>Version of formatting rules.</td>
<td>partial</td>
</tr>
<tr>
<td><a href="mto.toml.md#visibility_order">
<code>visibility_order</code></a></td>
<td>mto.toml</td>
<td>The order of visibilities within a kind</td>
<td>supported</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#where_single_line">
<code>where_single_line</code></a></td>
<td>rustfmt.toml</td>
<td>Force where-clauses to be on a single line.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#width_heuristics">
<code>width_heuristics</code></a></td>
<td>rustfmt.toml</td>
<td>'small' heuristic values.</td>
<td>missing</td>
</tr>
<tr>
<td><a href="rustfmt.toml.md#wrap_comments">
<code>wrap_comments</code></a></td>
<td>rustfmt.toml</td>
<td>Break comments to fit on the line.</td>
<td>missing</td>
</tr>
</table>
