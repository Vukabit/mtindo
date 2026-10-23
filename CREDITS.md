# Credits

`mto` is written from scratch and depends on nothing but the Rust standard
library. It owes a great deal to the projects below: what they define, what
they teach, and what they let it be tested against. None of their code is
part of `mto` except the Unicode data noted below.

## The standard formatter

[rustfmt](https://github.com/rust-lang/rustfmt) defines the Rust style that
`mto` formats in. `mto` takes after it in its formatting rules, its settings
(`rustfmt.toml` and every key in it), its command line and its exit codes,
and reproduces its decisions, including where it leaves code as written.

`mto` checks itself against rustfmt's own test suite, input and expected
output pairs with their settings, and against rustfmt's output on the
repositories listed below. Some of the fuzz inputs kept as regression tests
in `tests/fuzz_regressions/` grew from rustfmt's test files. rustfmt is
licensed under the Apache License 2.0 or the MIT license. Thanks to its
authors and maintainers.

## The Rust language

* [The Rust Reference](https://doc.rust-lang.org/reference/) and
  [rustc](https://github.com/rust-lang/rust) define what `mto` parses: its
  lexer is compared token by token with rustc's own, and its parser follows
  the grammar rustc accepts.
* The [Rust Style Guide](https://doc.rust-lang.org/style-guide/) describes
  the style, including the version sorting `mto` uses for names.

## Data

* The identifier and width tables in `src/text/unicode` are derived from the
  [Unicode Character Database](https://www.unicode.org/ucd/), under the terms
  in [LICENSE-UNICODE](LICENSE-UNICODE).
* `mto`'s TOML parser is checked against
  [toml-test](https://github.com/toml-lang/toml-test), the TOML conformance
  suite, which is fetched for the check and not included.

## The corpus

`mto` is measured on these repositories, pinned to a commit in
`compat/corpus.txt`, fetched for the checks and never included. Fifteen of
them are also built after layout, to prove they still compile. Each is the
work of its own authors, under its own licence.

anyhow, axum, base64, bitflags, bytes, chrono, clap, crossbeam, diesel,
futures-rs, hashbrown, hyper, indexmap, itertools, itoa, lazy_static, libc,
log, logos, memchr, nom, once_cell, parking_lot, proc-macro2, quote, rand,
rayon, regex, reqwest, ripgrep, rust-analyzer, rustfmt, serde, serde_json,
smallvec, syn, termcolor, thiserror, time, tokio, tracing, url, uuid and
wasm-bindgen; and, on request, rust itself.

## Ideas

* The lossless syntax tree, every byte kept and trivia attached to nodes,
  follows the red and green tree design of Roslyn and rust-analyzer.
* Flushing a run's writes once at its end, instead of after each file,
  follows git's `core.fsyncMethod = batch`.
