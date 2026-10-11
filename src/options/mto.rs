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
        summary: "Folder names mto never enters",
        doc: "Folder names skipped when mto walks a directory. A folder is skipped wherever \
              it appears; setting this list replaces the default list.",
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
              the standard formatter's `reorder_impl_items` is honored as well, as the standard formatter applies it.",
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
        name: "layout",
        file: File::Mto,
        kind: Kind::Bool,
        default: "false",
        default_2024: None,
        summary: "Rearrange the items of each file",
        doc: "Turns layout on for the files this mto.toml applies to. Off by default, so \
              mto changes nothing until a project opts in; `mto init` writes an mto.toml \
              with it on. An mto.toml without it may still override the standard formatter's settings.",
        channels: None,
        hidden: false,
        replaced_by: None,
    },
    Setting {
        name: "format",
        file: File::Mto,
        kind: Kind::Bool,
        default: "false",
        default_2024: None,
        summary: "Format code in the Rust style (preview)",
        doc: "Turns formatting on for the files this mto.toml applies to, beside or without \
              `layout`. A preview: lines are not broken or joined yet. The lines that start \
              items, statements, fields, variants, match arms and list elements are indented \
              one level past the line their block or list belongs to, and closing delimiters \
              at that line's indentation; the lines that continue them move along with them. \
              Within a line, tokens are spaced as the Rust style spaces them (`a + b`, \
              `f(x)`, `a::b`, `x: T`), honoring `spaces_around_ranges`, `space_before_colon`, \
              `space_after_colon` and `type_punctuation_density`; a pair no rule covers keeps \
              its spacing. Trailing whitespace is trimmed after code and line comments, blank lines are \
              kept within `blank_lines_upper_bound` and dropped at the start and end of a \
              container (a module body keeps its first), lines end as `newline_style` says and the file ends \
              with one line break, honoring `tab_spaces` and `hard_tabs`. As the standard \
              formatter does, mto leaves as written macro bodies, anything marked \
              `#[rustfmt::skip]`, `// mto: off` regions, `macro_rules!` definitions, \
              attributes whose arguments are not names, `name = literal` or lists of them, and \
              a statement or item with a line longer than `max_width`. Every change is checked to keep the program's tokens and \
              comments.",
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
              out of the list go after the listed ones. `#[cfg(test)]` modules go where \
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
