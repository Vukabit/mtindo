use super::*;
use crate::verify::format::verify_format;

fn formatted(source: &str) -> String {
  formatted_with(source, &FormatConfig::default())
}

fn formatted_with(source: &str, config: &FormatConfig) -> String {
  let config = config.clone();
  let once = format_file(source, Edition::E2021, &config);
  // Every output keeps the program's meaning and is stable.
  let found = verify_format(source, &once, Edition::E2021);
  assert!(found.is_empty(), "{:?}\n{}", found, once);
  assert_eq!(
    format_file(&once, Edition::E2021, &config),
    once,
    "not stable"
  );
  once
}

/// The width set with every heuristic at it, as `use_small_heuristics = "Max"` sets them.
fn max_heuristics(max_width: usize) -> FormatConfig {
  FormatConfig {
    style_edition: 2024,
    max_width,
    fn_call_width: max_width,
    chain_width: max_width,
    struct_lit_width: max_width,
    struct_variant_width: max_width,
    array_width: max_width,
    attr_fn_like_width: max_width,
    single_line_if_else_max_width: max_width,
    single_line_let_else_max_width: max_width,
    ..FormatConfig::default()
  }
}

/// A method chain overflowing a call keeps every chain inside it on one line, its closure's
/// `match` included, up to a block's insides: a scrutinee or an arm that must break then
/// puts the closure's body in a block.
#[test]
fn test_a_closure_match_whose_chain_must_break_inside_an_overflowing_chain_takes_a_block() {
  assert_eq!(
    formatted(
      "impl X {\n    fn new_coroutine_witness_for_coroutine() -> Self {\n        let coroutine_args = interner.mk_args_from_iter(coroutine_args.iter().map(|arg| match arg.kind() {\n                GenericArgKind::Type(_) | GenericArgKind::Const(_) => arg,\n                GenericArgKind::Lifetime(_) => {\n                    crate::next_solver::Region::new(interner, rustc_type_ir::RegionKind::ReErased)\n                        .into()\n                }\n            }));\n    }\n}\n"
    ),
    "impl X {\n    fn new_coroutine_witness_for_coroutine() -> Self {\n        let coroutine_args = interner.mk_args_from_iter(coroutine_args.iter().map(|arg| {\n            match arg.kind() {\n                GenericArgKind::Type(_) | GenericArgKind::Const(_) => arg,\n                GenericArgKind::Lifetime(_) => {\n                    crate::next_solver::Region::new(interner, rustc_type_ir::RegionKind::ReErased)\n                        .into()\n                }\n            }\n        }));\n    }\n}\n"
  );
}

#[test]
fn test_a_closure_whose_head_leaves_no_room_is_kept_in_a_block() {
  // `|SSSS:f|o` at the width exactly: the standard formatter gives up on it there and
  // keeps it as written, in the block it puts it in, so the operation breaks before it.
  let config = FormatConfig {
    max_width: 21,
    tab_spaces: 2,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with("fn t() {\nS | R || || || || |SSSS:f|o\n}\n", &config),
    "fn t() {\n  S | R\n    || || {\n      || {\n        || {\n          |SSSS:f|o\n        }\n      }\n    }\n}\n"
  );
}

#[test]
fn test_a_closure_with_a_let_body_keeps_it_out_of_a_block() {
  // In a block the `let` would be a statement.
  let config = FormatConfig {
    max_width: 47,
    tab_spaces: 2,
    ..FormatConfig::default()
  };
  formatted_with("fn t(){||let x =d||1}\n", &config);
}

/// `||` overflowing a call needs a column more than `|| {` and the one after it: with only
/// those left on the line, the closure goes on a line of its own and the chain stays on
/// the `let`'s line.
#[test]
fn test_a_closure_with_no_room_for_its_empty_parameters_is_not_overflowed() {
  let source = "impl X {\n    pub(crate) fn index_read(&self) -> anyhow::Result<grep_index::Index> {\n        let index = grep_index::IndexDiscovery::new().discover()?.ok_or_else(\n            || anyhow::anyhow!(\"failed to find an index to read\"),\n        )?;\n        Ok(index)\n    }\n}\n";
  assert_eq!(formatted_with(source, &max_heuristics(79)), source);
}

#[test]
fn test_a_comment_after_an_attribute_across_lines_moves_to_its_indentation() {
  let config = FormatConfig {
    max_width: 43,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "mod m {\n    #![f(\n     |B)] // Mirnoti xxxxxxxxx Wt cf c i t r xxxxxxxxxxx\n    #![c]\n}\n",
      &config
    ),
    "mod m {\n    #![f(\n     |B)]\n    // Mirnoti xxxxxxxxx Wt cf c i t r xxxxxxxxxxx\n    #![c]\n}\n"
  );
}

/// A chain's last link that fits on the chain's first line but cannot overflow there is
/// laid out in the chain's own shape, its block indented from the chain's indentation
/// though the link goes on a line of its own, as the standard formatter writes it.
#[test]
fn test_a_last_link_that_cannot_overflow_is_laid_out_from_the_chains_indentation() {
  assert_eq!(
    formatted_with(
      "fn f() {\n    let index = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa::new().discover()?.ok_or_else(\n        move || anyhow::anyhow!(\"failed to find an index to read\"),\n    )?;\n}\n",
      &max_heuristics(79)
    ),
    "fn f() {\n    let index = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa::new()\n        .discover()?\n        .ok_or_else(move || {\n        anyhow::anyhow!(\"failed to find an index to read\")\n    })?;\n}\n"
  );
}

/// A link's arguments holding a line past the width, laid out on the next line, are still
/// weighed there; the chain then fails as a whole, so it stays on the line it is on.
#[test]
fn test_a_link_with_a_line_past_the_width_fails_the_chain_not_the_link() {
  let source = "fn f() {\n    {\n        {\n            {\n                {\n                    _ = ctx.probe_all(method_resolution::Mode::MethodCall, self_ty).try_for_each(\n                        |candidate| {\n                            let id = Function { id: AnyFunctionId::FunctionId(id) };\n                            match candidate.candidate.kind {\n                                method_resolution::CandidateKind::InherentImplCandidate {\n                                    ..\n                                }\n                                | method_resolution::CandidateKind::ObjectCandidate(..)\n                                | method_resolution::CandidateKind::WhereClauseCandidate(..) => {\n                                    callback.on_inherent_method(id)\n                                }\n                                method_resolution::CandidateKind::TraitCandidate(..) => {\n                                    callback.on_trait_method(id)\n                                }\n                            }\n                        },\n                    );\n                }\n            }\n        }\n    }\n}\n";
  assert_eq!(formatted_with(source, &max_heuristics(100)), source);
}

/// `|mut name|` too wide for the parameters' room breaks after `mut`, and a closure whose
/// parameters span lines does not overflow: the chain takes the next line instead.
#[test]
fn test_a_mut_parameter_with_no_room_keeps_its_closure_from_overflowing() {
  let source = "fn f() {\n    {\n        {\n            let trait_gen_args =\n                old_impl.trait_()?.generic_arg_list().and_then(|mut trait_args| {\n                    let trait_args = &mut trait_args;\n                    if let Some(new_args) = transform_impl(\n                        ctx,\n                        ast_strukt,\n                        &old_impl,\n                        &transform_args,\n                        trait_args.clone(),\n                    ) {\n                        *trait_args = new_args.clone();\n                        Some(new_args)\n                    } else {\n                        None\n                    }\n                });\n        }\n    }\n}\n";
  assert_eq!(formatted_with(source, &max_heuristics(100)), source);
}

/// A struct pattern's field takes no room for its comma, which may then pass the line
/// width, and its pattern is laid out in the field's whole shape before it is placed after
/// the name.
#[test]
fn test_a_pattern_field_takes_no_room_for_its_comma() {
  let arm = "fn f() {\n    {\n        {\n            {\n                {\n                    {\n                        match *child_op {\n                            Op::Repeat {\n                                kind: parser::RepeatKind::ZeroOrMore | parser::RepeatKind::ZeroOrOne,\n                                ..\n                            } => true,\n                            _ => false,\n                        }\n                    }\n                }\n            }\n        }\n    }\n}\n";
  assert_eq!(formatted_with(arm, &max_heuristics(100)), arm);
  let nested = "fn f() {\n    let bidirectional::ExpandMacro {\n        lib,\n        env,\n        current_dir,\n        data:\n            bidirectional::ExpandMacroData {\n                macro_body,\n                macro_name,\n                attributes,\n                has_global_spans: bidirectional::ExpnGlobals { def_site, call_site, mixed_site, .. },\n                span_data_table,\n            },\n    } = task;\n}\n";
  assert_eq!(formatted_with(nested, &max_heuristics(100)), nested);
}

#[test]
fn test_an_arm_body_too_wide_for_its_line_goes_in_a_block() {
  // A `match` whose head does not fit after `=>`, or would span more lines there than on
  // the next line: in a block of its own on the next line.
  let narrow = FormatConfig {
    max_width: 21,
    tab_spaces: 2,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "const S: T = match b {\nFFFFFFFFFFFFF => match a {\n_ => a,\n},\n_ => a,\n};\n",
      &narrow
    ),
    "const S: T = match b\n{\n  FFFFFFFFFFFFF => {\n    match a {\n      _ => a,\n    }\n  }\n  _ => a,\n};\n"
  );
  let config = FormatConfig {
    max_width: 60,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "fn f() {\n    match x {\n        (None, Some(t_cast_cast_cast)) => match self.expr_ty.kind() {\n            TyKind::FnDef(..) => {\n                a\n            }\n            _ => b,\n        },\n        _ => c,\n    }\n}\n",
      &config
    ),
    "fn f() {\n    match x {\n        (None, Some(t_cast_cast_cast)) => {\n            match self.expr_ty.kind() {\n                TyKind::FnDef(..) => a,\n                _ => b,\n            }\n        }\n        _ => c,\n    }\n}\n"
  );
}

/// An arm whose pattern and guard leave no room for ` => ` (a guard kept as written past
/// the width): its block starts the next line, as the standard formatter lays it out.
#[test]
fn test_an_arm_with_no_room_for_its_arrow_opens_its_block_below() {
  assert_eq!(
    formatted(
      "fn f() {\n    match x {\n        TypeRef::Reference(ref_) if matches!(&data.store[ref_.ty], TypeRef::Path(p) if p.is_self_type()) => {\n            a();\n            b();\n        }\n        _ => {}\n    }\n}\n"
    ),
    "fn f() {\n    match x {\n        TypeRef::Reference(ref_) if matches!(&data.store[ref_.ty], TypeRef::Path(p) if p.is_self_type()) =>\n        {\n            a();\n            b();\n        }\n        _ => {}\n    }\n}\n"
  );
}

/// An async block that does not fit on one line where it stands, but whose plain block
/// fits the width less 6 counted from no column: the standard formatter comes back with it
/// on one line all the same, which an argument list then lays out vertically; a longer one
/// overflows as usual.
#[test]
fn test_an_async_block_almost_fitting_goes_on_a_line_of_its_own() {
  let call = |n: usize| {
    format!(
      "fn f() {{\n    loop {{\n        loop {{\n            loop {{\n                loop {{\n                    spawn_local(async move {{ {}.await }});\n                }}\n            }}\n        }}\n    }}\n}}\n",
      "a".repeat(n)
    )
  };
  let out = formatted(&call(48));
  assert!(
    out.contains("spawn_local(\n                        async move {"),
    "{}",
    out
  );
  let out = formatted(&call(51));
  assert!(out.contains("spawn_local(async move {\n"), "{}", out);
}

#[test]
fn test_arm_comments_aligned() {
  // Comments after arms aligned in their group, an arm's comma counted in its width; one
  // after a block's `}` one space after it.
  assert_eq!(
    formatted(
      "fn f() {\n    match x {\n        A => {\n            a();\n        }\t// Pop\n        EMPTY => return,  // w\n        NOTIFIED => return, // u\n        PARKED => {}   // g\n        _ => panic!(),\n    }\n}\n"
    ),
    "fn f() {\n    match x {\n        A => {\n            a();\n        } // Pop\n        EMPTY => return,    // w\n        NOTIFIED => return, // u\n        PARKED => {}        // g\n        _ => panic!(),\n    }\n}\n"
  );
}

#[test]
fn test_arms_keep_their_doc_comments() {
  let config = FormatConfig {
    tab_spaces: 7,
    max_width: 101,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "fn o() {match(){\n/// d\n#[a]\n/// e\n()=>o,  //\n /** x */\n_ => {}\n }}\n",
      &config
    ),
    "fn o() {\n       match () {\n              /// d\n              #[a]\n              /// e\n              () => o, //\n              /** x */\n              _ => {}\n       }\n}\n"
  );
}

#[test]
fn test_attribute_arguments_and_doc_comments() {
  // Keywords are names among an attribute's arguments, `true` a value; a doc comment after
  // an attribute starts its own line.
  assert_eq!(
    formatted(
      "#[salsa::tracked(lru\n\n=\t512,\treturns(ref))]\n#[t(a\n=\ntrue)] /// Doc.\nfn f() {}\n"
    ),
    "#[salsa::tracked(lru = 512, returns(ref))]\n#[t(a = true)]\n/// Doc.\nfn f() {}\n"
  );
  // Attributes a `rustfmt::skip::attributes` names are left as written.
  let skipped =
    "#[rustfmt::skip::attributes(other)]\nfn main() {\n    #[other(a,\nb)]\n    struct S {}\n}\n";
  assert_eq!(formatted(skipped), skipped);
}

