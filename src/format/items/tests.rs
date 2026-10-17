use crate::format::file::format_file;
use crate::options::format::FormatConfig;
use crate::syntax::Edition;
use crate::verify::format::verify_format;

/// `source` formatted, checked for meaning and stability.
fn formatted(source: &str) -> String {
  let config = FormatConfig::default();
  let once = format_file(source, Edition::E2021, &config);
  let found = verify_format(source, &once, Edition::E2021);
  assert!(found.is_empty(), "{:?}\n{}", found, once);
  assert_eq!(
    format_file(&once, Edition::E2021, &config),
    once,
    "not stable"
  );
  once
}

#[test]
fn test_brace_after_a_broken_parameter_list() {
  // The `{` stays while the closing line is within the line width less its indentation.
  let close = |n: usize| {
    let head = "        ) -> Poll<";
    format!("{}{}> {{", head, "x".repeat(n - head.len() - 3))
  };
  let source = |n: usize| {
    format!(
      "mod a {{\n    impl X {{\n        fn poll(\n            self: std::pin::Pin<&mut Self>,\n            cx: &mut std::task::Context<i8>,\n{}\n            x\n        }}\n    }}\n}}\n",
      close(n)
    )
  };
  assert!(formatted(&source(92)).contains("> {\n"));
  assert!(formatted(&source(93)).contains(">\n        {\n"));
}

#[test]
fn test_brace_style_always_next_line() {
  let config = FormatConfig {
    brace_next_line: true,
    ..FormatConfig::default()
  };
  let once = format_file("fn f(a: u8) {\n    a;\n}\n", Edition::E2021, &config);
  assert_eq!(once, "fn f(a: u8)\n{\n    a;\n}\n");
}

#[test]
fn test_declarations_keep_a_column_free() {
  // One line at 99 columns; at 100 the return type moves down; past it, parameters do.
  let decl = |n: usize| {
    let head = "    pub fn get_constants(this: &GpuProgrammableStage) -> Option<";
    format!("{}{}>;", head, "x".repeat(n - head.len() - 2))
  };
  let one = format!("extern \"C\" {{\n{}\n}}\n", decl(99));
  assert_eq!(formatted(&one), one);
  assert_eq!(
    formatted(&format!("extern \"C\" {{\n{}\n}}\n", decl(100))),
    format!(
      "extern \"C\" {{\n    pub fn get_constants(this: &GpuProgrammableStage)\n        -> Option<{}>;\n}}\n",
      "x".repeat(34)
    )
  );
  assert_eq!(
    formatted(&format!("extern \"C\" {{\n{}\n}}\n", decl(101))),
    format!(
      "extern \"C\" {{\n    pub fn get_constants(\n        this: &GpuProgrammableStage,\n    ) -> Option<{}>;\n}}\n",
      "x".repeat(35)
    )
  );
}

#[test]
fn test_fields_and_variants_take_a_line_each() {
  assert_eq!(
    formatted("pub struct Config { pub\nquiet: bool, #[a] x:\n  u8 }\n"),
    "pub struct Config {\n    pub quiet: bool,\n    #[a]\n    x: u8,\n}\n"
  );
  // A struct-like variant stays on one line while its fields fit in 35 columns.
  assert_eq!(
    formatted("enum E { A, B(u8, u16), C { x: u8, y: u8 } = 1 }\n"),
    "enum E {\n    A,\n    B(u8, u16),\n    C { x: u8, y: u8 } = 1,\n}\n"
  );
  let wide = "enum E {\n    A {\n        x: u8,\n        y: u8,\n        zzzzzzzzzzzzzzzzzzzz: u8,\n    },\n}\n";
  assert_eq!(formatted(wide), wide);
  // A documented variant makes every struct-like variant one field per line.
  assert_eq!(
    formatted("enum E {\n    /// doc\n    A,\n    B { x: u8 },\n}\n"),
    "enum E {\n    /// doc\n    A,\n    B {\n        x: u8,\n    },\n}\n"
  );
}

