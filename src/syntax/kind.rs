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
    // Punctuation the parser glues from adjacent single characters.
    Colon2 => "COLON2",
    ThinArrow => "THIN_ARROW",
    FatArrow => "FAT_ARROW",
    Dot2 => "DOT2",
    Dot3 => "DOT3",
    Dot2Eq => "DOT2EQ",
    EqEq => "EQ2",
    Neq => "NEQ",
    LtEq => "LTEQ",
    GtEq => "GTEQ",
    AmpAmp => "AMP2",
    PipePipe => "PIPE2",
    PlusEq => "PLUSEQ",
    MinusEq => "MINUSEQ",
    StarEq => "STAREQ",
    SlashEq => "SLASHEQ",
    PercentEq => "PERCENTEQ",
    CaretEq => "CARETEQ",
    AmpEq => "AMPEQ",
    PipeEq => "PIPEEQ",
    Shl => "SHL",
    Shr => "SHR",
    ShlEq => "SHLEQ",
    ShrEq => "SHREQ",
    // Keywords. The lexer produces identifiers; the parser gives keywords these kinds.
    AsKw => "AS_KW",
    AsyncKw => "ASYNC_KW",
    AutoKw => "AUTO_KW",
    AwaitKw => "AWAIT_KW",
    BoxKw => "BOX_KW",
    BreakKw => "BREAK_KW",
    ConstKw => "CONST_KW",
    ContinueKw => "CONTINUE_KW",
    CrateKw => "CRATE_KW",
    DefaultKw => "DEFAULT_KW",
    DynKw => "DYN_KW",
    ElseKw => "ELSE_KW",
    EnumKw => "ENUM_KW",
    ExternKw => "EXTERN_KW",
    FalseKw => "FALSE_KW",
    FnKw => "FN_KW",
    ForKw => "FOR_KW",
    GenKw => "GEN_KW",
    IfKw => "IF_KW",
    ImplKw => "IMPL_KW",
    InKw => "IN_KW",
    LetKw => "LET_KW",
    LoopKw => "LOOP_KW",
    MacroKw => "MACRO_KW",
    MacroRulesKw => "MACRO_RULES_KW",
    MatchKw => "MATCH_KW",
    ModKw => "MOD_KW",
    MoveKw => "MOVE_KW",
    MutKw => "MUT_KW",
    PubKw => "PUB_KW",
    RawKw => "RAW_KW",
    RefKw => "REF_KW",
    ReturnKw => "RETURN_KW",
    SafeKw => "SAFE_KW",
    SelfKw => "SELF_KW",
    SelfTypeKw => "SELF_TYPE_KW",
    StaticKw => "STATIC_KW",
    StructKw => "STRUCT_KW",
    SuperKw => "SUPER_KW",
    TraitKw => "TRAIT_KW",
    TrueKw => "TRUE_KW",
    TryKw => "TRY_KW",
    TypeKw => "TYPE_KW",
    UnionKw => "UNION_KW",
    UnsafeKw => "UNSAFE_KW",
    UseKw => "USE_KW",
    WhereKw => "WHERE_KW",
    WhileKw => "WHILE_KW",
    YieldKw => "YIELD_KW",
    // Past the last token. Never appears in a tree.
    Eof => "EOF",
    // Nodes.
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
    MacroDef => "MACRO_DEF",
    Attr => "ATTR",
    Meta => "META",
    Visibility => "VISIBILITY",
    Name => "NAME",
    NameRef => "NAME_REF",
    Abi => "ABI",
    Path => "PATH",
    PathSegment => "PATH_SEGMENT",
    GenericArgList => "GENERIC_ARG_LIST",
    TypeArg => "TYPE_ARG",
    LifetimeArg => "LIFETIME_ARG",
    ConstArg => "CONST_ARG",
    AssocTypeArg => "ASSOC_TYPE_ARG",
    ParenthesizedArgList => "PARENTHESIZED_ARG_LIST",
    RetType => "RET_TYPE",
    GenericParamList => "GENERIC_PARAM_LIST",
    LifetimeParam => "LIFETIME_PARAM",
    TypeParam => "TYPE_PARAM",
    ConstParam => "CONST_PARAM",
    WhereClause => "WHERE_CLAUSE",
    WherePred => "WHERE_PRED",
    TypeBoundList => "TYPE_BOUND_LIST",
    TypeBound => "TYPE_BOUND",
    ForBinder => "FOR_BINDER",
    UseBoundGenericArgs => "USE_BOUND_GENERIC_ARGS",
    ParamList => "PARAM_LIST",
    Param => "PARAM",
    SelfParam => "SELF_PARAM",
    RecordFieldList => "RECORD_FIELD_LIST",
    RecordField => "RECORD_FIELD",
    TupleFieldList => "TUPLE_FIELD_LIST",
    TupleField => "TUPLE_FIELD",
    VariantList => "VARIANT_LIST",
    Variant => "VARIANT",
    ItemList => "ITEM_LIST",
    AssocItemList => "ASSOC_ITEM_LIST",
    ExternItemList => "EXTERN_ITEM_LIST",
    UseTree => "USE_TREE",
    UseTreeList => "USE_TREE_LIST",
    Rename => "RENAME",
    TokenTree => "TOKEN_TREE",
    PathType => "PATH_TYPE",
    RefType => "REF_TYPE",
    PtrType => "PTR_TYPE",
    SliceType => "SLICE_TYPE",
    ArrayType => "ARRAY_TYPE",
    TupleType => "TUPLE_TYPE",
    ParenType => "PAREN_TYPE",
    NeverType => "NEVER_TYPE",
    InferType => "INFER_TYPE",
    FnPtrType => "FN_PTR_TYPE",
    DynTraitType => "DYN_TRAIT_TYPE",
    ImplTraitType => "IMPL_TRAIT_TYPE",
    MacroType => "MACRO_TYPE",
    // Placeholders that keep their tokens unparsed until expressions, statements and patterns
    // are parsed (stage 1 M5).
    OpaqueBlock => "OPAQUE_BLOCK",
    Opaque => "OPAQUE",
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
                name.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{} is not SCREAMING_CASE",
                name
            );
            assert!(seen.insert(name), "duplicate name {}", name);
        }
    }
}
