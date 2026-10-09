//! Every kind of token (and, from M3 on, every kind of node) in the syntax tree.

macro_rules! syntax_kinds {
    ($($variant:ident => $name:literal,)*) => {
        /// The kind of a token or node.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum SyntaxKind {
            $($variant,)*
        }

        impl SyntaxKind {
            /// Every kind, in declaration order.
            pub const ALL: &'static [SyntaxKind] = &[$(SyntaxKind::$variant,)*];

            /// The name used in tree dumps, such as `L_PAREN`.
            pub fn name(self) -> &'static str {
                match self {
                    $(SyntaxKind::$variant => $name,)*
                }
            }
        }
    };
}

syntax_kinds! {
    // Trivia and file-level tokens.
    Bom => "BOM",
    Shebang => "SHEBANG",
    Whitespace => "WHITESPACE",
    LineComment => "LINE_COMMENT",
    BlockComment => "BLOCK_COMMENT",
    OuterLineDoc => "OUTER_LINE_DOC",
    InnerLineDoc => "INNER_LINE_DOC",
    OuterBlockDoc => "OUTER_BLOCK_DOC",
    InnerBlockDoc => "INNER_BLOCK_DOC",
    // Names.
    Ident => "IDENT",
    RawIdent => "RAW_IDENT",
    Lifetime => "LIFETIME",
    // Literals; a literal token includes its suffix, as in `1u8` or `"x"suffix`.
    IntNumber => "INT_NUMBER",
    FloatNumber => "FLOAT_NUMBER",
    Char => "CHAR",
    Byte => "BYTE",
    Str => "STRING",
    ByteStr => "BYTE_STRING",
    CStr => "C_STRING",
    RawStr => "RAW_STRING",
    RawByteStr => "RAW_BYTE_STRING",
    RawCStr => "RAW_C_STRING",
    // Single-character punctuation. The parser joins them into `::`, `->`, `<<=` and so on.
    Semi => "SEMICOLON",
    Comma => "COMMA",
    Dot => "DOT",
    LParen => "L_PAREN",
    RParen => "R_PAREN",
    LBrace => "L_BRACE",
    RBrace => "R_BRACE",
    LBrack => "L_BRACK",
    RBrack => "R_BRACK",
    At => "AT",
    Pound => "POUND",
    Tilde => "TILDE",
    Question => "QUESTION",
    Colon => "COLON",
    Dollar => "DOLLAR",
    Eq => "EQ",
    Bang => "BANG",
    Lt => "L_ANGLE",
    Gt => "R_ANGLE",
    Minus => "MINUS",
    Amp => "AMP",
    Pipe => "PIPE",
    Plus => "PLUS",
    Star => "STAR",
    Slash => "SLASH",
    Caret => "CARET",
    Percent => "PERCENT",
    // A character that cannot start any token.
    Unknown => "UNKNOWN",
    // Nodes. The parser (stage 1 M4 on) adds the rest.
    SourceFile => "SOURCE_FILE",
    Error => "ERROR",
    Fn => "FN",
    Struct => "STRUCT",
    Enum => "ENUM",
    Union => "UNION",
    Trait => "TRAIT",
    Impl => "IMPL",
    TypeAlias => "TYPE_ALIAS",
    Const => "CONST",
    Static => "STATIC",
    Use => "USE",
    Module => "MODULE",
    ExternCrate => "EXTERN_CRATE",
    ExternBlock => "EXTERN_BLOCK",
    MacroRules => "MACRO_RULES",
    MacroCall => "MACRO_CALL",
}

impl SyntaxKind {
    /// Nodes that own the comments directly above them and the comment at the end of their last
    /// line. See `docs/design/comment-attachment.md`.
    pub fn is_attachable(self) -> bool {
        matches!(
            self,
            SyntaxKind::Fn
                | SyntaxKind::Struct
                | SyntaxKind::Enum
                | SyntaxKind::Union
                | SyntaxKind::Trait
                | SyntaxKind::Impl
                | SyntaxKind::TypeAlias
                | SyntaxKind::Const
                | SyntaxKind::Static
                | SyntaxKind::Use
                | SyntaxKind::Module
                | SyntaxKind::ExternCrate
                | SyntaxKind::ExternBlock
                | SyntaxKind::MacroRules
                | SyntaxKind::MacroCall
        )
    }

    /// Whitespace, comments, the byte-order mark and the shebang: tokens the parser skips.
    /// Doc comments are not trivia; they become attributes.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::Bom
                | SyntaxKind::Shebang
                | SyntaxKind::Whitespace
                | SyntaxKind::LineComment
                | SyntaxKind::BlockComment
        )
    }

    pub fn is_literal(self) -> bool {
        matches!(
            self,
            SyntaxKind::IntNumber
                | SyntaxKind::FloatNumber
                | SyntaxKind::Char
                | SyntaxKind::Byte
                | SyntaxKind::Str
                | SyntaxKind::ByteStr
                | SyntaxKind::CStr
                | SyntaxKind::RawStr
                | SyntaxKind::RawByteStr
                | SyntaxKind::RawCStr
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_every_kind_has_a_unique_name() {
        let mut seen = HashSet::new();
        for kind in SyntaxKind::ALL {
            let name = kind.name();
            assert!(!name.is_empty(), "{:?} has no name", kind);
            assert!(
                name.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                "{} is not SCREAMING_CASE",
                name
            );
            assert!(seen.insert(name), "duplicate name {}", name);
        }
    }
}