#[test]
fn test_headers_join_onto_one_line() {
  assert_eq!(
    formatted("pub\nfn\n  main<T>(\n  a: T,\n) -> u8\n{\n    1\n}\n"),
    "pub fn main<T>(a: T) -> u8 {\n    1\n}\n"
  );
  assert_eq!(
    formatted("extern\n    \"C\"\n{}\npub(crate)\n  struct\nS(u8)\n;\nimpl<T>\n  A for B<T> {}\n"),
    "extern \"C\" {}\npub(crate) struct S(u8);\nimpl<T> A for B<T> {}\n"
  );
}

#[test]
fn test_headers_left_as_written() {
  // A comment among the parameters.
  let kept = "fn f(\n    a: u8, // why\n) {\n}\n";
  assert_eq!(formatted(kept), kept);
}

#[test]
fn test_headers_with_parameter_attributes_and_trailing_comments() {
  // One attribute on a parameter stays on its line; a comment after a declaration's `;` is
  // not part of its header.
  assert_eq!(
    formatted("fn\nf(#[case]\n\n value: T, #[case] tokens: &[Token]) {}\n"),
    "fn f(#[case] value: T, #[case] tokens: &[Token]) {}\n"
  );
  assert_eq!(
    formatted("trait T {\n    fn\n m(self) ->\n Self::Mask; // note\n}\n"),
    "trait T {\n    fn m(self) -> Self::Mask; // note\n}\n"
  );
  // Two attributes on a parameter go on lines of their own, the parameters one per line.
  assert_eq!(
    formatted("fn h(#[a]\n#[b] x: u8) {}\n"),
    "fn h(\n    #[a]\n    #[b]\n    x: u8,\n) {\n}\n"
  );
}

#[test]
fn test_impl_headers_and_where_clauses() {
  // Too wide: `for Type` on the next line one level in, the `{` on a line of its own.
  assert_eq!(
    formatted(
      "impl QueryFragment<Pg, crate::pg::backend::PgOnConflictClause> for ConflictTarget<OnConstraint<'_>> {\n    fn f() {}\n}\n"
    ),
    "impl QueryFragment<Pg, crate::pg::backend::PgOnConflictClause>\n    for ConflictTarget<OnConstraint<'_>>\n{\n    fn f() {}\n}\n"
  );
  // Indented, its line may run past the width by the indentation.
  let indented = "mod m {\n    impl<T: UnwindSafe, F: UnwindSafe + RefUnwindSafe> UnwindSafe for Pool<T, Fxxxxxxxxxx> {}\n}\n";
  assert_eq!(formatted(indented), indented);
  // A blank line between predicates stays before style edition 2024.
  let blank = "fn f<T, U>(x: T) -> u8\nwhere\n    T: Clone,\n\n    U: Copy,\n{\n    1\n}\n";
  assert_eq!(formatted(blank), blank);
}

#[test]
fn test_long_parameter_lists_take_a_line_each() {
  assert_eq!(
    formatted(
      "pub fn get_exclude_for_backend<'a>(backend: &str, metadata: &'a Metadata, other: u8) -> Vec<&'a str> {\n    x\n}\n"
    ),
    "pub fn get_exclude_for_backend<'a>(\n    backend: &str,\n    metadata: &'a Metadata,\n    other: u8,\n) -> Vec<&'a str> {\n    x\n}\n"
  );
}

#[test]
fn test_no_blank_line_between_attributes() {
  // Only between two attributes; after the last, and around doc comments, blank lines stay.
  assert_eq!(
    formatted("#[a]\n\n#[b]\n\nfn f() {}\n\n/// doc\n\nfn g() {}\n"),
    "#[a]\n#[b]\n\nfn f() {}\n\n/// doc\n\nfn g() {}\n"
  );
}

