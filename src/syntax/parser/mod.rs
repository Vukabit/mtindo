//! Turns tokens into parser events. Until the grammar lands (stage 1 M4), every significant
//! token goes directly under the file node.

pub mod event;

use event::Event;

use crate::syntax::SyntaxKind;
use crate::syntax::lexer::Token;

/// The events for a whole file.
pub fn parse_events(tokens: &[Token]) -> Vec<Event> {
    let mut events = vec![Event::Start {
        kind: SyntaxKind::SourceFile,
        forward_parent: None,
    }];
    for token in tokens.iter() {
        if !token.kind.is_trivia() {
            events.push(Event::Token {
                kind: token.kind,
                n_raw_tokens: 1,
            });
        }
    }
    events.push(Event::Finish);
    events
}
