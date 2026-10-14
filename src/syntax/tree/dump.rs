//! An indented text dump of a tree, used in parser tests and `mto debug tree`.

use crate::syntax::tree::red::{SyntaxElement, SyntaxNode};

/// One line per node and token, e.g. `FN@0..9` and `IDENT@0..2 "fn"`, indented by depth.
pub fn debug_dump(node: &SyntaxNode) -> String {
  let mut out = String::new();
  write_node(node, 0, &mut out);
  out
}

fn write_node(node: &SyntaxNode, depth: usize, out: &mut String) {
  out.push_str(&"  ".repeat(depth));
  out.push_str(&format!("{:?}\n", node));
  for child in node.children_with_tokens_iter() {
    match child {
      SyntaxElement::Node(child) => write_node(&child, depth + 1, out),
      SyntaxElement::Token(token) => {
        out.push_str(&"  ".repeat(depth + 1));
        out.push_str(&format!("{:?}\n", token));
      }
    }
  }
}