#[test]
fn test_attributes_extern_crates_and_macro_calls() {
  // Inner attributes in a row with no blank line between; a `use`'s and an `extern crate`'s
  // attributes joined to it, another item's keeping a blank line; `extern crate` on one
  // line; `name! {` one space apart, `name!(` and `name![` touching; `vec![x; n]` on one
  // line.
  assert_eq!(
    formatted(
      "#![feature(test)]\n\n#![warn(rust_2018_idioms)]\n\n// c\n#![allow(x)]\n\n#[macro_use]\n\nextern\n    crate bitflags;\nextern  crate  alloc as std2 ;\n#[cfg(x)]\n\nuse a::b;\n#[derive(Debug)]\n\nstruct S;\nbitflags!\n{ pub struct A: u8 {} }\nfoo!\n(a, b);\nbar!   [1];\nfn f() {\n    m!\n    { x };\n    let x = n!\n    (1);\n    let vec\n=\nvec![33u8;\n    1024];\n}\n"
    ),
    "#![feature(test)]\n#![warn(rust_2018_idioms)]\n// c\n#![allow(x)]\n\n#[macro_use]\nextern crate bitflags;\nextern crate alloc as std2;\n#[cfg(x)]\nuse a::b;\n#[derive(Debug)]\n\nstruct S;\nbitflags! { pub struct A: u8 {} }\nfoo!(a, b);\nbar![1];\nfn f() {\n    m! { x };\n    let x = n!(1);\n    let vec = vec![33u8; 1024];\n}\n"
  );
}

#[test]
fn test_blank_lines_and_trailing_whitespace() {
  assert_eq!(
    formatted("\n\nfn a() {   \n\n\n    x();  \n\n}\n\n\n\nfn b() {} // note   \n\n\n"),
    "fn a() {\n    x();\n}\n\nfn b() {} // note\n"
  );
}

#[test]
fn test_blank_lines_between_variant_fields() {
  // A blank line between a struct-like variant's fields stays.
  assert_eq!(
    formatted(
      "enum E {\n    Item {\n        meta: Vec<syn::Attribute>,\n\n        vis: syn::Visibility,\n        tail: proc_macro2::TokenStream,\n    },\n    A { x: u8,\n\n y: u8 },\n}\n"
    ),
    "enum E {\n    Item {\n        meta: Vec<syn::Attribute>,\n\n        vis: syn::Visibility,\n        tail: proc_macro2::TokenStream,\n    },\n    A {\n        x: u8,\n\n        y: u8,\n    },\n}\n"
  );
}

#[test]
fn test_blocks_standing_as_statements_and_commented_chains() {
  // A block standing as a statement, with attributes too, is on lines around a statement given
  // up on; an `if` chain with a commented block is on lines throughout; a block's last
  // expression no layout fits is left as written; a chain after a block keeps its links.
  let c2021 = FormatConfig::default();
  let c2024 = FormatConfig {
    style_edition: 2024,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "fn f() {\n    let _p = 1;\n\n#[cfg(x)]\n\n{\tErr(anyhow::anyhow!(\n\"Memory profiling.\\n\\n\\\n            To build rust-analyzer with profiling support, pass `--features dhat --profile dev-rel` to `cargo build`\n            when.\"\n\n))\n    }\n    foo();\n}\n",
      &c2021
    ),
    "fn f() {\n    let _p = 1;\n\n    #[cfg(x)]\n    {\n        Err(anyhow::anyhow!(\n\"Memory profiling.\\n\\n\\\n            To build rust-analyzer with profiling support, pass `--features dhat --profile dev-rel` to `cargo build`\n            when.\"\n\n))\n    }\n    foo();\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let _p = 1;\n\n\n{\tErr(anyhow::anyhow!(\n\"Memory profiling.\\n\\n\\\n            To build rust-analyzer with profiling support, pass `--features dhat --profile dev-rel` to `cargo build`\n            when.\"\n\n))\n    }\n    foo();\n}\n",
      &c2021
    ),
    "fn f() {\n    let _p = 1;\n\n    {\n        Err(anyhow::anyhow!(\n\"Memory profiling.\\n\\n\\\n            To build rust-analyzer with profiling support, pass `--features dhat --profile dev-rel` to `cargo build`\n            when.\"\n\n))\n    }\n    foo();\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let x = g.map(|t| {\n        if a {\n            c\n            // underscore\n        } else if b { e } else { d }\n    });\n    let x = g.map(|t| {\n        if a {\n            c\n            // underscore\n        } else if b {\n            quote!{ _\n    } } else { d }\n    });\n}\n",
      &c2021
    ),
    "fn f() {\n    let x = g.map(|t| {\n        if a {\n            c\n            // underscore\n        } else if b {\n            e\n        } else {\n            d\n        }\n    });\n    let x = g.map(|t| {\n        if a {\n            c\n            // underscore\n        } else if b {\n            quote! { _\n            }\n        } else {\n            d\n        }\n    });\n}\n"
  );
  assert_eq!(
    formatted_with(
      "impl X {\n    fn deserialize_reader<R: borsh::io::Read>(reader: &mut R) -> Serial<Self> {\n    let\n    length\n=\n\tu64::deserialize_reader(reader)?;\trepeat_with(||\nItem::deserialize_reader(reader))\n.take(length.try_into().map_err(|_|\nError::new(  ErrorKind::OutOfMemory,\n\n\"Cannot deserialize a sequence with more than usize::MAX elements in this machine\"\n))?)\t.collect()  }\n}\n",
      &c2024
    ),
    "impl X {\n    fn deserialize_reader<R: borsh::io::Read>(reader: &mut R) -> Serial<Self> {\n        let length = u64::deserialize_reader(reader)?;\n        repeat_with(||\nItem::deserialize_reader(reader))\n.take(length.try_into().map_err(|_|\nError::new(  ErrorKind::OutOfMemory,\n\n\"Cannot deserialize a sequence with more than usize::MAX elements in this machine\"\n))?)\t.collect()\n    }\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let x = (||\n        {\n        tt\n        = // FIXME\n\tfoo(1)?;\n        Ok(1)\n    })()\n    .map_err(expect_literal).foo;\n    (|| {\n        tt\n        = // FIXME\n\tfoo(1)?;\n        Ok(1)\n    })().map_err(expect_literal)?.foo();\n}\n",
      &c2024
    ),
    "fn f() {\n    let x = (|| {\n        tt\n        = // FIXME\n\tfoo(1)?;\n        Ok(1)\n    })()\n    .map_err(expect_literal)\n    .foo;\n    (|| {\n        tt\n        = // FIXME\n\tfoo(1)?;\n        Ok(1)\n    })()\n    .map_err(expect_literal)?\n    .foo();\n}\n"
  );
  // A block a loop iterates over or tests is not its body: on one line where it fits.
  assert_eq!(
    formatted(
      "fn f() {\n    for site in unsafe { std::slice::from_raw_parts(chunk, CHUNK) } {\n        site.live.store(0, Ordering::Relaxed);\n    }\n    while {\n        a\n    } {\n        b;\n    }\n}\n"
    ),
    "fn f() {\n    for site in unsafe { std::slice::from_raw_parts(chunk, CHUNK) } {\n        site.live.store(0, Ordering::Relaxed);\n    }\n    while { a } {\n        b;\n    }\n}\n"
  );
}

#[test]
fn test_braced_macro_line_inside_a_string_not_measured() {
  // A braced call's last line starting inside a string spanning lines is kept as written
  // and not measured: measured, its column 0 would pull the other lines in again on every
  // run.
  assert_eq!(
    formatted("fn f() {\n    let e = vec!{\n        #[a\n    b = \"x\n}\n)\"]\n};\n}\n"),
    "fn f() {\n    let e = vec! {\n            #[a\n        b = \"x\n}\n)\"]\n    };\n}\n"
  );
}

#[test]
fn test_braced_macros_holding_strings_across_lines() {
  // Re-indented around the string: a line continuing it after `\` moves, one starting
  // inside it otherwise is the string's own and stays.
  assert_eq!(
    formatted(
      "cfg_if!\n\t{\n\tif #[cfg(x)] {\n            compile_error!(\n                \"one \\\n            two\"  );\n}\n}\nfn g() {\n  m! {\n     let s = \"a\n  b\";\n        x\n  }\n}\n"
    ),
    "cfg_if! {\n    if #[cfg(x)] {\n            compile_error!(\n                \"one \\\n            two\"  );\n}\n}\nfn g() {\n    m! {\n       let s = \"a\n  b\";\n          x\n    }\n}\n"
  );
}

#[test]
fn test_chain_comments_postfix_use_and_type_alias_where_comments() {
  // Comments among a chain's `?`s go after them all, the first on their line; comments on
  // one line keep the space between them; a block comment across lines keeps its layout
  // from the links' column; a tuple field after a tuple field keeps its space on a line of
  // its own before the 2024 style; `.use` is a link; a block comment holding an empty
  // line keeps its layout.
  let c2021 = FormatConfig::default();
  let c2024 = FormatConfig {
    style_edition: 2024,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "fn foo() {\n    let x = foo\n        .bar? ?? // comment\n    // comment\n        ? ??\n    // comment\n        ?  ??\n    // comment\n        ???  \n    // comment\n        ? ? ?\n        .baz;\n\n    parent /* comment */ .child;\n    parent /* comment1 */     /* comment2 */ .child;\n    parent /* comment1 */   // comment 2\n            .child;\n    parent /* ???no tries here */ // nor ??? here ????\n    ???.child;\n    parent/*\n        some interesting shaped comment\n    */.child;\n\n    parent /* comment */\n    // another comment\n        .child1(some, args) /* comment */ // again\n    .await /* longer\n         comment\n    */ // more comments!\n        .use // here's a comment that reaches right to the default width limit aaaaaaaaaaaaaaaaaaaa\n    .end;\n\n    parent? /* spacing is interesting */    ?   /* blah */.child;\n\n    // NB: recording the current state of things\n    // but is possibly a bug: issue 6433\n    parent //comment\n           .1 .2 .3;\n}\n",
      &c2021
    ),
    "fn foo() {\n    let x = foo\n        .bar??????????????? // comment\n        // comment\n        // comment\n        // comment\n        // comment\n        .baz;\n\n    parent /* comment */\n        .child;\n    parent /* comment1 */     /* comment2 */\n        .child;\n    parent /* comment1 */   // comment 2\n        .child;\n    parent??? /* ???no tries here */ // nor ??? here ????\n        .child;\n    parent /*\n            some interesting shaped comment\n        */\n        .child;\n\n    parent /* comment */\n        // another comment\n        .child1(some, args) /* comment */ // again\n        .await /* longer\n             comment\n        */ // more comments!\n        .use // here's a comment that reaches right to the default width limit aaaaaaaaaaaaaaaaaaaa\n        .end;\n\n    parent?? /* spacing is interesting */       /* blah */\n        .child;\n\n    // NB: recording the current state of things\n    // but is possibly a bug: issue 6433\n    parent //comment\n        .1\n         .2\n         .3;\n}\n"
  );
  assert_eq!(
    formatted_with(
      "macro_rules! foo {\n    () => {\n        // comment\n        /*\n\n        */\n    };\n}\nfn f() {\n    let x =   a.b  .use .c;\n}\n",
      &c2024
    ),
    "macro_rules! foo {\n    () => {\n        // comment\n        /*\n\n        */\n    };\n}\nfn f() {\n    let x = a.b.use.c;\n}\n"
  );
  // A type alias's `where` clauses with line comments: those before `where` and after the
  // predicates on lines of their own, `where` alone, those after it one level in.
  assert_eq!(
    formatted_with(
      "type Foo // c1\nwhere\nA: B,\n= E;\n\ntype Foo\nwhere // c2\nA: B,\n= E;\n\ntype Foo<T>\nwhere\nA: B, // c3\n= E;\n\ntype Foo\nwhere\nA: B,\n// c4\n= E;\n\ntype Foo = E // c5\nwhere A: B;\n\ntype Foo = E\nwhere A: B, // c6\nC: D;\n\nimpl X {\n    type Foo // c1\n    where\n    A: B, // c3\n    // c4\n    = E;\n}\n\ntype Foo /* b1 */ where A: B = E;\n",
      &c2021
    ),
    "type Foo\n// c1\nwhere\n    A: B,\n= E;\n\ntype Foo\nwhere\n    // c2\n    A: B,\n= E;\n\ntype Foo<T>\nwhere\n    A: B, // c3\n= E;\n\ntype Foo\nwhere\n    A: B,\n// c4\n= E;\n\ntype Foo\n    = E\n// c5\nwhere\n    A: B;\n\ntype Foo\n    = E\nwhere\n    A: B, // c6\n    C: D;\n\nimpl X {\n    type Foo\n    // c1\n    where\n        A: B, // c3\n    // c4\n    = E;\n}\n\ntype Foo\n/* b1 */\nwhere\n    A: B,\n= E;\n"
  );
  assert_eq!(
    formatted_with(
      "type Foo = E where A: B;\ntype Foo where A: B = E;\ntype Foo<T> = E where A: B, C: D;\n",
      &c2021
    ),
    "type Foo\n    = E\nwhere\n    A: B;\ntype Foo\nwhere\n    A: B,\n= E;\ntype Foo<T>\n    = E\nwhere\n    A: B,\n    C: D;\n"
  );
}

#[test]
fn test_closure_block_where_no_room_is_left() {
  // A closure written with a block of one expression, with no room after it on its line,
  // is laid out as one written without its block is: stable. (Found by the generated
  // fuzz target.)
  let config = FormatConfig {
    max_width: 21,
    tab_spaces: 1,
    ..FormatConfig::default()
  };
  let once = formatted_with(
    "fn f() {\n let x: HashMap<u8, char> = get()[|value| \"with {} braces\"];\n}\n",
    &config,
  );
  assert_eq!(
    once,
    "fn f() {\n let x: HashMap<\n  u8,\n  char,\n > = get()[|value| {\n  \"with {} braces\"\n }];\n}\n"
  );
}