#[test]
fn test_parameters_with_comments_and_long_names() {
  assert_eq!(
    formatted(
      "async fn\n        users_create(State(state): State<AppState>,\t// Make sure\n  // second\n AppJson(params): AppJson<Params>, x: u8) -> u8 {\n    1\n}\n"
    ),
    "async fn users_create(\n    State(state): State<AppState>, // Make sure\n    // second\n    AppJson(params): AppJson<Params>,\n    x: u8,\n) -> u8 {\n    1\n}\n"
  );
  assert_eq!(
    formatted(
      "extern \"C\" {\n    pub  fn  init_storage_event_with_can_bubble_and_cancelable_and_key_and_old_value_and_new_value_and_url(\n    this: &StorageEvent, type_arg: &str);\n}\n"
    ),
    "extern \"C\" {\n    pub fn init_storage_event_with_can_bubble_and_cancelable_and_key_and_old_value_and_new_value_and_url(\n        this: &StorageEvent,\n        type_arg: &str,\n    );\n}\n"
  );
}

#[test]
fn test_trait_bounds() {
  // On the header's line when they fit there, else on the next line together or one per
  // line led by `+`; then `{` on a line of its own; a `where` clause after them.
  assert_eq!(
    formatted(
      "pub trait Extension: std::fmt::Debug + Clone + std::any::Any + Send + Sync + 'static {}\npub trait Parser: FromArgMatches + CommandFactory + Sized {\n    fn a();\n}\npub trait LongTraitNameXxxxxxx: std::fmt::Debug + Clone + std::any::Any + Send + Sync + 'static + Foo {}\npub trait LongTraitNameXxxxxxx: std::fmt::Debug + Clone + std::any::Any + Send + Sync + 'static + Foo + Barrrrrrrrrrrrrrrrrrrrrrrrr + Bazzzzzzzzzzzzzzz {\n    fn a();\n}\ntrait T<A>: Foo<A> where A: Bar {}\nmod m {\n    pub trait LongTraitNameXxxxxxx: std::fmt::Debug + Clone + std::any::Any + Send + Sync + 'static {\n        fn a();\n    }\n}\n"
    ),
    "pub trait Extension: std::fmt::Debug + Clone + std::any::Any + Send + Sync + 'static {}\npub trait Parser: FromArgMatches + CommandFactory + Sized {\n    fn a();\n}\npub trait LongTraitNameXxxxxxx:\n    std::fmt::Debug + Clone + std::any::Any + Send + Sync + 'static + Foo\n{\n}\npub trait LongTraitNameXxxxxxx:\n    std::fmt::Debug\n    + Clone\n    + std::any::Any\n    + Send\n    + Sync\n    + 'static\n    + Foo\n    + Barrrrrrrrrrrrrrrrrrrrrrrrr\n    + Bazzzzzzzzzzzzzzz\n{\n    fn a();\n}\ntrait T<A>: Foo<A>\nwhere\n    A: Bar,\n{\n}\nmod m {\n    pub trait LongTraitNameXxxxxxx:\n        std::fmt::Debug + Clone + std::any::Any + Send + Sync + 'static\n    {\n        fn a();\n    }\n}\n"
  );
}

#[test]
fn test_type_aliases_consts_and_statics_on_one_line() {
  assert_eq!(
    formatted(
      "pub\ntype\nX<T> =\n  Vec<T>;\npub const\nN: usize = 1 +\n  2;\nstatic S: &str =\n\"s\";\n"
    ),
    "pub type X<T> = Vec<T>;\npub const N: usize = 1 + 2;\nstatic S: &str = \"s\";\n"
  );
}

