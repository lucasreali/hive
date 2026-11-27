# Spike 28.15: Code intelligence in the editor

Design task 28.15 (`TODO.md`, D34.1, D34.4: no production code before the human approves). Decisions read: #6 ("sem LSP na v1"), #31 (CodeMirror 6), #37 (domain logic in Rust), #47 (settings file), 23.10 (find in a file), the `search_files` search (`hive::search`), the remote hosts (Stage 26, 26.19's limits). Written 2026-11-27 against `main` at `89ec3828`, on the Apple Silicon host (macOS 26). All experiments ran from throwaway scripts outside the committed tree (section 11) and were deleted; nothing in the repository changed except this file.

## 1. Recommendation in one paragraph

**Highlighting stays in the frontend, on CodeMirror's Lezer grammars**, and is fixed where it was measured to be poor: fenced code in Markdown, YAML values, JSON with comments, a list of file names that get no language at all (`Cargo.lock`, `.env`, `.gitignore`, `Dockerfile.*`, `Makefile`, `.zshrc`, `*.plist`…), and the syntax colours of One Light, which today are One Dark's (1.8–3.2:1 contrast). Moving highlighting to the service as tree-sitter tokens would make the service binary two to four times larger, put a round trip (an ssh one on a remote space) between each keystroke and its colours, and buy nothing the editor needs. **Knowing where a symbol is used is done by language servers, run by the service** (this **changes #6**): measured on this repository, TypeScript 7's own server (`tsc --lsp`, already in `node_modules`) finds the 21 real uses of `Viewer.show` in 6 files where text search finds 223 lines in 57 files and syntax alone cannot tell one `view` parameter from 57 others. The service starts a server lazily per worktree and language, only for a project the user trusts (a server runs the project's build code), finds it on the login `PATH` or in the project and never installs one, caps how many run and how much memory each uses (an 8,800-file TypeScript repository took 2.6 GiB), stops idle ones, and treats everything a server says as untrusted, size-limited input. Without a server, the editor falls back to **text matches, labelled as such**. tree-sitter is not added. For 1.0: occurrences in the file with their count, find all references, go to definition, hover, diagnostics in the gutter and a file outline (about 47 points); rename and completion are left for later.

## 2. What was measured, and how

- **Highlighting:** the editor's setup as it is (`src/viewer/editor.ts`: `@codemirror/language-data` 6.5.2 picked by file name, `oneDark`'s highlight style), run in Bun (JavaScriptCore, the engine of the macOS WebView) on real files: this repository (all 135 `.rs`, 177 `.ts`, 106 `.tsx`, 36 `.md` files and the others) and files of the human's other repositories under `~/Development` (read only; named here only by language). The measure: the share of non-blank characters that get a colour, parse time, and parse errors (Lezer's error nodes).
- **What the human's repositories hold** (51,000 files under `~/Development`, generated folders left out): TypeScript (26,000 `.ts`, 6,300 `.tsx`), JSON (4,300), SQL (2,750), Rust (2,000), Markdown (1,900), `.mjs` (1,100), Java (600), shell (370), YAML (190), TOML (150), Python (150), HTML, CSV, `.env` (33), CSS, XML, Prisma, GraphQL, Dockerfiles (10). No Go: the Go sample is a typical 140-line file written for the spike.
- **tree-sitter:** a scratch Rust crate (outside the repository) with `tree-sitter` 0.27 and `tree-sitter-highlight`, and the grammars for Rust, TypeScript/TSX, JavaScript, Python, Go, Bash, Markdown, TOML, YAML, JSON, CSS, HTML, SQL and Dockerfile, run on the same files (release build).
- **Language servers:** a 150-line LSP client (Bun) timing `initialize`, references, document highlight, definition, hover, document symbols, completion, rename and diagnostics, and polling the server's resident memory (with its children). Run on: TypeScript 7's native server (`node_modules/@typescript/typescript-darwin-arm64/lib/tsc --lsp --stdio`) on this repository and on a work TypeScript repository of 8,800 files; `vtsls` (the classic `tsserver` on Node) on the latter; `basedpyright` on a 63-file Python repository. The two Node servers were the copies Zed had already installed, run with a temporary `HOME`.
- **Not measured:** `rust-analyzer` (the rustup proxy on `PATH` has no component behind it; installing it changes the human's toolchain, rule 5, and downloading the release binary was refused by this session's permission check) and `gopls` (no Go toolchain on the host). Q7 asks how to measure them before 30.2.

