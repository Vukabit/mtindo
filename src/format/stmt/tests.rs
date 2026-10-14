use crate::format::file::format_file;
use crate::options::format::FormatConfig;
use crate::syntax::Edition;
use crate::verify::format::verify_format;

/// `source` formatted, checked for meaning and stability.
fn formatted(source: &str) -> String {
  formatted_with(source, &FormatConfig::default())
}

/// `source` formatted under `config`, checked for meaning and stability.
fn formatted_with(source: &str, config: &FormatConfig) -> String {
  let once = format_file(source, Edition::E2021, config);
  let found = verify_format(source, &once, Edition::E2021);
  assert!(found.is_empty(), "{:?}\n{}", found, once);
  assert_eq!(
    format_file(&once, Edition::E2021, config),
    once,
    "not stable"
  );
  once
}

#[test]
fn test_a_closure_last_keeps_its_list_on_one_line() {
  // The last link takes a line of its own rather than breaking the closure's list.
  let kept = "impl X {\n    fn split(self) {\n        let update = self\n            .split_count\n            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| c.checked_sub(1));\n    }\n}\n";
  assert_eq!(formatted(kept), kept);
}

#[test]
fn test_block_tails_laid_out_as_statements() {
  // A block's last expression, with no `;`: joined, broken a link per line, inside a
  // closure; a block as the tail is left to the rest of the formatter.
  assert_eq!(
    formatted(
      "fn f() -> Vec<u8> {\n    let a = 1;\n    foo(\n        a, b,\n    )\n}\n\nfn g() -> X {\n    Command::new(\"x\").arg(first_argument_name).arg(second_argument_name).arg(third_argument_name_long)\n}\n\nfn h() {\n    items.iter().for_each(|x| {\n        let y = 1;\n        call(x,\n          y)\n    })\n}\n\nfn k() {\n    b.iter(|| {\n        let p = Owned::new(1).into_shared(guard);\n        unsafe {\n            guard.defer_destroy(p);\n        }\n    });\n}\n"
    ),
    "fn f() -> Vec<u8> {\n    let a = 1;\n    foo(a, b)\n}\n\nfn g() -> X {\n    Command::new(\"x\")\n        .arg(first_argument_name)\n        .arg(second_argument_name)\n        .arg(third_argument_name_long)\n}\n\nfn h() {\n    items.iter().for_each(|x| {\n        let y = 1;\n        call(x, y)\n    })\n}\n\nfn k() {\n    b.iter(|| {\n        let p = Owned::new(1).into_shared(guard);\n        unsafe {\n            guard.defer_destroy(p);\n        }\n    });\n}\n"
  );
}

#[test]
fn test_blocks_of_one_expression() {
  // On one line where an expression goes, and after `unsafe`, `const` or `async` as a
  // statement or tail; a plain block as a statement, a block with attributes and a
  // function's body keep their lines; one too wide holds its expression one level in.
  assert_eq!(
    formatted(
      "fn f() {\n    let a = unsafe {\n        g()\n    };\n    let b = {\n        g()\n    };\n    let d = async move {\n        g()\n    };\n    let l = 'a: {\n        g()\n    };\n    foo(unsafe {\n        g()\n    });\n    unsafe {\n        g()\n    };\n    {\n        g()\n    };\n    let s = S { x: unsafe {\n        g()\n    } };\n    let m = unsafe {\n        g()\n    }.h();\n    let n = unsafe { g(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb) };\n    #[cfg(miri)]\n    unsafe {\n        g()\n    }\n}\n\nfn h() -> u8 {\n    {\n        g()\n    }\n}\n"
    ),
    "fn f() {\n    let a = unsafe { g() };\n    let b = { g() };\n    let d = async move { g() };\n    let l = 'a: { g() };\n    foo(unsafe { g() });\n    unsafe { g() };\n    {\n        g()\n    };\n    let s = S { x: unsafe { g() } };\n    let m = unsafe { g() }.h();\n    let n = unsafe {\n        g(\n            aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n            bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,\n        )\n    };\n    #[cfg(miri)]\n    unsafe {\n        g()\n    }\n}\n\nfn h() -> u8 {\n    {\n        g()\n    }\n}\n"
  );
}

