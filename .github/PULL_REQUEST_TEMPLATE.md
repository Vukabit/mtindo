## What it does

<!-- For someone using mto: what changes for them. -->

## Checks

- [ ] Commits follow `kind(scope): subject` (CONTRIBUTING.md); the hooks
      are on (`cargo xtask bootstrap --go`).
- [ ] A test covers the change; a formatting fix's expected output is the
      standard formatter's.
- [ ] For formatting: `cargo xtask diff` passes; its numbers:

<!-- corpus files and lines, before and after -->
