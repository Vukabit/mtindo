//! Typed views over the syntax tree, for the parts stage 2's layout needs: a file's items,
//! their kind, name, visibility, attributes and doc comments, an impl's self type and trait,
//! and the root of a `use` path.
//!
//! A view wraps a [`SyntaxNode`] and reads it on demand; it never copies or changes the
//! tree. Views are lenient: a broken file still gives views, and a part that is missing
//! (an impl with no self type yet) comes back as `None`.

use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemKind {
    Const,
    Enum,
    ExternBlock,
    ExternCrate,
    Fn,
    Impl,
    MacroCall,
    MacroDef,
    MacroRules,
    Module,
    Reuse,
    Static,
    Struct,
    Trait,
    TypeAlias,
    Union,
    Use,
}

/// Who can see an item. `In` holds the path of `pub(in path)` as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Visibility {
    Private,
    Pub,
    PubCrate,
    PubSelf,
    PubSuper,
    PubIn(String),
}

/// An attribute, `#[path ...]` or `#![path ...]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr(SyntaxNode);

impl Attr {
    pub fn cast(node: SyntaxNode) -> Option<Attr> {
        (node.kind() == SyntaxKind::Attr).then_some(Attr(node))
    }

    /// `#![...]` rather than `#[...]`.
    pub fn is_inner(&self) -> bool {
        self.0
            .children_with_tokens()
            .iter()
            .filter(|e| !e.kind().is_trivia())
            .nth(1)
            .is_some_and(|e| e.kind() == SyntaxKind::Bang)
    }

    /// Whether it tells rustfmt to leave its item (or, inner, its file) alone, in any of the
    /// spellings rustfmt honors: `rustfmt::skip`, the older `rustfmt_skip`, and either inside
    /// `cfg_attr(rustfmt, ...)`.
    pub fn is_rustfmt_skip(&self) -> bool {
        match self.path().as_str() {
            "rustfmt::skip" | "rustfmt_skip" => true,
            "cfg_attr" => {
                let text: String = significant_text(&self.0);
                let inner = text
                    .split_once("cfg_attr(")
                    .map_or("", |(_, rest)| rest.trim_end_matches([']', ')']));
                let mut parts = inner.split(',');
                parts.next() == Some("rustfmt")
                    && parts.any(|p| p == "rustfmt::skip" || p == "rustfmt_skip")
            }
            _ => false,
        }
    }