#[test]
fn test_calls_are_joined_and_broken_to_fit() {
  assert_eq!(
    formatted("fn f() {\n    let x = foo(\n        a,\n        b,\n    );\n}\n"),
    "fn f() {\n    let x = foo(a, b);\n}\n"
  );
  let long = "fn f() {\n    call(first_argument_name, second_argument_name, third_argument_name, fourth);\n}\n";
  assert_eq!(
    formatted(long),
    "fn f() {\n    call(\n        first_argument_name,\n        second_argument_name,\n        third_argument_name,\n        fourth,\n    );\n}\n"
  );
}

#[test]
fn test_chains_as_the_standard_formatter_lays_them_out() {
  for kept in [
    // A tuple receiver spanning lines: its links one level in.
    "fn f() {\n    let res = (\n        StatusCode::SEE_OTHER,\n        [(\"location\", \"foo\")],\n        StatusCode::NO_CONTENT,\n    )\n        .into_response();\n}\n",
    // A call receiver spanning lines: its links at its indentation.
    "fn f() {\n    cmd.error(\n        ErrorKind::ArgumentConflict,\n        \"Can't do relative and absolute version change\",\n    )\n    .exit();\n}\n",
    // A chain overflowing an argument list keeps its links on one line.
    "fn f() {\n    let h = tokio::spawn(KEY.scope(\n        42,\n        MyFuture {\n            tx_poll: Some(tx_poll),\n            tx_drop: Some(tx_drop),\n        },\n    ));\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_chains_inside_an_overflowing_chain_stay_on_one_line() {
  // A chain overflowing `Some(...)` keeps every chain inside it on one line: the field's
  // value goes to the next line instead (use_small_heuristics = "Max").
  let max = FormatConfig {
    fn_call_width: 100,
    chain_width: 100,
    struct_lit_width: 100,
    array_width: 100,
    ..FormatConfig::default()
  };
  let kept = "impl X {\n    fn f() {\n        Some(orig_range_with_focus(db, file_id, value.syntax(), value.name()).map(\n            |(FileRange { file_id, range: full_range }, focus_range)| NavigationTarget {\n                file_id,\n                name:\n                    self.name(db).map_or_else(|| sym::underscore.clone(), |it| it.symbol().clone()),\n                alias: None,\n            },\n        ))\n    }\n}\n";
  assert_eq!(
    formatted_with(
      "impl X {\n    fn f() {\n        Some(orig_range_with_focus(db, file_id, value.syntax(), value.name()).map(\n            |(FileRange { file_id, range: full_range }, focus_range)| NavigationTarget {\n                file_id,\n                name: self.name(db).map_or_else(|| sym::underscore.clone(), |it| it.symbol().clone()),\n                alias: None,\n            },\n        ))\n    }\n}\n",
      &max
    ),
    kept
  );
}

#[test]
fn test_closures_with_a_struct_literal_body() {
  // The body breaks after the closure's `|...|` when the call overflows it; a closure whose
  // body is a macro called with nothing takes a line of its own; a chain's last link on its
  // own line indents the body from the link; the arguments break when even `|err| {` does
  // not fit after the chain.
  assert_eq!(
    formatted(
      "fn f() {\n    let x = foo_bar_baz_quux_long_function_name_and_more_more(|it| EvaluatedConst { allocation: it, def: 1 });\n    let matches_x = Arc::new(Pool::with_available_parallelism_capacityxxxxxxxxxxxxxxxxxxxxxxxxxx(|| vec![]));\n    let into_js_generic_impl = if no_into_js_generic {\n        quote! {}\n    } else {\n        let mut clause = declaration_generics\n            .where_clause\n            .clone()\n            .unwrap_or_else(|| syn::WhereClause {\n                where_token: Default::default(),\n                predicates: Default::default(),\n            });\n        clause\n    };\n    Regex::builder().syntax(syntax).configure(config).build(pat).map_err(|err| Error { glob: Some(pat.to_string()), kind: ErrorKind::Regex(err.to_string()) })\n}\n"
    ),
    "fn f() {\n    let x = foo_bar_baz_quux_long_function_name_and_more_more(|it| EvaluatedConst {\n        allocation: it,\n        def: 1,\n    });\n    let matches_x =\n        Arc::new(Pool::with_available_parallelism_capacityxxxxxxxxxxxxxxxxxxxxxxxxxx(|| vec![]));\n    let into_js_generic_impl = if no_into_js_generic {\n        quote! {}\n    } else {\n        let mut clause = declaration_generics\n            .where_clause\n            .clone()\n            .unwrap_or_else(|| syn::WhereClause {\n                where_token: Default::default(),\n                predicates: Default::default(),\n            });\n        clause\n    };\n    Regex::builder()\n        .syntax(syntax)\n        .configure(config)\n        .build(pat)\n        .map_err(|err| Error {\n            glob: Some(pat.to_string()),\n            kind: ErrorKind::Regex(err.to_string()),\n        })\n}\n"
  );
}

#[test]
fn test_generic_arguments_broken_to_overflow() {
  // A path ending in generic arguments too wide for an overflowing call's shape breaks
  // them one per line; a call not overflowed keeps them.
  assert_eq!(
    formatted(
      "fn f() {\n    Err(PathDeserializationError::unsupported_type(type_name::<V::Valueeeeeeeeeee>()));\n    Some(TypeId::of::<AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA>());\n    assert!(query_to_sql_equality::<Range<Int4>, std::ops::RangeInclusive<i32>>(expected, value));\n    assert_eq!(query_id_of(&value), Some(TypeId::of::<TypeParamBorrowedForALifetime<'static, MarkerA>>()));\n}\n"
    ),
    "fn f() {\n    Err(PathDeserializationError::unsupported_type(type_name::<\n        V::Valueeeeeeeeeee,\n    >()));\n    Some(TypeId::of::<\n        AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,\n    >());\n    assert!(query_to_sql_equality::<\n        Range<Int4>,\n        std::ops::RangeInclusive<i32>,\n    >(expected, value));\n    assert_eq!(\n        query_id_of(&value),\n        Some(TypeId::of::<TypeParamBorrowedForALifetime<'static, MarkerA>>())\n    );\n}\n"
  );
}

#[test]
fn test_layouts_keep_every_token() {
  // A float before a method keeps its space; a rest with no base keeps its `..`; generic
  // lists closing together stay as they are.
  for kept in [
    "fn f() {\n    0. .to_string();\n}\n",
    "fn f() {\n    let s = S { x: 5, .. };\n}\n",
    "fn f() {\n    let x = foo::<Vec<u8>>(a, b);\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_let_patterns_and_types() {
  // A struct pattern past `struct_lit_width` takes a field per line, the initializer after
  // its closing brace; a pattern spanning lines leaves the rest of its last line.
  assert_eq!(
    formatted("fn f() {\n    let Tokens { all_columns, all_variants } = generate(&a, kind);\n}\n"),
    "fn f() {\n    let Tokens {\n        all_columns,\n        all_variants,\n    } = generate(&a, kind);\n}\n"
  );
  for kept in [
    "fn f() {\n    let [a, .., b] = f(x);\n}\n",
    "fn f() {\n    let T(a, b) = f(x);\n}\n",
    "fn f() {\n    let (a,) = f(x);\n}\n",
    "fn f() {\n    let S { x, .. }: S = e(1);\n}\n",
    // A tuple pattern, with a path or without, is held to the line width only.
    "fn f() {\n    let (mut p0, mut p1, mut p2, mut p3, mut p4, mut p5, mut p6, mut p7) =\n        (0., 0., 0., 0., 0., 0., 0., 0.);\n}\n",
    "fn f() {\n    let Point(first_coordinate, second_coordinate, third_coordinate, fourth) = p;\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
  // A type is laid out with the rest of the line.
  assert_eq!(
    formatted("fn f() {\n    let x: Vec<u8> = f(\n        a,\n    );\n}\n"),
    "fn f() {\n    let x: Vec<u8> = f(a);\n}\n"
  );
}

#[test]
fn test_macros_laid_out_as_calls_and_arrays() {
  assert_eq!(
    formatted(
      "fn f() {\n    println!(\"name: {:?}\",\ncli.name);\n    let v = vec![1,\n 2,];\n}\n"
    ),
    "fn f() {\n    println!(\"name: {:?}\", cli.name);\n    let v = vec![1, 2];\n}\n"
  );
  // A format-like macro: the format string on a line of its own, the simple rest together.
  assert_eq!(
    formatted(
      "fn f() {\n    println!(\"a very long format string that goes on and on and on {} {}\", first_arg, second_arg);\n}\n"
    ),
    "fn f() {\n    println!(\n        \"a very long format string that goes on and on and on {} {}\",\n        first_arg, second_arg\n    );\n}\n"
  );
  // Inside a macro a written trailing comma stays, one not written is not added.
  assert_eq!(
    formatted(
      "fn f() {\n    m!(a,);\n    m!(alpha_beta_gamma_delta, alpha_beta_gamma_delta, alpha_beta_gamma_delta);\n}\n"
    ),
    "fn f() {\n    m!(a,);\n    m!(\n        alpha_beta_gamma_delta,\n        alpha_beta_gamma_delta,\n        alpha_beta_gamma_delta\n    );\n}\n"
  );
  // A macro last in a call overflows it though an argument holds generics, each argument
  // fitting on its own line.
  assert_eq!(
    formatted(
      "fn f() {\n    out.push_str(\n        &format!(\n            \"\\n| {} |\",\n            row.iter().map(|c| c).collect::<Vec<_>>().join(\" | \")\n        ),\n    );\n}\n"
    ),
    "fn f() {\n    out.push_str(&format!(\n        \"\\n| {} |\",\n        row.iter().map(|c| c).collect::<Vec<_>>().join(\" | \")\n    ));\n}\n"
  );
  // A chain past `chain_width` in an operand breaks for the standard formatter, so what holds
  // it is not joined onto a line.
  let kept = "fn f() {\n    assert!(\n        std::fs::read_to_string(repo.0.join(\"g.txt\"))\n            .unwrap()\n            .matches(\"<<<<<<<\")\n            .count()\n            == 2\n    );\n}\n";
  assert_eq!(formatted(kept), kept);
}

#[test]
fn test_macros_left_as_written() {
  // Arguments rustc would not parse, or that this parser reads differently; braces.
  for kept in [
    "fn f() {\n    let x: Token![,] = y;\n}\n",
    "fn f() {\n    demo!(i <- 0);\n}\n",
    "fn f() {\n    arg!(-a <\"v\"> \"s\");\n}\n",
    "fn f() {\n    m!(a => b);\n}\n",
    "fn f() {\n    m! {  a  };\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_other_statements_on_one_line_when_they_fit() {
  assert_eq!(
    formatted("fn f() {\n    let name =\n    \"my-app\";\n    x\n        += 1;\n    let y;\n}\n"),
    "fn f() {\n    let name = \"my-app\";\n    x += 1;\n    let y;\n}\n"
  );
  // A call inside held to its width keeps the statement as written when it would break.
  let kept = "fn f() {\n    let x = a + call(first_argument_name, second_argument_name, third_argument);\n}\n";
  assert_eq!(formatted(kept), kept);
}

#[test]
fn test_packed_items_count_their_comma() {
  // An item joins a packed line only with its own comma within the width.
  assert_eq!(
    formatted(
      "fn f() {\n    if gnu {\n        cfg.skip_const(|s| {\n            [\n                \"EXTA\", \"EXTB\", \"B0\", \"B1000000\", \"B110\", \"B115200\", \"B1152000\", \"B1200\", \"B134\",\n                \"B14400\", \"B150\", \"B1500000\", \"B153600\", \"B1800\", \"B19200\", \"B200\", \"B2000000\",\n                \"B230400\", \"B2400\", \"B2500000\", \"B28800\", \"B300\", \"B3000000\", \"B307200\", \"B3500000\",\n                \"B38400\", \"B4000000\", \"B460800\",\n            ]\n            .contains(&s.ident())\n        });\n    }\n}\n"
    ),
    "fn f() {\n    if gnu {\n        cfg.skip_const(|s| {\n            [\n                \"EXTA\", \"EXTB\", \"B0\", \"B1000000\", \"B110\", \"B115200\", \"B1152000\", \"B1200\", \"B134\",\n                \"B14400\", \"B150\", \"B1500000\", \"B153600\", \"B1800\", \"B19200\", \"B200\", \"B2000000\",\n                \"B230400\", \"B2400\", \"B2500000\", \"B28800\", \"B300\", \"B3000000\", \"B307200\",\n                \"B3500000\", \"B38400\", \"B4000000\", \"B460800\",\n            ]\n            .contains(&s.ident())\n        });\n    }\n}\n"
  );
}

#[test]
fn test_returns_tries_and_closures() {
  // `return` gets a column less; `f(..)?` lays out the call; a closure stays on one line.
  assert_eq!(
    formatted("fn f() {\n    return Err(\n        g(a),\n    );\n}\n"),
    "fn f() {\n    return Err(g(a));\n}\n"
  );
  assert_eq!(
    formatted(
      "fn f() {\n    let v = g(\n        a,\n    )?;\n    let w = v.iter().map(|x| x + 1).collect();\n}\n"
    ),
    "fn f() {\n    let v = g(a)?;\n    let w = v.iter().map(|x| x + 1).collect();\n}\n"
  );
}

#[test]
fn test_statements_after_their_attributes() {
  // The statement is laid out after its attribute, with no blank line between.
  assert_eq!(
    formatted("fn f() {\n    #[allow(unused_mut)]\n\n    let mut ret: Self =\n        g(x);\n}\n"),
    "fn f() {\n    #[allow(unused_mut)]\n    let mut ret: Self = g(x);\n}\n"
  );
}

/// Statements inside a closure's or a match arm's block are laid out as any statement is:
/// an assignment's sides as a `let`'s (a long chain broken, also behind `*`), a statement
/// after its attributes, and a pattern checked against its width even inside a `match`
/// that ends in `?`.

#[test]
fn test_statements_inside_blocks_in_expressions() {
  let assigned = "fn f() {\n    g(|x| {\n        right_folder = right_folder.consume(counter.element()).consume(counter.element());\n        *seen.lock().unwrap().entry(meta.callsite()).or_insert(0usize) += 1;\n    });\n}\n";
  assert_eq!(
    formatted(assigned),
    "fn f() {\n    g(|x| {\n        right_folder = right_folder\n            .consume(counter.element())\n            .consume(counter.element());\n        *seen\n            .lock()\n            .unwrap()\n            .entry(meta.callsite())\n            .or_insert(0usize) += 1;\n    });\n}\n"
  );
  let attributed = "fn f() {\n    g(|| {\n        #[cfg(x)]\n        insert_into(posts::table).values(&data).execute(&mut conn).unwrap();\n    });\n}\n";
  assert_eq!(
    formatted(attributed),
    "fn f() {\n    g(|| {\n        #[cfg(x)]\n        insert_into(posts::table)\n            .values(&data)\n            .execute(&mut conn)\n            .unwrap();\n    });\n}\n"
  );
  let pattern = "fn f() {\n    let s = match k {\n        A(ref a) => {\n            let ast::Fn {\n                defaultness,\n                ref sig,\n                ident,\n                ref generics,\n                ..\n            } = **a;\n            x\n        }\n    }?;\n}\n";
  assert_eq!(formatted(pattern), pattern);
}

#[test]
fn test_statements_left_to_the_whole_file_rules() {
  // A comment, a block, a macro: kept as written.
  for kept in [
    "fn f() {\n    let x = foo(\n        a, // why\n        b,\n    );\n}\n",
    "fn f() {\n    let x = foo(\n        a,\n        || {\n            b\n        },\n    );\n}\n",
    "fn f() {\n    let x = foo(\n        a,\n        m! { b },\n    );\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

/// A statement holding one block laid out to the block's `{`, the block's insides on their
/// lines and its tail (`})?;`) after the `}`, a trailing comma dropped.

#[test]
fn test_statements_up_to_their_block() {
  for (source, expected) in [
    (
      "fn f() {\n    let v = items\n        .iter()\n        .map(|x| {\n            a(x);\n            b(x)\n        });\n}\n",
      "fn f() {\n    let v = items.iter().map(|x| {\n        a(x);\n        b(x)\n    });\n}\n",
    ),
    (
      "fn f() {\n    let r = run(\n        &cx,\n        |cx| {\n            a(cx);\n            b(cx)\n        },\n    )?;\n}\n",
      "fn f() {\n    let r = run(&cx, |cx| {\n        a(cx);\n        b(cx)\n    })?;\n}\n",
    ),
    (
      "fn f() {\n    CURRENT.with(\n        |Data {\n             ctx,\n             wake,\n         }| {\n            a(ctx);\n            b(wake);\n        },\n    );\n}\n",
      "fn f() {\n    CURRENT.with(|Data { ctx, wake }| {\n        a(ctx);\n        b(wake);\n    });\n}\n",
    ),
    (
      "fn f() {\n    g(\n        |x| { a(x); b(x) },\n    );\n}\n",
      "fn f() {\n    g(|x| {\n        a(x);\n        b(x)\n    });\n}\n",
    ),
  ] {
    assert_eq!(formatted(source), expected);
  }
  // Kept as the standard formatter writes them: a chain past `chain_width`, a closure after
  // another among the arguments, a block followed by `.await`.
  for kept in [
    "fn f() {\n    let symbols = sema\n        .find_namelike_at_offset_with_descend(syntax, offset)\n        .map(|name_like| {\n            a(name_like);\n            b(name_like)\n        });\n}\n",
    "fn f() {\n    delimited(\n        p,\n        || \"expected\".into(),\n        |p| {\n            a(p);\n            b(p)\n        },\n    );\n}\n",
    "async fn f() {\n    LocalSet::new()\n        .run_until(async {\n            a().await;\n        })\n        .await;\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_struct_literals_keep_a_blank_line_between_fields() {
  // One blank line after a field's comma stays, before a field or the rest; not before `}`,
  // and not between call arguments.
  assert_eq!(
    formatted(
      "fn f() {\n    let x = S {\n        alpha_field_name: 1,\n\n        beta_field_name: 2, gamma_field_name: 3,\n\n\n        delta: 4,\n\n    };\n    let x = S {\n        alpha_field_name: 1,\n        beta_field_name: 2,\n        gamma_field_name: 3,\n\n        ..Default::default()\n    };\n    call(\n        alpha_argument_number_one,\n\n        beta_argument_number_two, gamma_argument_number_three_long,\n    );\n}\n"
    ),
    "fn f() {\n    let x = S {\n        alpha_field_name: 1,\n\n        beta_field_name: 2,\n        gamma_field_name: 3,\n\n        delta: 4,\n    };\n    let x = S {\n        alpha_field_name: 1,\n        beta_field_name: 2,\n        gamma_field_name: 3,\n\n        ..Default::default()\n    };\n    call(\n        alpha_argument_number_one,\n        beta_argument_number_two,\n        gamma_argument_number_three_long,\n    );\n}\n"
  );
}

#[test]
fn test_what_the_standard_formatter_leaves_as_written() {
  // Ranges chained in a macro (a pattern), a struct literal past struct_lit_width among
  // binary operands, a `const` block a chain starts with, and a binary field value it
  // breaks at its operator.
  let kept = "fn f() {\n    let a =\n        matches!(byte, b'*' | b'-' | b'.' | b'0' ..= b'9' | b'A' ..= b'Z' | b'_' | b'a' ..= b'z');\n    let b = matches!(\n        self.kind,\n        Length(0)\n            | Chunked {\n                state: ChunkedState::End,\n                ..\n            }\n            | Eof(true)\n    );\n    const {\n        S::new(\n            \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\n        )\n    }\n    .go();\n    let x = S {\n        searches_with_match: self.searches_with_match_xxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n            + rhs.searches_with_match,\n    };\n}\n";
  assert_eq!(formatted(kept), kept);
}

#[test]
fn test_width_settings_reach_the_layout() {
  // At the defaults a 66-column argument list breaks (fn_call_width is 60); with
  // use_small_heuristics = "Max" it may run to the line width.
  let source =
    "fn f() {\n    call(first_argument_name, second_argument_name, third_argument, fourth);\n}\n";
  let broken = format_file(source, Edition::E2021, &FormatConfig::default());
  assert!(broken.contains("call(\n"), "{}", broken);
  let max = FormatConfig {
    fn_call_width: 100,
    chain_width: 100,
    struct_lit_width: 100,
    array_width: 100,
    ..FormatConfig::default()
  };
  assert_eq!(format_file(source, Edition::E2021, &max), source);
}
