//! Layer 1: Rust syntax. Built alongside the stage 0 engine; layout does not use it yet.
//! [`ast`] gives typed views over the tree for the parts layout needs.

pub mod ast;
pub mod edition;
pub mod kind;
pub mod lexer;
pub mod parser;
pub mod tree;

use std::rc::Rc;

pub use edition::Edition;
pub use kind::SyntaxKind;
pub use tree::builder::SyntaxError;
pub use tree::{SyntaxElement, SyntaxNode, SyntaxToken, TextRange};

/// The result of parsing a file: a tree that always reproduces the input exactly, and the
/// errors found while lexing and parsing it.
#[derive(Debug, Clone)]
pub struct Parse {
    green: Rc<tree::GreenNode>,
    errors: Vec<SyntaxError>,
}

impl Parse {
    pub fn errors(&self) -> &[SyntaxError] {
        &self.errors
    }

    pub fn syntax_node(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }
}

/// Parses a whole source file.
pub fn parse(text: &str, edition: Edition) -> Parse {
    let tokens = lexer::split_tuple_field_floats(text, lexer::tokenize(text, edition));
    let events = parser::parse_events(text, &tokens, edition);
    let (green, errors) = tree::builder::build_tree(text, &tokens, events);
    Parse {
        green: Rc::new(green),
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_parse_reports_lex_errors_with_ranges() {
        let parse = parse("fn a() { \"open", Edition::E2021);
        let lex_error = parse
            .errors()
            .iter()
            .find(|e| e.message == "string is not closed")
            .expect("the lexer error is reported");
        assert_eq!(lex_error.range, TextRange::new(9, 14));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]

        #[test]
        fn prop_tree_text_is_the_input(text in any::<String>()) {
            for edition in [Edition::E2015, Edition::E2018, Edition::E2021, Edition::E2024] {
                let tree = parse(&text, edition).syntax_node();
                prop_assert_eq!(tree.text(), text.clone());
                prop_assert_eq!(tree.text_range(), TextRange::new(0, text.len() as u32));
            }
        }
    }
}
