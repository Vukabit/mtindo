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
    choice("ConstDecl", "Constants: `const`."),
    choice("StaticDecl", "Statics: `static`."),
    choice("TraitDecl", "Traits, including trait aliases."),
    choice("EnumDecl", "Enums."),
    choice("StructDecl", "Structs."),
    choice("ImplBlock", "Impl blocks."),
    choice("FnDecl", "Functions."),
    choice(
        "Unknown",
        "Every other item: type aliases, unions, `extern` blocks, `macro` definitions, \
         `extern crate` and macro calls (the last two never move).",
    ),
];

pub(super) static SETTINGS: &[Setting] = &[
    Setting {
        name: "blank_lines_between_blocks",
        file: File::Mto,
        kind: Kind::Integer,
        default: "1",
        default_2024: None,
        summary: "Blank lines between groups of items of different kinds",
        doc: "The number of blank lines placed between items. Imports of the same group and \
              `mod name;` declarations stay on consecutive lines.",
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
        name: "sort_alphabetically",
        file: File::Mto,
        kind: Kind::Bool,
        default: "true",
        default_2024: None,
        summary: "Sort items of the same kind by name",
        doc: "Within each kind of `type_order`, items are ordered by visibility (`pub`, \
              `pub(super)`, `pub(crate)`, private), then by name when this is on. When off, \
              items of the same kind and visibility keep their original order.",
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
              `super` and `crate` paths ahead of the rest, then by path.",
        channels: None,
        hidden: false,
        replaced_by: None,
    },
    Setting {
        name: "type_order",
        file: File::Mto,
        kind: Kind::Sequence(ITEM_KINDS),
        default: "[\"ModDecl\", \"UseDecl\", \"MacroDecl\", \"ConstDecl\", \"StaticDecl\", \
                  \"TraitDecl\", \"EnumDecl\", \"StructDecl\", \"ImplBlock\", \"FnDecl\", \
                  \"Unknown\"]",
        default_2024: None,
        summary: "The order of item kinds in a file",
        doc: "The order in which kinds of items appear in a file, top to bottom. Kinds left \
              out of the list go after the listed ones. `#[cfg(test)]` modules always go \
              last.",
        channels: None,
        hidden: false,
        replaced_by: None,
    },
];

const fn choice(name: &'static str, doc: &'static str) -> Choice {
    Choice {
        name,
        doc,
        channels: None,
    }
}
