use super::*;
use crate::syntax::Edition;
use crate::syntax::lexer::tokenize;
use crate::syntax::tree::{SyntaxNode, debug_dump};

/// A tiny script for writing parser events by hand.
enum Op {
  Start(SyntaxKind),
  StartWrapped(SyntaxKind, u32),
  Tok,
  Glue(SyntaxKind, u8),
  Finish,
  Error(&'static str),
}

/// Builds `text` with events from `ops`; `Tok` consumes the next significant token as itself.
fn build(text: &str, ops: &[Op]) -> (SyntaxNode, Vec<SyntaxError>) {
  let tokens = tokenize(text, Edition::E2021);
  let mut significant = tokens
    .iter()
    .filter(|t| !t.kind.is_trivia())
    .map(|t| t.kind);
  let mut events = Vec::new();
  let mut messages = Vec::new();
  for op in ops {
    events.push(match op {
      Op::Start(kind) => Event::Start {
        kind: *kind,
        forward_parent: None,
      },
      Op::StartWrapped(kind, distance) => Event::Start {
        kind: *kind,
        forward_parent: std::num::NonZeroU32::new(*distance),
      },
      Op::Tok => Event::Token {
        kind: significant.next().expect("ran out of tokens"),
        n_raw_tokens: 1,
      },
      Op::Glue(kind, n) => {
        for _ in 0..*n {
          significant.next();
        }
        Event::Token {
          kind: *kind,
          n_raw_tokens: *n,
        }
      }
      Op::Finish => Event::Finish,
      Op::Error(message) => {
        messages.push(message.to_string());
        Event::Error {
          message: messages.len() as u32 - 1,
        }
      }
    });
  }
  let parsed = crate::syntax::parser::Parsed { events, messages };
  let (green, errors, _) = build_tree(text, &tokens, parsed);
  let root = SyntaxNode::new_root(green);
  assert_eq!(root.text(), text, "the tree must reproduce the input");
  (root, errors)
}

/// The text of each child node of the root, by kind.
fn child_texts(root: &SyntaxNode) -> Vec<(&'static str, String)> {
  root
    .children()
    .iter()
    .map(|n| (n.kind().name(), n.text()))
    .collect()
}

fn file(items: Vec<Vec<Op>>) -> Vec<Op> {
  let mut ops = vec![Op::Start(SyntaxKind::SourceFile)];
  for item in items {
    ops.extend(item);
  }
  ops.push(Op::Finish);
  ops
}

fn item(tokens: usize) -> Vec<Op> {
  let mut ops = vec![Op::Start(SyntaxKind::Fn)];
  for _ in 0..tokens {
    ops.push(Op::Tok);
  }
  ops.push(Op::Finish);
  ops
}

#[test]
fn test_a_macro_call_in_an_expression_owns_no_comments() {
  let text = "fn f() {\n    // above\n    m!(x) // after\n}\nm!(y); // item\n";
  let root = crate::syntax::parse(text, Edition::E2021).syntax_node();
  let calls: Vec<String> = root
    .descendant_nodes_where(|kind| kind == SyntaxKind::MacroCall)
    .map(|call| call.text().to_string())
    .collect();
  assert_eq!(
    calls,
    vec!["m!(x)".to_string(), "m!(y); // item".to_string()]
  );
}

/// A file makes about three events per token: each is kept to 8 bytes.
#[test]
fn test_an_event_is_small() {
  assert!(
    std::mem::size_of::<Event>() <= 8,
    "{}",
    std::mem::size_of::<Event>()
  );
}

#[test]
fn test_blank_line_keeps_comment_outside() {
  let text = "// section\n\nfn a() {}\n";
  let (root, _) = build(text, &file(vec![item(6)]));
  assert_eq!(child_texts(&root), vec![("FN", "fn a() {}".to_string())]);
}

#[test]
fn test_comment_directly_above_belongs_to_the_item() {
  let text = "// about a\nfn a() {}\n";
  let (root, _) = build(text, &file(vec![item(6)]));
  assert_eq!(
    child_texts(&root),
    vec![("FN", "// about a\nfn a() {}".to_string())]
  );
}

#[test]
fn test_errors_are_positioned() {
  let text = "a b";
  let ops = vec![
    Op::Start(SyntaxKind::SourceFile),
    Op::Tok,
    Op::Error("expected item"),
    Op::Tok,
    Op::Finish,
  ];
  let (_, errors) = build(text, &ops);
  assert_eq!(errors.len(), 1);
  assert_eq!(errors[0].message, "expected item");
  // Positioned at the next significant token, `b`, not before the whitespace.
  assert_eq!(errors[0].range, TextRange::new(2, 2));
}

#[test]
fn test_file_start_tokens_stay_at_the_root() {
  let text = "\u{feff}#!/bin/run\n// about a\nfn a() {}";
  let (root, _) = build(text, &file(vec![item(6)]));
  assert_eq!(
    child_texts(&root),
    vec![("FN", "// about a\nfn a() {}".to_string())]
  );
  let first = root.first_token().unwrap();
  assert_eq!(first.kind(), SyntaxKind::Bom);
  assert_eq!(first.parent().kind(), SyntaxKind::SourceFile);
}

#[test]
fn test_forward_parent_wraps_a_finished_node() {
  // FN is completed first, then wrapped in MACRO_CALL, as the parser does for `a b`.
  let text = "a b";
  let ops = vec![
    Op::Start(SyntaxKind::SourceFile),
    Op::StartWrapped(SyntaxKind::Fn, 3),
    Op::Tok,
    Op::Finish,
    Op::Start(SyntaxKind::MacroCall),
    Op::Tok,
    Op::Finish,
    Op::Finish,
  ];
  let (root, _) = build(text, &ops);
  assert_eq!(
    debug_dump(&root),
    "SOURCE_FILE@0..3\n  MACRO_CALL@0..3\n    FN@0..1\n      IDENT@0..1 \"a\"\n    WHITESPACE@1..2 \" \"\n    IDENT@2..3 \"b\"\n"
  );
}

#[test]
fn test_glued_tokens_become_one_tree_token() {
  let text = "a :: b";
  let ops = vec![
    Op::Start(SyntaxKind::SourceFile),
    Op::Tok,
    Op::Glue(SyntaxKind::Colon, 2),
    Op::Tok,
    Op::Finish,
  ];
  let (root, _) = build(text, &ops);
  let texts: Vec<String> = root
    .children_with_tokens()
    .iter()
    .filter_map(|e| match e {
      crate::syntax::SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t.text().to_string()),
      _ => None,
    })
    .collect();
  assert_eq!(texts, vec!["a", "::", "b"]);
}