#[test]
fn test_comments_after_a_trailing_comment_and_before_a_brace() {
  // Comment lines after a statement's trailing comment align under it, before the 2024
  // style; from it, they take the statement's indentation.
  let messy = "fn f() {\n    let b = g(); // one\n// two\n    if x {\n        y();\n// three\n    } else {\n        z();\n    }\n}\n";
  let tidy = "fn f() {\n    let b = g(); // one\n                 // two\n    if x {\n        y();\n    // three\n    } else {\n        z();\n    }\n}\n";
  assert_eq!(formatted(messy), tidy);
  let config = FormatConfig {
    style_edition: 2024,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(messy, &config),
    "fn f() {\n    let b = g(); // one\n    // two\n    if x {\n        y();\n    // three\n    } else {\n        z();\n    }\n}\n"
  );
  // At the end of a block, a comment takes the block's indentation.
  assert_eq!(
    formatted("fn f() {\n    g();\n// end\n}\n"),
    "fn f() {\n    g();\n    // end\n}\n"
  );
}

#[test]
fn test_comments_after_arms_in_nested_matches() {
  // Comments after arms aligned as a list's, in a match inside a statement.
  assert_eq!(
    formatted(
      "fn f() {\n    let target_indent = match target_indent {\n        Some(x) => x, // in all other cases, take previous indentation and indent once\n        None => IndentLevel::from_node(&parent) + 1,\n    };\n    let y = match a {\n        Aaaa => 1, // one\n        B => 22222, // two\n        C => { foo(); bar() }\n        D => 4, // four\n    };\n}\n"
    ),
    "fn f() {\n    let target_indent = match target_indent {\n        Some(x) => x, // in all other cases, take previous indentation and indent once\n        None => IndentLevel::from_node(&parent) + 1,\n    };\n    let y = match a {\n        Aaaa => 1,  // one\n        B => 22222, // two\n        C => {\n            foo();\n            bar()\n        }\n        D => 4, // four\n    };\n}\n"
  );
}

#[test]
fn test_comments_after_attributes_and_last_statements() {
  // A comment under one ending a kept attribute's line is the attribute gap's: aligned under it
  // before the 2024 style, after a blank line from it; before the 2024 style a comment after
  // a block's last statement goes to the next line when it would start past column 80; from
  // the 2024 style a block comment after code has its lines trimmed.
  let c2021 = FormatConfig::default();
  let c2024 = FormatConfig {
    style_edition: 2024,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "#[derive(A)]\n#[primary_key\n        =  id]  //~^ ERROR: a\n    //~| ERROR: b\nstruct U {\n    id: i32,\n}\n#[derive(A)]\n#[primary_key =  id]  //~^ ERROR: a\n    //~| ERROR: b\nstruct V {\n    id: i32,\n}\n",
      &c2021
    ),
    "#[derive(A)]\n#[primary_key\n        =  id] //~^ ERROR: a\n               //~| ERROR: b\nstruct U {\n    id: i32,\n}\n#[derive(A)]\n#[primary_key =  id] //~^ ERROR: a\n                     //~| ERROR: b\nstruct V {\n    id: i32,\n}\n"
  );
  assert_eq!(
    formatted_with(
      "#[derive(A)]\n#[primary_key\n        =  id]  //~^ ERROR: a\n    //~| ERROR: b\nstruct U {\n    id: i32,\n}\n#[derive(A)]\n#[primary_key =  id]  //~^ ERROR: a\n    //~| ERROR: b\nstruct V {\n    id: i32,\n}\n",
      &c2024
    ),
    "#[derive(A)]\n#[primary_key\n        =  id] //~^ ERROR: a\n\n//~| ERROR: b\nstruct U {\n    id: i32,\n}\n#[derive(A)]\n#[primary_key =  id] //~^ ERROR: a\n\n//~| ERROR: b\nstruct V {\n    id: i32,\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    console_log_yyyyyyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a1\n}\nfn f() {\n    console_log_yyyyyyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a2\n    foo();\n}\nfn f() {\n    let a = h(|| {\n        console_log_yyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a3\n    });\n}\nfn f() {\n    let a = h(|| {\n        console_log_yyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a4\n        foo();\n    });\n}\nfn f() {\n    if x {\n        console_log_yyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a5\n    }\n}\nfn f() {\n    let a = h(|| {\n        console_log_yyyyyyy(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a6\n    });\n}\nfn f() {\n    let a = h(|| {\n        let x = console_log(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a7\n    });\n}\nfn f() {\n    {\n        console_log_yyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a8\n    }\n}\n",
      &c2021
    ),
    "fn f() {\n    console_log_yyyyyyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec());\n    // a1\n}\nfn f() {\n    console_log_yyyyyyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a2\n    foo();\n}\nfn f() {\n    let a = h(|| {\n        console_log_yyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec());\n        // a3\n    });\n}\nfn f() {\n    let a = h(|| {\n        console_log_yyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // a4\n        foo();\n    });\n}\nfn f() {\n    if x {\n        console_log_yyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec());\n        // a5\n    }\n}\nfn f() {\n    let a = h(|| {\n        console_log_yyyyyyy(\"Blob received {}bytes: {:?}\", len, array.to_vec());\n        // a6\n    });\n}\nfn f() {\n    let a = h(|| {\n        let x = console_log(\"Blob received {}bytes: {:?}\", len, array.to_vec());\n        // a7\n    });\n}\nfn f() {\n    {\n        console_log_yyyyyy!(\"Blob received {}bytes: {:?}\", len, array.to_vec());\n        // a8\n    }\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn start_websocket() -> Result<(), JsValue> {\n    let onmessage_callback = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {\n        {\n            let onloadend_cb = Closure::<dyn FnMut(_)>::new(move |_e: E| {\n                console_log!(\"Blob received {}bytes: {:?}\", len, array.to_vec()); // here you can for example use the received image/png data\n            });\n        }\n    });\n}\n",
      &c2021
    ),
    "fn start_websocket() -> Result<(), JsValue> {\n    let onmessage_callback = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {\n        {\n            let onloadend_cb = Closure::<dyn FnMut(_)>::new(move |_e: E| {\n                console_log!(\"Blob received {}bytes: {:?}\", len, array.to_vec());\n                // here you can for example use the received image/png data\n            });\n        }\n    });\n}\n"
  );
  assert_eq!(
    formatted_with(
      "trait T {\n    fn a(self) {\n        b\n        }\t/*\n    fn with_map<U, F>(self, f: F) -> WithMap<Self, U, F>\n        where F: FnMut(U) -> Self::SinkItem,\n              Self: Sized;\n\n    fn with_filter<F>(self, f: F);\n     */\n\t/// Transforms.\n\tfn c();\n}\n",
      &c2024
    ),
    "trait T {\n    fn a(self) {\n        b\n    } /*\n    fn with_map<U, F>(self, f: F) -> WithMap<Self, U, F>\n    where F: FnMut(U) -> Self::SinkItem,\n    Self: Sized;\n\n    fn with_filter<F>(self, f: F);\n     */\n    /// Transforms.\n    fn c();\n}\n"
  );
}

#[test]
fn test_comments_among_items_follow_the_items() {
  assert_eq!(
    formatted("fn f() {\n        // first\n  a();\n      /* second */\n  b();\n}\n"),
    "fn f() {\n    // first\n    a();\n    /* second */\n    b();\n}\n"
  );
}

#[test]
fn test_comments_around_attributes_list_ends_and_bodies() {
  // A file's last inner attribute aligns the comments under its own; an attribute's comment
  // joins it; a type macro in a field is laid out; the first comment after a list's last
  // item joins it, or aligns under that item's comment; a tuple field's comment is placed;
  // a function's body of comments closes on a line of its own; a `const` block stands on
  // one line.
  assert_eq!(
    formatted(
      "#![allow(warnings)] // A module\n// across lines.\n\nstruct S<T> {\n    #[serde(rename = \"b\")]\n\n    // About the field.\n    tables: Punctuated<Path,\nToken![,]>,\n\n    // After the last field.\n}\npub enum E<A> {\n    Left(/* #[pin] */\n        A),\n    Last, // first\n    // second\n}\nfn g() {\n    /* a */ }\nconst {\n    g(x)\n}\n"
    ),
    "#![allow(warnings)] // A module\n                    // across lines.\n\nstruct S<T> {\n    #[serde(rename = \"b\")]\n    // About the field.\n    tables: Punctuated<Path, Token![,]>,\n    // After the last field.\n}\npub enum E<A> {\n    Left(/* #[pin] */ A),\n    Last, // first\n          // second\n}\nfn g() {\n    /* a */\n}\nconst { g(x) }\n"
  );
}

#[test]
fn test_comments_inside_macro_arguments() {
  // Comments inside a macro's arguments placed as code's.
  assert_eq!(
    formatted(
      "fn f() {\n    try_likely_ok!(self.format_two_digits(\n        // Safety: `month` is guaranteed to be in the range `1..=12`.\n    unsafe\n\t{\tru8::new_unchecked(u8::from(value.month(state)))\t},\nPadding::Zero,\n));\n    assert_eq!( *s2.get_or_insert_with( 7,  || {\n    *called2.lock().unwrap()\n        =  true;\n\n// allow main thread to run before we return result\n        std::thread::sleep(std::time::Duration::from_secs(4));\n70  },\n\tguard,\n\t)\n\n.value(), 700\n);\n}\n"
    ),
    "fn f() {\n    try_likely_ok!(self.format_two_digits(\n        // Safety: `month` is guaranteed to be in the range `1..=12`.\n        unsafe { ru8::new_unchecked(u8::from(value.month(state))) },\n        Padding::Zero,\n    ));\n    assert_eq!(\n        *s2.get_or_insert_with(\n            7,\n            || {\n                *called2.lock().unwrap() = true;\n\n                // allow main thread to run before we return result\n                std::thread::sleep(std::time::Duration::from_secs(4));\n                70\n            },\n            guard,\n        )\n        .value(),\n        700\n    );\n}\n"
  );
}

#[test]
fn test_comments_kept_where_the_standard_formatter_keeps_them() {
  // Just before a closing delimiter: where written, relative to the container.
  let before_close = "fn f() {\n    if a {\n        b();\n    // c\n    } else {\n    }\n}\n";
  assert_eq!(formatted(before_close), before_close);
  // Under a comment that ends a line of code.
  let aligned = "fn f() {\n    let a = 1; // one\n               // two\n    a;\n}\n";
  assert_eq!(formatted(aligned), aligned);
  // Doc comments keep trailing whitespace; module bodies keep a leading blank line.
  let kept = "mod m {\n\n    /// a   \n    fn f() {}\n}\n";
  assert_eq!(formatted(kept), kept);
}

#[test]
fn test_contents_follow_the_line_their_owner_starts_on() {
  // A closure whose parameters span lines: its body follows the closure's first line.
  assert_eq!(
    formatted(
      "fn f() {\n    let g = |S { aaaaaaaa, bbbbbbbbbbbb }: S,\n             b: u8| {\n  a\n    };\n}\n"
    ),
    "fn f() {\n    let g = |S {\n                 aaaaaaaa,\n                 bbbbbbbbbbbb,\n             }: S,\n             b: u8| { a };\n}\n"
  );
  // A comment after an argument: the arguments one per line, the comment after its item.
  assert_eq!(
    formatted("fn f() {\n    x\n        .g(\n  1, // c\n  );\n}\n"),
    "fn f() {\n    x.g(\n        1, // c\n    );\n}\n"
  );
  // A comment between links is the chain layout's.
  assert_eq!(
    formatted("fn f() {\n    x // c\n        .g(\n  1,\n  );\n}\n"),
    "fn f() {\n    x // c\n        .g(1);\n}\n"
  );
  // A block standing as a statement owns itself.
  assert_eq!(
    formatted("fn f() {\n    {\n  a();\n    }\n    unsafe {\n b();\n    }\n}\n"),
    "fn f() {\n    {\n        a();\n    }\n    unsafe {\n        b();\n    }\n}\n"
  );
}

#[test]
fn test_continuation_lines_move_with_their_construct() {
  assert_eq!(
    formatted("fn f() {\n  let x = foo // c\n      .bar()\n      .baz();\n}\n"),
    "fn f() {\n    let x = foo // c\n        .bar()\n        .baz();\n}\n"
  );
  // A comment after `(` goes above the first argument; short ones are packed.
  assert_eq!(
    formatted("fn f() {\n    g( // c\n  a,\n          b,\n  );\n}\n"),
    "fn f() {\n    g(\n        // c\n        a, b,\n    );\n}\n"
  );
}

#[test]
fn test_default_field_values_left_as_written() {
  // A field's default value is left as written.
  assert_eq!(
    formatted(
      "#![feature(default_struct_values)]\n\n// Test for now that nightly default field values are left alone for now.\n\nstruct Foo {\n    default_field:    Spacing =    /* uwu */ 0,\n}\n\nstruct Foo2 {\n    #[rustfmt::skip]\n    default_field:    Spacing =    /* uwu */ 0,\n}\n\na_macro!(\n    struct Foo2 {\n        default_field:    Spacing =    /* uwu */ 0,\n    }\n);\n"
    ),
    "#![feature(default_struct_values)]\n\n// Test for now that nightly default field values are left alone for now.\n\nstruct Foo {\n    default_field:    Spacing =    /* uwu */ 0,\n}\n\nstruct Foo2 {\n    #[rustfmt::skip]\n    default_field:    Spacing =    /* uwu */ 0,\n}\n\na_macro!(\n    struct Foo2 {\n        default_field:    Spacing =    /* uwu */ 0,\n    }\n);\n"
  );
}

