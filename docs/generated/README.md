# Generated pages

Written from `src/` and [records/](../../records/); never edit them by hand.
`cargo xtask docs` writes them and `cargo xtask docs --check` (the `docs`
gate) fails when one is out of date. Code generated into `src/` is listed too,
since it is written the same way.

<!-- registry: written by cargo xtask docs from xtask/src/registry.rs -->

- **[manual.md](manual.md)** (page): the manual, as `mto --man` prints it. From
  src/cli/manual.rs.
- **[commands.md](commands.md)** (page): every xtask command and its arguments.
  From xtask's command table.
- **[config/README.md](config/README.md)** (page): the settings reference's
  index. From src/options, records/policy/capabilities.txt.
- **[config/mto.toml.md](config/mto.toml.md)** (page): mto.toml's settings. From
  src/options (the settings table).
- **[config/rustfmt.toml.md](config/rustfmt.toml.md)** (page): rustfmt.toml's
  settings, as mto honors them. From src/options,
  records/policy/capabilities.txt.
- **[config/config.toml.md](config/config.toml.md)** (page): the per-user
  config.toml's settings. From src/options (the settings table).
- **[config/mto.schema.json](config/mto.schema.json)** (page): mto.toml's JSON
  Schema, for editors. From src/options (the settings table).
- **[metrics/README.md](metrics/README.md)** (page): the metrics pages' index.
  From records/results/*.
- **[metrics/compatibility.md](metrics/compatibility.md)** (page): compatibility
  with the standard formatter. From records/results/compatibility.txt, history.
- **[metrics/meaning.md](metrics/meaning.md)** (page): the meaning check and its
  exceptions. From records/policy/exceptions.txt, history.
- **[metrics/performance.md](metrics/performance.md)** (page): speed and memory.
  From records/results/ratchet.txt, baseline.txt; .dev/series.
- **[metrics/fuzzing.md](metrics/fuzzing.md)** (page): fuzz targets and kept
  regression inputs. From fuzz/, tests/fuzz_regressions, history; .dev.
- **[metrics/layout.md](metrics/layout.md)** (page): layout's checks. From the
  suites; .dev.
- **[metrics/compile-safety.md](metrics/compile-safety.md)** (page): real crates
  still compiling after mto. From records/pins/corpus.txt; .dev.
- **[metrics/scrambled-pairs.md](metrics/scrambled-pairs.md)** (page): scrambled
  corpus pairs. From .dev/series.
- **[metrics/dictionary.md](metrics/dictionary.md)** (page): what each metric
  means. From xtask/src/papers/dictionary.rs.
- **`src/options/rustfmt.rs`** (code): the standard formatter's settings, from
  its pinned releases. From the standard formatter's sources (records/upstream).
- **`src/options/support.rs`** (code): how far mto honors each option and value.
  From records/policy/capabilities.txt.
- **`src/text/unicode/ident_tables.rs`** (code): identifier character classes.
  From the Unicode Character Database.
- **`src/text/unicode/punctuation_tables.rs`** (code): other punctuation, where
  comments may break. From the Unicode Character Database.
- **`src/text/unicode/width_tables.rs`** (code): display widths. From the
  Unicode Character Database.

<!-- end registry -->
