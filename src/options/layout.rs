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
  MacroDef,
  ConstDecl,
  StaticDecl,
  TypeAlias,
  TraitDecl,
  EnumDecl,
  StructDecl,
  Union,
  ImplBlock,
  ExternBlock,
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
      "MacroDef" => ItemKind::MacroDef,
      "TypeAlias" => ItemKind::TypeAlias,
      "Union" => ItemKind::Union,
      "ExternBlock" => ItemKind::ExternBlock,
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

/// How items of one kind are sorted by name (`name_sort`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSort {
  /// The Rust style guide's version sorting.
  Version,
  /// Byte by byte.
  Bytes,
  /// Version sorting with case ignored, then with case as a tie-break.
  CaseInsensitive,
}

/// How trait impls under one type are ordered (`trait_impl_sort`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraitImplSort {
  /// By trait name.
  Name,
  /// As written.
  Original,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Visibility {
  Pub,
  PubCrate,
  /// `pub(in path)`, apart from `pub(in crate)` and `pub(in super)`, which the standard
  /// formatter writes as `pub(crate)` and `pub(super)`.
  PubIn,
  PubSuper,
  /// No visibility, `pub(self)` or `pub(in self)`.
  Private,
}

impl Visibility {
  pub fn from_name(name: &str) -> Option<Visibility> {
    Some(match name {
      "Pub" => Visibility::Pub,
      "PubCrate" => Visibility::PubCrate,
      "PubIn" => Visibility::PubIn,
      "PubSuper" => Visibility::PubSuper,
      "Private" => Visibility::Private,
      _ => return None,
    })
  }
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
  /// Blank lines between consecutive imports of a group, and between `mod name;`
  /// declarations: the standard formatter's `blank_lines_lower_bound`.
  pub blank_lines_tight: usize,
  pub ignore_dirs: Vec<String>,
  /// The style edition (2015, 2018, 2021, 2024, ...): decides how imports and modules are
  /// sorted, as the standard formatter sorts them.
  pub style_edition: u16,
  /// The Rust edition: from 2018, a leading `::` in an import names the root.
  pub edition: u16,
  /// `import_sort`: the style edition imports are sorted by, when not the file's.
  pub import_sort: Option<u16>,
  pub name_sort: NameSort,
  pub trait_impl_sort: TraitImplSort,
  /// `test_module = "in_place"`: `#[cfg(test)]` modules keep their place instead of going
  /// last.
  pub test_module_in_place: bool,
  /// The standard formatter's `reorder_impl_items`.
  pub reorder_impl_items: bool,
  /// `impl_method_order = "trait"`: a trait impl's functions follow the trait's order.
  pub impl_method_order_trait: bool,
  /// Traits whose impls come first, in this order (`trait_impl_order`).
  pub trait_impl_order: Vec<String>,
  pub visibility_order: Vec<Visibility>,
}

impl LayoutConfig {
  /// Whether imports sort by version sort (style edition 2024 and later).
  pub fn version_sorted_imports(&self) -> bool {
    self.import_sort.unwrap_or(self.style_edition) >= 2024
  }
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
        ItemKind::MacroDef,
        ItemKind::ConstDecl,
        ItemKind::StaticDecl,
        ItemKind::TypeAlias,
        ItemKind::TraitDecl,
        ItemKind::EnumDecl,
        ItemKind::StructDecl,
        ItemKind::Union,
        ItemKind::ImplBlock,
        ItemKind::ExternBlock,
        ItemKind::FnDecl,
        ItemKind::Unknown,
      ],
      group_impls: true,
      sort_imports: true,
      import_grouping: true,
      blank_lines_between_blocks: 1,
      blank_lines_tight: 0,
      ignore_dirs: vec![
        ".git".to_string(),
        "target".to_string(),
        "node_modules".to_string(),
        "vendor".to_string(),
        "dist".to_string(),
        ".venv".to_string(),
      ],
      style_edition: 2015,
      edition: 2015,
      import_sort: None,
      name_sort: NameSort::Version,
      trait_impl_sort: TraitImplSort::Name,
      test_module_in_place: false,
      reorder_impl_items: false,
      impl_method_order_trait: false,
      trait_impl_order: Vec::new(),
      visibility_order: vec![
        Visibility::Pub,
        Visibility::PubCrate,
        Visibility::PubIn,
        Visibility::PubSuper,
        Visibility::Private,
      ],
    }
  }
}
