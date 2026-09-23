# Coverage exclusions

Files excluded from `cargo llvm-cov` (via `--ignore-filename-regex`) and from `cargo mutants` (via `.cargo/mutants.toml`). Every entry was approved by the human.

| File | Reason | Approved |
|---|---|---|
| `src-tauri/src/main.rs` | Pure wiring. It builds the Tauri app, registers plugins and commands, and runs the event loop, which needs a display and a WebView. It may only contain `tauri::Builder` setup; commands and any logic live in `src-tauri/src/lib.rs`, which is covered. | 2026-09-23 |
