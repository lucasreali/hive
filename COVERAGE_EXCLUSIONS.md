# Coverage exclusions

Files excluded from `cargo llvm-cov` (via `--ignore-filename-regex`) and from `cargo mutants` (via `.cargo/mutants.toml`). Every entry was approved by the human.

| File | Reason | Approved |
|---|---|---|
| `src-tauri/src/main.rs` | Pure wiring. It builds the Tauri app, registers plugins and commands, and runs the event loop, which needs a display and a WebView. It may only contain `tauri::Builder` setup; commands and any logic live in `src-tauri/src/lib.rs`, which is covered. | 2026-09-23 |
| `crates/hive/src/macos.rs` | macOS-only glue, not compiled on Linux; tested with mutants and coverage by the macOS CI in 5.3. | 2026-09-24 |
| `src-tauri/src/macos.rs` | macOS-only glue (17.19: the notification center through the Objective-C runtime), not compiled on Linux; excluded only from the Linux mutants (`.cargo/mutants.toml`) and tested with coverage (100% lines) and mutants by `macos.yml`, like `crates/hive/src/macos.rs`. | pending the human (17.19) |
| `crates/hive/src/windows*` | Native-Windows-only code (12.5), not compiled on Linux; tested with coverage (100% lines) and mutants by the Windows CI (`windows.yml`). | 2026-09-28 |