#[test]
fn test_derives_with_comments() {
  // Comments among derived names: on one line when they fit and none ends its line, else
  // one name per line.
  assert_eq!(
    formatted(
      "#[derive(\n/* ---------- Some really important comment that just had to go inside the derive --------- */\nDebug, Clone,/* Another comment */Eq, PartialEq,\n)]\nstruct Foo {\n    a: i32,\n    b: T,\n}\n"
    ),
    "#[derive(\n    /* ---------- Some really important comment that just had to go inside the derive --------- */\n    Debug,\n    Clone,\n    /* Another comment */ Eq,\n    PartialEq,\n)]\nstruct Foo {\n    a: i32,\n    b: T,\n}\n"
  );
  assert_eq!(
    formatted("#[derive(/*Debug, */Clone)]\nstruct Foo;\n"),
    "#[derive(/*Debug, */ Clone)]\nstruct Foo;\n"
  );
}

#[test]
fn test_doc_comments_and_foreign_items() {
  // Inner doc comments at the top take the file's indentation; a foreign item joins its
  // attribute, an item in a file keeps the blank line after its attribute.
  assert_eq!(
    formatted(
      "//! a\n    //! b\n        //! c\nextern \"C\" {\n    #[a]\n\n    pub fn f();\n}\n#[b]\n\nfn g() {}\n"
    ),
    "//! a\n//! b\n//! c\nextern \"C\" {\n    #[a]\n    pub fn f();\n}\n#[b]\n\nfn g() {}\n"
  );
}

#[test]
fn test_doc_comments_consts_and_macro_bodies() {
  // A doc comment joins a field, variant, statement or `use` with no blank line, as an
  // attribute does; a long const's value goes to the next line; a braced macro call's
  // arguments are re-indented keeping their layout (tabs counting `tab_spaces`), unless a
  // `rustfmt::skip::macros` around it names the macro.
  assert_eq!(
    formatted(
      "/// doc\n\nstruct S {\n    /// a\n\n    a: u8,\n}\nenum E {\n    /// b\n\n    B,\n}\n/// f\n\nfn f() {\n    /// c\n\n    let x = 1;\n}\n/// u\n\nuse a;\ntrait T: Clone + Send {}\npub trait OsStrExt: private::Sealed {}\nconst A_BILLION: i64 = 1_000_000_000; // Maximum\npub const TLS_1_2_VERSION_LONG_NAME_XXXXXXXXXXXXXXXXXXXXX: __u16 = TLS_VERSION_NUMBER_LONG(TLS_1_2_VERSION_MAJOR);\nmod m {\n    bitflags! {\n            pub struct Flags: u32 {\n        \tconst A = 1;\n\n                    const B = 2;\n            }\n    }\n}\n#[rustfmt::skip::macros(html)]\nfn main() {\n    let x = html! { <div>\nthis stays</div>\n    };\n}\n"
    ),
    "/// doc\n\nstruct S {\n    /// a\n    a: u8,\n}\nenum E {\n    /// b\n    B,\n}\n/// f\n\nfn f() {\n    /// c\n    let x = 1;\n}\n/// u\nuse a;\ntrait T: Clone + Send {}\npub trait OsStrExt: private::Sealed {}\nconst A_BILLION: i64 = 1_000_000_000; // Maximum\npub const TLS_1_2_VERSION_LONG_NAME_XXXXXXXXXXXXXXXXXXXXX: __u16 =\n    TLS_VERSION_NUMBER_LONG(TLS_1_2_VERSION_MAJOR);\nmod m {\n    bitflags! {\n            pub struct Flags: u32 {\n            const A = 1;\n\n                    const B = 2;\n            }\n    }\n}\n#[rustfmt::skip::macros(html)]\nfn main() {\n    let x = html! { <div>\nthis stays</div>\n    };\n}\n"
  );
}

#[test]
fn test_dots_kept_apart_in_chains() {
  // A link after a range keeps its space (`...` would read as one token; the standard
  // formatter puts the closure in parentheses); a float before a link is parenthesized.
  assert_eq!(
    formatted(
      "struct S;\nimpl S {\n    fn method(&self) {}\n}\nfn f() {\n    || .. .method();\n    || 1.. .method();\n}\n"
    ),
    "struct S;\nimpl S {\n    fn method(&self) {}\n}\nfn f() {\n    || .. .method();\n    || 1.. .method();\n}\n"
  );
  assert_eq!(
    formatted("fn f() {\n    0. .to_string();\n}\n"),
    "fn f() {\n    (0.).to_string();\n}\n"
  );
}

#[test]
fn test_empty_bodies_after_long_heads() {
  // An empty body keeps its braces apart after `else if`, after a head on one line with
  // `{` on the next, and when `{}` would pass the line width.
  assert_eq!(
    formatted(
      "fn main() {\n    while loooooooooooooooooooooooooooooooooong_variable_name + another_value > some_other_value_xxxxxxx {}\n    if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa || bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb || cccccccccc {}\n    if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa || bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb || cccccccccc {} else {}\n    for xxxxxxxxxxxxxxxxxxxxxxx in yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy.zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz() {}\n}\n"
    ),
    "fn main() {\n    while loooooooooooooooooooooooooooooooooong_variable_name + another_value\n        > some_other_value_xxxxxxx\n    {}\n    if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n        || bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n        || cccccccccc\n    {}\n    if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n        || bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n        || cccccccccc\n    {\n    } else {\n    }\n    for xxxxxxxxxxxxxxxxxxxxxxx in\n        yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy.zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz()\n    {}\n}\n"
  );
  assert_eq!(
    formatted(
      "fn main() {\n    'a: while loooooooooooooooooooooooooooooooooong_variable_name + another_value > some_other_value {}\n    while aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa > bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb {}\n    let x = if a {\n        1\n    } else if condition__no_alignment_for_later_else__0 || condition__no_alignment_for_later_else__1 || condition__no_alignment_for_later_else__2 {};\n}\n"
    ),
    "fn main() {\n    'a: while loooooooooooooooooooooooooooooooooong_variable_name + another_value > some_other_value\n    {\n    }\n    while aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa > bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb {\n    }\n    let x = if a {\n        1\n    } else if condition__no_alignment_for_later_else__0\n        || condition__no_alignment_for_later_else__1\n        || condition__no_alignment_for_later_else__2\n    {\n    };\n}\n"
  );
}

#[test]
fn test_empty_braced_macro() {
  // A braced macro call holding nothing closes up.
  assert_eq!(
    formatted("lazy_static::lazy_static!{\n}\n"),
    "lazy_static::lazy_static! {}\n"
  );
}

#[test]
fn test_empty_generic_lists_dropped() {
  // An empty `<>` says nothing.
  assert_eq!(
    formatted("impl<> Foo<> for Bar<> {}\nfn f(x: Foo<>) {}\n"),
    "impl Foo for Bar {}\nfn f(x: Foo) {}\n"
  );
}

#[test]
fn test_empty_where_clauses() {
  // An empty `where` goes, but a function's.
  assert_eq!(
    formatted("trait Foo where {}\nstruct S where;\n"),
    "trait Foo {}\nstruct S;\n"
  );
  assert_eq!(
    formatted("fn f() -> C\nwhere\n{\n    1\n}\nstruct S where {}\nimpl A where {}\n"),
    "fn f() -> C\nwhere\n{\n    1\n}\nstruct S {}\nimpl A {}\n"
  );
}

#[test]
fn test_failed_macro_statement_kept() {
  // A macro statement that does not lay out is kept as written; the rest is laid out.
  assert_eq!(
    formatted(
      "fn f() {\n    match name.as_str() {\n        \"threaded_scheduler\" | \"multi_thread\" =>\n {  format!(\n\n\"Set the runtime flavor with #[{macro_name}(flavor = \\\"multi_thread\\\")]. xxxxxxxxxxxxxx\"\n )\n        }\n        _ => 1,\n    };\n}\n"
    ),
    "fn f() {\n    match name.as_str() {\n        \"threaded_scheduler\" | \"multi_thread\" => {\n            format!(\n\n\"Set the runtime flavor with #[{macro_name}(flavor = \\\"multi_thread\\\")]. xxxxxxxxxxxxxx\"\n )\n        }\n        _ => 1,\n    };\n}\n"
  );
}

#[test]
fn test_field_comments_aligned_and_joined() {
  // Comments after fields aligned in groups a blank line ends; a comment above a field
  // joins it; one after a statement's `;` one space after it.
  assert_eq!(
    formatted(
      "struct S {\n    aliases: Vec<(Str, bool)>,\t// (name, visible)\n    a: u8,  // x\n\n    c: u8,   // z\n        // Hash buckets\n\nentries: u8,\n}\nconst A: u8 = 1;  /* one */\n"
    ),
    "struct S {\n    aliases: Vec<(Str, bool)>, // (name, visible)\n    a: u8,                     // x\n\n    c: u8, // z\n    // Hash buckets\n    entries: u8,\n}\nconst A: u8 = 1; /* one */\n"
  );
}

#[test]
fn test_files_left_alone_by_their_settings() {
  let messy = "// @generated by a tool\nfn  f() {\nx();\n}\n";
  let skip_generated = FormatConfig {
    format_generated_files: false,
    ..FormatConfig::default()
  };
  assert_eq!(format_file(messy, Edition::E2021, &skip_generated), messy);
  assert_ne!(
    format_file(messy, Edition::E2021, &FormatConfig::default()),
    messy
  );
  let disabled = FormatConfig {
    disable_all_formatting: true,
    ..FormatConfig::default()
  };
  assert_eq!(format_file(messy, Edition::E2021, &disabled), messy);
}

#[test]
fn test_for_head_comment_keeps_statement() {
  // A comment in a loop's head leaves the statement as written.
  assert_eq!(
    formatted(
      "fn main() {\n    for variable_in_x /* ... */ in 0..1 {\n        if false {\n\n        } else {\n\n        }\n    }\n}\n"
    ),
    "fn main() {\n    for variable_in_x /* ... */ in 0..1 {\n        if false {\n\n        } else {\n\n        }\n    }\n}\n"
  );
}

#[test]
fn test_for_head_with_commented_array() {
  // A loop's iterator may hold comments its list places.
  assert_eq!(
    formatted(
      "fn f() {\n    for\n  f in [\n            // these drift without `float_roundtrip`\n            -0.20221894534048165,\n            6.178787134922198e305, // c\n        ] {\n        g(f);\n    }\n}\n"
    ),
    "fn f() {\n    for f in [\n        // these drift without `float_roundtrip`\n        -0.20221894534048165,\n        6.178787134922198e305, // c\n    ] {\n        g(f);\n    }\n}\n"
  );
}

#[test]
fn test_imports_of_nothing_dropped() {
  // A `use` of an empty list goes, with its line.
  assert_eq!(
    formatted("use std::{};\nuse a::{b::{}};\nuse std::borrow::Cow;\n"),
    "use std::borrow::Cow;\n"
  );
}

#[test]
fn test_index_expressions_and_ranges() {
  // An index after what it indexes, else on the next line; a range breaks as a binary
  // expression does; a string spanning lines under `&` and indexed.
  assert_eq!(
    formatted(
      "fn f() {\n    match x {\n        PushParam => {\n            stack.push(mparams[match cur.to_digit(10) { Some(d) => d as usize - 1, None => return Err(\"bad param number\".to_owned()) }])\n        }\n    }\n    foo(mparams[match cur.to_digit(10) { Some(d) => d as usize - 1, None => return Err(\"bad param number\".to_owned()) }]);\n}\n"
    ),
    "fn f() {\n    match x {\n        PushParam => stack.push(\n            mparams[match cur.to_digit(10) {\n                Some(d) => d as usize - 1,\n                None => return Err(\"bad param number\".to_owned()),\n            }],\n        ),\n    }\n    foo(mparams[match cur.to_digit(10) {\n        Some(d) => d as usize - 1,\n        None => return Err(\"bad param number\".to_owned()),\n    }]);\n}\n"
  );
  assert_eq!(
    formatted(
      "fn params_in_repr(db: &dyn HirDatabase, def_id: AdtId) -> Box<[bool]> {\n    let mut params_in_repr = (0..generics.len_early_bound_lifetimes() + generics.len_type_or_consts()).map(|_| false).collect::<Box<[bool]>>();\n}\n"
    ),
    "fn params_in_repr(db: &dyn HirDatabase, def_id: AdtId) -> Box<[bool]> {\n    let mut params_in_repr = (0..generics.len_early_bound_lifetimes()\n        + generics.len_type_or_consts())\n        .map(|_| false)\n        .collect::<Box<[bool]>>();\n}\n"
  );
  assert_eq!(
    formatted(
      "fn parse_category_test() {\n  let   ini_file =\n  &b\"[category]\n\nparameter=value\nkey = value2\"[..];\n  let x = foo(&b\"[category]\n\nparameter=value\nkey = value2\"[..], bar);\n}\n"
    ),
    "fn parse_category_test() {\n    let ini_file = &b\"[category]\n\nparameter=value\nkey = value2\"[..];\n    let x = foo(\n        &b\"[category]\n\nparameter=value\nkey = value2\"[..],\n        bar,\n    );\n}\n"
  );
}

