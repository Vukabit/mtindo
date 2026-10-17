# Metrics

What mtindo measures about itself, one paper per category. Each paper is
Markdown with YAML front matter: the front matter holds the figures as
data, and the body explains the question, the method, the results, what
could make them wrong, and how to reproduce them. Every field and figure is
defined once in the [data dictionary](dictionary.md).

The figures and tables are generated from data the repository records; the
prose around them is written by hand. A generated part sits between
`<!-- generated: NAME -->` and `<!-- end generated -->` comments, and
`cargo xtask docs metrics` rewrites only those parts and the front matter.
It reads what the suites recorded and runs none of them.

| part | read from | checked by the gate |
|---|---|---|
| front matter, most tables | tracked records and their history | yes |
| tables marked as local | records on the generating machine | no |
| `generated`, `commit` | when and where the page last changed | no |

The `metrics-docs` gate of `cargo xtask test` fails when a checked part is
out of date with the tracked records it comes from.

## Papers

The headline figure of each paper:

<!-- generated: index -->
| paper | headline |
|---|---|
| [compatibility](compatibility.md) | 98.98% of scrambled-corpus lines match |
| [meaning](meaning.md) | 32 exceptions listed, 10 not modelled yet |
| [fuzzing](fuzzing.md) | 8 targets, 20 kept regression inputs |
| [performance](performance.md) | 9.702 MB/s formatting on one thread |
| [scrambled-pairs](scrambled-pairs.md) | no data recorded yet |
| [compile-safety](compile-safety.md) | no data recorded yet |
| [layout](layout.md) | no data recorded yet |
<!-- end generated -->

A paper whose status is `planned` describes a measurement the repository
does not record yet; it holds no figures until a command records them.
