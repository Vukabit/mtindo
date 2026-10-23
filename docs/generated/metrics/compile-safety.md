---
paper: "compile-safety"
title: "Compile safety"
status: "planned"
generated: "2026-10-15T21:59:29.408Z"
commit: "44f55b3-dirty.3b716a01"
command: "none yet"
sources: []
seed: "none"
metrics: {}
---

# Compile safety

## Abstract

<!-- generated: abstract -->
No data is recorded for this paper yet. Section 2 describes the planned method;
section 3 will hold the results once a command records them.
<!-- end generated -->

## 1 Question

Does a real crate that compiles before mto lays it out still compile after,
and do its tests still pass? The meaning check compares tokens; compiling
the result is the independent test that nothing the check cannot see
(item order that matters, macros that read their surroundings) was broken.

## 2 Method

Planned as a recorded measurement; a pass or fail check already exists.

Today the suite's real-world test (in the `heavy` group of `cargo xtask
test`) checks out 15 crates of the corpus at their pinned commits and lays
each out with three configurations: the defaults, then two mixes that
between them set every layout setting away from its default. Each crate's
library must pass `cargo check` after each layout (a crate that fails
before layout is reported as not mto's fault). A 500-file generated
repository is laid out and compiled the same way in the `tests` group. A
failure fails the suite, and a crate that stops compiling is filed as a
`compile-break` incident. Only pass or fail is kept, on the machine that ran
it.

The planned measurement would record, per crate and configuration: whether
the crate compiled before, whether it compiled after, whether its tests
passed after, and how many files and items layout moved. The same would run
with formatting on (`format = true`) once formatting is out of preview. The
counts would be kept in a tracked file and committed, so that this page can
be generated from them and checked like the others.

## 3 Results

<!-- generated: results -->
No data is recorded yet.
<!-- end generated -->

## 4 Threats to validity

- Compiling proves the result is a valid program, not the same one: a moved
  item can compile and behave differently (for example, through a macro that
  depends on order). The meaning check and the crate's own tests cover that.
- Fifteen crates are a small sample; they were chosen for varied syntax
  (macros, async code, unsafe code, generics) and sizes.
- Compilation depends on the toolchain and on the network for dependencies;
  a failure for those reasons must not be counted against mto.

## 5 Reproduction

```bash
# The real-world compile check as it stands (network, slow)
cargo xtask test --target domain_13_rw
```

No command records the planned figures yet. When one does, it will be listed
here with the command that regenerates this page, `cargo xtask docs
metrics`.

## 6 Changelog

- 2026-10-15: skeleton; the method is planned and no data is recorded.
