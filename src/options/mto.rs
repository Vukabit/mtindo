//! mto's own settings, which only `mto.toml` and `.mto.toml` may hold. Their defaults must match
//! [`LayoutConfig::default`](super::layout::LayoutConfig); a test checks it.

use super::{Choice, File, Kind, Setting};

/// The item categories `type_order` arranges, in their default order.
pub(super) static ITEM_KINDS: &[Choice] = &[
  choice(
    "ModDecl",
    "Module declarations: `mod name;` and `mod name { ... }`.",
  ),
  choice("UseDecl", "Imports: `use` declarations."),
  choice("MacroDecl", "`macro_rules!` definitions."),
  choice("MacroDef", "`macro` definitions (declarative macros 2.0)."),
  choice("ConstDecl", "Constants: `const`."),
  choice("StaticDecl", "Statics: `static`."),
  choice("TypeAlias", "Type aliases: `type Name = ...;`."),
  choice("TraitDecl", "Traits, including trait aliases."),
  choice("EnumDecl", "Enums."),
  choice("StructDecl", "Structs."),
  choice("Union", "Unions."),
  choice("ImplBlock", "Impl blocks."),
  choice("ExternBlock", "`extern` blocks: `extern \"C\" { ... }`."),
  choice("FnDecl", "Functions."),
  choice("Unknown", "Items of no other kind, such as `reuse`."),
];

/// Values `type_order` refuses with a reason: kinds that never move.
pub(super) static PINNED_KINDS: &[&str] = &["ExternCrate", "MacroCall"];