#[test]
fn test_type_and_trait_aliases_broken() {
  // Aliases with const block arguments, trait aliases and associated types with a `where`
  // clause after them, as the standard formatter writes them.
  assert_eq!(
    formatted(
      r#"type A = B<{ N + 1 }>;
type Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa = Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb<{ N + 1 }>;
trait   Foo   =   Bar  +  Baz ;
pub trait Fooooooooooooooooooooooooooooooo<T> = Aaaaaaaaaaaaaaaaaaaaaaaaaaaa + Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb;
"#
    ),
    r#"type A = B<{ N + 1 }>;
type Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa =
    Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb<{ N + 1 }>;
trait Foo = Bar + Baz;
pub trait Fooooooooooooooooooooooooooooooo<T> =
    Aaaaaaaaaaaaaaaaaaaaaaaaaaaa + Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb;
"#
  );
  assert_eq!(
    formatted(
      r#"impl<'a> Row<'a, Pg> for PgRow {
    type Field<'f> = PgField<'f> where 'a: 'f, Self: 'f;
}
pub trait LoadQuery<'query, Conn, U, B = DefaultLoadingMode>: RunQueryDsl<Conn> {
    type RowIter<'conn>: Iterator<Item = QueryResult<U>> where Conn: 'conn;
    type Field<'f>: Field<'f, DB> where 'a: 'f, Self: 'f;
}
"#
    ),
    r#"impl<'a> Row<'a, Pg> for PgRow {
    type Field<'f>
        = PgField<'f>
    where
        'a: 'f,
        Self: 'f;
}
pub trait LoadQuery<'query, Conn, U, B = DefaultLoadingMode>: RunQueryDsl<Conn> {
    type RowIter<'conn>: Iterator<Item = QueryResult<U>>
    where
        Conn: 'conn;
    type Field<'f>: Field<'f, DB>
    where
        'a: 'f,
        Self: 'f;
}
"#
  );
}

#[test]
fn test_types_and_generics_broken_in_headers() {
  // Generic parameters, return types, bounds and field types broken as the standard formatter breaks them.
  assert_eq!(
    formatted(
      r#"fn maybe_grouped<GB>(something: bool) -> Box<dyn BoxableExpression<users::table, Pg, GB, <users::id as ValidGrouping<GB>>::IsAggregate, SqlType = Integer>> {}
fn f() {
    let b: Box<dyn BoxableExpression<users::table, self::multi_connection_impl::MultiBackend, SqlType = _>> = x;
    let c: Box<impl BoxableExpression<users::table, self::multi_connection_impl::MultiBackend, SqlType = _>> = x;
    let d: Box<&BoxableExpression<users::table, self::multi_connection_impl::MultiBackend, SqlType = _, A = B>> = x;
}
"#
    ),
    r#"fn maybe_grouped<GB>(
    something: bool,
) -> Box<
    dyn BoxableExpression<
        users::table,
        Pg,
        GB,
        <users::id as ValidGrouping<GB>>::IsAggregate,
        SqlType = Integer,
    >,
> {
}
fn f() {
    let b: Box<
        dyn BoxableExpression<users::table, self::multi_connection_impl::MultiBackend, SqlType = _>,
    > = x;
    let c: Box<
        impl BoxableExpression<users::table, self::multi_connection_impl::MultiBackend, SqlType = _>,
    > = x;
    let d: Box<
        &BoxableExpression<
            users::table,
            self::multi_connection_impl::MultiBackend,
            SqlType = _,
            A = B,
        >,
    > = x;
}
"#
  );
  assert_eq!(
    formatted(
      r#"fn f() -> impl Fn(AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, BBBBBBBBBBBBBBBBBBBBBBBBBBB) -> CCCCCCCCCCCCCCCCCCCC + Send + 'static {}
fn h<T: Fn(AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, BBBBBBBBBBBBBBBBBBBBBBBBBBB) -> CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC + Send>() {}
pub type BasicClient = oauth2::basic::BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;
"#
    ),
    r#"fn f(
) -> impl Fn(AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, BBBBBBBBBBBBBBBBBBBBBBBBBBB) -> CCCCCCCCCCCCCCCCCCCC
       + Send
       + 'static {
}
fn h<
    T: Fn(
            AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,
            BBBBBBBBBBBBBBBBBBBBBBBBBBB,
        ) -> CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC
        + Send,