#[test]
fn test_item_alone_as_macro_argument() {
  // An item alone in a macro's arguments on a line of its own.
  assert_eq!(
    formatted(
      "cfg_net_unix!(\n    impl ReadyEvent {\n        pub(crate) fn with_ready(&self, ready: Ready) -> Self {\n            Self {\n                ready,\n                tick: self.tick,\n            }\n        }\n    }\n);\nm!(  impl\n\nA\n{  fn f()\n{ 1 }\n}\n);\nm!(  struct\n\nA\n{  x: u8\n}\n);\nm!(fn f() { 1 });\nm!(fn f() {\n   1 });\n"
    ),
    "cfg_net_unix!(\n    impl ReadyEvent {\n        pub(crate) fn with_ready(&self, ready: Ready) -> Self {\n            Self {\n                ready,\n                tick: self.tick,\n            }\n        }\n    }\n);\nm!(  impl\n\nA\n{  fn f()\n{ 1 }\n}\n);\nm!(\n    struct A {\n        x: u8,\n    }\n);\nm!(\n    fn f() {\n        1\n    }\n);\nm!(\n    fn f() {\n        1\n    }\n);\n"
  );
}

#[test]
fn test_leading_pipes_dropped() {
  // A `|` before a pattern's first alternative goes.
  assert_eq!(
    formatted(
      "fn f() {\n    let (| a | a, | b) = ((), ());\n    let R { a: | 1 | 2 } = 0;\n    let [| a, ..] = [];\n    let S(| a) = ();\n    match x { | A | B => 1, | C => 2 }\n}\n"
    ),
    "fn f() {\n    let (a | a, b) = ((), ());\n    let R { a: 1 | 2 } = 0;\n    let [a, ..] = [];\n    let S(a) = ();\n    match x {\n        A | B => 1,\n        C => 2,\n    }\n}\n"
  );
}

#[test]
fn test_line_endings_and_tabs() {
  assert_eq!(
    formatted("fn f() {\r\nx();\r\n}\r\n"),
    "fn f() {\r\n    x();\r\n}\r\n"
  );
  let tabs = FormatConfig {
    hard_tabs: true,
    ..FormatConfig::default()
  };
  assert_eq!(
    format_file("fn f() {\n    x();\n}\n", Edition::E2021, &tabs),
    "fn f() {\n\tx();\n}\n"
  );
  let unix = FormatConfig {
    newline_style: NewlineStyle::Unix,
    ..FormatConfig::default()
  };
  assert_eq!(
    format_file("fn f() {}\r\n", Edition::E2021, &unix),
    "fn f() {}\n"
  );
  // A string spanning lines keeps the file's endings, which are part of it.
  let crlf_string = "fn f() {\r\n    g(\"a\r\nb\");\r\n}\r\n";
  assert_eq!(format_file(crlf_string, Edition::E2021, &unix), crlf_string);
  let windows = FormatConfig {
    newline_style: NewlineStyle::Windows,
    tab_spaces: 2,
    ..FormatConfig::default()
  };
  assert_eq!(
    format_file("fn f() {\nx();\n}\n", Edition::E2021, &windows),
    "fn f() {\r\n  x();\r\n}\r\n"
  );
  let native = FormatConfig {
    newline_style: NewlineStyle::Native,
    ..FormatConfig::default()
  };
  let ending = if cfg!(windows) { "\r\n" } else { "\n" };
  assert_eq!(
    format_file("fn f() {}\r\n", Edition::E2021, &native),
    format!("fn f() {{}}{}", ending)
  );
}

#[test]
fn test_lines_starting_constructs_follow_their_block() {
  assert_eq!(
    formatted("fn f() {\nlet a = 1;\n        if a {\n  g();\n            }\n}\n"),
    "fn f() {\n    let a = 1;\n    if a {\n        g();\n    }\n}\n"
  );
  assert_eq!(
    formatted("mod m {\nstruct S {\na: u8,\n  b: u8,\n}\n}\n"),
    "mod m {\n    struct S {\n        a: u8,\n        b: u8,\n    }\n}\n"
  );
}

#[test]
fn test_lone_semicolon_after_comments_indented() {
  // A `;` ending an item on a line of its own is indented as the item.
  assert_eq!(
    formatted(
      "trait Foo {\n    fn bar(&self)\n    // where\n    //     Self: Bar\n    ;\n}\n\ntrait Foo {\n    fn bar(&self)\n    // where\n    //     Self: Bar\n;\n}\n\ntrait Foo {\n    fn bar(&self)\n    // where\n    //     Self: Bar\n        ;\n}\n"
    ),
    "trait Foo {\n    fn bar(&self)\n    // where\n    //     Self: Bar\n    ;\n}\n\ntrait Foo {\n    fn bar(&self)\n    // where\n    //     Self: Bar\n    ;\n}\n\ntrait Foo {\n    fn bar(&self)\n    // where\n    //     Self: Bar\n    ;\n}\n"
  );
}

#[test]
fn test_long_attribute_values_and_inner_docs() {
  // An attribute's `name = "literal"` keeps the literal however long, its arguments on
  // lines when they do not fit.
  assert_eq!(
    formatted(
      "#[deprecated(\n\tnote\t=\n \"Use [std::iter::FromFn](https://doc.rust-lang.org/std/iter/struct.FromFn.html) instead\",  since = \"0.13.0\")]\nfn g() {}\n"
    ),
    "#[deprecated(\n    note = \"Use [std::iter::FromFn](https://doc.rust-lang.org/std/iter/struct.FromFn.html) instead\",\n    since = \"0.13.0\"\n)]\nfn g() {}\n"
  );
  // A file's doc comment after its attribute starts its own line.
  assert_eq!(
    formatted("#![allow(x)] //! Docs.\n\nfn f() {}\n"),
    "#![allow(x)]\n//! Docs.\n\nfn f() {}\n"
  );
}

#[test]
fn test_macro_arguments_read_as_the_standard_formatter_reads_them() {
  // A braced call's later lines from the least indented, the last measured too; a type with a
  // comma read as one when no expression comes before the comma; a `use` alone as an item; a
  // `let` outside a condition left as written; a call kept as written, one line or braced,
  // as a list's last item past the line width.
  let c2021 = FormatConfig::default();
  let c2024 = FormatConfig {
    style_edition: 2024,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "fn f() {\n    if x {\n    abort!  {  item.kind().span(),\n\"a {} b\", item.kind().name(),  }\n\n}\n}\n",
      &c2021
    ),
    "fn f() {\n    if x {\n        abort! {  item.kind().span(),\n        \"a {} b\", item.kind().name(),  }\n    }\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let ty = m!(&Closure<dyn  FnMut(T)\n -> Result<R,  E>>);\n    let ty = m!(&Closure<dyn  FnMut(T) -> Result<R,  E>>);\n}\n",
      &c2021
    ),
    "fn f() {\n    let ty = m!(&Closure<dyn FnMut(T) -> Result<R, E>>);\n    let ty = m!(&Closure<dyn FnMut(T) -> Result<R, E>>);\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let ty = m!(HashMap<Vec<u8>,  E>);\n    let ty = m!(HashMap<Vec<u8  >,  E>);\n    let ty = m!(HashMap<[u8; 4],  E>);\n    let ty = m!(HashMap<&str,  E>);\n    let ty = m!(HashMap<(A, B),  E>);\n    let ty = m!(HashMap<A::B,  E>);\n    let ty = m!(fn(A,  B));\n    let ty = m!(Box<dyn Fn(A,  B)>);\n    let ty = m!(HashMap<Box<dyn Fn()>,  E>);\n}\n",
      &c2021
    ),
    "fn f() {\n    let ty = m!(HashMap<Vec<u8>, E>);\n    let ty = m!(HashMap<Vec<u8>, E>);\n    let ty = m!(HashMap<[u8; 4],  E>);\n    let ty = m!(HashMap<&str,  E>);\n    let ty = m!(HashMap<(A, B),  E>);\n    let ty = m!(HashMap<A::B,  E>);\n    let ty = m!(fn(A, B));\n    let ty = m!(Box<dyn Fn(A, B)>);\n    let ty = m!(HashMap<Box<dyn Fn()>, E>);\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let imports = if a { vec![parse_quote!(  use  diesel::sql_types::*;\n\t)] } else { b };\n    let i = vec![parse_quote!(  use  diesel::sql_types::*;\n\t)];\n    let i = parse_quote!(  use  diesel::sql_types::*;\n\t);\n}\n",
      &c2024
    ),
    "fn f() {\n    let imports = if a {\n        vec![parse_quote!(\n            use diesel::sql_types::*;\n        )]\n    } else {\n        b\n    };\n    let i = vec![parse_quote!(\n        use diesel::sql_types::*;\n    )];\n    let i = parse_quote!(\n        use diesel::sql_types::*;\n    );\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let tokens = Group::new(Delimiter::None, quote!(let\n    None\n\n=  None)).to_token_stream();\n    let t = quote!(let  None =  None);\n    let t = m!(a, let  None =  None);\n}\n",
      &c2021
    ),
    "fn f() {\n    let tokens = Group::new(\n        Delimiter::None,\n        quote!(let\n    None\n\n=  None),\n    )\n    .to_token_stream();\n    let t = quote!(let  None =  None);\n    let t = m!(a, let  None =  None);\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    m!(if let  Some(x) =  y { 1 } else { 2 });\n    m!(while let  Some(x) =  y.next() {});\n}\n",
      &c2021
    ),
    "fn f() {\n    m!(if let Some(x) = y { 1 } else { 2 });\n    m!(while let Some(x) = y.next() {});\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    if a {\n        if b {\n            Ok(quote!(self.#field_name.as_ref().map(|x| diesel::ExpressionMethods::eq(#table_name::#column_name, x))))\n        } else {\n            c\n        }\n    }\n    foo(quote!(self.#field_name.as_ref().map(|x| diesel::ExpressionMethods::eq(#table_name::#column_name, x))));\n    foo(a, quote!(self.#field_name.as_ref().map(|x| diesel::ExpressionMethods::eq(#table_name::#column_name, x))));\n    let x = foo(quote!(self.#field_name.as_ref().map(|x| diesel::ExpressionMethods::eq(#table_name::#column_name, x))));\n}\n",
      &c2024
    ),
    "fn f() {\n    if a {\n        if b {\n            Ok(\n                quote!(self.#field_name.as_ref().map(|x| diesel::ExpressionMethods::eq(#table_name::#column_name, x))),\n            )\n        } else {\n            c\n        }\n    }\n    foo(\n        quote!(self.#field_name.as_ref().map(|x| diesel::ExpressionMethods::eq(#table_name::#column_name, x))),\n    );\n    foo(\n        a,\n        quote!(self.#field_name.as_ref().map(|x| diesel::ExpressionMethods::eq(#table_name::#column_name, x))),\n    );\n    let x = foo(\n        quote!(self.#field_name.as_ref().map(|x| diesel::ExpressionMethods::eq(#table_name::#column_name, x))),\n    );\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    if a {\n        let  inner_ty\n\t=  inner_of_option_ty(ty);\n    Ok(quote_spanned!\n\n{span=>  std::option::Option<diesel::dsl::Eq<\t#table_name::#column_name,  #inner_ty,\n\t>>  })\n} else\n    {\n        b\n    }\n}\n",
      &c2024
    ),
    "fn f() {\n    if a {\n        let inner_ty = inner_of_option_ty(ty);\n        Ok(\n            quote_spanned! {span=>  std::option::Option<diesel::dsl::Eq<\t#table_name::#column_name,  #inner_ty,\n            >>  },\n        )\n    } else {\n        b\n    }\n}\n"
  );
}

#[test]
fn test_macros_in_ranges_keep_their_arguments() {
  // Arguments that do not parse stay as written, spaces and all.
  let kept = "fn main() {\n    a..format!(        na, issue =  );\n    let x = ..format!(        na, issue =  );\n}\n";
  assert_eq!(formatted(kept), kept);
}

#[test]
fn test_match_arm_indent_off_keeps_arms_at_the_match() {
  let config = FormatConfig {
    match_arm_indent: false,
    ..FormatConfig::default()
  };
  let source = "fn f() {\n    match x {\n        0 => {\n            a();\n        }\n        _ => b(),\n    }\n}\n";
  let want = "fn f() {\n    match x {\n    0 => {\n        a();\n    }\n    _ => b(),\n    }\n}\n";
  let once = format_file(source, Edition::E2021, &config);
  assert_eq!(once, want);
  assert_eq!(format_file(&once, Edition::E2021, &config), once);
}

#[test]
fn test_nested_parentheses_dropped() {
  // Parentheses directly inside parentheses go, but around a comment.
  assert_eq!(
    formatted(
      "fn main() {\n    let x = (((1)));\n    foo((((a + b))), c);\n    let z = ((a)).b();\n}\n"
    ),
    "fn main() {\n    let x = (1);\n    foo((a + b), c);\n    let z = (a).b();\n}\n"
  );
}

#[test]
fn test_nested_parentheses_kept_by_setting() {
  // `remove_nested_parens = false` keeps them.
  let config = FormatConfig {
    remove_nested_parens: false,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with("fn f() {\n    let x = ((1));\n}\n", &config),
    "fn f() {\n    let x = ((1));\n}\n"
  );
}

#[test]
fn test_nested_tuple_fields_spaced() {
  // Before style edition 2024 a tuple field after a tuple field keeps a space.
  assert_eq!(
    formatted("fn f() {\n    foo(x.0 .0, y.1.2);\n}\n"),
    "fn f() {\n    foo(x.0 .0, y.1 .2);\n}\n"
  );
}

