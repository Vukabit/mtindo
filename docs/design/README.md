# How mto works

These pages describe the code as it is: what each part does and why. Start
with the overview, which follows one run from start to finish.

* [Overview](overview.md): one run end to end, the source map, the layers.
* [Syntax](syntax.md): the lexer, the parser and the lossless syntax tree.
* [Comment attachment](comment-attachment.md): which node each comment
  belongs to.
* [Layout](layout.md): how items are split into units, ordered and held.
* [Formatting](formatting.md): how code is laid out in the Rust style.
* [The meaning check](meaning-check.md): what is checked before any write.
* [Writes and history](writes.md): atomic writes, durability, history,
  restore.
* [The build guard](build-guard.md): checking the build and narrowing edits
  down.

For using `mto`, see the [guide](../README.md#guide).
