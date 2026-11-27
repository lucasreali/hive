# Source of `rust-best-practices`

- **Upstream:** https://github.com/apollographql/skills, path `skills/rust-best-practices/`
- **Commit:** `8d0129fdc96a1b80d999a6682e48fc11e137dc09`
- **License:** MIT
- **Review:** `docs/review/skills.md` (task 28.3); recommendation **Adapt**, approved by the human on 2026-11-27.
- **Copied:** Hive task 28.4. Fetched on 2026-11-27 as the pinned commit's tarball (`codeload.github.com/<repo>/tar.gz/<commit>`) into a new empty folder outside the repository (the agent's worktree isolation refused a `git clone` there); read as text, no script from it was run.

## What changed

- Added "Hive's rules come first"; `allowed-tools` removed; the clippy command is Hive's gate.
- Removed advice that silences gates or installs tools: `#[expect]` / `#[allow]` with a justification (chapter 2 rewritten to "fix it or ask the human"), `#[ignore]` (chapter 5), `rustup update`, `cargo +nightly fmt` (chapter 1), global `cargo install flamegraph` / `cargo-insta` (chapters 3, 5: host tools the human installs; `insta` is a dependency needing approval).

## License notice

```
MIT License

Copyright (c) 2024 Apollo Graph, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
