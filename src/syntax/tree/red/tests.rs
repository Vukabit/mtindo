use super::*;
use crate::syntax::{Edition, parse};

fn all(root: &SyntaxNode) -> Vec<SyntaxElement> {
  let mut out = Vec::new();
  reference_descendants(root, &mut out);
  out
}

/// The children as the tree had them before the iterators: every one made, in order.
fn reference_children(node: &SyntaxNode) -> Vec<SyntaxElement> {
  let mut out = Vec::new();
  let mut offset = node.0.offset;
  for (index, child) in node.0.green.children().iter().enumerate() {
    out.push(node.make_child(*child, index as u32, offset));
    offset += child.text_len();
  }
  out
}

fn reference_descendants(node: &SyntaxNode, out: &mut Vec<SyntaxElement>) {
  out.push(SyntaxElement::Node(node.clone()));
  for child in reference_children(node) {
    match child {
      SyntaxElement::Node(child) => reference_descendants(&child, out),
      token => out.push(token),
    }
  }
}

/// Elements that are the same and know the same parents and places.
fn same(a: &SyntaxElement, b: &SyntaxElement) -> bool {
  let parents = |e: &SyntaxElement| match e {
    SyntaxElement::Node(n) => n.parent().map(|p| p.text_range()),
    SyntaxElement::Token(t) => Some(t.parent().text_range()),
  };
  a == b && a.text_range() == b.text_range() && parents(a) == parents(b)
}

fn same_all(a: &[SyntaxElement], b: &[SyntaxElement]) -> bool {
  a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
}

#[test]
fn test_filtered_walks_give_what_a_full_walk_would() {
  let wanted = |kind: SyntaxKind| {
    matches!(
      kind,
      SyntaxKind::Attr | SyntaxKind::BlockExpr | SyntaxKind::Ident | SyntaxKind::LineComment
    )
  };
  for root in trees() {
    for start in root.descendants() {
      let expected: Vec<SyntaxElement> = all(&start)
        .into_iter()
        .filter(|e| wanted(e.kind()))
        .collect();
      let walked: Vec<SyntaxElement> = start.descendants_where(wanted).collect();
      assert!(same_all(&walked, &expected));
      let nodes: Vec<SyntaxElement> = expected
        .iter()
        .filter(|e| matches!(e, SyntaxElement::Node(_)))
        .cloned()
        .collect();
      let walked: Vec<SyntaxElement> = start
        .descendant_nodes_where(wanted)
        .map(SyntaxElement::Node)
        .collect();
      assert!(same_all(&walked, &nodes));
      assert_eq!(start.subtree_has(wanted), !expected.is_empty());
      let kinds = start
        .children_of_kind(SyntaxKind::Attr)
        .map(SyntaxElement::Node)
        .collect::<Vec<_>>();
      let expected: Vec<SyntaxElement> = reference_children(&start)
        .into_iter()
        .filter(|e| matches!(e, SyntaxElement::Node(n) if n.kind() == SyntaxKind::Attr))
        .collect();
      assert!(same_all(&kinds, &expected));
      let texts: Vec<(SyntaxKind, String, TextRange)> = start
        .token_texts()
        .map(|(kind, text, range)| (kind, text.to_string(), range))
        .collect();
      let expected: Vec<(SyntaxKind, String, TextRange)> = all(&start)
        .into_iter()
        .filter_map(|e| match e {
          SyntaxElement::Token(t) => Some((t.kind(), t.text().to_string(), t.text_range())),
          SyntaxElement::Node(_) => None,
        })
        .collect();
      assert_eq!(texts, expected);
    }
  }
}

#[test]
fn test_siblings_and_edge_tokens_match_the_reference() {
  for root in trees() {
    for element in all(&root) {
      let (parent, index) = match &element {
        SyntaxElement::Node(n) => match n.parent() {
          Some(parent) => (parent, n.0.index as usize),
          None => continue,
        },
        SyntaxElement::Token(t) => (t.parent(), t.index as usize),
      };
      let siblings = reference_children(&parent);
      let (next, prev) = match &element {
        SyntaxElement::Node(n) => (n.next_sibling_or_token(), n.prev_sibling_or_token()),
        SyntaxElement::Token(t) => (t.next_sibling_or_token(), t.prev_sibling_or_token()),
      };
      let check = |got: Option<SyntaxElement>, want: Option<&SyntaxElement>| match (got, want) {
        (None, None) => {}
        (Some(got), Some(want)) => assert!(same(&got, want)),
        (got, want) => panic!("{:?} is not {:?}", got, want),
      };
      check(next, siblings.get(index + 1));
      check(prev, index.checked_sub(1).and_then(|i| siblings.get(i)));
      if let SyntaxElement::Node(node) = &element {
        let tokens: Vec<SyntaxElement> = all(node)
          .into_iter()
          .filter(|e| matches!(e, SyntaxElement::Token(_)))
          .collect();
        check(node.first_token().map(SyntaxElement::Token), tokens.first());
        check(node.last_token().map(SyntaxElement::Token), tokens.last());
      }
    }
  }
}

#[test]
fn test_walks_match_the_reference_in_order() {
  for root in trees() {
    let expected = all(&root);
    let walked: Vec<SyntaxElement> = root.descendants_with_tokens_iter().collect();
    assert!(same_all(&walked, &expected));
    assert!(same_all(&root.descendants_with_tokens(), &expected));
    let nodes: Vec<SyntaxElement> = expected
      .iter()
      .filter(|e| matches!(e, SyntaxElement::Node(_)))
      .cloned()
      .collect();
    let walked: Vec<SyntaxElement> = root.descendants_iter().map(SyntaxElement::Node).collect();
    assert!(same_all(&walked, &nodes));
    for element in &expected {
      let SyntaxElement::Node(node) = element else {
        continue;
      };
      let children = reference_children(node);
      let walked: Vec<SyntaxElement> = node.children_with_tokens_iter().collect();
      assert!(same_all(&walked, &children));
      let mut reversed: Vec<SyntaxElement> = node.children_with_tokens_iter().rev().collect();
      reversed.reverse();
      assert!(same_all(&reversed, &children));
      let child_nodes: Vec<SyntaxElement> = children
        .iter()
        .filter(|e| matches!(e, SyntaxElement::Node(_)))
        .cloned()
        .collect();
      let walked: Vec<SyntaxElement> = node.children_iter().map(SyntaxElement::Node).collect();
      assert!(same_all(&walked, &child_nodes));
      let mut reversed: Vec<SyntaxElement> = node
        .children_iter()
        .rev()
        .map(SyntaxElement::Node)
        .collect();
      reversed.reverse();
      assert!(same_all(&reversed, &child_nodes));
      for n in 0..=children.len() {
        let got = node.children_with_tokens_iter().nth(n);
        assert_eq!(got.is_some(), n < children.len());
        if let Some(got) = got {
          assert!(same(&got, &children[n]));
        }
      }
    }
  }
}

/// Trees to walk: this file's own module and a few shapes with attributes, comments, empty
/// nodes and deep nesting.
fn trees() -> Vec<SyntaxNode> {
  [
    include_str!("../red.rs"),
    include_str!("../builder/mod.rs"),
    "#![allow(x)]\n// lead\nfn f() { let a = (1, [2, 3]); /* c */ g(a)?.h().await; }\n",
    "struct S { #[a] x: u8, y: (u8,), }\nenum E { A, B(u8) }\nimpl T for S {}\n",
    "fn f(",
    "",
  ]
  .iter()
  .map(|text| parse(text, Edition::E2021).syntax_node())
  .collect()
}