    /// The attribute's path without spaces, such as `derive` or `cfg_attr`.
    pub fn path(&self) -> String {
        let meta = child(&self.0, SyntaxKind::Meta);
        meta.and_then(|m| child(&m, SyntaxKind::Path))
            .map(|path| significant_text(&path))
            .unwrap_or_default()
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// An `impl` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Impl(SyntaxNode);

impl Impl {
    pub fn cast(node: SyntaxNode) -> Option<Impl> {
        (node.kind() == SyntaxKind::Impl).then_some(Impl(node))
    }

    /// `impl !Trait for T`.
    pub fn is_negative(&self) -> bool {
        tokens(&self.0).iter().any(|t| t.kind() == SyntaxKind::Bang)
    }

    /// The type the impl is for: `T` in `impl T` and in `impl Trait for T`.
    pub fn self_ty(&self) -> Option<SyntaxNode> {
        let (before, after) = self.types_around_for();
        if self.has_for() { after } else { before }
    }

    /// The last path segment of the self type, such as `S` for `impl Trait for crate::a::S<T>`
    /// or `&S`; what layout groups impls by.
    pub fn self_ty_name(&self) -> Option<String> {
        self.self_ty().and_then(|ty| type_name(&ty))
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The last path segment of the trait, such as `Display` for `impl fmt::Display for S`.
    pub fn trait_name(&self) -> Option<String> {
        self.trait_ref().and_then(|ty| type_name(&ty))
    }

    /// The trait in `impl Trait for T`; `None` for an inherent impl.
    pub fn trait_ref(&self) -> Option<SyntaxNode> {
        if self.has_for() {
            self.types_around_for().0
        } else {
            None
        }
    }

    fn has_for(&self) -> bool {
        tokens(&self.0)
            .iter()
            .any(|t| t.kind() == SyntaxKind::ForKw)
    }

    /// The first type before `for` and the first type after it.
    fn types_around_for(&self) -> (Option<SyntaxNode>, Option<SyntaxNode>) {
        let mut before = None;
        let mut after = None;
        let mut seen_for = false;
        for element in self.0.children_with_tokens() {
            match element {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::ForKw => seen_for = true,
                SyntaxElement::Node(node) if is_type(node.kind()) => {
                    if seen_for && after.is_none() {
                        after = Some(node);
                    } else if !seen_for && before.is_none() {
                        before = Some(node);
                    }
                }
                _ => {}
            }
        }
        (before, after)
    }
}

/// An item: anything that can stand at the top of a file or module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item(SyntaxNode);

impl Item {
    /// The outer attributes, in order; doc comments are not included (see [`Item::docs`]).
    pub fn attrs(&self) -> Vec<Attr> {
        self.0
            .children()
            .into_iter()
            .filter_map(Attr::cast)
            .filter(|attr| !attr.is_inner())
            .collect()
    }

    pub fn cast(node: SyntaxNode) -> Option<Item> {
        item_kind(node.kind()).map(|_| Item(node))
    }

    /// The doc comments on the item (`///` and `/** */`), as written, in order.
    pub fn docs(&self) -> Vec<String> {
        tokens(&self.0)
            .iter()
            .filter(|t| {
                matches!(
                    t.kind(),
                    SyntaxKind::OuterLineDoc | SyntaxKind::OuterBlockDoc
                )
            })
            .map(|t| t.text().to_string())
            .collect()
    }

    pub fn kind(&self) -> ItemKind {
        item_kind(self.0.kind()).unwrap_or(ItemKind::MacroCall)
    }

    /// An `impl` view, if this item is one.
    pub fn as_impl(&self) -> Option<Impl> {
        Impl::cast(self.0.clone())
    }

    /// A `use` view, if this item is one.
    pub fn as_use(&self) -> Option<Use> {
        Use::cast(self.0.clone())
    }

    /// The name the item declares: `f` in `fn f`, `m` in `macro_rules! m`, the alias in
    /// `extern crate a as b` (else the crate). Items that declare no name (impls, uses,
    /// extern blocks, macro calls, `const _`) give `None`.
    pub fn name(&self) -> Option<String> {
        if self.kind() == ItemKind::ExternCrate {
            return child(&self.0, SyntaxKind::Rename)
                .and_then(|rename| child(&rename, SyntaxKind::Name))
                .or_else(|| child(&self.0, SyntaxKind::NameRef))
                .map(|node| significant_text(&node));
        }
        child(&self.0, SyntaxKind::Name)
            .map(|node| significant_text(&node))
            .filter(|name| name != "_")
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    pub fn visibility(&self) -> Visibility {
        let Some(node) = child(&self.0, SyntaxKind::Visibility) else {
            return Visibility::Private;
        };
        let words: Vec<SyntaxToken> = node
            .descendants_with_tokens()
            .into_iter()
            .filter_map(|e| match e {
                SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t),
                _ => None,
            })
            .collect();
        match words.get(2).map(|t| t.kind()) {
            None => Visibility::Pub,
            Some(SyntaxKind::CrateKw) => Visibility::PubCrate,
            Some(SyntaxKind::SelfKw) => Visibility::PubSelf,
            Some(SyntaxKind::SuperKw) => Visibility::PubSuper,
            Some(SyntaxKind::InKw) => Visibility::PubIn(
                child(&node, SyntaxKind::Path)
                    .map(|path| significant_text(&path))
                    .unwrap_or_default(),
            ),
            Some(_) => Visibility::Pub,
        }
    }
}

/// A whole file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile(SyntaxNode);

impl SourceFile {
    pub fn cast(node: SyntaxNode) -> Option<SourceFile> {
        (node.kind() == SyntaxKind::SourceFile).then_some(SourceFile(node))
    }

    /// The inner attributes at the top of the file (`#![...]`).
    pub fn attrs(&self) -> Vec<Attr> {
        self.0
            .children()
            .into_iter()
            .filter_map(Attr::cast)
            .filter(Attr::is_inner)
            .collect()
    }

    /// The top-level items, in order. Nodes that are not items, such as an `ERROR` node
    /// around damaged text, are left out.
    pub fn items(&self) -> Vec<Item> {
        self.0
            .children()
            .into_iter()
            .filter_map(Item::cast)
            .collect()
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// A `use` item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use(SyntaxNode);

impl Use {
    pub fn cast(node: SyntaxNode) -> Option<Use> {
        (node.kind() == SyntaxKind::Use).then_some(Use(node))
    }

    /// The first segment of the imported path: `std` for `use std::io`, `crate` for
    /// `use crate::a`, and `std` for `use ::std::io`. `None` for `use {a, b}` and `use *`.
    pub fn root(&self) -> Option<String> {
        let tree = self.tree()?;
        let path = child(&tree, SyntaxKind::Path)?;
        let mut first = path;
        while let Some(inner) = child(&first, SyntaxKind::Path) {
            first = inner;
        }
        child(&first, SyntaxKind::PathSegment).map(|segment| significant_text(&segment))
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The use tree after `use`, such as `std::{io, fs}`.
    pub fn tree(&self) -> Option<SyntaxNode> {
        child(&self.0, SyntaxKind::UseTree)
    }
}

fn child(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    node.children().into_iter().find(|c| c.kind() == kind)
}

fn is_type(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::PathType
            | SyntaxKind::RefType
            | SyntaxKind::PtrType
            | SyntaxKind::SliceType
            | SyntaxKind::ArrayType
            | SyntaxKind::TupleType
            | SyntaxKind::ParenType
            | SyntaxKind::NeverType
            | SyntaxKind::InferType
            | SyntaxKind::FnPtrType
            | SyntaxKind::DynTraitType
            | SyntaxKind::ImplTraitType
            | SyntaxKind::MacroType
            | SyntaxKind::UnsafeBinderType
    )
}

fn item_kind(kind: SyntaxKind) -> Option<ItemKind> {
    Some(match kind {
        SyntaxKind::Const => ItemKind::Const,
        SyntaxKind::Enum => ItemKind::Enum,
        SyntaxKind::ExternBlock => ItemKind::ExternBlock,
        SyntaxKind::ExternCrate => ItemKind::ExternCrate,
        SyntaxKind::Fn => ItemKind::Fn,
        SyntaxKind::Impl => ItemKind::Impl,
        SyntaxKind::MacroCall => ItemKind::MacroCall,
        SyntaxKind::MacroDef => ItemKind::MacroDef,
        SyntaxKind::MacroRules => ItemKind::MacroRules,
        SyntaxKind::Module => ItemKind::Module,
        SyntaxKind::Reuse => ItemKind::Reuse,
        SyntaxKind::Static => ItemKind::Static,
        SyntaxKind::Struct => ItemKind::Struct,
        SyntaxKind::Trait => ItemKind::Trait,
        SyntaxKind::TypeAlias => ItemKind::TypeAlias,
        SyntaxKind::Union => ItemKind::Union,
        SyntaxKind::Use => ItemKind::Use,
        _ => return None,
    })
}

/// A node's text without trivia: `crate :: a` reads `crate::a`.
fn significant_text(node: &SyntaxNode) -> String {
    tokens_deep(node)
        .iter()
        .filter(|t| !t.kind().is_trivia())
        .map(|t| t.text())
        .collect()
}

/// The direct child tokens of a node.
fn tokens(node: &SyntaxNode) -> Vec<SyntaxToken> {
    node.children_with_tokens()
        .into_iter()
        .filter_map(|e| match e {
            SyntaxElement::Token(t) => Some(t),
            SyntaxElement::Node(_) => None,
        })
        .collect()
}

fn tokens_deep(node: &SyntaxNode) -> Vec<SyntaxToken> {
    node.descendants_with_tokens()
        .into_iter()
        .filter_map(|e| match e {
            SyntaxElement::Token(t) => Some(t),
            SyntaxElement::Node(_) => None,
        })
        .collect()
}

/// The name a type is known by: the last path segment of `crate::a::S<T>`, through
/// references, pointers and parentheses (`&S`, `*const S`, `(S)`), and the first bound
/// of `dyn S + Send` or `impl S`. Tuples, slices, arrays, function pointers and `!` have no
/// such name.
fn type_name(ty: &SyntaxNode) -> Option<String> {
    match ty.kind() {
        SyntaxKind::PathType => {
            let path = child(ty, SyntaxKind::Path)?;
            let segment = path
                .children()
                .into_iter()
                .rfind(|n| n.kind() == SyntaxKind::PathSegment)?;
            let name = child(&segment, SyntaxKind::NameRef)?;
            Some(significant_text(&name))
        }
        SyntaxKind::RefType | SyntaxKind::PtrType | SyntaxKind::ParenType => ty
            .children()
            .into_iter()
            .find(|n| is_type(n.kind()))
            .and_then(|inner| type_name(&inner)),
        SyntaxKind::DynTraitType | SyntaxKind::ImplTraitType => ty
            .descendants()
            .into_iter()
            .find(|n| n.kind() == SyntaxKind::PathType)
            .and_then(|path| type_name(&path)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