#[test]
fn test_parameter_and_tuple_field_attributes() {
  // An attribute too long for the parameter's line goes on a line of its own; one
  // breaking across lines puts the field on the next.
  assert_eq!(
    formatted(
      "fn a1(\n    #[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa] a: u8,\n) {\n}\n"
    ),
    "fn a1(\n    #[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]\n    a: u8,\n) {\n}\n"
  );
  assert_eq!(
    formatted(
      "struct Fields(\n    #[cfg_attr(feature = \"serde_derive\", serde(state_with = \"::base::serialization::shared\"))] Arc<Vec<InternedStr>>,\n);\npub enum State {\n    Closure(#[cfg_attr(feature = \"serde_derive\", serde(state_with = \"::serialization::closure\"))] GcPtr<ClosureData>),\n}\n"
    ),
    "struct Fields(\n    #[cfg_attr(\n        feature = \"serde_derive\",\n        serde(state_with = \"::base::serialization::shared\")\n    )]\n    Arc<Vec<InternedStr>>,\n);\npub enum State {\n    Closure(\n        #[cfg_attr(\n            feature = \"serde_derive\",\n            serde(state_with = \"::serialization::closure\")\n        )]\n        GcPtr<ClosureData>,\n    ),\n}\n"
  );
}

#[test]
fn test_parameters_with_block_comments() {
  // Block comments among parameters on the signature's line when it fits.
  assert_eq!(
    formatted(
      "fn foo(  /* comment */\n) {}\nfn foo_one_pre(/* Important comment1 */\n    self) {}\nfn foo_one_post(self\n    /* Important comment1 */) {}\nfn two(/*a*/ a: u8, b: u8 /*b*/) {}\n"
    ),
    "fn foo(/* comment */) {}\nfn foo_one_pre(/* Important comment1 */ self) {}\nfn foo_one_post(self /* Important comment1 */) {}\nfn two(/*a*/ a: u8, b: u8 /*b*/) {}\n"
  );
}

#[test]
fn test_raw_string_arguments() {
  // Lines inside a raw string argument are not measured.
  assert_eq!(
    formatted(
      "fn overloaded_binop() {\n    check_number(r#\"\n    const GOAL: bool = {\n        x == Yellow && y == Red && Red + Green == Yellow && Red + Red == Yellow && Yellow + Green == Yellow\n    };\n    \"#, 1);\n    check_number(r#\"\n    const GOAL: bool = {\n        x == Yellow\n    };\n    \"#, 1);\n}\n"
    ),
    "fn overloaded_binop() {\n    check_number(\n        r#\"\n    const GOAL: bool = {\n        x == Yellow && y == Red && Red + Green == Yellow && Red + Red == Yellow && Yellow + Green == Yellow\n    };\n    \"#,\n        1,\n    );\n    check_number(\n        r#\"\n    const GOAL: bool = {\n        x == Yellow\n    };\n    \"#,\n        1,\n    );\n}\n"
  );
}

#[test]
fn test_raw_strings_arm_head_comments_closure_heads_and_aligned_block_comments() {
  let c2021 = FormatConfig::default();
  let c2024 = FormatConfig {
    style_edition: 2024,
    ..FormatConfig::default()
  };
  // A line holding a raw string is not measured, one spanning lines in a struct literal's
  // field too: the standard formatter keeps raw strings whatever their width.
  assert_eq!(
    formatted_with(
      "pub const DEFAULT_LINTS: &[Lint] = &[  Lint { label:\n        \"aarch64_softfloat_neon\", description:\tr##\"detects code that could be affected by ABI issues on aarch64 softfloat targets\"##,\n\tdefault_severity:\n        Severity::Warning,\nwarn_since:\n\tNone,  deny_since:\nNone,\n},\n    Lint\n    {  label: \"absolute_paths_not_starting_with_crate\",\ndescription:\n    r##\"fully qualified paths that start with a module name\"##,\ndefault_severity: Severity::Allow, warn_since: None, deny_since: None },\n];\n",
      &c2024
    ),
    "pub const DEFAULT_LINTS: &[Lint] = &[\n    Lint {\n        label: \"aarch64_softfloat_neon\",\n        description: r##\"detects code that could be affected by ABI issues on aarch64 softfloat targets\"##,\n        default_severity: Severity::Warning,\n        warn_since: None,\n        deny_since: None,\n    },\n    Lint {\n        label: \"absolute_paths_not_starting_with_crate\",\n        description: r##\"fully qualified paths that start with a module name\"##,\n        default_severity: Severity::Allow,\n        warn_since: None,\n        deny_since: None,\n    },\n];\n"
  );
  assert_eq!(
    formatted_with(
      "pub const FEATURES: &[Lint] = &[ Lint { label: \"abi_avr_interrupt\", description:\n        r##\"# `abi_avr_interrupt`\n\nThe tracking issue for this feature is: [#69664]\n\n------------------------\n\"##, default_severity: Severity::Allow,\n        warn_since: None,\n        deny_since: None,\n    },\n];\n",
      &c2024
    ),
    "pub const FEATURES: &[Lint] = &[Lint {\n    label: \"abi_avr_interrupt\",\n    description: r##\"# `abi_avr_interrupt`\n\nThe tracking issue for this feature is: [#69664]\n\n------------------------\n\"##,\n    default_severity: Severity::Allow,\n    warn_since: None,\n    deny_since: None,\n}];\n"
  );
  // A comment in a match arm's head before its `=>`, not in its guard's condition: the
  // statement holding the `match` is left as written.
  assert_eq!(
    formatted_with(
      "fn f() {\n    match   x {\n        A   =>   1,\n        B(v)\n            // c\n            if v.foo() => {\n                2\n            }\n        _ => {}\n    }\n}\n",
      &c2024
    ),
    "fn f() {\n    match   x {\n        A   =>   1,\n        B(v)\n            // c\n            if v.foo() => {\n                2\n            }\n        _ => {}\n    }\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    match   x {\n        A   =>   1,\n        B(v) if v.foo()\n            // c\n            => 2,\n        _ => {}\n    }\n}\nfn g() {\n    match   x {\n        A   =>   1,\n        B(v) if v\n            .foo() // c\n            && v.bar() => 2,\n        _ => {}\n    }\n}\nfn h() {\n    match   x {\n        A   =>   1,\n        // c\n        B(v) /* d */ => 2,\n        _ => {}\n    }\n}\n",
      &c2024
    ),
    "fn f() {\n    match   x {\n        A   =>   1,\n        B(v) if v.foo()\n            // c\n            => 2,\n        _ => {}\n    }\n}\nfn g() {\n    match x {\n        A => 1,\n        B(v) if v\n            .foo() // c\n            && v.bar() =>\n        {\n            2\n        }\n        _ => {}\n    }\n}\nfn h() {\n    match   x {\n        A   =>   1,\n        // c\n        B(v) /* d */ => 2,\n        _ => {}\n    }\n}\n"
  );
  // A closure whose head no layout fits gives up its statement, whatever comments its
  // body holds.
  assert_eq!(
    formatted_with(
      "fn f() {\n    let migrations_inner =\n|harness:\n        &mut\nHarnessWithOutput<Conn,\n\n_>|\t->  Result<(),\n\tBox<dyn\n\nError\n    +\tSend  +\n    Sync\n    +\t'static>>\n\t{\n    // revert all the migrations\n        let x = 1;\n        Ok(())\n    };\n    let migrations_inner2 =\n|harness:\n        &mut\nHarnessWithOutput<Conn,\n\n_>|\t->  Result<(),\n\tBox<dyn\n\nError\n    +\tSend  +\n    Sync\n    +\t'static>>\n\t{\n        let x = 1;\n        Ok(())\n    };\n}\n",
      &c2021
    ),
    "fn f() {\n    let migrations_inner =\n|harness:\n        &mut\nHarnessWithOutput<Conn,\n\n_>|\t->  Result<(),\n\tBox<dyn\n\nError\n    +\tSend  +\n    Sync\n    +\t'static>>\n\t{\n    // revert all the migrations\n        let x = 1;\n        Ok(())\n    };\n    let migrations_inner2 =\n|harness:\n        &mut\nHarnessWithOutput<Conn,\n\n_>|\t->  Result<(),\n\tBox<dyn\n\nError\n    +\tSend  +\n    Sync\n    +\t'static>>\n\t{\n        let x = 1;\n        Ok(())\n    };\n}\n"
  );
  // A block comment across lines under a comment ending a line of code lines up with it,
  // before the 2024 style, its lines laid out from that column.
  assert_eq!(
    formatted_with(
      "impl X {\n    fn f() {\n        a();\n    } // FIXME: one\n      // two\n/*\n    /// doc\n    fn g() {}\n    */\n    fn h() {}\n}\n",
      &c2021
    ),
    "impl X {\n    fn f() {\n        a();\n    } // FIXME: one\n      // two\n      /*\n      /// doc\n      fn g() {}\n      */\n    fn h() {}\n}\n"
  );
}

#[test]
fn test_repeat_arrays_and_vec() {
  // `[x; n]` breaks as a pair, `vec![x; n]` with each part on a line of its own.
  assert_eq!(
    formatted(
      "fn issue1693() {\n    let pixel_data = vec![(f16::from_f32(0.82), f16::from_f32(1.78), f16::from_f32(0.21)); 256 * 256];\n}\n"
    ),
    "fn issue1693() {\n    let pixel_data = vec![\n        (\n            f16::from_f32(0.82),\n            f16::from_f32(1.78),\n            f16::from_f32(0.21)\n        );\n        256 * 256\n    ];\n}\n"
  );
  assert_eq!(
    formatted(
      "fn foo() {\n    let a = [(); const { let x = 1; x }];\n}\n\nfn foo() {\n    let x = [(); const { 1 }];\n}\n"
    ),
    "fn foo() {\n    let a = [(); const {\n        let x = 1;\n        x\n    }];\n}\n\nfn foo() {\n    let x = [(); const { 1 }];\n}\n"
  );
}