>() {
}
pub type BasicClient = oauth2::basic::BasicClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
>;
"#
  );
  assert_eq!(
    formatted(
      r#"pub struct AggregateExpression<Fn, Prefix = NoPrefix, Order = NoOrder, Filter = NoFilter, Window = NoWindow> {
    a: u8,
}
pub struct CombinationClause<Combinator, Rule, Source, Rhs, Order = NoOrderClause, LimitOffset = LimitOffsetClause<NoLimitClause, NoOffsetClause>> {}
pub struct Unit<Combinator, Rule, Source, Rhs, Order = NoOrderClause, LimitOffset = LimitOffsetClauseXXXX>;
enum E<Aaaaaaaaaaaaaaaaaaaaaaaaaa, Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, Cccccccccccccccccccccccccc> where A: B { X }
"#
    ),
    r#"pub struct AggregateExpression<
    Fn,
    Prefix = NoPrefix,
    Order = NoOrder,
    Filter = NoFilter,
    Window = NoWindow,
> {
    a: u8,
}
pub struct CombinationClause<
    Combinator,
    Rule,
    Source,
    Rhs,
    Order = NoOrderClause,
    LimitOffset = LimitOffsetClause<NoLimitClause, NoOffsetClause>,
> {}
pub struct Unit<
    Combinator,
    Rule,
    Source,
    Rhs,
    Order = NoOrderClause,
    LimitOffset = LimitOffsetClauseXXXX,
>;
enum E<Aaaaaaaaaaaaaaaaaaaaaaaaaa, Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, Cccccccccccccccccccccccccc>
where
    A: B,
{
    X,
}
"#
  );
  assert_eq!(
    formatted(
      r#"struct Xxxxxxxxxxxxxxxxxxxxxx<T> where T: Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb {}
enum E<T> where T: Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa {}
"#
    ),
    r#"struct Xxxxxxxxxxxxxxxxxxxxxx<T>
where
    T: Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, {}
enum E<T>
where
    T: Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, {}
"#
  );
  assert_eq!(
    formatted(
      r#"impl<Aaaaaaaaaaaaaaaaaaaaaa, Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, Cccccccccccccccccccccccccc, Ddddddddddddddd> Foo for Bar {}
impl<T, V, QId, Op, Ret, const STATIC_QUERY_ID: bool> DebugQueryHelper<Yes> for DebugQuery<'_, InsertStatement<T, BatchInsert<Vec<ValuesClause<V, T>>, T, QId, STATIC_QUERY_ID>, Op, Ret>, Sqlite> {
    fn f() {}
}
impl<Expr, DB> QueryFragment<DB, crate::backend::sql_dialect::returning_clause::PgLikeReturningClause> for ReturningClause<Expr> {}
mod m {
    unsafe impl<Aaaaaaaaaaaaaaaaaaaaaa, Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, Cccccccccccccccccccccccccc, Ddddd> Foo for Bar {}
}
"#
    ),
    r#"impl<
        Aaaaaaaaaaaaaaaaaaaaaa,
        Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
        Cccccccccccccccccccccccccc,
        Ddddddddddddddd,
    > Foo for Bar
{
}
impl<T, V, QId, Op, Ret, const STATIC_QUERY_ID: bool> DebugQueryHelper<Yes>
    for DebugQuery<
        '_,
        InsertStatement<T, BatchInsert<Vec<ValuesClause<V, T>>, T, QId, STATIC_QUERY_ID>, Op, Ret>,
        Sqlite,
    >
{
    fn f() {}
}
impl<Expr, DB>
    QueryFragment<DB, crate::backend::sql_dialect::returning_clause::PgLikeReturningClause>
    for ReturningClause<Expr>
{
}
mod m {
    unsafe impl<
            Aaaaaaaaaaaaaaaaaaaaaa,
            Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
            Cccccccccccccccccccccccccc,
            Ddddd,
        > Foo for Bar
    {
    }
}
"#
  );
  assert_eq!(
    formatted(
      r#"pub struct Foo(pub HashMap<Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb>);
pub struct Bar(#[serde(skip)] u8, pub(crate) String);
pub struct Baz(Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb);
const X: HashMap<Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb> = HashMap::new();
static CPU_HEAVY_THREAD_POOL: OnceLock<tokio::sync::mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>> = OnceLock::new();
"#
    ),
    r#"pub struct Foo(
    pub  HashMap<
        Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
        Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
    >,
);
pub struct Bar(#[serde(skip)] u8, pub(crate) String);
pub struct Baz(
    Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
    Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
);
const X: HashMap<
    Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
    Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
> = HashMap::new();
static CPU_HEAVY_THREAD_POOL: OnceLock<
    tokio::sync::mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
> = OnceLock::new();
"#
  );
  assert_eq!(
    formatted(
      r#"mod m {
    trait Very_______________________Long__________________Name_______________________________Trait {
        fn method(&self) -> u64;
    }
}
pub trait Something<A, B, C> {}
pub trait Somethingggggggggggggggggggggggggggggggggggg<Aaaaaaaaaaaaaaaaaaaaaaa, Bbbbbbbbbbbbbbbbbbbbbbbbb, C> {
    fn f();
}
"#
    ),
    r#"mod m {
    trait Very_______________________Long__________________Name_______________________________Trait
    {
        fn method(&self) -> u64;
    }
}
pub trait Something<A, B, C> {}
pub trait Somethingggggggggggggggggggggggggggggggggggg<
    Aaaaaaaaaaaaaaaaaaaaaaa,
    Bbbbbbbbbbbbbbbbbbbbbbbbb,
    C,
