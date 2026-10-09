//! Which comments belong to which node. The rules are specified in
//! `docs/design/comment-attachment.md`; this module implements them over runs of trivia.

use crate::syntax::SyntaxKind;

/// How many tokens at the end of `trivia` (the trivia just before an attachable node) belong
/// inside that node: the comments directly above it, with no blank line between them and the
/// node, plus the whitespace between those comments and the node.
pub(super) fn leading(trivia: &[(SyntaxKind, &str)]) -> usize {
    let mut attached = 0;
    for (count, (kind, text)) in trivia.iter().rev().enumerate() {
        match kind {
            SyntaxKind::Whitespace if text.matches('\n').count() >= 2 => break,
            SyntaxKind::Whitespace => {}
            SyntaxKind::LineComment | SyntaxKind::BlockComment => attached = count + 1,
            _ => break,
        }
    }
    attached
}

/// How many tokens at the start of `trivia` (the trivia just after an attachable node) belong
/// inside that node: a comment on the node's last line, and the spaces before it. A block
/// comment only counts if it ends on that line and nothing else follows it there.
pub(super) fn trailing(trivia: &[(SyntaxKind, &str)]) -> usize {
    let mut idx = 0;
    if let Some((SyntaxKind::Whitespace, text)) = trivia.first() {
        if text.contains('\n') {
            return 0;
        }
        idx = 1;
    }
    match trivia.get(idx) {
        Some((SyntaxKind::LineComment, _)) => idx + 1,
        Some((SyntaxKind::BlockComment, text)) if !text.contains('\n') => {
            let line_ends_after = match trivia.get(idx + 1) {
                None => true,
                Some((SyntaxKind::Whitespace, ws)) => ws.contains('\n'),
                Some(_) => false,
            };
            if line_ends_after { idx + 1 } else { 0 }
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SyntaxKind::{BlockComment, LineComment, Whitespace};

    #[test]
    fn test_leading_takes_comments_directly_above() {
        assert_eq!(leading(&[(LineComment, "// a"), (Whitespace, "\n")]), 2);
        assert_eq!(
            leading(&[
                (LineComment, "// a"),
                (Whitespace, "\n"),
                (LineComment, "// b"),
                (Whitespace, "\n")
            ]),
            4
        );
    }

    #[test]
    fn test_leading_stops_at_blank_line() {
        let trivia = [
            (LineComment, "// a"),
            (Whitespace, "\n\n"),
            (LineComment, "// b"),
            (Whitespace, "\n"),
        ];
        assert_eq!(leading(&trivia), 2);
        assert_eq!(leading(&[(LineComment, "// a"), (Whitespace, "\n\n")]), 0);
    }

    #[test]
    fn test_leading_ignores_plain_whitespace_and_file_start_tokens() {
        assert_eq!(leading(&[(Whitespace, "\n    ")]), 0);
        assert_eq!(
            leading(&[(SyntaxKind::Shebang, "#!/x"), (Whitespace, "\n")]),
            0
        );
        assert_eq!(leading(&[]), 0);
    }

    #[test]
    fn test_trailing_takes_same_line_comment() {
        assert_eq!(
            trailing(&[
                (Whitespace, " "),
                (LineComment, "// note"),
                (Whitespace, "\n")
            ]),
            2
        );
        assert_eq!(trailing(&[(LineComment, "// note")]), 1);
        assert_eq!(
            trailing(&[
                (Whitespace, " "),
                (BlockComment, "/* note */"),
                (Whitespace, "\n")
            ]),
            2
        );
        assert_eq!(
            trailing(&[(Whitespace, " "), (BlockComment, "/* note */")]),
            2
        );
    }

    #[test]
    fn test_trailing_rejects_other_lines_and_shared_lines() {
        assert_eq!(trailing(&[(Whitespace, "\n"), (LineComment, "// next")]), 0);
        assert_eq!(
            trailing(&[(Whitespace, " "), (BlockComment, "/* a\nb */")]),
            0
        );
        assert_eq!(
            trailing(&[
                (Whitespace, " "),
                (BlockComment, "/* a */"),
                (Whitespace, " ")
            ]),
            0
        );
        assert_eq!(trailing(&[(Whitespace, " ")]), 0);
    }
}