#[test]
fn test_settles_in_one_run() {
  // Each settles where a second run leaves it (`formatted_with` checks that).
  let cases: &[(&str, usize, usize, &str)] = &[
    // An enum with a variant on lines and one on a line has its struct variants on lines, a
    // tuple variant measured with the column its fields leave and without the commas
    // written in it.
    (
      "mod a {\n    mod b {\n        enum State {\n            A { x: [[usize; 4]; 4] },\n            C(HashMap<(u8, i32), [char; 4]>),\n        }\n    }\n}\n",
      54,
      7,
      "mod a {\n       mod b {\n              enum State {\n                     A {\n                            x: [[usize; 4]; 4],\n                     },\n                     C(\n                            HashMap<\n                                   (u8, i32),\n                                   [char; 4],\n                            >,\n                     ),\n              }\n       }\n}\n",
    ),
    // A block holding a statement given up on, its `vec!` as written, and another `vec!`
    // laid out with brackets: laid out whole.
    (
      "pub\n const MAX:&usize = {\n    vec!(\"{}\",&mut (1_000.25));\n    for 5..=10 in run(2e3, 2e3)[Bar::B] {\n         vec!(\"{}\",a);\n    }\n};\n",
      20,
      4,
      "pub const MAX:\n    &usize = {\n    vec!(\"{}\",&mut (1_000.25));\n    for 5..=10 in\n        run(\n            2e3,\n            2e3,\n        )[Bar::B]\n    {\n        vec![\n            \"{}\",\n            a,\n        ];\n    }\n};\n",
    ),
    // A value whose place hangs on a statement kept as written settles where a second run
    // puts it.
    (
      "const SIZE:i32 = if if len(b, b, b)[b] {\n    Ok(())\n}\n{\n     {\n    }\n      let mut a: i32 = [Bar::default(),\n      self];\n};\n",
      30,
      6,
      "const SIZE: i32 =\n      if if len(b, b, b)[b] {\n            Ok(())\n      } {\n            {}\n            let mut a: i32 = [\n            Bar::default(),\n            self,\n      ];\n      };\n",
    ),
    // Packed, a comment written after the item before loses the blank line under its first
    // line, as a second run reads it; one further down is kept.
    (
      "fn f() {\n    g(\n        x,\n\n        // one\n\n        // two\n\n        // three\n        y,\n    );\n}\n",
      100,
      4,
      "fn f() {\n    g(\n        x, // one\n        // two\n\n        // three\n        y,\n    );\n}\n",
    ),
    // A closure body no layout fits kept as written in the block made for it.
    (
      "fn f() {\n  let a = || || || || || || || || || || || SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS::fro;\n}\n",
      47,
      2,
      "fn f() {\n  let a = || {\n    || {\n      || {\n        || {\n          || {\n            || {\n              || {\n                || {\n                  || {\n                    || {\n                      || {\n                        SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS::fro\n                      }\n                    }\n                  }\n                }\n              }\n            }\n          }\n        }\n      }\n    }\n  };\n}\n",
    ),
    // Closures laid out with their blocks weighed, so a binary operator before them breaks once.
    (
      "#[test]\nfn test() {\n  SSSSSSSSSSSS||||||||||||||||||||||||SSSSSSSSSSSSSSSSS ::fro;\n}\n",
      47,
      2,
      "#[test]\nfn test() {\n  SSSSSSSSSSSS\n    || || {\n      || {\n        || {\n          || {\n            || {\n              || {\n                || {\n                  || {\n                    || {\n                      || {\n                        || {\n                          SSSSSSSSSSSSSSSSS ::fro\n                        }\n                      }\n                    }\n                  }\n                }\n              }\n            }\n          }\n        }\n      }\n    };\n}\n",
    ),
    // A macro holding a string across lines, kept as written only where its closing line sits.
    (
      "fn f() {\n        let pos = s.find(1).ok_or(l | _see | format!(\"invalid KEY=value: no found e(Copy,\ne(Debug)]in\"))?;\n}\n",
      66,
      8,
      "fn f() {\n        let pos =\n                s.find(1).ok_or(l | _see | format!(\n                        \"invalid KEY=value: no found e(Copy,\ne(Debug)]in\"\n                ))?;\n}\n",
    ),
    // A closure given a block ends like one: the chain after it stays at its indentation.
    (
      "const S: u8 = (|a, b| |c, d| self).len(|e, f| |g, h| self, |i, j| |k, l| true)?.iter();\n",
      21,
      2,
      "const S: u8 =\n  (|a, b| {\n    |c, d| self\n  })\n  .len(\n    |e, f| {\n      |g, h| self\n    },\n    |i, j| {\n      |k, l| true\n    },\n  )?\n  .iter();\n",
    ),
    // A statement given up on keeps its vec! delimiters as written.
    (
      "const MAX: impl Display = //\n {\n     vec!(\"{}\", y[1_000.25]);\n};\n",
      20,
      3,
      "const MAX: impl Display =\n   //\n   {\n      vec!(\"{}\", y[1_000.25]);\n   };\n",
    ),
    // A head holding a match whose arms lose their commas.
    (
      "fn f() {\n    let\n    y = match match a {\n        a => {\n        },\n        _ => {\n        },\n    } {\n        a => // note\n         a[b],\n        _ => 1,\n    };\n}\n",
      100,
      4,
      "fn f() {\n    let y = match match a {\n        a => {}\n        _ => {}\n    } {\n        a => // note\n         a[b],\n        _ => 1,\n    };\n}\n",
    ),
    // A field's comma on the line after it joins it.
    (
      "struct Foo {\n    a: i32\n ,   // x\n    // y\n    b: u8\n ,   // z\n}\n",
      100,
      4,
      "struct Foo {\n    a: i32, // x\n    // y\n    b: u8, // z\n}\n",
    ),
    // A doc comment ending the file keeps its two spaces.
    ("//! x  \n", 100, 4, "//! x  \n"),
  ];
  for &(source, max_width, tab_spaces, expected) in cases {
    let config = FormatConfig {
      max_width,
      tab_spaces,
      ..FormatConfig::default()
    };
    assert_eq!(formatted_with(source, &config), expected, "{}", source);
  }
  // Closures nested 160 deep, each in a block: laid out once per level, not once per shape
  // every level around them tries (over two minutes at this depth). The bound leaves room
  // for slow machines, which the old cost was far past.
  let deep = format!(
    "fn main() {{\n    f(|v| {}{{\n        match v {{\n            _ => 1,\n        }}\n    }});\n}}\n",
    "|| ".repeat(160)
  );
  // On a stack as large as mto runs files on: an unoptimized build needs it at this depth.
  let started = std::time::Instant::now();
  std::thread::Builder::new()
    .stack_size(crate::STACK)
    .spawn(move || formatted(&deep))
    .unwrap()
    .join()
    .unwrap();
  assert!(
    started.elapsed() < std::time::Duration::from_secs(30),
    "{:?}",
    started.elapsed()
  );
  // Closures nested 130 deep without blocks, around an operand too wide for any line: each
  // tried in a block on one line, and the statement measured with no line width, at every
  // level (over six seconds at this depth, optimized).
  let chained = format!(
    "fn f() {{\n  x = a || {}b * {};\n}}\n",
    "|| ".repeat(130),
    "c".repeat(140)
  );
  let config = FormatConfig {
    max_width: 139,
    tab_spaces: 1,
    ..FormatConfig::default()
  };
  let started = std::time::Instant::now();
  std::thread::Builder::new()
    .stack_size(crate::STACK)
    .spawn(move || formatted_with(&chained, &config))
    .unwrap()
    .join()
    .unwrap();
  assert!(
    started.elapsed() < std::time::Duration::from_secs(30),
    "{:?}",
    started.elapsed()
  );
  // Closures nested 100 deep around a string spanning lines: none tried on its line, where
  // the string's lines could never go (over seven seconds at this depth, optimized).
  let stringed = format!(
    "fn f() {{\n  x = {}m(h, r#\"\nq\n\"#);\n}}\n",
    "a * || ".repeat(100)
  );
  let config = FormatConfig {
    max_width: 47,
    tab_spaces: 2,
    ..FormatConfig::default()
  };
  let started = std::time::Instant::now();
  std::thread::Builder::new()
    .stack_size(crate::STACK)
    .spawn(move || formatted_with(&stringed, &config))
    .unwrap()
    .join()
    .unwrap();
  assert!(
    started.elapsed() < std::time::Duration::from_secs(30),
    "{:?}",
    started.elapsed()
  );
  // An argument's comma after comments that run to the list's end: written once.
  formatted(
    "fn f() {\n    g(\n        x,\n        R // a\n\n        // b\n        ,\n        // c\n    );\n}\n",
  );
}

#[test]
fn test_skipped_item_keeps_its_attributes() {
  // A skipped item's attributes stay as written.
  assert_eq!(
    formatted(
      "#[link_section=\".vectors\"]\n#[rustfmt::skip]\npub static ISSUE_1284: [i32; 16] = [];\n"
    ),
    "#[link_section=\".vectors\"]\n#[rustfmt::skip]\npub static ISSUE_1284: [i32; 16] = [];\n"
  );
}

#[test]
fn test_skipped_macro_invocations() {
  // The calls of the macros `skip_macro_invocations` names, by their path as written or all
  // for `*`, are left as written.
  let named = FormatConfig {
    skip_macro_invocations: vec!["items".to_string(), "self::m".to_string()],
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "items!(\n        const _: u8 = 0;\n);\nother!(\n        const _: u8 = 0;\n);\nself::m!(\n        const _: u8 = 0;\n);\n",
      &named
    ),
    "items!(\n        const _: u8 = 0;\n);\nother!(\n    const _: u8 = 0;\n);\nself::m!(\n        const _: u8 = 0;\n);\n"
  );
  let all = FormatConfig {
    skip_macro_invocations: vec!["*".to_string()],
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with("fn f() {\n    foo!(a,   b);\n}\n", &all),
    "fn f() {\n    foo!(a,   b);\n}\n"
  );
}

#[test]
fn test_statements_the_standard_formatter_gives_up_on() {
  // A line past `max_width` after the first: the statement is left as written.
  let long = format!(
    "fn f() {{\n    g(\n  \"{}\",\n      );\n}}\n",
    "x".repeat(100)
  );
  assert_eq!(formatted(&long), long);
  // A statement marked to be skipped.
  let skipped = "fn f() {\n    #[rustfmt::skip]\n    let a = [1,0,\n               0,1];\n}\n";
  assert_eq!(formatted(skipped), skipped);
}

#[test]
fn test_stray_semicolon_lines_dropped() {
  // A `;` alone on its line goes with the line.
  assert_eq!(
    formatted(
      "fn main() {\n    ;\n           /* extra comment */            ;\n}\n\nfn main() {\n    println!(\"\");\n    // comment 1\n    // comment 2\n    // comment 3\n    // comment 4\n    ;\n}\n"
    ),
    "fn main() {\n    /* extra comment */\n}\n\nfn main() {\n    println!(\"\");\n    // comment 1\n    // comment 2\n    // comment 3\n    // comment 4\n}\n"
  );
}

#[test]
fn test_string_receivers_spanning_lines() {
  // A string spanning lines as a method's receiver.
  assert_eq!(
    formatted(
      "fn f() {\n        let mut deps\n        =\n        \"wasm-bindgen = { path = '{root}' }\\n\\\n             wasm-bindgen-futures = { path = '{root}/crates/futures' }\\n\"\n            .to_owned();\n}\n"
    ),
    "fn f() {\n    let mut deps = \"wasm-bindgen = { path = '{root}' }\\n\\\n             wasm-bindgen-futures = { path = '{root}/crates/futures' }\\n\"\n        .to_owned();\n}\n"
  );
}

#[test]
fn test_super_let() {
  // A `super let` is laid out as a `let`.
  assert_eq!(
    formatted(
      "#![feature(super_let)]\nfn main() {\n    super let x =(\n            &1,\n\n            ) else { 3};\n}\n"
    ),
    "#![feature(super_let)]\nfn main() {\n    super let x = (&1,) else { 3 };\n}\n"
  );
}

#[test]
fn test_tail_assignments_keep_no_room_for_a_semicolon() {
  let config = FormatConfig {
    max_width: 38,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "fn f() {\n    aaaa = bbbbbbbbbbbbbbbbbbbbbbbbbbb\n}\nfn g() {\n    aaaa = bbbbbbbbbbbbbbbbbbbbbbbbbbb;\n}\n",
      &config
    ),
    "fn f() {\n    aaaa = bbbbbbbbbbbbbbbbbbbbbbbbbbb\n}\nfn g() {\n    aaaa =\n        bbbbbbbbbbbbbbbbbbbbbbbbbbb;\n}\n"
  );
}

#[test]
fn test_trailing_comment_after_use_attribute() {
  // A use after an attribute ending in a comment joins it.
  assert_eq!(
    formatted(
      "#[cfg(feature = \"tokio\")]\n#[allow(unreachable_pub, unused_imports)] // https://github.com/rust-lang/rust/issues/57411\n\npub use self::tokio::*;\n#[allow(x)] // c\n\nfn f() {}\n"
    ),
    "#[cfg(feature = \"tokio\")]\n#[allow(unreachable_pub, unused_imports)] // https://github.com/rust-lang/rust/issues/57411\npub use self::tokio::*;\n#[allow(x)] // c\n\nfn f() {}\n"
  );
}

#[test]
fn test_try_blocks_on_one_line() {
  // A `try` block of one expression on one line wherever it stands.
  assert_eq!(
    formatted(
      "fn main() {\n    let _x: Option<_> = try {\n        4\n    };\n    let y = try bikeshed Option<_> {\n        4\n    };\n    try {\n        foo()?\n    }\n    try {}\n}\n"
    ),
    "fn main() {\n    let _x: Option<_> = try { 4 };\n    let y = try bikeshed Option<_> { 4 };\n    try { foo()? }\n    try {}\n}\n"
  );
}

#[test]
fn test_tuple_field_attributes() {
  // A tuple field's attributes on its line when one line, else above it.
  assert_eq!(
    formatted(
      "pub struct NewUser(#[diesel(column_name\n    =\tname)] String);\npub struct NewComment<'a>(#[diesel(column_name\t=\tpost_id)] pub\n    i32,\n    #[diesel(column_name\n    =\ttext)]\n    &'a str);\n#[diesel(table_name\n  = users)]\nstruct S;\n"
    ),
    "pub struct NewUser(\n    #[diesel(column_name\n    =\tname)]\n    String,\n);\npub struct NewComment<'a>(\n    #[diesel(column_name\t=\tpost_id)] pub i32,\n    #[diesel(column_name\n    =\ttext)]\n    &'a str,\n);\n#[diesel(table_name\n  = users)]\nstruct S;\n"
  );
}

#[test]
fn test_tuple_struct_with_where() {
  // `struct S<T>(..)` with no space before `(`.
  assert_eq!(
    formatted(
      "struct Exactly100CharsToSemicolon<A, B, C, D, E>\n    (LongLongTypename, i32, i32)\n    where A: LongTrait1234;\nstruct S<A>(A) where A: T;\n"
    ),
    "struct Exactly100CharsToSemicolon<A, B, C, D, E>(LongLongTypename, i32, i32)\nwhere\n    A: LongTrait1234;\nstruct S<A>(A)\nwhere\n    A: T;\n"
  );
}

#[test]
fn test_tuple_type_comments_ranges_closures_and_strings() {
  // Comments among a tuple type's elements in variants, tuple structs and parameters; a range
  // broken before `..`; a closure with a comment before its body as written; a string
  // spanning lines after a binary operator.
  let c2021 = FormatConfig::default();
  assert_eq!(
    formatted_with(
      "enum V {\n    #[regex(r\"[[:alpha:]][[:alnum:]]*\", variable_definition)]\n    Id((String /* variable name */,\n    String\n\t/* value */)),\n    Equals,\n}\nenum W {\n    Id((String /* variable name */,\n    String\n\t/* value */)),\n    Equals,\n}\nenum X {\n    Id((String /* variable name */, String /* value */)),\n}\n",
      &c2021
    ),
    "enum V {\n    #[regex(r\"[[:alpha:]][[:alnum:]]*\", variable_definition)]\n    Id((String /* variable name */, String /* value */)),\n    Equals,\n}\nenum W {\n    Id((String /* variable name */, String /* value */)),\n    Equals,\n}\nenum X {\n    Id((String /* variable name */, String /* value */)),\n}\n"
  );
  assert_eq!(
    formatted_with(
      "type A = (String /* variable name */,\n    String\n\t/* value */);\nstruct S(\n    (String /* variable name */,\n    String\n\t/* value */),\n);\nfn f(a: (String /* variable name */,\n    String\n\t/* value */)) {}\n",
      &c2021
    ),
    "type A = (String /* variable name */, String /* value */);\nstruct S((String /* variable name */, String /* value */));\nfn f(a: (String /* variable name */, String /* value */)) {}\n"
  );
  assert_eq!(
    formatted_with(
      "impl X {\n    fn span(&self) -> Span {\n        let x = self.offset_of_buffer_in_file + self.start_buffer_offset - self.last_token_size..self.offset_of_buffer_in_file + self.start_buffer_offset;\n        foo(self.offset_of_buffer_in_file + self.start_buffer_offset - self.last_token_size..self.offset_of_buffer_in_file + self.start_buffer_offset);\n        self.offset_of_buffer_in_file + self.start_buffer - self.last_token_size..self.offset_of_buffer_in_file\n    }\n}\n",
      &c2021
    ),
    "impl X {\n    fn span(&self) -> Span {\n        let x = self.offset_of_buffer_in_file + self.start_buffer_offset - self.last_token_size\n            ..self.offset_of_buffer_in_file + self.start_buffer_offset;\n        foo(\n            self.offset_of_buffer_in_file + self.start_buffer_offset - self.last_token_size\n                ..self.offset_of_buffer_in_file + self.start_buffer_offset,\n        );\n        self.offset_of_buffer_in_file + self.start_buffer - self.last_token_size\n            ..self.offset_of_buffer_in_file\n    }\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    foo(|x|   // c\n      x + 1);\n    foo(|x|   /* c */ {\n      a;\n  b\n    });\n    let a =  |x|  // c\n        {   a;  b };\n    foo(|x| {\n        a;   // c\n        b\n    });\n}\n",
      &c2021
    ),
    "fn f() {\n    foo(|x|   // c\n      x + 1);\n    foo(|x|   /* c */ {\n      a;\n  b\n    });\n    let a = |x|  // c\n        {   a;  b };\n    foo(|x| {\n        a; // c\n        b\n    });\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let s = a + \"x\n  y\";\n    let s = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + \"x\n  y\";\n    foo(&(format!(\"a\") + r#\"\nb\n\"#));\n    x == \"a\nb\";\n}\n",
      &c2021
    ),
    "fn f() {\n    let s = a + \"x\n  y\";\n    let s = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n        + \"x\n  y\";\n    foo(&(format!(\"a\")\n        + r#\"\nb\n\"#));\n    x == \"a\nb\";\n}\n"
  );
  // A pattern's `..` with attributes, which rustc rejects, is left as written.
  assert_eq!(
    formatted("fn f() {\n    let S { #[cfg(any())] .. } = ();\n}\n"),
    "fn f() {\n    let S { #[cfg(any())] .. } = ();\n}\n"
  );
}