## 3. Highlighting today

### 3.1 Measured

| File (KiB) | Language | Parser | Coloured | Parse | Notes |
|---|---|---|---|---|---|
| `hive-protocol/src/lib.rs` (186) | Rust | Lezer | 94 % | 69 ms | |
| `hive/src/file.rs` (122) | Rust | Lezer | 88 % | 42 ms | |
| `transport/mock.ts` (95) | TypeScript | Lezer | 91 % | 36 ms | |
| `shell/RightPanel.tsx` (57) | TSX | Lezer | 90 % | 38 ms | |
| Go sample (3) | Go | Lezer | 91 % | 4 ms | |
| `guard.py` (4) | Python | Lezer | 96 % | 4 ms | |
| Java test (8) | Java | Lezer | 93 % | 4 ms | |
| SQL dump (41), migration (15) | SQL | Lezer | 94 %, 92 % | 9, 4 ms | |
| `styles.css` (104), a Tailwind CSS (17) | CSS | Lezer | 91 %, 94 % | 58, 5 ms | |
| `index.html`, a page (9) | HTML | Lezer | 91 %, 95 % | 1, 3 ms | |
| `package.json`, `biome.json` | JSON | Lezer | 99 %, 96 % | <1 ms | |
| `Cargo.toml`, `mutants.toml` | TOML | legacy mode | 98 %, 99 % | 2 ms | |
| `gates.sh` (3) | Shell | legacy mode | 82 % | 2 ms | |
| a Dockerfile (2) | Dockerfile | legacy mode | 60 % | 4 ms | instruction arguments plain |
| `ci.yml` (14), an issue form (2), `pnpm-lock.yaml` (260) | YAML | Lezer | **42 %, 31 %, 53 %** | 6, 1, 58 ms | values plain |
| `TODO.md` (387), `architecture.md` (362), `README.md` | Markdown | Lezer | 31 %, 6 %, 12 % | 56, 35 ms | prose is plain by design; **fenced code plain** |
| `tsconfig.json` | JSON | Lezer | | | **7 error nodes** on its comments |
| `Cargo.lock` (147), `bun.lock` (90), `.gitignore`, a `.env.example`, `Info.plist`, a Prisma schema (134), `deno.jsonc`, `.grit`, CSV | — | **none** | 0 % | | |