#[test]
fn test_navigation_and_ranges() {
  let text = "fn a() {}\nfn b() {}";
  let (root, _) = build(text, &file(vec![item(6), item(6)]));
  let items = root.children();
  assert_eq!(items[0].text_range(), TextRange::new(0, 9));
  assert_eq!(items[1].text_range(), TextRange::new(10, 19));
  assert_eq!(items[1].parent().unwrap(), root);
  assert_eq!(items[1].ancestors().len(), 2);
  assert_eq!(items[0].first_token().unwrap().text(), "fn");
  assert_eq!(items[0].last_token().unwrap().text(), "}");
  let between = items[0].next_sibling_or_token().unwrap();
  assert_eq!(between.kind(), SyntaxKind::Whitespace);
  assert_eq!(
    items[1].prev_sibling_or_token().unwrap().kind(),
    SyntaxKind::Whitespace
  );
  assert_eq!(root.descendants().len(), 3);
  assert_eq!(root.descendants_with_tokens().len(), 1 + 2 + 2 * 8 + 1);
}

#[test]
fn test_next_line_comment_leads_the_next_item() {
  let text = "fn a() {}\n// about b\nfn b() {}\n";
  let (root, _) = build(text, &file(vec![item(6), item(6)]));
  assert_eq!(
    child_texts(&root),
    vec![
      ("FN", "fn a() {}".to_string()),
      ("FN", "// about b\nfn b() {}".to_string())
    ]
  );
}

#[test]
fn test_non_attachable_nodes_leave_comments_to_their_parent() {
  let text = "// note\nx";
  let ops = vec![
    Op::Start(SyntaxKind::SourceFile),
    Op::Start(SyntaxKind::Error),
    Op::Tok,
    Op::Finish,
    Op::Finish,
  ];
  let (root, _) = build(text, &ops);
  assert_eq!(child_texts(&root), vec![("ERROR", "x".to_string())]);
}

#[test]
fn test_only_the_comment_run_touching_the_item_attaches() {
  let text = "// one\n\n// two\n// three\nfn a() {}";
  let (root, _) = build(text, &file(vec![item(6)]));
  assert_eq!(
    child_texts(&root),
    vec![("FN", "// two\n// three\nfn a() {}".to_string())]
  );
}

#[test]
fn test_same_line_comment_trails_the_item() {
  let text = "fn a() {} // about a\nfn b() {}\n";
  let (root, _) = build(text, &file(vec![item(6), item(6)]));
  assert_eq!(
    child_texts(&root),
    vec![
      ("FN", "fn a() {} // about a".to_string()),
      ("FN", "fn b() {}".to_string())
    ]
  );
}