pub(super) static SETTINGS: &[Setting] = &[
  Setting {
    name: "blank_lines_between_blocks",
    file: File::Mto,
    kind: Kind::Integer,
    default: "1",
    default_2024: None,
    summary: "Blank lines between items",
    doc: "The number of blank lines placed between items. Imports of the same group and \
              `mod name;` declarations stay on consecutive lines. It is kept within the standard formatter's \
              `blank_lines_lower_bound` and `blank_lines_upper_bound` (0 and 1 by default), \
              with a warning, so that the standard formatter leaves the result alone; imports and \
              declarations get the lower bound.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "directive_reasons",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice(
        "optional",
        "A directive may give its reason after ` -- `, or not.",
      ),
      choice(
        "required",
        "A `// mto: pin`, `// mto: hold` or `// mto: off` without ` -- why` gets a \
         warning, and with --strict fails its file.",
      ),
    ]),
    default: "\"optional\"",
    default_2024: None,
    summary: "Whether a directive must say why",
    doc: "With `required`, every `// mto: pin`, `// mto: hold` and `// mto: off` must give \
              its reason after ` -- ` (`// mto: off -- aligned by hand`), so a project's \
              exceptions do not pile up unexplained; `mto pins` lists them all.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "reexports",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice(
        "with_imports",
        "Among the other imports, as the standard formatter sorts them.",
      ),
      choice(
        "apart",
        "A group of their own after the other imports, a blank line between.",
      ),
      choice("last", "After the other items of their section."),
    ]),
    default: "\"with_imports\"",
    default_2024: None,
    summary: "Where pub use re-exports go",
    doc: "A re-export is a `use` with any visibility but private: the module's API, apart \
              from what it uses. With `last` they close their section, as a module's \
              list of what it offers; with `order_keys = []` they keep their place.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "cfg_groups",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice("off", "Each item moves on its own."),
      choice(
        "together",
        "Items under the same `#[cfg(...)]` written with no blank line between them move \
         as one, kept as written.",
      ),
      choice(
        "gather",
        "Every item under a `#[cfg(...)]` follows the first of them in the order, \
         within its section.",
      ),
    ]),
    default: "\"off\"",
    default_2024: None,
    summary: "Items under the same #[cfg(...)] moving as one",
    doc: "Keeps code for one platform or feature in one place. Predicates are the same \
              when their text is, spacing aside. A group written together is ordered as \
              its first item is; a gathered one goes where its first item in the order \
              does, never past a held item, a section's start or a macro definition one \
              of them uses. Imports, `mod name;` declarations, `#[cfg(test)]` modules \
              and macro definitions are never grouped.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "flow",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice("off", "Items are ordered by the sort keys alone."),
      choice(
        "top_down",
        "An item goes above the items it uses: read from the top, each item comes \
         before what it is built from.",
      ),
      choice(
        "bottom_up",
        "An item goes below the items it uses: each item comes after what it is built \
         from.",
      ),
    ]),
    default: "\"off\"",
    default_2024: None,
    summary: "Items above or below the items they use",
    doc: "Within one kind (functions among functions, types among types; among all items \
              but imports, modules and macros when `kind` is not in `order_keys`), puts \
              an item above or below the items it uses, keeping the sort keys' order \
              wherever no use decides. Uses are found by name, without type checking. \
              Items that use each other in a circle keep the sort keys' order among \
              themselves; a type moves with its impls, and `#[cfg]` variants together.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "generated_markers",
    file: File::Mto,
    kind: Kind::List,
    default: "[\"@generated\"]",
    default_2024: None,
    summary: "Texts that mark a file as generated",
    doc: "A file is generated when a comment among its first \
              `generated_marker_line_search_limit` lines holds one of these texts. Layout leaves \
              a generated file's items where they are unless `layout_generated_files` is set, \
              and formatting leaves it as written when `format_generated_files = false`. A \
              folder of generated code is better left out with `ignore_dirs`.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "group_impls",
    file: File::Mto,
    kind: Kind::Bool,
    default: "true",
    default_2024: None,
    summary: "Place each impl block right after the type it is for",
    doc: "Rank each impl block as the struct, enum or trait it is for and place it right \
              after that item, instead of in the `ImplBlock` group of `type_order`.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "ignore_dirs",
    file: File::Mto,
    kind: Kind::List,
    default: "[\".git\", \"target\", \"node_modules\", \"vendor\", \"dist\", \".venv\"]",
    default_2024: None,
    summary: "Folders mto never enters, by name or by path",
    doc: "Folders skipped when mto walks a directory. A name (`target`) skips every \
              folder of that name, wherever it is; an entry with a `/` is a path \
              (`tests/fixtures`, `./cache`, or an absolute one), read from the folder of the \
              file that sets it, and skips that folder only. Setting this list replaces the \
              default list.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "impl_method_order",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice(
        "written",
        "Functions in impl blocks keep the order they were written in.",
      ),
      choice(
        "trait",
        "In an impl of a trait declared in the same file, the functions follow the \
                 order of the trait's declaration; functions the trait does not declare \
                 come after them.",
      ),
    ]),
    default: "\"written\"",
    default_2024: None,
    summary: "The order of functions in trait impls",
    doc: "The order of the functions inside an impl of a trait declared in the same \
              file. Only the functions trade places; other items and blank lines stay where \
              they are. A trait declared under the same name twice in the file is skipped. \
              The standard formatter's `reorder_impl_items` is honored as well, as the standard formatter applies it.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "import_grouping",
    file: File::Mto,
    kind: Kind::Bool,
    default: "true",
    default_2024: None,
    summary: "Group imports into std, external crates and local paths",
    doc: "Separate `use` declarations into three groups, in this order, with a blank line \
              between groups: the standard library (`std`, `core`, `alloc`), external \
              crates, and local paths (`crate`, `self`, `super`). Only applies when \
              `sort_imports` is on.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "import_sort",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice(
        "style_edition",
        "Sort imports as the standard formatter does for the style edition in effect.",
      ),
      choice(
        "2015",
        "the standard formatter's order before style edition 2024: snake_case, then CamelCase, then \
                 UPPER_SNAKE_CASE, each by bytes, so `a10` comes before `a2`.",
      ),
      choice(
        "2024",
        "Version sorting, as from style edition 2024: numbers by value, so `u8` \
                 comes before `u16`, and uppercase before lowercase.",
      ),
    ]),
    default: "\"style_edition\"",
    default_2024: None,
    summary: "The style edition imports are sorted by",
    doc: "Imports are sorted exactly as the standard formatter sorts them, so the standard formatter leaves them alone. \
              This picks which of the standard formatter's two orders to use when it should differ from the \
              style edition in effect.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "inherent_method_order",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice(
        "written",
        "Functions in impl blocks with no trait keep the order they were written in.",
      ),
      choice(
        "constructors_first",
        "Functions with no receiver that return `Self` or the type, the ways to make \
         one, come first, then the rest; each part as written.",
      ),
      choice(
        "by_receiver",
        "Functions with no receiver first, then `&self`, then `&mut self`, then \
         `self`; each part as written.",
      ),
    ]),
    default: "\"written\"",
    default_2024: None,
    summary: "The order of functions in impls with no trait",
    doc: "The order of the functions inside an impl with no trait (`impl Type`). Only the \
              functions trade places; other items and blank lines stay where they are. \
              Trait impls are ordered by `impl_method_order`.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "keep_helpers_near",
    file: File::Mto,
    kind: Kind::Bool,
    default: "false",
    default_2024: None,
    summary: "Keep a private item used by one sibling directly next to it",
    doc: "A private function, constant, static, type alias, struct, enum or union that \
              exactly one other item in its file or module uses stays directly next to that \
              item and moves with it: above it, or below it when written below it. A type \
              takes its own impls along, and a helper of a helper stays next to the item it \
              helps. Uses are found by name, without type checking, so an item whose name \
              is ambiguous (defined twice) sorts as usual, as does one whose user is in \
              another section or keeps its place.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "keep_paragraphs",
    file: File::Mto,
    kind: Kind::Bool,
    default: "false",
    default_2024: None,
    summary: "Keep items written with no blank line between them together",
    doc: "Items written with no blank line between them form a paragraph that moves as \
              one: it is placed by its first item, never sorted or split inside, and keeps \
              its tight spacing. Imports, `mod name;` declarations, `#[cfg(test)]` modules, \
              macro definitions and pinned items never join a paragraph.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "layout",
    file: File::Mto,
    kind: Kind::Bool,
    default: "false",
    default_2024: None,
    summary: "Rearrange the items of each file",
    doc: "Turns layout on for the files this mto.toml applies to. Off by default, so \
              mto moves no items until a project opts in; `mto init` writes an mto.toml \
              with it on. An mto.toml without it may still set formatting and override the \
              standard formatter's settings.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "format",
    file: File::Mto,
    kind: Kind::Bool,
    default: "true",
    default_2024: None,
    summary: "Format code in the Rust style",
    doc: "Formats the files this mto.toml applies to in the Rust style, as the standard \
              formatter does with the same settings, beside or without `layout`. On by \
              default, so mto with no configuration is a drop-in for the standard \
              formatter; `format = false` turns it off. Formatting changes whitespace and \
              the few tokens the Rust style rewrites (a trailing comma, the order of \
              imports). Like the standard formatter, it leaves as written what it would \
              give up on: macro bodies it cannot read as code, `macro_rules!` definitions, \
              anything marked `#[rustfmt::skip]`, `// mto: off` regions, a statement or item \
              that cannot fit within `max_width`, and files that `disable_all_formatting` or \
              `format_generated_files = false` leave alone. Every change is checked to keep \
              the program's tokens and comments, and a second run changes nothing.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "layout_generated_files",
    file: File::Mto,
    kind: Kind::Bool,
    default: "false",
    default_2024: None,
    summary: "Lay out generated files too",
    doc: "Whether layout reorders the items of a generated file (one holding a \
              `generated_markers` text near its top). Off by default, so generated code keeps \
              the order its generator wrote; formatting follows `format_generated_files` \
              instead.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "mto_version",
    file: File::Mto,
    kind: Kind::Text,
    default: "\"*\"",
    default_2024: None,
    summary: "The mto versions this project's files are laid out and formatted with",
    doc: "A version requirement for mto itself, as Cargo writes them (`0.1`, `>=0.1.2`, \
              `=0.1.0`): any other mto refuses to run here, with exit code 2, so two people \
              on different builds never rewrite each other's files. `required_version` pins \
              the standard formatter's version mto reproduces in the same way.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "name_sort",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice(
        "version",
        "The Rust style guide's version sorting: numbers by value (`u8` before \
                 `u16`), underscores first, uppercase before lowercase.",
      ),
      choice("bytes", "Byte by byte, so `a10` comes before `a2`."),
      choice(
        "case_insensitive",
        "Version sorting with case ignored (`alpha`, `Beta`, `gamma`), case \
                 deciding only between otherwise equal names.",
      ),
    ]),
    default: "\"version\"",
    default_2024: None,
    summary: "How items of one kind are sorted by name",
    doc: "How `sort_alphabetically` compares names. Imports and modules are not \
              affected: they are always sorted exactly as the standard formatter sorts them.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "order_keys",
    file: File::Mto,
    kind: Kind::Sequence(&[
      choice(
        "kind",
        "The order of kinds, `type_order`; an impl beside its type with `group_impls`.",
      ),
      choice(
        "visibility",
        "The order of visibilities, `visibility_order`.",
      ),
      choice(
        "name",
        "Names, by `name_sort`, when `sort_alphabetically` is on.",
      ),
      choice("entry", "`fn main` before the other items."),
      choice(
        "deprecated",
        "`#[deprecated]` items after the others; an impl as its type, with `group_impls`.",
      ),
      choice(
        "hidden",
        "`#[doc(hidden)]` items after the others; an impl as its type, with `group_impls`.",
      ),
    ]),
    default: "[\"kind\", \"visibility\", \"name\"]",
    default_2024: None,
    summary: "What items are ordered by, in turn; none keeps the written order",
    doc: "What items are ordered by, the first deciding and the next breaking its ties. \
              `[\"visibility\", \"kind\", \"name\"]` puts the public items first; \
              `[\"kind\", \"deprecated\", \"visibility\", \"name\"]` sinks deprecated \
              items to the bottom of their kind. Unless \
              `kind` comes first, imports, `mod` declarations and macros still come \
              first, each kind with its own rules, and an impl goes with its type. `[]` keeps the written \
              order: only runs of imports and of `mod` declarations are sorted where \
              they stand, `#[cfg(test)]` modules go last, and the settings that keep \
              structure (`sections`, `keep_paragraphs`, `keep_helpers_near`) still apply.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "sections",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice(
        "off",
        "No sections: a loose comment moves with the item below it.",
      ),
      choice(
        "comments",
        "Every loose comment (a blank line after it) starts a section.",
      ),
      choice(
        "headings",
        "Heading-like loose comments start a section: one or two lines, not code and \
         not a TODO, FIXME, SAFETY or NOTE. Other loose comments keep their place.",
      ),
      choice(
        "markers",
        "Only `// mto: section` comments start a section.",
      ),
    ]),
    default: "\"off\"",
    default_2024: None,
    summary: "Comments that start a section items never leave",
    doc: "Cuts each file, module and impl body into sections at the comments this names: \
              items are ordered within their section and never cross into another, and \
              the comment stays where it was written, a blank line on each side. \
              `#[cfg(test)]` modules go last in their section.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "sort_alphabetically",
    file: File::Mto,
    kind: Kind::Bool,
    default: "true",
    default_2024: None,
    summary: "Sort items of the same kind by name",
    doc: "Within each kind of `type_order`, items are ordered by `visibility_order`, \
              then by name (see `name_sort`) when this is on. When off, items of the same \
              kind and visibility keep their original order.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "sort_imports",
    file: File::Mto,
    kind: Kind::Bool,
    default: "true",
    default_2024: None,
    summary: "Sort imports by path",
    doc: "Sort `use` declarations: by group when `import_grouping` is on, then `self`, \
              `super` and `crate` paths ahead of the rest, then by path. When off, imports \
              keep the order they were written in; pair it with the standard formatter's \
              `reorder_imports = false`, or the standard formatter sorts them.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "test_module",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice("last", "At the end of the file."),
      choice(
        "in_place",
        "Where it was written: it keeps its place, like a pinned item.",
      ),
    ]),
    default: "\"last\"",
    default_2024: None,
    summary: "Where `#[cfg(test)]` modules go",
    doc: "Where `#[cfg(test)]` modules go: last in the file, or kept where they are.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "test_helpers",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice("in_place", "Sorted with the other items, as any item is."),
      choice(
        "last",
        "At the end of their section, just before the `#[cfg(test)]` modules.",
      ),
    ]),
    default: "\"in_place\"",
    default_2024: None,
    summary: "Where #[cfg(test)] items outside a test module go",
    doc: "Items under `#[cfg(test)]` that are not a test module, such as test-only \
              helpers and impls, go with the tests at the end of their section with \
              `last`. Imports, `mod` declarations and macro definitions stay where \
              their kind puts them.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "trait_impl_order",
    file: File::Mto,
    kind: Kind::List,
    default: "[]",
    default_2024: None,
    summary: "Traits whose impls come first under a type",
    doc: "Trait impls for these traits come right after the type's own impls, in this \
              order, for example `[\"Default\", \"Clone\", \"Debug\", \"Display\"]`. A trait \
              is matched by its name without its path, so `fmt::Display` matches \
              `Display`. Other trait impls follow, as `trait_impl_sort` says.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "trait_impl_sort",
    file: File::Mto,
    kind: Kind::Choice(&[
      choice("name", "By trait name, in `name_sort` order."),
      choice("original", "As written."),
    ]),
    default: "\"name\"",
    default_2024: None,
    summary: "How trait impls under a type are ordered",
    doc: "The impls of one type go in this order: the type's own impls, as written; the \
              trait impls listed in `trait_impl_order`; then the other trait impls, by trait \
              name or as written.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "type_order",
    file: File::Mto,
    kind: Kind::Sequence(ITEM_KINDS),
    default: "[\"ModDecl\", \"UseDecl\", \"MacroDecl\", \"MacroDef\", \"ConstDecl\", \"StaticDecl\", \"TypeAlias\", \"TraitDecl\", \"EnumDecl\", \"StructDecl\", \"Union\", \"ImplBlock\", \"ExternBlock\", \"FnDecl\", \"Unknown\"]",
    default_2024: None,
    summary: "The order of item kinds in a file",
    doc: "The order in which kinds of items appear in a file, top to bottom. Kinds left \
              out of the list go after the listed ones, in their default order. `#[cfg(test)]` modules go where \
              `test_module` says. `extern crate` items and item macro calls (such as `include!`) are not \
              kinds here: they never move, because moving them can change what the code \
              means.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "visibility_order",
    file: File::Mto,
    kind: Kind::Sequence(VISIBILITIES),
    default: "[\"Pub\", \"PubCrate\", \"PubIn\", \"PubSuper\", \"Private\"]",
    default_2024: None,
    summary: "The order of visibilities within a kind",
    doc: "Within each kind of `type_order`, items are ordered by visibility in this \
              order before they are sorted by name. Visibilities left out go last. Modules \
              ignore it, as the standard formatter does.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
];

/// The visibilities `visibility_order` arranges, widest first as by default.
pub(super) static VISIBILITIES: &[Choice] = &[
  choice("Pub", "`pub`."),
  choice(
    "PubCrate",
    "`pub(crate)`, and `pub(in crate)`, which the standard formatter writes so.",
  ),
  choice("PubIn", "`pub(in path)` for any other path."),
  choice("PubSuper", "`pub(super)`, and `pub(in super)`."),
  choice("Private", "No visibility, `pub(self)` or `pub(in self)`."),
];

const fn choice(name: &'static str, doc: &'static str) -> Choice {
  Choice {
    name,
    doc,
    channels: None,
  }
}