Across the whole repository (one cold parse per file, so the JIT's warm-up is included): Rust 1.1 MiB/s, TypeScript 1.2, TSX 1.0, Markdown 2.9. CodeMirror parses only up to the visible part and then in the background, in slices, and re-parses incrementally on each edit, so even the 387 KiB `TODO.md` never blocks the frame.

**Parse errors** (where Lezer's grammar is older than the language): 4 of 135 Rust files (let chains, `if let … && let …`, from the 2024 edition; the half-open pattern `n @ 1..`), 4 of 177 `.ts` (a type predicate on an arrow's parameter, `(o): o is T =>`; `override` on a constructor's parameter property), 9 of 106 `.tsx` (JSX comments `{/* … */}` in some positions). Lezer recovers, so the cost is a few wrongly coloured tokens around the spot, not a broken file. `@lezer/rust` 1.0.3 and `@lezer/javascript` 1.5.6 are current; the fix is upstream.

### 3.2 What is missing or poor, and the fix

| Gap | Seen on | Fix (frontend, small) | Measured after the fix |
|---|---|---|---|
| Fenced code in Markdown is plain in the editor (the rendered Markdown view colours it, 21.x) | `README.md`: 9 % of the file is in fences | `markdown({ codeLanguages: languages })` instead of language-data's bare `markdown()` | README 12 % → 16 % (all its fenced code coloured; prose stays plain) |
| YAML plain values and block scalars (`run: \|` scripts) uncoloured: `@lezer/yaml` tags them `content`, which `oneDark` does not colour | every workflow, compose file, lock file | a highlight style colouring `tags.content` in YAML as strings (as VS Code does) | `ci.yml` 42 % → 100 %, issue form 31 % → 100 % |
| JSON with comments (`tsconfig.json`, `deno.jsonc`, `bun.lock`, `.vscode/*.json`, `biome.json` with comments) | `tsconfig.json`: 7 error nodes | a JSONC language: `@lezer/json` has no comments, so either the JavaScript parser in expression mode or a ten-line Lezer grammar extension; `*.jsonc`, `tsconfig*.json`, `jsconfig*.json`, `bun.lock` mapped to it | not built |
| File names with no language | `Cargo.lock`, `uv.lock` (TOML); `.env`, `.env.*` (properties); `.gitignore`, `.dockerignore`, `.npmignore` (comments and negation, a tiny stream mode); `Dockerfile.*`, `*.Dockerfile`, `Containerfile`; `.zshrc`, `.bashrc`, `*.zsh` (shell); `*.plist`, `*.csproj` (XML); `Makefile`, `justfile` (no mode exists: a small stream mode) | a list of extra `LanguageDescription`s before language-data's | not built |
| Prisma, GraphQL, Svelte, Astro, MDX, Terraform, Nix | 7 Prisma, 15 GraphQL files | each needs a new package (`cm6-graphql`, community Prisma/Svelte modes…); **only with the human's yes** (rule 3) | Q2 |
| **One Light shows One Dark's syntax colours** (`editor.ts` puts `oneDark` in every state, whatever the theme) | every file in One Light | a One Light highlight style, swapped with the theme; the colours become theme keys in 28.16 | contrast on `#f0f0ee` today: plain text `#abb2bf` 1.9:1, strings 1.8:1, functions 2.1:1, numbers 2.2:1, keywords 2.6:1, names 2.8:1, comments 3.2:1 (WCAG AA asks 4.5:1) |

The table view (23.6) already opens CSV and TSV files; they need no highlighting.

## 4. Lezer in the frontend or tree-sitter in the service

### 4.1 Measured side by side

| | Lezer (today) | tree-sitter in the service |
|---|---|---|
| Parse, whole file, cold | Rust 1.1 MiB/s, TS 1.2, TSX 1.0 (Bun/JSC) | Rust 4.3 MiB/s, TS 5.0, TSX 6.6 (native) |
| Highlight pass, whole file | included in the above (styles come from the tree) | Rust 0.5 MiB/s, TS 1.4, TSX 0.8 (`tree-sitter-highlight` runs queries); 113 ms for the 186 KiB `lib.rs` |
| Parse errors on this repository | 4 Rust, 4 TS, 9 TSX files | none (let chains, type predicates, JSX comments all parse) |
| Size | 10–29 KiB gzip per grammar, loaded on demand in the app bundle | per grammar as a static library: JSON 8 KiB, TOML 30, CSS 137, YAML 198, Go 224, JavaScript 420, Python 459, Markdown 791, Rust 1,112, Bash 1,364, TypeScript+TSX 2,835, **SQL 13,087**; the 16 grammars make a 22 MiB stripped binary. Today's Linux service is 5.9–6.8 MB (`hive-linux-*`, uploaded over ssh to each host, 26.13) |
| Tokens sent | none | 0.35 KiB of tokens per KiB of source for a whole file (68 KiB for `lib.rs`, start/length/scope varints); a visible range is a few KiB |
| Typing | colours update in the same frame, incremental re-parse | each edit goes to the service, is re-parsed and re-highlighted, and the tokens come back: a local round trip, an ssh one on a remote space, with stale colours meanwhile |
| Unsaved buffer | in the editor already | must be mirrored in the service (the same sync language servers need, section 6.5) |
| Languages | ~100 in language-data, plus the gaps of 3.2 | each grammar compiled in (C, through `cc`), each with its own query files to keep up |
| Plugins (30.7) | a grammar is a JS module, or **data** (legacy-modes' `simple-mode`: regex rules in a JSON-like shape), run in the webview | a grammar is native code (a C parser) or WebAssembly through tree-sitter's `wasm` feature and `wasmtime`, a much larger runtime |

### 4.2 Weighed against #37 and speed

#37 puts domain logic in Rust and lets the frontend render. Colouring a buffer the editor holds is rendering: CodeMirror has done it since #31 and the rendered Markdown does it with the same parsers. D34.1 says "whatever analyses code runs in the service"; the reading proposed here is that **tokenising for colour is painting**, and **anything about symbols** (occurrences, references, definitions, diagnostics, outline) **is analysis** and goes to the service. tree-sitter's real gain is 17 files without local mis-colourings; its costs are a binary 2–4 times larger on every remote host, a round trip per keystroke, a buffer mirror for colours alone, and a native-code grammar story for plugins. **Recommendation: keep Lezer; fix 3.2's gaps (30.1).** Q1 asks the human to confirm this reading of D34.1.

## 5. Where a symbol is used

### 5.1 The three ways, measured

| Symbol (this repository) | Text search (`rg -w`, what `git grep -w` gives) | Syntax only (name nodes of that text in the file) | Language server (TypeScript 7) |
|---|---|---|---|
| `sendComments` (a function) | 8 hits, 3 files (one in `TODO.md`) | 4 in its file | **7 in 2 files** |
| `Viewer.show` (a method) | 223 lines in 57 files (17 calls of some `.show(`) | 2 in its file (its declaration and one other `show:`) | **21 in 6 files** |
| `view`, the parameter of `highlighting()` in `editor.ts` | 78 in the file | 58 in the file (12 declarations, 42 uses, 4 properties, all different bindings) | **3** |
| `listAccessWindows` (work repository, 8,800 files) | 3 in 2 files, 0.47 s | — | 3 in 2 files |

- **Text search** is instant and needs no setup, and is right for names that are unique in the worktree; it is wrong for common names (`show`, `path`, `open`, `new`, `run`), counts comments, strings and docs, and knows nothing of imports and renames. Hive already has it (`hive::search`, `git grep`).
- **tree-sitter or Lezer scopes** separate locals inside one file when the grammar ships a `locals` query (TypeScript and JavaScript do; `tree-sitter-rust` does not), but stop at the file: no fields, methods, imports or other files, and no types. They would get the `view` row right and the `Viewer.show` row wrong. They cost the grammars of section 4 in the service, or analysis in the frontend.
- **Language servers** are exact (types, imports, re-exports, methods), across the worktree, at a price in memory, start-up and setup (section 6.6).

### 5.2 Recommendation

Language servers for the answer; text as the labelled fallback; no scopes layer.

- **In the file:** with a server, the occurrences under the cursor come from `textDocument/documentHighlight` (0–3 ms once warm), painted with their count ("3 occurrences" beside the find bar's place, 23.10) and next / previous by keyboard. Without one (no server for the language, not trusted, starting, crashed), the editor shows **whole-word text matches**, the count labelled "text matches". Whole-word matching in the open buffer is the same operation as 23.10's find in a file, which runs in CodeMirror today; Q3 asks whether that stays in the frontend.
- **Across the worktree:** "Find all references" asks the service; with a server, `textDocument/references`; without one, `git grep -w -F` (a whole-word, case-sensitive variant of `hive::search`), the panel's title saying "Text matches for `show`", grouped by file with a count each, click to open at the line.

## 6. Language servers (changes #6)

### 6.1 Which ones

| Language | Server | Where Hive looks | Measured |
|---|---|---|---|
| TypeScript, JavaScript | TypeScript 7's own (`tsc --lsp --stdio`, the native binary of `@typescript/typescript-<os>-<cpu>`) | the project's `node_modules` (TypeScript ≥ 7) | this repository: references 0.3 s cold then 3–8 ms, 330–410 MiB. 8,800 files: references 2.1 s cold then 61 ms, **2.6 GiB peak, 2.0 GiB idle** |
| | `typescript-language-server` or `vtsls` (Node + the project's `tsserver`) | `PATH`, for TypeScript < 7 | 8,800 files with `vtsls`: references 3.7 s cold, 460 MiB; it found 1 reference where TypeScript 7 found 3 (it loads projects lazily) |
| Rust | `rust-analyzer` | `PATH`, checked by `--version` (a rustup proxy without the component fails, as on this host) | not measured (Q7) |
| Python | `pyright-langserver` or `basedpyright-langserver` | `PATH`, or the project's `.venv/bin` | 63 files: references 3.2 s cold, 361 MiB, 29 diagnostics pushed on one file at 3.7 s |
| Go | `gopls` | `PATH` | not measured (no Go here; the human has no Go repositories) |
| Others | any server, from the settings file | `code.servers` in `settings.json` (#47): languages, file globs, the program and its arguments as a list | |

Everything with **separate arguments**, never a shell string; the program is resolved on the service's login `PATH` (`hive::login_path`) or in the project, never downloaded or installed by Hive. Every request was answered in well under 100 ms once a server was warm; the cost is the first query and memory.

### 6.2 Trust: a server runs the project's code

`rust-analyzer` runs build scripts and procedural macros and `cargo check`; `tsserver` loads plugins named in `tsconfig.json`; `gopls` runs `go list`, which can fetch modules; pyright asks the project's Python for its paths. Starting a server on a repository is close to building it. Hive's agents already run code there, but Hive itself never has. So: **servers start only for a project the user has allowed** ("Code intelligence runs this project's build tools: Allow / Not now", asked once per project the first time a file of a language with a server is opened; kept in the settings file per project, revocable in Settings). Q4.

### 6.3 Lifecycle and limits

- **Start:** lazily, when the editor opens the first file of that language in that worktree (`code_open`), never at service start. One server per (worktree, language), its root the worktree.
- **Stop:** when no file of its language is open in that worktree for 10 minutes (setting), when the worktree is removed, when the project stops being followed or trusted, and with the service. `shutdown` + `exit`, then SIGTERM after 2 s, then SIGKILL, the whole process group (servers spawn children: `tsserver`, `cargo check`).
- **How many:** at most 4 running at once (setting); a fifth stops the least recently used.
- **Memory:** the service polls each server's process group's resident memory (`hive::procs`: `libproc` on macOS, `/proc` on Linux) every 10 s; above 3 GiB (setting) it is stopped and the user told why. No `RLIMIT_AS` (Node and Go reserve far more address space than they use, so it breaks them). **CPU:** started at a lower priority (`setpriority`, nice 10) so the terminals stay responsive.
- **Crashes:** restarted once at once, then after 10 s and 60 s; a fourth crash in 10 minutes leaves it stopped with "Restart" in the file's header.
- **Output untrusted:** `Content-Length` capped at 16 MiB per message (a larger one drops the server as broken), results capped (references 2,000, document highlights 1,000, symbols 5,000, diagnostics 500 per file, hover 32 KiB of text, clipped lines 300 characters as `hive::search` does), paths checked to be inside the worktree (a definition outside it, e.g. the standard library, is shown as read-only text: Q5), its stderr kept in a 64 KiB ring for the "Show log" link and never written to `daemon.log` at the default level.
- **Files:** only text files under the viewer's 1 MiB limit (`file::TEXT_LIMIT`) are sent to a server.

### 6.4 Remote spaces (Stage 26)

The service on the host runs the servers, found on the host's `PATH` and in the project there, with the same rules. Nothing about language servers crosses the ssh link but the app's requests and their capped answers; 26.19's per-link caps gain the new messages (requests debounced to at most 10 per second, list sizes as in 6.3). A query costs the link's round trip on top of the server's few milliseconds. Unlike tree-sitter, nothing is added to the uploaded binary.

### 6.5 Unsaved edits

A server must see the buffer, not the file on disk. When the editor opens a file, the service reads it from disk (it already has the text and its version token, `open_file`) and sends `didOpen`; each edit in the editor sends the changes, not the text (`code_edit`, CodeMirror's change set as `{from, to, text}` in UTF-16 offsets, which are also LSP's default positions, so no conversion); closing sends `didClose`. A save or a reload from disk (the agent changed the file, #31) re-opens it with the new text.

### 6.6 What the user sees

The file header (where the "agent working here" badge is) gets a small status: nothing when the server is ready; "Rust: indexing…" with the server's own progress; "No Rust server: `rust-analyzer` not found on PATH" with a link to the docs; "Rust server stopped: used 3.1 GiB" or "crashed" with "Restart" and "Show log"; "Not allowed for this project" with "Allow". In each case the features fall back to text matches, labelled. Settings gets a **Code intelligence** section: the servers found per language (path, version), on/off per language, the trusted projects, the limits.

## 7. What else for 1.0, with costs

The points assume the server manager of 6 (13 points) is built first.

| Feature | What | Cost | Recommended |
|---|---|---|---|
| Occurrences in the file | painted, with the count, next / previous by keyboard; text matches without a server | 5 | **yes** |
| Find all references | a panel grouped by file with counts, click to open at the line; text matches without a server | 8 | **yes** |
| Go to definition | ⌘-click and a keymap command (28.7); opens the file and line, or the read-only text of one outside the worktree (Q5) | 5 | **yes** |
| Hover | the server's type and docs (Markdown, rendered as 21.x does, clipped) after a pause over a name | 3 | **yes** |
| Diagnostics in the gutter | the server's errors and warnings as gutter marks and wavy underlines, a count in the file header; pull (`textDocument/diagnostic`, TypeScript 7) or push (pyright). Noisy on some servers (29 on one Python file), so off per language in Settings | 8 | **yes** |
| File outline | the file's symbols (`documentSymbol`: 15–51 per file measured) as a list to jump to, from the palette (6.3) or a right-panel view | 5 | **yes** |
| Rename | a workspace edit over many files, some not open, saved by the service with version checks while an agent may be editing them; needs a preview of every change | 13 | later |
| Completion | a popup while typing, with snippets and imports; the server's slowest query (1.2 s cold on pyright); the human mostly reads and reviews what agents wrote | 13 | later |

The recommended set: 13 + 5 + 8 + 5 + 3 + 8 + 5 = **47 points**, inside Stage 30's 63 reserved for 30.2–30.4 (8 + 21 + 34). Rename and completion would add 26.

## 8. Protocol

New `Control` messages (channel 0), in the catalog's style. Positions are `{line, character}`, 0-based, `character` in UTF-16 code units (CodeMirror's and LSP's default); `seq` is the editor's change counter for the file, so an answer for an older text is dropped.

| Message | Direction | Meaning |
|---|---|---|
| `code_open {worktree, path, version}` | app → service | The editor shows `path` (of a followed worktree, the rules of `open_file`) at disk `version`. Starts the language's server if trusted and needed; answered by `code_status` |
| `code_edit {worktree, path, seq, changes}` | app → service | The buffer's changes since `seq - 1`, each `{from, to, text}` (offsets in the old text, UTF-16); at most 1,000 changes and 1 MiB of text per message |
| `code_close {worktree, path}` | app → service | The editor no longer shows it |
| `code_status {worktree, language, state, message, progress}` | service → app | A server's state for that worktree: `starting`, `indexing`, `ready`, `missing`, `not_trusted`, `stopped` (with why), `crashed`; `progress` from its `$/progress` |
| `find_occurrences {id, worktree, path, seq, at}` | app → service | The symbol under `at`; answered by `occurrences {id, ranges, source}`, `source` `server` or `text` |
| `find_references {id, worktree, path, seq, at}` | app → service | Answered by `references {id, symbol, files: [{path, matches: [{line, character, length, text}]}], source, truncated, error}` |
| `find_definition {id, worktree, path, seq, at}` | app → service | Answered by `definition {id, locations: [{path, line, character, external}]}`; `external` true for a file outside the worktree (Q5) |
| `hover {id, worktree, path, seq, at}` | app → service | Answered by `hover_info {id, markdown, range}` (clipped to 32 KiB) |
| `outline {id, worktree, path, seq}` | app → service | Answered by `symbols {id, items: [{name, kind, line, character, depth}]}` |
| `diagnostics {worktree, path, seq, items}` | service → app | Pushed when a server's diagnostics for an open file change; each `{from, to, severity, message, code}`, at most 500 |
| `cancel_query {id}` | app → service | The cursor moved on; the service sends `$/cancelRequest` |
| `restart_code_server {worktree, language}` | app → service | The header's "Restart" |
| `trust_project {project, allowed}` | app → service | The trust question's answer (or kept in `settings` with the other per-project settings: Q4) |

The browser mock answers them with fixed data, as for every message; `tests/app-messages.json` gains the samples.

## 9. Stage 30, refined (after approval)

- **30.1 Highlighting fixes** · 5 pts — 3.2's table: Markdown fenced code, YAML values, JSONC, the extra file names and stream modes, a One Light highlight style (its colours become theme keys in 30.5). New packages (Prisma, GraphQL…) only if Q2 says so. Tests: bun, the coverage per fixture file (the measure of section 3) and the language per file name; e2e screenshots in both themes.
- **30.2 Language servers in the service, and occurrences** · 13 + 5 pts — `hive::code`: discovery, trust, spawning, an LSP codec with 6.3's limits, the lifecycle and the memory watch, the buffer sync, `code_status`; the file header's status and Settings' Code intelligence section; occurrences in the file with their count and keys. Tests: Rust with a fake language server (a small binary like `fake_claude`, answering scripted messages, crashing or flooding on demand), every limit and the lifecycle with a fake clock; bun; e2e on the mock; integration with TypeScript 7's server on a temp project when it is installed.
- **30.3 Find all references and go to definition** · 8 + 5 pts — the references panel, the text fallback (`git grep -w`), ⌘-click and the keymap command, external definitions as Q5 decides. Tests: Rust, bun, e2e on the mock.
- **30.4 Hover, diagnostics and outline** · 3 + 8 + 5 pts — one task each (30.4a–c) or one, as the orchestrator splits them.

Rename and completion, if the human wants them, become 31.x tasks after 1.0 or join 30.4 with 26 more points.

## 10. Questions for the human

- **Q1 (D34.1, #37):** Is colouring (Lezer, in the frontend) painting, so that only symbol analysis must be in the service? Recommended: yes; the alternative, tree-sitter in the service, is costed in section 4.
- **Q2 (rule 3):** New grammar packages for Prisma and GraphQL (and others you use)? Recommended: Prisma and GraphQL only, after naming each package.
- **Q3 (#37):** Without a server, whole-word matches in the open file computed by CodeMirror (as 23.10's find), or by the service? Recommended: CodeMirror, for the same reason as Q1.
- **Q4 (trust):** Ask once per project before starting its servers, kept per project in the settings file? Recommended: yes.
- **Q5:** A definition outside the worktree (the standard library, a crate in `~/.cargo`, Python's site-packages): open it read-only in a tab (a new exception to `open_file`'s rule that paths stay in the worktree), or only say where it is? Recommended: read-only tab, the path checked to be a file the server named and under the 1 MiB limit.
- **Q6:** The default limits: 4 servers, 3 GiB each, 10 minutes idle. TypeScript 7 alone took 2.6 GiB on an 8,800-file repository; with 24 GB and several worktrees of that repository open, a lower cap (2 servers of a language) may be wiser.
- **Q7:** How to measure `rust-analyzer` before 30.2: `rustup component add rust-analyzer` (changes your toolchain), or a release binary in a scratch folder (this session could not fetch it)? `gopls` can stay unmeasured unless you use Go.
- **Q8 (#6):** `docs/hive.md` #6 says "sem LSP na v1"; if you approve, you update it (and D34.1's note becomes a decision).

## 11. Reproducing

The scripts were throwaway and are not kept; each fits on a screen:

- **Coverage:** for each file, `LanguageDescription.matchFilename(languages, name)`, `load()`, `language.parser.parse(text)`, then `highlightTree(tree, oneDarkHighlightStyle, …)` summing the non-blank characters inside styled ranges; error nodes by `tree.iterate` and `type.isError`. Run with `bun` from a checkout with `node_modules`.
- **tree-sitter:** `cargo new` in a scratch folder, `cargo add tree-sitter tree-sitter-highlight` and the grammar crates (`tree-sitter-rust` 0.24, `-typescript` 0.23, `-javascript` 0.25, `-python` 0.25, `-go` 0.25, `-bash` 0.25, `-md` 0.5, `-toml-ng` 0.7, `-yaml` 0.7, `-json` 0.24, `-css` 0.25, `-html` 0.23, `-sequel` 0.3, `-containerfile` 0.9), each grammar's `HIGHLIGHTS_QUERY` (TypeScript's joined with JavaScript's), `Highlighter::highlight`; grammar sizes from `target/release/build/*/out/*.a`.
- **Language servers:** a JSON-RPC client over the server's stdio (`Content-Length` frames), `initialize` with the worktree as root, `didOpen` of one file, then the requests of section 6.1, timed with `performance.now()`; memory by `ps -A -o pid=,ppid=,rss=` every 100 ms over the server's process tree. Node servers run with a temporary `HOME` and `XDG_CACHE_HOME`.