>
{
    fn f();
}
"#
  );
  assert_eq!(
    formatted(
      r#"pub trait PrettyPrinter<'tcx>: Printer<'tcx, Error = fmt::Error, Path = Self, Region = Self, Type = Self, DynExistential = Self, Const = Self> {
    //
}
pub trait PrettyPrinter<'tcx>: Printer<'tcx, Error = fmt::Error, Path = Self, Region = Self, Type = Self, DynExistential = Self, Const = Self> + fmt::Write {
    //
}
pub trait Foo: Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb + Cccccccccccccccccccccccccc {}
pub trait Foo: Sized + Send {}
mod m {
    pub trait Fooooooooooooooooooooooooo: Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb + Cccccc {
        fn f();
    }
}
"#
    ),
    r#"pub trait PrettyPrinter<'tcx>:
    Printer<
    'tcx,
    Error = fmt::Error,
    Path = Self,
    Region = Self,
    Type = Self,
    DynExistential = Self,
    Const = Self,
>
{
    //
}
pub trait PrettyPrinter<'tcx>:
    Printer<
        'tcx,
        Error = fmt::Error,
        Path = Self,
        Region = Self,
        Type = Self,
        DynExistential = Self,
        Const = Self,
    > + fmt::Write
{
    //
}
pub trait Foo:
    Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb + Cccccccccccccccccccccccccc
{
}
pub trait Foo: Sized + Send {}
mod m {
    pub trait Fooooooooooooooooooooooooo:
        Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb + Cccccc
    {
        fn f();
    }
}
"#
  );
}

#[test]
fn test_what_a_long_header_keeps() {
  // A C variadic takes no comma.
  let long = format!(
    "extern \"C\" {{\n    pub fn sched_setscheduler(pid: crate::pid_t, policy: c_int, param: *const sched_param, {}: u8, ...) -> c_int;\n}}\n",
    "x".repeat(10)
  );
  assert!(formatted(&long).contains("        ...\n    ) -> c_int;"));
}

