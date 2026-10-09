use super::*;
use crate::syntax::{Edition, parse};

fn items(text: &str) -> Vec<Item> {
    let file = SourceFile::cast(parse(text, Edition::E2021).syntax_node()).unwrap();
    file.items()
}

fn one(text: &str) -> Item {
    let mut all = items(text);
    assert_eq!(all.len(), 1, "{}", text);
    all.remove(0)
}

#[test]
fn test_attrs_and_docs() {
    let item = one(
        "/// one\n/** two */\n#[derive(Debug)]\n#[cfg_attr(test, allow(x))]\n// plain\nstruct S;",
    );
    assert_eq!(item.docs(), vec!["/// one", "/** two */"]);
    let paths: Vec<String> = item.attrs().iter().map(Attr::path).collect();
    assert_eq!(paths, vec!["derive", "cfg_attr"]);
}

#[test]
fn test_impl_self_type_and_trait() {
    let view = |text: &str| one(text).as_impl().unwrap();
    let inherent = view("impl<T> Wrapper<T> {}");
    assert_eq!(inherent.self_ty_name().as_deref(), Some("Wrapper"));
    assert!(inherent.trait_ref().is_none());

    let trait_impl = view("impl<T: Clone> fmt::Display for crate::a::S<T> where T: Copy {}");
    assert_eq!(trait_impl.self_ty_name().as_deref(), Some("S"));
    assert_eq!(trait_impl.trait_name().as_deref(), Some("Display"));
    assert!(!trait_impl.is_negative());

    let negative = view("unsafe impl !Send for &'static [Node] {}");
    assert!(negative.is_negative());
    assert_eq!(negative.self_ty_name(), None);
    assert_eq!(negative.trait_name().as_deref(), Some("Send"));

    let name = |text: &str| view(text).self_ty_name();
    assert_eq!(name("impl<A, B> Trait for (A, B) {}"), None);
    assert_eq!(name("impl<T> Trait for [T] {}"), None);
    assert_eq!(
        name("impl Trait for *const Node {}"),
        Some("Node".to_string())
    );
    assert_eq!(name("impl dyn Error + Send {}"), Some("Error".to_string()));
    assert_eq!(name("impl<T: ?Sized> Ref<T> {}"), Some("Ref".to_string()));

    assert!(one("fn f() {}").as_impl().is_none());
}

#[test]
fn test_inner_attributes_belong_to_the_file() {
    let file = SourceFile::cast(
        parse(
            "#![allow(dead_code)]\n#![no_std]\nfn f() {}\n",
            Edition::E2021,
        )
        .syntax_node(),
    )
    .unwrap();
    let paths: Vec<String> = file.attrs().iter().map(Attr::path).collect();
    assert_eq!(paths, vec!["allow", "no_std"]);
    assert!(file.attrs().iter().all(Attr::is_inner));
    assert_eq!(file.items().len(), 1);
    assert!(file.items()[0].attrs().is_empty());
}

#[test]
fn test_items_kinds_and_names() {
    let text = "fn f() {}\nstruct S;\nenum E {}\nunion U { a: u8 }\ntrait T {}\nimpl S {}\ntype A = u8;\nconst C: u8 = 1;\nconst _: () = ();\nstatic mut X: u8 = 0;\nuse a::b;\nmod m;\nextern crate alloc;\nextern crate core as c;\nextern \"C\" {}\nmacro_rules! mac { () => {} }\nmac!();\n";
    let found: Vec<(ItemKind, Option<String>)> =
        items(text).iter().map(|i| (i.kind(), i.name())).collect();
    let name = |s: &str| Some(s.to_string());
    assert_eq!(
        found,
        vec![
            (ItemKind::Fn, name("f")),
            (ItemKind::Struct, name("S")),
            (ItemKind::Enum, name("E")),
            (ItemKind::Union, name("U")),
            (ItemKind::Trait, name("T")),
            (ItemKind::Impl, None),
            (ItemKind::TypeAlias, name("A")),
            (ItemKind::Const, name("C")),
            (ItemKind::Const, None),
            (ItemKind::Static, name("X")),
            (ItemKind::Use, None),
            (ItemKind::Module, name("m")),
            (ItemKind::ExternCrate, name("alloc")),
            (ItemKind::ExternCrate, name("c")),
            (ItemKind::ExternBlock, None),
            (ItemKind::MacroRules, name("mac")),
            (ItemKind::MacroCall, None),
        ]
    );
}

#[test]
fn test_use_root() {
    let root = |text: &str| one(text).as_use().unwrap().root();
    assert_eq!(root("use std::io;").as_deref(), Some("std"));
    assert_eq!(root("use ::std::io;").as_deref(), Some("std"));
    assert_eq!(root("use crate::a::{b, c};").as_deref(), Some("crate"));
    assert_eq!(root("pub use self::x as y;").as_deref(), Some("self"));
    assert_eq!(root("use {a, b};"), None);
}

#[test]
fn test_views_survive_broken_code() {
    let all = items("fn f( {}\nimpl for {}\nstruct\n");
    assert!(!all.is_empty());
    for item in all {
        let _ = (
            item.kind(),
            item.name(),
            item.visibility(),
            item.attrs(),
            item.docs(),
        );
        if let Some(view) = item.as_impl() {
            let _ = (view.self_ty_name(), view.trait_name());
        }
    }
}

#[test]
fn test_visibility() {
    let vis = |text: &str| one(text).visibility();
    assert_eq!(vis("fn f() {}"), Visibility::Private);
    assert_eq!(vis("pub fn f() {}"), Visibility::Pub);
    assert_eq!(vis("pub(crate) fn f() {}"), Visibility::PubCrate);
    assert_eq!(vis("pub(super) fn f() {}"), Visibility::PubSuper);
    assert_eq!(vis("pub(self) fn f() {}"), Visibility::PubSelf);
    assert_eq!(
        vis("pub(in crate :: a) fn f() {}"),
        Visibility::PubIn("crate::a".to_string())
    );
}