#[test]
fn test_type_alias_where_before_value() {
  // A `where` before a type alias's value: the clause, then `= value;` on its line.
  assert_eq!(
    formatted(
      "pub type Adder<F, T> where T: Clone, F: Copy = impl Fn(T) -> T;\ntype A<T> where T: Clone = Vec<T>;\ntype B<T> = Vec<T> where T: Clone;\ntrait X { type C<T> where T: Clone = Vec<T>; type D<'a> where Self: 'a; }\n"
    ),
    "pub type Adder<F, T>\nwhere\n    T: Clone,\n    F: Copy,\n= impl Fn(T) -> T;\ntype A<T>\nwhere\n    T: Clone,\n= Vec<T>;\ntype B<T>\n    = Vec<T>\nwhere\n    T: Clone;\ntrait X {\n    type C<T>\n    where\n        T: Clone,\n    = Vec<T>;\n    type D<'a>\n    where\n        Self: 'a;\n}\n"
  );
}

#[test]
fn test_use_comment_before_semicolon_moves_after_it() {
  // A block comment between a use's tree and its `;` goes after the `;`.
  assert_eq!(
    formatted(
      "use a::{item /* comment */};\nuse b::{\n    a,\n    // comment\n    item,\n};\nuse c::item /* comment */;\nuse d::item; // really long comment (with `use` exactly 100 characters) ____________________________\n\nuse std::e::{/* it's a comment! */ bar /* and another */};\nuse std::f::{/* it's a comment! */ bar};\nuse std::g::{bar /* and another */};\n"
    ),
    "use a::{item /* comment */};\nuse b::{\n    a,\n    // comment\n    item,\n};\nuse c::item; /* comment */\nuse d::item; // really long comment (with `use` exactly 100 characters) ____________________________\n\nuse std::e::{/* it's a comment! */ bar /* and another */};\nuse std::f::{/* it's a comment! */ bar};\nuse std::g::{bar /* and another */};\n"
  );
}

#[test]
fn test_use_lists_with_comments() {
  // Comments in a use list move with their items when sorted; block comments alone stay
  // on one line.
  assert_eq!(
    formatted(
      "use super::{\n    C,\n    A,\n    // x1\n    B,\n};\nuse foo::{ self /* a */ };\nuse foo::{\n    self // this is important\n};\n"
    ),
    "use super::{\n    A,\n    // x1\n    B,\n    C,\n};\nuse foo::{self /* a */};\nuse foo::{\n    self, // this is important\n};\n"
  );
}

#[test]
fn test_use_trees_normalized() {
  // Braces around one item go, `a::self` is `a`, nested lists are sorted.
  assert_eq!(
    formatted(
      "use super::{auxvec};\nuse crate::lexer::self;\nuse a::{l::{self, m, n::o, p::*}};\nuse std::{*};\nuse outer::tree::{inner::tree};\nuse a::{b as c, self as d};\npub use x::{y::self as z};\nuse ::log::{error, info, log};\n"
    ),
    "use super::auxvec;\nuse crate::lexer;\nuse ::log::{error, info, log};\nuse a::l::{self, m, n::o, p::*};\nuse a::{self as d, b as c};\nuse outer::tree::inner::tree;\nuse std::*;\npub use x::y as z;\n"
  );
}

#[test]
fn test_variant_fields_only_a_comment() {
  // Fields that are only a block comment on one line close up around it.
  assert_eq!(
    formatted(
      "enum E {\n    A {\n        /* c */\n    } = 1,\n    B(\n        /* c */\n    ) = 2,\n}\n"
    ),
    "enum E {\n    A {/* c */} = 1,\n    B(/* c */) = 2,\n}\n"
  );
}

#[test]
fn test_visibility_paths() {
  // `pub(in crate)` is `pub(crate)`; a path from the root loses its `::`.
  assert_eq!(
    formatted("pub(in   crate) struct S;\npub(in self) fn f() {}\npub(in crate::a) struct T;\n"),
    "pub(crate) struct S;\npub(self) fn f() {}\npub(in crate::a) struct T;\n"
  );
  assert_eq!(
    formatted("pub(in ::global::path) fn f() {}\npub(in   ::a) struct S;\n"),
    "pub(in global::path) fn f() {}\npub(in a) struct S;\n"
  );
}

#[test]
fn test_what_f5b_leaves_as_the_standard_formatter_does() {
  // Extern bodies keep a blank line at either end.
  let extern_block = "extern \"C\" {\n\n    fn f();\n\n}\n";
  assert_eq!(formatted(extern_block), extern_block);
  // A closure's lines count toward the statement around it: one line too long and the
  // whole statement is left as written.
  let long = format!(
    "fn f() {{\n    let v = x.map(|a| {{\n\n          g(\"{}\")\n    }});\n}}\n",
    "y".repeat(100)
  );
  assert_eq!(formatted(&long), long);
  // A block of one expression that is a binary operator's operand goes on one line.
  let operand = "fn f() {\n    a\n        && {\n            b\n        };\n}\n";
  assert_eq!(formatted(operand), "fn f() {\n    a && { b };\n}\n");
}

#[test]
fn test_what_is_left_as_written() {
  // A macro definition whose transcriber repeats (`$(..)`).
  let source = "macro_rules! m {\n  ($($a:expr),*) => {\n        $($a)*\n  };\n}\n#[rustfmt::skip]\nfn  f() {\n  x( );\n}\n// mto: off\nfn   g() {\n x();\n}\n// mto: on\n";
  assert_eq!(formatted(source), source);
  let skipped = "#![rustfmt::skip]\nfn  f() {\n  x();\n}\n";
  assert_eq!(formatted(skipped), skipped);
  let broken = "fn f( {\n  x();\n";
  assert_eq!(
    format_file(broken, Edition::E2021, &FormatConfig::default()),
    broken
  );
}

#[test]
fn test_what_no_layout_fits_and_macros_kept_as_written() {
  let config = FormatConfig {
    max_width: 79,
    ..FormatConfig::default()
  };
  // A statement no layout fits is left as written whole, its spaces too; a macro call
  // whose arguments do not fit on the next line is kept there as written (`write!(  f,`),
  // which the next line is chosen for.
  assert_eq!(
    formatted_with(
      "impl X {\n    fn f() {\n        let\tall\n    =\t<<V as\n\tVector>::Eq\n    as  CmpEq>::Mask::all_zeros_except_least_significant(0);\n        let x = write!(  f,\n        \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n    );\n        match *self {\n            A =>\n    write!(  f,\n        \"error computing start state because of cache inefficiency\"\n    ),\n            _ => 1,\n        }\n    }\n}\n",
      &config
    ),
    "impl X {\n    fn f() {\n        let\tall\n    =\t<<V as\n\tVector>::Eq\n    as  CmpEq>::Mask::all_zeros_except_least_significant(0);\n        let x = write!(\n            f,\n            \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n        );\n        match *self {\n            A => {\n                write!(  f,\n        \"error computing start state because of cache inefficiency\"\n    )\n            }\n            _ => 1,\n        }\n    }\n}\n"
  );
  // A method chain holding a line past the line width is left as written; a block of
  // comments keeps the first on its `{`'s line, the others a column further in; a
  // `lazy_static!` call's entries are laid out as statics; a macro definition left as
  // written has its attributes laid out, and one space before its comment.
  assert_eq!(
    formatted(
      "fn f() {\n    fields.map(|field| { a; { match x { None => { (w, t) = quote!({ _serde::de::SeqAccess::next_element::<#wrapper_ty>(&mut __seq) { } }) } }; } });\n    match res {\n        Ok(_) => {  // first\n            // second\n        }\n        Err(a) => {  // third\n            g();\n        }\n    }\n}\nlazy_static! {\n        static  ref  A:\tMutex<u32> = {\n  Mutex::new(1)\n};\n    pub static ref B: u8 =\n2;\n}\n/// Doc one\n        /// doc two\n#[macro_export]\n\tmacro_rules!\n    m\n{ ($($tt:tt)*) => { if  a() && b() && c() && d() && e() && f() && g() && h() && i() && j() && k() { $($tt)* } } }\t// After.\n"
    ),
    "fn f() {\n    fields.map(|field| { a; { match x { None => { (w, t) = quote!({ _serde::de::SeqAccess::next_element::<#wrapper_ty>(&mut __seq) { } }) } }; } });\n    match res {\n        Ok(_) => { // first\n             // second\n        }\n        Err(a) => {\n            // third\n            g();\n        }\n    }\n}\nlazy_static! {\n    static ref A: Mutex<u32> = { Mutex::new(1) };\n    pub static ref B: u8 = 2;\n}\n/// Doc one\n/// doc two\n#[macro_export]\nmacro_rules!\n    m\n{ ($($tt:tt)*) => { if  a() && b() && c() && d() && e() && f() && g() && h() && i() && j() && k() { $($tt)* } } } // After.\n"
  );
}

#[test]
fn test_widths_tabs_and_continued_strings() {
  // A tab counts one column; from the 2024 style a string whose lines but the last end with `\`
  // is kept however wide, its call broken around it; a struct pattern's field moved to the
  // next line keeps the field's width.
  let c2021 = FormatConfig::default();
  let c2024 = FormatConfig {
    style_edition: 2024,
    ..FormatConfig::default()
  };
  let cmax = FormatConfig {
    style_edition: 2024,
    fn_call_width: 100,
    chain_width: 100,
    struct_lit_width: 100,
    struct_variant_width: 100,
    array_width: 100,
    attr_fn_like_width: 100,
    single_line_if_else_max_width: 100,
    single_line_let_else_max_width: 100,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "fn f() {\n    if a {\n        if b {\n            if c {\n                if d {\n                    error_inferred.insert(ty, quote!(::thiserror::#private::Errorrrrrrrr\t+ 'static));\n                }\n            }\n        }\n    }\n}\n",
      &c2021
    ),
    "fn f() {\n    if a {\n        if b {\n            if c {\n                if d {\n                    error_inferred\n                        .insert(ty, quote!(::thiserror::#private::Errorrrrrrrr\t+ 'static));\n                }\n            }\n        }\n    }\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let query = format!(\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n         FROM\");\n    let query = foo(\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n         FROM\");\n    let query = foo(a, \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n         FROM\");\n    foo(\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n         FROM\");\n}\n",
      &c2024
    ),
    "fn f() {\n    let query = format!(\n        \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n         FROM\"\n    );\n    let query = foo(\n        \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n         FROM\",\n    );\n    let query = foo(\n        a,\n        \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n         FROM\",\n    );\n    foo(\n        \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n         FROM\",\n    );\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    let bind = \"?\";\n    let\nquery\n\t=\tformat!(  \"SELECT u.id as myuser_id, u.name, u.hair_color, p.id as post_id, p.user_id , p.title, p.body \\\n         FROM users as u LEFT JOIN posts as p on u.id = p.user_id WHERE u.hair_color = {bind}\"\n    );\n}\n",
      &c2024
    ),
    "fn f() {\n    let bind = \"?\";\n    let query = format!(\n        \"SELECT u.id as myuser_id, u.name, u.hair_color, p.id as post_id, p.user_id , p.title, p.body \\\n         FROM users as u LEFT JOIN posts as p on u.id = p.user_id WHERE u.hair_color = {bind}\"\n    );\n}\n"
  );
  assert_eq!(
    formatted_with(
      "fn f() {\n    {\n        {\n            if let NameRefContext { kind: NameRefKind::Path(PathCompletionCtx { kind: PathKind::Expr { .. }, path, .. }, ..), .. } = &nameref_ctx && is_in_token_of_for_loop(path) {}\n        }\n    }\n}\n",
      &cmax
    ),
    "fn f() {\n    {\n        {\n            if let NameRefContext {\n                kind:\n                    NameRefKind::Path(PathCompletionCtx { kind: PathKind::Expr { .. }, path, .. }, ..),\n                ..\n            } = &nameref_ctx\n                && is_in_token_of_for_loop(path)\n            {}\n        }\n    }\n}\n"
  );
}
