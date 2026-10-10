//! Layout settings and the item categories they refer to: plain data, no parsing or I/O.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportGroup {
    Std = 0,
    External = 1,
    Local = 2,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ItemKind {
    ModDecl,
    UseDecl,
    MacroDecl,
    ConstDecl,
    StaticDecl,
    TraitDecl,
    EnumDecl,
    StructDecl,
    ImplBlock,
    FnDecl,
    Unknown,
}

impl ItemKind {
    pub fn from_name(s: &str) -> Self {
        let cleaned = s.trim().trim_matches(|c| c == '"' || c == '\'');
        match cleaned {
            "ModDecl" => ItemKind::ModDecl,
            "UseDecl" => ItemKind::UseDecl,
            "MacroDecl" => ItemKind::MacroDecl,
            "ConstDecl" => ItemKind::ConstDecl,
            "StaticDecl" => ItemKind::StaticDecl,
            "TraitDecl" => ItemKind::TraitDecl,
            "EnumDecl" => ItemKind::EnumDecl,
            "StructDecl" => ItemKind::StructDecl,
            "ImplBlock" => ItemKind::ImplBlock,
            "FnDecl" => ItemKind::FnDecl,
            _ => ItemKind::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Visibility {
    Pub,
    PubSuper,
    PubCrate,
    Private,
}

/// How items are arranged in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutConfig {
    /// Reserved for the formatter. Has no effect on layout.
    pub max_width: usize,
    /// Reserved for the formatter. Has no effect on layout.
    pub tab_width: usize,
    pub sort_alphabetically: bool,
    pub type_order: Vec<ItemKind>,
    pub group_impls: bool,
    pub sort_imports: bool,
    pub import_grouping: bool,
    pub blank_lines_between_blocks: usize,
    pub ignore_dirs: Vec<String>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        LayoutConfig {
            max_width: 100,
            tab_width: 4,
            sort_alphabetically: true,
            type_order: vec![
                ItemKind::ModDecl,
                ItemKind::UseDecl,
                ItemKind::MacroDecl,
                ItemKind::ConstDecl,
                ItemKind::StaticDecl,
                ItemKind::TraitDecl,
                ItemKind::EnumDecl,
                ItemKind::StructDecl,
                ItemKind::ImplBlock,
                ItemKind::FnDecl,
                ItemKind::Unknown,
            ],
            group_impls: true,
            sort_imports: true,
            import_grouping: true,
            blank_lines_between_blocks: 1,
            ignore_dirs: vec![
                ".git".to_string(),
                "target".to_string(),
                "node_modules".to_string(),
                "vendor".to_string(),
                "dist".to_string(),
                ".venv".to_string(),
            ],
        }
    }
}
