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
`compat/corpus.txt`, fetched for the checks and never included. Each is the
work of its own authors, under its own licence, and is named here with the
account or organization that hosts it.

Formatted and laid out, then built, to prove they still compile:

* [anyhow](https://github.com/dtolnay/anyhow), by dtolnay
* [bitflags](https://github.com/bitflags/bitflags), by bitflags
* [bytes](https://github.com/tokio-rs/bytes), by tokio-rs
* [clap](https://github.com/clap-rs/clap), by clap-rs
* [crossbeam](https://github.com/crossbeam-rs/crossbeam), by crossbeam-rs
* [itoa](https://github.com/dtolnay/itoa), by dtolnay
* [lazy_static](https://github.com/rust-lang-nursery/lazy-static.rs),
  by rust-lang-nursery
* [logos](https://github.com/maciejhirsz/logos), by maciejhirsz
* [once_cell](https://github.com/matklad/once_cell), by matklad
* [rand](https://github.com/rust-random/rand), by rust-random
* [rayon](https://github.com/rayon-rs/rayon), by rayon-rs
* [serde](https://github.com/serde-rs/serde), by serde-rs
* [termcolor](https://github.com/BurntSushi/termcolor), by BurntSushi
* [tokio](https://github.com/tokio-rs/tokio), by tokio-rs
* [uuid](https://github.com/uuid-rs/uuid), by uuid-rs

Formatted and laid out:

* [axum](https://github.com/tokio-rs/axum), by tokio-rs
* [base64](https://github.com/marshallpierce/rust-base64), by marshallpierce
* [chrono](https://github.com/chronotope/chrono), by chronotope
* [diesel](https://github.com/diesel-rs/diesel), by diesel-rs
* [futures-rs](https://github.com/rust-lang/futures-rs), by rust-lang
* [hashbrown](https://github.com/rust-lang/hashbrown), by rust-lang
* [hyper](https://github.com/hyperium/hyper), by hyperium
* [indexmap](https://github.com/indexmap-rs/indexmap), by indexmap-rs
* [itertools](https://github.com/rust-itertools/itertools), by rust-itertools
* [libc](https://github.com/rust-lang/libc), by rust-lang
* [log](https://github.com/rust-lang/log), by rust-lang
* [memchr](https://github.com/BurntSushi/memchr), by BurntSushi
* [nom](https://github.com/rust-bakery/nom), by rust-bakery
* [parking_lot](https://github.com/Amanieu/parking_lot), by Amanieu
* [proc-macro2](https://github.com/dtolnay/proc-macro2), by dtolnay
* [quote](https://github.com/dtolnay/quote), by dtolnay
* [regex](https://github.com/rust-lang/regex), by rust-lang
* [reqwest](https://github.com/seanmonstar/reqwest), by seanmonstar
* [ripgrep](https://github.com/BurntSushi/ripgrep), by BurntSushi
* [rust-analyzer](https://github.com/rust-lang/rust-analyzer), by rust-lang
* [rustfmt](https://github.com/rust-lang/rustfmt), by rust-lang
* [serde_json](https://github.com/serde-rs/json), by serde-rs
* [smallvec](https://github.com/servo/rust-smallvec), by servo
* [syn](https://github.com/dtolnay/syn), by dtolnay
* [thiserror](https://github.com/dtolnay/thiserror), by dtolnay
* [time](https://github.com/time-rs/time), by time-rs
* [tracing](https://github.com/tokio-rs/tracing), by tokio-rs
* [url](https://github.com/servo/rust-url), by servo
* [wasm-bindgen](https://github.com/wasm-bindgen/wasm-bindgen), by wasm-bindgen

On request (`--with-rust`), also [rust](https://github.com/rust-lang/rust), by
rust-lang.

## Ideas

* The lossless syntax tree, every byte kept and trivia attached to nodes,
  follows the red and green tree design of Roslyn and rust-analyzer.
* Flushing a run's writes once at its end, instead of after each file,
  follows git's `core.fsyncMethod = batch`.