#[test]
fn test_where_clauses_and_empty_bodies() {
  // `where` on a line of its own, a predicate per line; an `impl` with one predicate and no
  // items on one line when it fits, else with `where` ending its line; `) where` after
  // parameters one per line; blank lines between those kept; an empty body `{}` after a
  // header on one line, `{` and `}` apart after one across lines or an `if` with an `else`;
  // a comment after `{` on the next line.
  assert_eq!(
    formatted(
      "pub fn spawn<F>(func: F) where F: FnOnce() + Send + 'static { a() }\nfn f<T>(x: T) where T: X;\nfn g<T>(x: T) -> u8 where T: X, T: Y;\nimpl<T> X for Y<T> where T: Z { fn a() {} }\nstruct S<T> where T: X { a: T }\nstruct U<T>(T) where T: X;\nenum E<T> where T: X { A(T) }\ntrait Tr<T> where T: X { fn a(); }\nunsafe impl<T> Send for RwLock<T> where T: ?Sized + Send {}\nimpl<T, A: Allocator> Send for RawTable<T, A> where T: Send, A: Send {}\nimpl<Lhs, Rhs, Tab> UndecoratedInsertRecord<Tab> for Option<Eq<Lhs, Rhs>> where Eq<Lhs, Rhs>: UndecoratedInsertRecord<Tab> {}\nfn bbbbbbbbbbbbbbbbbb<T>(aaaaaaaaaaaaaaa: u32, bbbbbbbbbbbbbbbbbbbb: u32, cccccccccccccccccccccc: u32) where T: X { a() }\nfn aaaaaaaaaaaaaaa<T>(aaaaaaaaaaaaaaa: u32, bbbbbbbbbbbbbbbbbbbb: u32, cccccccccccccccccccccc: u32) -> u8 where T: X { a() }\nfn k(\n    aaaaaaaaaaaaaaaaaaaa: u32,\n\n    bbbbbbbbbbbbbbbbbbbbbbbbbbbbb: u32, cccccccccccccccccccccccccccccccccccc: u32,\n) {\n}\nfn short(\n    a: u32,\n) {\n}\nfn m() { // a comment\n    a();\n}\nfn n() {\n    if x { // c\n        a();\n    }\n    if a {} else { b() }\n    if a {} else if c {} else {}\n    while a {\n    }\n    'label: loop {\n        // Just comments\n    }\n}\n"
    ),
    "pub fn spawn<F>(func: F)\nwhere\n    F: FnOnce() + Send + 'static,\n{\n    a()\n}\nfn f<T>(x: T)\nwhere\n    T: X;\nfn g<T>(x: T) -> u8\nwhere\n    T: X,\n    T: Y;\nimpl<T> X for Y<T>\nwhere\n    T: Z,\n{\n    fn a() {}\n}\nstruct S<T>\nwhere\n    T: X,\n{\n    a: T,\n}\nstruct U<T>(T)\nwhere\n    T: X;\nenum E<T>\nwhere\n    T: X,\n{\n    A(T),\n}\ntrait Tr<T>\nwhere\n    T: X,\n{\n    fn a();\n}\nunsafe impl<T> Send for RwLock<T> where T: ?Sized + Send {}\nimpl<T, A: Allocator> Send for RawTable<T, A>\nwhere\n    T: Send,\n    A: Send,\n{\n}\nimpl<Lhs, Rhs, Tab> UndecoratedInsertRecord<Tab> for Option<Eq<Lhs, Rhs>> where\n    Eq<Lhs, Rhs>: UndecoratedInsertRecord<Tab>\n{\n}\nfn bbbbbbbbbbbbbbbbbb<T>(\n    aaaaaaaaaaaaaaa: u32,\n    bbbbbbbbbbbbbbbbbbbb: u32,\n    cccccccccccccccccccccc: u32,\n) where\n    T: X,\n{\n    a()\n}\nfn aaaaaaaaaaaaaaa<T>(\n    aaaaaaaaaaaaaaa: u32,\n    bbbbbbbbbbbbbbbbbbbb: u32,\n    cccccccccccccccccccccc: u32,\n) -> u8\nwhere\n    T: X,\n{\n    a()\n}\nfn k(\n    aaaaaaaaaaaaaaaaaaaa: u32,\n\n    bbbbbbbbbbbbbbbbbbbbbbbbbbbbb: u32,\n    cccccccccccccccccccccccccccccccccccc: u32,\n) {\n}\nfn short(a: u32) {}\nfn m() {\n    // a comment\n    a();\n}\nfn n() {\n    if x {\n        // c\n        a();\n    }\n    if a {\n    } else {\n        b()\n    }\n    if a {\n    } else if c {\n    } else {\n    }\n    while a {}\n    'label: loop {\n        // Just comments\n    }\n}\n"
  );
}
