---
name: rust-best-practices
description: >
  Guide for writing idiomatic Rust code based on Apollo GraphQL's best practices handbook. Use this skill when:
  (1) writing new Rust code or functions,
  (2) reviewing or refactoring existing Rust code,
  (3) deciding between borrowing vs cloning or ownership patterns,
  (4) implementing error handling with Result types,
  (5) optimizing Rust code for performance,
  (6) writing tests or documentation for Rust projects.
license: MIT
compatibility: Rust 1.70+, Cargo
metadata:
  author: apollographql
  version: "1.1.2"
---

# Rust Best Practices

Apply these guidelines when writing or reviewing Rust code. Based on Apollo GraphQL's [Rust Best Practices Handbook](https://github.com/apollographql/rust-best-practices).

## Hive's rules come first

- **Never silence a lint or a test** (`CLAUDE.md` rule 6): no `#[allow(...)]`, no `#[expect(...)]`, no `#[ignore]`, no `#[coverage(off)]`, no `#[mutants::skip]`. Fix the code; a lint you believe is wrong goes to the human. Changing `clippy.toml` or the workspace lint table is changing a gate: the human's decision.
- **No `unwrap()` / `expect()` outside tests**, binaries included: a panic in the service kills every terminal.
- **No new dependency or host tool** (`insta`, `rstest`, `flamegraph`, a nightly toolchain) without the human (`CLAUDE.md` rule 3); crates go in with `cargo add -p <crate>`, never by editing `Cargo.toml`.
- **The gates:** `cargo fmt --all --check` (stable rustfmt, the repository's config) and `cargo clippy --workspace --all-targets -- -D warnings`; 100% line coverage and no missed mutant still apply.
- Agent shells need `export PATH=$HOME/.cargo/bin:$PATH`.

## Best Practices Reference

Before reviewing, familiarize yourself with Apollo's Rust best practices. Read ALL relevant chapters in the same turn in parallel. Reference these files when providing feedback:

- [Chapter 1 - Coding Styles and Idioms](references/chapter_01.md): Borrowing vs cloning, Copy trait, Option/Result handling, iterators, comments, when to extract a function (duplication vs. wrong abstraction)
- [Chapter 2 - Clippy and Linting](references/chapter_02.md): Clippy configuration, important lints, workspace lint setup
- [Chapter 3 - Performance Mindset](references/chapter_03.md): Profiling, avoiding redundant clones, stack vs heap, zero-cost abstractions
- [Chapter 4 - Error Handling](references/chapter_04.md): Result vs panic, thiserror vs anyhow, error hierarchies
- [Chapter 5 - Automated Testing](references/chapter_05.md): Test naming, one assertion per test, snapshot testing
- [Chapter 6 - Generics and Dispatch](references/chapter_06.md): Static vs dynamic dispatch, trait objects
- [Chapter 7 - Type State Pattern](references/chapter_07.md): Compile-time state safety, when to use it
- [Chapter 8 - Comments vs Documentation](references/chapter_08.md): When to comment, doc comments, rustdoc
- [Chapter 9 - Understanding Pointers](references/chapter_09.md): Thread safety, Send/Sync, pointer types

## Quick Reference

### Borrowing & Ownership
- Prefer `&T` over `.clone()` unless ownership transfer is required
- Use `&str` over `String`, `&[T]` over `Vec<T>` in function parameters
- Small `Copy` types (≤24 bytes) can be passed by value
- Use `Cow<'_, T>` when ownership is ambiguous

### Error Handling
- Return `Result<T, E>` for fallible operations; avoid `panic!` in production
- Never use `unwrap()`/`expect()` outside tests
- Use `thiserror` for library errors, `anyhow` for binaries only
- Prefer `?` operator over match chains for error propagation

### Performance
- Always benchmark with `--release` flag
- Run `cargo clippy -- -D clippy::perf` for performance hints
- Avoid cloning in loops; use `.iter()` instead of `.into_iter()` for Copy types
- Prefer iterators over manual loops; avoid intermediate `.collect()` calls

### Linting
Run regularly: `cargo clippy --workspace --all-targets -- -D warnings` (Hive's gate)

Key lints to watch:
- `redundant_clone` - unnecessary cloning
- `large_enum_variant` - oversized variants (consider boxing)
- `needless_collect` - premature collection

Fix the warning; never silence it with `#[expect]` or `#[allow]` (Hive rule 6). A false positive goes to the human.

### Testing
- Name tests descriptively: `process_should_return_error_when_input_empty()`
- One assertion per test when possible
- Use doc tests (`///`) for public API examples
- Snapshot testing (`insta`) is not a Hive dependency: adding it is the human's call

### Generics & Dispatch
- Prefer generics (static dispatch) for performance-critical code
- Use `dyn Trait` only when heterogeneous collections are needed
- Box at API boundaries, not internally

### Type State Pattern
Encode valid states in the type system to catch invalid operations at compile time:
```rust
struct Connection<State> { /* ... */ _state: PhantomData<State> }
struct Disconnected;
struct Connected;

impl Connection<Connected> {
    fn send(&self, data: &[u8]) { /* only connected can send */ }
}
```

### Documentation
- `//` comments explain *why* (safety, workarounds, design rationale)
- `///` doc comments explain *what* and *how* for public APIs
- Every `TODO` needs a linked issue: `// TODO(#42): ...`
- Enable `#![deny(missing_docs)]` for libraries
