# Spike 28.16: The theme file

Design task 28.16 (`TODO.md`, D34.2: themes are configuration, not plugins; D34.4: no code before the human approves). No production code was written. Written 2026-11-27 against `main` at `89ec3828`. Read: #23 (One Dark, One Light optional), #37 (logic in Rust), #47 (the settings file), D22.16 (Hive's folder is `~/.hive`), 18.33 (the launch colour), D18.10 b (`system`), Stage 26 (remote hosts), `docs/ui-reference.md`, `docs/architecture.md` → Settings, and the code listed in section 2.

## 1. Recommendation in one paragraph

A theme is a JSON file in **`~/.hive/themes/<id>.json`** (Hive's folder since D22.16, beside `settings.json`), read by the **service** as `settings.json` is: size-limited, typed, never trusted. It holds a `name`, an `appearance` (`dark` or `light`), an optional **`extends`** (a built-in or another theme) and sections of keys that each map to **one CSS variable** or one xterm colour: `colors`, `states`, `git`, `diff`, `syntax`, `terminal`, `fonts`, `text`, `space`, `sizes`, `radius`, `borders`, plus a `colorblind` section for 28.6's diff colours. A theme sets only what it changes; the rest comes from its parent. **One Dark and One Light are two such files** compiled into the service (appendix A and B), so the built-ins are just themes and the CSS loses its One Light block. The service resolves the chain, checks every value (a colour is `#rgb[a]`/`#rrggbb[aa]`, a size a number in a range, a font a plain family list), runs the **WCAG contrast check** (28.6's, in Rust) and sends the app a ready map of CSS variables plus the terminal palette; the app only applies it (`style.setProperty`, `terminal.options.theme`). The service **watches the folder** and sends a new map when a file changes, so editing a theme in any editor shows at once. A theme that cannot be used (bad JSON, too large, a cycle, a missing parent) **falls back** to the built-in of its appearance, with the reason shown in Settings → Appearance and a toast; a single bad value falls back to its parent's value only. Settings picks a theme by id in `appearance.theme`; with `system`, two new keys `appearance.dark_theme` / `appearance.light_theme` say which theme each appearance shows. Building it is mostly a tokenisation of the CSS (about 20 hard-coded colours, the raw spacing values, the editor's own `oneDark` colours) before any theme code: section 12.

## 2. Today: what a theme would have to set

### 2.1 Where the look lives

| Where | What | Notes |
|---|---|---|
| `src/styles.css` `:root` (l. 33–100) | 52 CSS variables: 21 surface/text colours, 7 states, 4 git, 8 syntax (`--syn-*`, the Markdown preview only), 3 font families, 6 font sizes, 4 radii, 2 bar heights, `color-scheme` | One Dark, the default (#23) |
| `src/styles.css` `:root[data-theme="one-light"]` (l. 105–149) | 41 of them overridden | Fonts, sizes, radii and `--close-hover` are not |
| `src/styles.css` l. 237 | `-webkit-font-smoothing: auto` in One Light (17.15) | Follows the appearance |
| `src/styles.css`, rest (5 500 lines, 787 `var(--…)`) | ~20 hard-coded colours (2.2), every padding/margin/gap as raw px (2.5), 81 `1px solid` borders, 14 raw font sizes (11.5 px ×4, 10, 12, 14, 16, 20) and 13 raw line heights | Not themeable until tokenised |
| `src/viewer/editor.ts` | `oneDark` (CodeMirror's theme **and** highlight style) in both themes, then Hive's overrides: `fontSize: 12px`, `lineHeight: 18px`, the diff's line and text backgrounds as `rgba(…)`, the hunk header's `rgba(116,173,232,0.08)`, `{ dark: true }` always | See 2.3 |
| `src/viewer/gutterMarkers.ts` | `--git-added/-modified/-deleted` | Already tokens |
| `src/shell/Markdown.tsx` + CSS l. 5139–5170 | `tok-*` classes (the editor's One Dark groups) coloured by `--syn-*`; comments italic | Already per theme |
| `src/terminals.ts` | `ONE_DARK` / `ONE_LIGHT` xterm `ITheme` (22 colours each), `MATCHES` (5 search colours per theme, opaque `#rrggbb` because xterm wants them), `lineHeight: 1.2` | Font family and size are the settings' (`terminal.font_family`, `font_size`, #47) |
| `src/window.ts` | `THEME_BACKGROUND` (each theme's `--bg`, the native window's background) and `nativeTheme` (the window's light/dark) | |
| `src/persist.ts`, `src/main.tsx` | `hive.theme` in `localStorage`, applied before the first render (18.33) | |
| `src-tauri/tauri.conf.json` | `backgroundColor: #282c33` at launch | Before any JS runs |
| `src-tauri/src/tray.rs` | The menu bar icon's state tints (One Light's state colours, fixed) | The menu bar is the system's; out of the theme (Q11) |
| `crates/hive-protocol` `Theme` | `one-dark` \| `one-light` \| `system` (`appearance.theme`) | An enum today |

### 2.2 Hard-coded colours outside the tokens

| Where | Value | Proposed key |
|---|---|---|
| `.window-controls button.close:hover` text | `#fff` | `colors.on-close-hover` |
| `.conflict-banner` background / border | `rgba(222,193,132,0.10)` / `0.28` | `colors.warning-surface` / `colors.warning-border` |
| `.result-line mark` | `rgba(222,193,132,0.3)` | `colors.match` |
| `.primary kbd` border | `rgba(31,35,41,0.35)` | `colors.on-accent-border` |
| `.primary:disabled` | `#3e4a5c` (both themes) | `colors.accent-disabled` |
| `.primary.danger:hover` | `#db8a8e` (both themes) | `colors.danger-hover` |
| Dialog backdrops (×2) | `rgba(18,20,24,0.55)` | `colors.backdrop` |
| Dialog and popover shadows (×6) | `rgba(0,0,0,0.45)` | `colors.shadow` |
| Picker shadow | `rgba(0,0,0,0.4)` | `colors.shadow-picker` |
| Toast shadow | `rgba(0,0,0,0.3)` | `colors.shadow-toast` |
| Editor diff (editor.ts) | `rgba(161,193,129,0.12/0.25)`, `rgba(208,114,119,0.12/0.25)` | `diff.added-line/-text`, `diff.removed-line/-text` |
| Editor hunk header (editor.ts) | `rgba(116,173,232,0.08)`, text `--accent` | `diff.hunk`, `diff.hunk-text` |
| Merge view gutter (editor.ts) | `--state-idle`, `--state-error` | `diff.added-gutter`, `diff.removed-gutter` |
| `.page-frame` (the dev-server preview) | `#fff` | **Not themed**: a web page assumes white |

The six `color-mix(…)` uses already derive from tokens and follow any theme.

### 2.3 A finding: the editor ignores One Light's syntax colours

By reading the code (to confirm with a screenshot in 30.5a): the editor and the diff view load `oneDark` in both themes, so in One Light the editor's text is One Dark's `#abb2bf` and its syntax colours are One Dark's, on `#f0f0ee`. Computed contrast there: strings 1.77:1, types 1.51:1, functions 2.07:1. The Markdown preview is right (its `tok-*` classes use `--syn-*`). The theme build fixes this for free by giving the editor the same class highlighter as the Markdown preview (`tagHighlighter`, `tok-*`), coloured by the theme (Q6). Smaller differences that move with it: the editor colours definitions and separators `#abb2bf` (the Markdown preview: the text colour) and comments `#7d8799` (the preview: Zed's `#7f848e`, italic).

### 2.4 Fonts

UI: `--font-sans` IBM Plex Sans, `--font-mono` IBM Plex Mono (both `@fontsource`, weights 400/500/600). Editor: `--font-code` Hive Mono + Symbols Nerd Font, 12 px / 18 px (in editor.ts, not a token). Terminal: the settings' `terminal.font_family` and `font_size` (#47), line height 1.2 (fixed), ⌘= / ⌘- zoom on top. Text sizes: the six `--fs-*` tokens, body line height 18 px, plus the raw ones of 2.1.

### 2.5 Spacing, sizes, radius, borders

- **Spacing has no scale today.** Every padding, margin and gap is a raw px value; the most used are 8 (106), 6 (91), 12 (60), 4 (58), 10 (30), 2 (27), 16 (12), 24 (26, mostly heights). A theme cannot change spacing until they become tokens (12, step a).
- **Sizes:** `--bar-h` 34, `--status-h` 24 are tokens; row heights (project 26, worktree 24, file 22, picker 30, branch 24, subagent 18), buttons (28, ghost 24), fields (30) are raw.
- **Radius:** four tokens (3, 4, 6, 8), used consistently.
- **Borders:** always 1 px (81 `1px solid`), focus rings `0 0 0 1px`; no token.

## 3. Where themes live

- **Folder:** `~/.hive/themes/` (`$HIVE_HOME/themes/`), beside `settings.json`. The TODO's `$XDG_CONFIG_HOME/hive/themes/` predates D22.16, which moved every file of Hive's into `~/.hive` (Q1). The service makes the folder (0700) at start so the watcher has something to watch and "Open themes folder" always opens.
- **Id:** the file name without `.json`, lower case: `^[a-z0-9][a-z0-9._-]{0,63}$`. Other names in the folder are listed as problems ("Solarized.json: a theme's file name is lower case letters, digits, '.', '_' and '-'"). `one-dark` and `one-light` are reserved: a file with a built-in's id is refused with "one-dark is a built-in theme: name yours otherwise and set \"extends\": \"one-dark\"".
- **Built-ins:** `crates/hive/themes/one-dark.json` and `one-light.json`, compiled into the service (`include_str!`), so they cannot be missing or broken at run time (a test parses them with zero problems).
- **Local only.** Themes, like the settings, are this Mac's: a remote host's `themes` and `theme` are dropped by `REMOTE_TO_UI` with `settings` (architecture → Remote hosts, item 1), and nothing in a remote's `~/.hive/themes` is read.

## 4. Format

### 4.1 Shape

```json
{
  "$schema": "ignored: any string, for editors",
  "name": "Dusk",
  "appearance": "dark",
  "extends": "one-dark",
  "colors": { "bg": "#1e2127", "panel": "#252931" },
  "syntax": { "comment": { "color": "#6b7280", "italic": true } },
  "radius": { "md": 8, "lg": 12 },
  "fonts": { "ui": { "family": "\"Inter\", system-ui, sans-serif" } }
}
```

- **JSON**, as `settings.json` (serde, no new dependency). Comments and trailing commas are refused, which hurts a hand-edited file; JSONC would need a crate (Q2).
- **Sections are fixed** (section 9); each key in a section is one CSS variable (`colors.bg` → `--bg`, `space.3` → `--space-3`) or one xterm field. Names follow today's tokens so `docs/ui-reference.md` still reads true.
- **`extends`:** the id of a built-in or another file. Resolution: the parent resolved first, then this file's keys on top, key by key (a section is merged, never replaced). At most 8 levels; a cycle or a missing parent makes the theme unusable (5.2). No `extends`: the built-in of its `appearance` is the parent, so **every theme is complete**, and a file with only `{"appearance": "dark", "colors": {"accent": "#e5c07b"}}` is valid.
- **`appearance`** is required when there is no `extends` and inherited otherwise. It drives `color-scheme`, the window's native look (`nativeTheme`), font smoothing (17.15), CodeMirror's `dark` flag, and which built-in a broken theme falls back to.
- **Linked keys.** Three terminal keys follow a colour unless some theme in the chain sets them: `terminal.background` and `terminal.cursor-accent` follow `colors.bg`, `terminal.foreground` follows `colors.text`. The built-ins leave them unset, so a theme that changes `colors.bg` gets a matching terminal. The native window's background is always `colors.bg`.
- **No references, no maths.** A value is a literal (`"#74ade8"`, `13`), never `"$colors.accent"` or `darken(…)`; the CSS's own `color-mix` uses keep deriving what they derive. Simpler to check, and to explain.
- **Value types:**
  - **colour:** `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, case-insensitive, normalised to lower-case 6 or 8 digits. Named colours, `rgb()`, `hsl()` and `color-mix()` are refused (the message names the form expected). The terminal's search keys must be opaque (xterm), said so when not.
  - **px:** a number (integer or one decimal), each key with its range (section 9), sent as `"<n>px"`.
  - **font family:** a string of at most 256 bytes, a comma-separated list of names, each either a bare name of letters, digits, spaces and `-` or a quoted one (`"…"` or `'…'`) without quotes, backslashes or control characters inside. No `url()`, `;`, `{`, `}`: nothing that could leave the declaration.
  - **syntax style:** a colour, or `{ "color": <colour>, "italic": bool, "bold": bool }`.
  - **number:** the terminal's line height, 1.0–2.0.

### 4.2 Fonts in a theme

A theme names families only: the bundled ones (IBM Plex Sans and Mono, Hive Mono, Symbols Nerd Font) or fonts installed on the Mac. A theme cannot ship font files (no `url()`, no network, the CSP stays). A family that is not installed falls through the list as CSS does; the service cannot know the Mac's fonts, so no warning (Q12).

The terminal's font is a setting today (#47, edited in Settings → Terminal, zoomed by ⌘= / ⌘-). Two ways to give themes a say (Q4):

- **A (recommended):** the theme's `fonts.terminal` is the default; the settings' `terminal.font_family` / `font_size` become optional overrides (`null` = the theme's). A file whose value equals today's default reads as `null`, so nothing changes on upgrade. Settings → Terminal shows the theme's value as the placeholder and a "Use the theme's" reset.
- **B:** the terminal's font stays only in the settings; themes set the UI and editor fonts. Smaller, but the TODO lists the terminal font as a theme key.

## 5. Checks

All in the service (`hive::theme`, #37). The app never parses a theme.

### 5.1 Reading

- Regular files only (`symlink_metadata`; a link is a problem, never followed), at most **64 KiB** each (`git::read_limited`), at most **64 themes** listed (sorted by id; the rest: "Only 64 themes are read: <ids> ignored").
- A JSON object at the top. Unknown sections and keys are **ignored, and listed as hints** ("colors.bgg is not a theme key"), since a typo otherwise silently does nothing (Q9). #47 ignores unknown keys without a word; a hint does not refuse anything.
- `name`: trimmed, 1–64 characters, no control characters; missing: the id.

### 5.2 Two levels of failure

| Failure | Effect | Shown |
|---|---|---|
| Unreadable, too large, not JSON (serde's line and column), not an object, a link, a reserved or bad id, `appearance` missing without `extends` or not `dark`/`light`, `extends` naming a missing theme, a cycle, more than 8 levels | **The theme is unusable.** The app shows the built-in of its `appearance` (One Dark when unknown) | "Dusk can't be used: dusk.json line 4, column 12: expected `,` or `}`. Showing One Dark." |
| One bad value (`colors.accent: "blu"`, `radius.md: 99`) | **That key only** keeps its parent's value; the rest of the theme applies | "dusk.json: colors.accent must be a colour like #rrggbb (got \"blu\")" |

Per-key fallback is friendlier than #47's whole-file rule while a user edits a theme live: one typo does not throw the whole look away (Q3). Never a crash, never a blank window: the built-ins are compiled in and checked by `cargo test`.

### 5.3 Contrast (WCAG AA)

Warnings only; a theme is never refused for contrast. The check is **28.6's**, built in parallel for the diff colours: the relative luminance and contrast ratio of WCAG 2.x, with a translucent colour composited over its background first. It must run in the service, so it lives in Rust (`hive::contrast`); if 28.6 lands it in TypeScript (a bun test over the CSS), 30.5b ports the same function and test vectors to Rust and the built-ins' checks move to `cargo test`. The pairs checked:

| Level | Pairs | Threshold |
|---|---|---|
| Text | `text`, `text-2` on `bg`, `panel`, `bar`, `hover`, `selected`; `on-accent` on `accent`; `terminal.foreground` on `terminal.background`; `text` on `diff.added-line` and `diff.removed-line` (over `bg`); `syntax.*` on `bg` | 4.5:1 (1.4.3) |
| Secondary and icons | `text-3` on `bg`, `panel`; `states.*` and `git.*` on `panel`; `diff.hunk-text` on `diff.hunk`; `focus` on `bg` | 3:1 (1.4.11, non-text; and large text) |
| Not checked | `text-4` (separators, line numbers: decorative), borders and lines (subtle by design), the terminal's 16 ANSI colours (programs pick them; `black` on a dark background is meant to vanish) | — |

**The built-ins fail some of these today** (computed for this spike):

| Theme | Pair | Ratio | Needs |
|---|---|---|---|
| One Dark | `text-2` on `selected` | 4.03 | 4.5 |
| One Dark | `syntax.name` on `bg` | 4.39 | 4.5 |
| One Dark | `syntax.comment` on `bg` (Zed's `#7f848e`) | 3.73 | 4.5 |
| One Dark | `focus` on `bg` | 2.47 | 3 |
| One Light | `on-accent` on `accent` (the primary button) | 3.74 | 4.5 |
| One Light | `syntax.string` 2.81, `syntax.type` 2.80, `syntax.comment` 2.26, `syntax.name` 3.21, `syntax.function` 3.55, `syntax.operator` 3.66, `syntax.constant` 4.26 on `bg` | — | 4.5 |
| One Light | `states.permission` 2.75, `states.idle` 2.52, `states.error` 2.9995 on `panel`, and the same three as `git.modified`, `git.added`, `git.deleted` | — | 3 |
| One Light | `focus` on `bg` | 2.95 | 3 |

That is 4 pairs in One Dark and 15 in One Light. Passing: everything else of the table, e.g. `text` ≥ 6.7 everywhere, `text-3` on `bg`/`panel` (3.6–4.1, at the 3:1 level), the diff backgrounds under `text` (≥ 8.2), the hunk header (5.17 and 3.34). (One Dark's `text-3` on `bar` is 2.99 and One Light's `text-3` on `selected` 2.99: only `bg` and `panel` are checked for `text-3`.) Atom One Light's syntax colours were made for `#fafafa`; Hive's softened `#f0f0ee` lowers them further. So the built-ins either get a short list of accepted pairs or new values (Q10). Warnings show in Settings → Appearance under the theme ("3 contrast warnings"), each with its pair, ratio and threshold; none for an accepted pair of a built-in.

## 6. Live reload

- The service watches `~/.hive/themes/` with `notify` (already a dependency: `files.rs`, `registry.rs`, `session_watch.rs`), not recursive. Editors save by rename, so the folder is watched, not the file.
- Events are gathered for 150 ms, then the changed files are read again. The themes in use (the one shown, and with `system` both) and every theme that extends a changed file are resolved again; the app gets a new `theme` only when the result differs, and `themes` (the list) when a file came, went or changed its name or problems.
- A file deleted while in use: the built-in of its appearance, with "dusk.json was removed. Showing One Dark." The setting is kept, so putting the file back brings it back.
- The settings file is not watched (unchanged, #47): a theme is chosen with `set_settings`.

## 7. Protocol

| Message | Direction | Content |
|---|---|---|
| `list_themes` | app → service | Answered by `themes`. |
| `themes {themes[{id, name, appearance, builtin, problems}]}` | service → app | Built-ins first, then files by id; `problems` the count of errors (5.2), so the select can mark a broken one. Sent after `settings` on connect, when asked, and unasked when the folder changes. |
| `theme {dark, light, shown}` | service → app | `shown`: the theme the setting gives (`dark`/`light` when it is `system`). Each of `dark` and `light`: `{id, name, appearance, vars{"--bg": "#282c33", …}, terminal{background, foreground, …, brightWhite, searchMatch…}, fallback, problems[], hints[], warnings[]}`. `vars` names come from the service's fixed table, so the app sets them as they are; `terminal` is xterm's `ITheme` plus the 5 search keys and `lineHeight`. With a named theme `dark` and `light` are the same object once (the app takes `shown`); with `system` both are resolved so the app switches without asking (`prefers-color-scheme`, D18.10 b). Sent after `settings`, after a saved `set_settings` that changes a theme key, and on reload. |
| `open_theme {id}` | app → service | Open a user theme's file in the editor, as `open_settings_file`. Answered by `editor_target`. A built-in: refused ("one-dark is a built-in theme: make a new theme that extends it"). |
| `open_themes_folder` | app → service | Reveal the folder (Finder). |
| `create_theme {id, name, extends}` | app → service | "New theme…": writes `{"name", "extends"}` and an empty `colors` to `<id>.json` (0600, atomic, refused if it exists), then answers `themes` and `editor_target`. The only file the app ever writes there (Q8). |

`settings` changes: `appearance.theme` becomes a string (a theme id or `system`; `one-dark`, `one-light` and `system` keep meaning what they mean, so files and the protocol stay compatible), plus `appearance.dark_theme` (default `one-dark`) and `appearance.light_theme` (default `one-light`), used with `system`. `set_settings` refuses an id with no theme ("No theme dusk in ~/.hive/themes"). At start, a setting naming a theme whose file is gone shows the fallback with its problem; the setting is not rewritten. These keys extend #47's list; the human updates `docs/hive.md` (#47) if approved.

## 8. The app's side

- **Applying:** one `<style id="theme">` element whose text is `:root{--bg:#282c33;…}`, replaced on each `theme` (one style recalculation, no per-variable `setProperty` loop); `data-appearance="dark|light"` on the root replaces `data-theme` for the three appearance rules (`color-scheme`, font smoothing, anything native). Every open terminal gets `options.theme` and `lineHeight` (as a theme switch does today); CodeMirror needs nothing (the `tok-*` classes and diff colours are CSS variables) except its `dark` flag in a compartment. `THEME_BACKGROUND` and the `ONE_*` / `MATCHES` tables go; `nativeTheme` takes `appearance` and `vars["--bg"]`.
- **Before the service answers (18.33):** `:root` in `styles.css` keeps One Dark's values (the built-in), so a first frame always has a full theme. `localStorage` keeps, besides `hive.theme`, the last `theme` message's `shown` entry (`hive.theme.vars`, at most 32 KiB); `main.tsx` applies it before the first render when every name matches `^--[a-z0-9-]+$` and every value `^[#a-z0-9 .,"'-]+$`, else ignores it. So a custom light theme launches light, not One Dark then a flash. `tauri.conf.json`'s launch colour stays One Dark's.
- **Drift guard:** a test compares `styles.css`'s `:root` block with One Dark's resolved `vars`, so the fallback cannot drift from the built-in.
- **Settings → Appearance:**
  - **Theme** select (28.18's labelled groups): "System", then "Built-in" (One Dark, One Light), then "Your themes" (each with its appearance, a warning icon on one with problems; still selectable, it then shows its fallback and the problems).
  - With System: **Dark theme** and **Light theme** selects (dark ones and light ones).
  - Under the selects, for the theme shown: its problems (red), hints, and "N contrast warnings" (a disclosure listing pair, ratio, threshold).
  - Buttons: **New theme…** (asks a name, the id derived from it, extends the theme shown), **Edit theme** (user themes: `open_theme`), **Open themes folder**. Hidden in a remote space as "Open settings file" is (architecture → Hosts in the UI, item 8).
  - A toast when the theme shown starts falling back ("Dusk can't be used: … Showing One Dark."), not again for the same problem.
- **Palette (6.3):** "Theme: Select…" (the same list, previewed on arrow keys, Esc restores), "Theme: Edit current", "Theme: Open folder".
- **Mock transport:** the built-ins and two fake themes (one fine, one broken), `theme` built by a small TS copy of the resolved built-ins (fixtures generated from the Rust test, never hand-written).

## 9. Reference of every key (for `docs/themes.md`)

The user reference ships as `docs/themes.md` (30.5e), linked from the README and from Settings → Appearance. A test checks that every key of the service's table appears in it. Defaults are One Dark's / One Light's (appendices).

### Top level

| Key | Type | Notes |
|---|---|---|
| `name` | text, 1–64 | Shown in Settings. Default: the file's id |
| `appearance` | `dark` \| `light` | Required without `extends`. Native controls, scrollbars, window, font smoothing, the fallback |
| `extends` | theme id | A built-in or another file; unset: the built-in of `appearance` |
| `$schema` | any | Ignored |

### `colors` (each `--<key>`)

| Key | Colours |
|---|---|
| `bg` | Terminal and editor background, active tab, fields, the window behind the page |
| `panel` | Sidebar, tab bar, right panel, dialogs |
| `bar` | Title bar, status bar |
| `hover` | Hover on rows and buttons |
| `selected` | Selected row, active icon button, scrollbar thumb |
| `selected-hover` | Hover on a selected-coloured button; scrollbar thumb on hover |
| `border` | Borders |
| `border-subtle` | Inner separators |
| `tree-line` | Subagent tree lines, Claude Code box border |
| `focus` | Focus ring, resize handle |
| `accent` | Primary action, links, commented lines' bar |
| `accent-hover` | Primary button hover |
| `accent-disabled` | Disabled primary button |
| `on-accent` | Text on the primary button |
| `on-accent-border` | Kbd chip border on the primary button |
| `text` | Text |
| `text-2` | Secondary text |
| `text-3` | Tertiary text |
| `text-4` | Separators, line numbers, kbd borders |
| `close-hover` | Window close button hover |
| `on-close-hover` | Its cross |
| `danger-hover` | Danger button hover |
| `warning-surface`, `warning-border` | The editor's conflict banner |
| `match` | Search result highlights in lists |
| `active-line` | The editor's cursor line (translucent) |
| `editor-selection` | Selected text in the editor and viewer |
| `backdrop` | Behind a modal dialog |
| `shadow`, `shadow-picker`, `shadow-toast` | Shadow colours (their geometry is fixed) |

### `states` (`--state-<key>`), `git` (`--git-<key>`)

`states`: `permission` (also plan and question), `error`, `you`, `working`, `subagents`, `idle`, `ended`: the agent state icons and dots (the menu bar icon excepted). `git`: `modified`, `added`, `deleted`, `renamed`: file rows, the editor's change gutter, the Changes panel.

### `diff` (`--diff-<key>`)

`added-line`, `added-text` (the changed words), `removed-line`, `removed-text`, `added-gutter`, `removed-gutter`, `hunk` (collapsed lines' background), `hunk-text`.

### `colorblind`

The same keys as `git` and `diff` (as `git.added`, `diff.added-line`, …), used instead of them when Settings → "Diff colours" is "Colour-blind" (28.6). Unset keys: 28.6's built-in pair. The appendix values are placeholders until 28.6 merges (Q13).

### `syntax` (`--syn-<key>`)

`keyword`, `name` (variables, properties, macros), `function`, `constant` (numbers' and atoms' group in One Dark's map: booleans, `self`-like names), `type` (types, classes, numbers), `operator` (also URLs, escapes, regexps), `comment`, `string`, `plain` (definitions, separators; unset: the text colour). A colour or `{color, italic, bold}`. 30.1 may add groups; they are new keys in this section.

### `terminal`

`background`¹, `foreground`¹, `cursor`, `cursor-accent`¹, `selection`, `scrollbar`, `scrollbar-hover`, `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `bright-black` … `bright-white`, `search-match`², `search-match-active`², `search-match-border`², `search-ruler`², `search-ruler-active`², `line-height` (1.0–2.0). ¹ linked (4.1). ² opaque only.

### `fonts`

| Key | Fields | Range |
|---|---|---|
| `ui` | `family` (`--font-sans`) | |
| `ui-mono` | `family` (`--font-mono`: worktree, branch, tab names) | |
| `code` | `family` (`--font-code`), `size` (`--fs-code`), `line-height` (`--lh-code`) | 8–32, 10–48 |
| `terminal` | `family`, `size` | 8–32 (Q4) |

### `text` (`--fs-<key>`, px)

`title` 15, `body` 13, `small` 12, `mono` 12.5, `meta` 11, `kbd` 10.5, `button-small` 11.5 (new), `body-line-height` 18 (`--lh-body`, new). Each 8–32 (line height 10–48).

### `space` (`--space-<n>`, px, new)

`1` 2, `2` 4, `3` 6, `4` 8, `5` 10, `6` 12, `7` 16, `8` 24. Each 0–48. Paddings, margins and gaps.

### `sizes` (px)

`bar` 34 (`--bar-h`: title bar, tab bar, panel headers; 24–64), `status` 24 (`--status-h`; 16–48), `row-project` 26, `row-worktree` 24, `row-file` 22, `row-picker` 30, `row-branch` 24, `row-subagent` 18, `button` 28, `button-ghost` 24, `field` 30 (rows and controls 14–64). Panel widths are not here: the user drags them.

### `radius` (px)

`sm` 3 (`--radius-sm`), `base` 4 (`--radius`), `md` 6 (`--radius-md`), `lg` 8 (`--radius-lg`). Each 0–16.

### `borders` (px)

`width` 1 (`--border-w`, 0–3), `focus-width` 1 (`--focus-w`, 1–3).

## 10. Security

- Theme values reach CSS, so every value is **re-serialised by Rust from its typed form** (a parsed colour, a number, a checked family list), never copied from the file; variable names come from the service's fixed table. A file cannot inject `; background: url(…)`, a selector, or a font file; the CSP is untouched.
- Size limits on the file (64 KiB), the folder (64 themes), names (64), families (256 bytes), chain depth (8). No link followed. Problems and hints quote at most 80 characters of the file.
- The `localStorage` copy is checked by a strict pattern before use (8), since anything in the webview could have written it.
- Remote hosts never send themes (3). `create_theme` writes only `~/.hive/themes/<valid id>.json`, never over an existing file.

## 11. Questions for the human

| # | Question | Recommendation |
|---|---|---|
| Q1 | Folder: `~/.hive/themes/` (D22.16) or the TODO's `$XDG_CONFIG_HOME/hive/themes/`? | `~/.hive/themes/`: every file of Hive's is in `~/.hive` since 22.1 |
| Q2 | Plain JSON (as `settings.json`, no dependency) or JSONC (comments, trailing commas; a new crate)? | JSON for 1.0; JSONC later if hand-editing hurts |
| Q3 | A bad value: that key falls back to its parent (recommended) or the whole theme does (#47's rule for the settings)? | Per key: a typo while editing live should not throw the whole look away |
| Q4 | The terminal's font: the theme's with the settings as overrides (A), or the settings only (B)? | A, with today's default read as "the theme's" so nothing changes |
| Q5 | Spacing: tokenise every padding/margin/gap into an 8-step scale (step a, the biggest part of the build) or only sizes, radius and fonts for 1.0? | The scale: D34.2 names spacing; done as a pixel-identical refactor |
| Q6 | The editor switches from `oneDark` to the theme's `tok-*` colours, which changes One Light's editor (2.3) and slightly One Dark's (definitions, comments)? | Yes: today One Light's editor is unreadable by computation |
| Q7 | With `system`: `dark_theme` / `light_theme` keys (as Zed) | Yes |
| Q8 | "New theme…" writes a starter file that extends the current theme and opens it | Yes: the only write in `themes/` |
| Q9 | Unknown keys: ignored silently (#47) or ignored with a hint? | A hint: typos otherwise do nothing |
| Q10 | The built-ins fail 19 checked pairs (5.3: 4 in One Dark, 15 in One Light): accept them as listed, or adjust the colours to pass? | Adjust One Light's syntax, state/git colours and on-accent in 30.5 (screenshots for approval); accept One Dark's four (its look is #23's reference) |
| Q11 | The menu bar icon's tints stay fixed (the menu bar is the system's) | Yes |
| Q12 | No font files in themes (installed or bundled families only) | Yes for 1.0; plugins (30.7) may bring fonts |
| Q13 | The colour-blind diff colours (28.6) live in each theme's `colorblind` section | Yes; values from 28.6 |
| Q14 | 30.5 is estimated 13 points; this design measures about 21 (12) | Split 30.5 into 30.5a–e as below |

## 12. Build plan (30.5, after approval and 30.1)

| Step | What | Tests | Pts |
|---|---|---|---|
| **a. Tokens** | The hard-coded colours of 2.2 become variables; the spacing scale, sizes, borders, `--fs-code` / `--lh-code` / `--fs-button-small` / `--lh-body` replace raw values; the editor and diff view use `tok-*` classes and `--diff-*` instead of `oneDark` (the only intended visual change, Q6). One Light's CSS block stays until c. | e2e screenshots of every existing spec in both themes before and after: pixel-identical except the editor's syntax in One Light (reviewed) | 8 |
| **b. `hive::theme`** | `crates/hive/themes/one-dark.json`, `one-light.json`; parse, resolve `extends`, linked keys, value checks, the two failure levels, limits, `hive::contrast` (28.6's function), the fixed key → variable table; protocol types (`themes`, `theme`, the settings' string `theme` and `dark_theme`/`light_theme`) | Rust: every rule of 5 and every key's range; the built-ins with zero problems and only accepted warnings; a cycle, a missing parent, a link, 65 files, 64 KiB + 1; `mutants` on the module | 5 |
| **c. Service** | Folder made at start, `notify` watch + debounce, `list_themes`, `create_theme`, `open_theme`, `open_themes_folder`, `theme` on connect/save/reload, `set_settings` checks of theme ids; `REMOTE_TO_UI` drops `themes`/`theme`; architecture's catalog and Settings section | Rust unit + integration (`common::Env`): edit a file → one `theme`; delete the one in use → fallback; a remote's `theme` dropped | 3 |
| **d. App** | The `<style>` element, `data-appearance`, terminals and CodeMirror from `theme`, the launch cache, the drift test, One Light's CSS block and `ONE_*`/`MATCHES`/`THEME_BACKGROUND` removed; Settings → Appearance, palette commands, mock transport | bun: applying, the cache's pattern, fallback toast once, the selects and groups, System's two selects; e2e: a custom theme, a broken one, live reload in the mock, both built-ins unchanged | 4 |
| **e. Docs** | `docs/themes.md` (section 9, with a worked example), `docs/ui-reference.md` pointed at it, README link | The key-coverage test of 8 | 1 |

New dependencies: none (`notify`, `serde_json` are in already).

## Appendix A: `one-dark.json`

Today's values, unchanged, with the hard-coded colours of 2.2 written as 8-digit hex (the same colour). The linked terminal keys are left unset (4.1).

```json
{
  "name": "One Dark",
  "appearance": "dark",
  "colors": {
    "bg": "#282c33",
    "panel": "#2f343e",
    "bar": "#3b414d",
    "hover": "#363c46",
    "selected": "#454a56",
    "selected-hover": "#4f5563",
    "border": "#464b57",
    "border-subtle": "#363c46",
    "tree-line": "#4a505c",
    "focus": "#47679e",
    "accent": "#74ade8",
    "accent-hover": "#8bbbee",
    "accent-disabled": "#3e4a5c",
    "on-accent": "#1f2329",
    "on-accent-border": "#1f232959",
    "text": "#dce0e5",
    "text-2": "#a9afbc",
    "text-3": "#878a98",
    "text-4": "#5d636f",
    "close-hover": "#c42b1c",
    "on-close-hover": "#ffffff",
    "danger-hover": "#db8a8e",
    "warning-surface": "#dec1841a",
    "warning-border": "#dec18447",
    "match": "#dec1844d",
    "active-line": "#ffffff0a",
    "editor-selection": "#3e4451",
    "backdrop": "#1214188c",
    "shadow": "#00000073",
    "shadow-picker": "#00000066",
    "shadow-toast": "#0000004d"
  },
  "states": {
    "permission": "#dec184",
    "error": "#d07277",
    "you": "#e08a5a",
    "working": "#74ade8",
    "subagents": "#b477cf",
    "idle": "#a1c181",
    "ended": "#878a98"
  },
  "git": {
    "modified": "#dec184",
    "added": "#a1c181",
    "deleted": "#d07277",
    "renamed": "#74ade8"
  },
  "diff": {
    "added-line": "#a1c1811f",
    "added-text": "#a1c18140",
    "removed-line": "#d072771f",
    "removed-text": "#d0727740",
    "added-gutter": "#a1c181",
    "removed-gutter": "#d07277",
    "hunk": "#74ade814",
    "hunk-text": "#74ade8"
  },
  "colorblind": {
    "git": { "added": "#74ade8", "deleted": "#e08a5a" },
    "diff": {
      "added-line": "#74ade81f",
      "added-text": "#74ade840",
      "removed-line": "#e08a5a1f",
      "removed-text": "#e08a5a40",
      "added-gutter": "#74ade8",
      "removed-gutter": "#e08a5a"
    }
  },
  "syntax": {
    "keyword": "#c678dd",
    "name": "#e06c75",
    "function": "#61afef",
    "constant": "#d19a66",
    "type": "#e5c07b",
    "operator": "#56b6c2",
    "comment": { "color": "#7f848e", "italic": true },
    "string": "#98c379",
    "plain": "#abb2bf"
  },
  "terminal": {
    "cursor": "#dce0e5",
    "selection": "#454a56",
    "scrollbar": "#454a56",
    "scrollbar-hover": "#4f5563",
    "black": "#3b414d",
    "red": "#d07277",
    "green": "#a1c181",
    "yellow": "#dec184",
    "blue": "#74ade8",
    "magenta": "#b477cf",
    "cyan": "#6eb4bf",
    "white": "#a9afbc",
    "bright-black": "#5d636f",
    "bright-red": "#d07277",
    "bright-green": "#a1c181",
    "bright-yellow": "#dec184",
    "bright-blue": "#74ade8",
    "bright-magenta": "#b477cf",
    "bright-cyan": "#6eb4bf",
    "bright-white": "#dce0e5",
    "search-match": "#5f594b",
    "search-match-active": "#958564",
    "search-match-border": "#dec184",
    "search-ruler": "#dec184",
    "search-ruler-active": "#e08a5a",
    "line-height": 1.2
  },
  "fonts": {
    "ui": { "family": "\"IBM Plex Sans\", system-ui, sans-serif" },
    "ui-mono": { "family": "\"IBM Plex Mono\", monospace" },
    "code": { "family": "\"Hive Mono\", \"Symbols Nerd Font\", monospace", "size": 12, "line-height": 18 },
    "terminal": { "family": "\"Hive Mono\", \"Symbols Nerd Font\", monospace", "size": 13 }
  },
  "text": {
    "title": 15,
    "body": 13,
    "small": 12,
    "mono": 12.5,
    "meta": 11,
    "kbd": 10.5,
    "button-small": 11.5,
    "body-line-height": 18
  },
  "space": { "1": 2, "2": 4, "3": 6, "4": 8, "5": 10, "6": 12, "7": 16, "8": 24 },
  "sizes": {
    "bar": 34,
    "status": 24,
    "row-project": 26,
    "row-worktree": 24,
    "row-file": 22,
    "row-picker": 30,
    "row-branch": 24,
    "row-subagent": 18,
    "button": 28,
    "button-ghost": 24,
    "field": 30
  },
  "radius": { "sm": 3, "base": 4, "md": 6, "lg": 8 },
  "borders": { "width": 1, "focus-width": 1 }
}
```

`syntax.plain` `#abb2bf` and the comment's italic are the two values where today's editor and Markdown preview differ (2.3); the file takes the editor's `plain` and the preview's comment. The `colorblind` values are placeholders (the theme's own blue and orange) until 28.6 merges (Q13).

## Appendix B: `one-light.json`

Written in full (no `extends`), so either built-in reads alone. Values from today's One Light block; what that block does not override (fonts, sizes, radius, `close-hover`, the hard-coded colours, the diff's backgrounds) is today's One Dark value, as shown in One Light now. `hunk-text` is One Light's accent, as `var(--accent)` gives today. Q10 may change the syntax and state colours.

```json
{
  "name": "One Light",
  "appearance": "light",
  "colors": {
    "bg": "#f0f0ee",
    "panel": "#e6e6e3",
    "bar": "#dbdbd8",
    "hover": "#dcdcd9",
    "selected": "#d2d2ce",
    "selected-hover": "#c8c8c4",
    "border": "#cfcfcb",
    "border-subtle": "#dcdcd9",
    "tree-line": "#cfcfcb",
    "focus": "#7d82e8",
    "accent": "#5c78e2",
    "accent-hover": "#4a67d6",
    "accent-disabled": "#3e4a5c",
    "on-accent": "#f7f7f5",
    "on-accent-border": "#1f232959",
    "text": "#2e3036",
    "text-2": "#55575c",
    "text-3": "#74767c",
    "text-4": "#aaacb1",
    "close-hover": "#c42b1c",
    "on-close-hover": "#ffffff",
    "danger-hover": "#db8a8e",
    "warning-surface": "#dec1841a",
    "warning-border": "#dec18447",
    "match": "#dec1844d",
    "active-line": "#0000000a",
    "editor-selection": "#5c78e23d",
    "backdrop": "#1214188c",
    "shadow": "#00000073",
    "shadow-picker": "#00000066",
    "shadow-toast": "#0000004d"
  },
  "states": {
    "permission": "#a48819",
    "error": "#d36151",
    "you": "#ad6e25",
    "working": "#5c78e2",
    "subagents": "#a449ab",
    "idle": "#669f59",
    "ended": "#7e8086"
  },
  "git": {
    "modified": "#a48819",
    "added": "#669f59",
    "deleted": "#d36151",
    "renamed": "#5c78e2"
  },
  "diff": {
    "added-line": "#a1c1811f",
    "added-text": "#a1c18140",
    "removed-line": "#d072771f",
    "removed-text": "#d0727740",
    "added-gutter": "#669f59",
    "removed-gutter": "#d36151",
    "hunk": "#74ade814",
    "hunk-text": "#5c78e2"
  },
  "colorblind": {
    "git": { "added": "#5c78e2", "deleted": "#ad6e25" },
    "diff": {
      "added-line": "#5c78e21f",
      "added-text": "#5c78e240",
      "removed-line": "#ad6e251f",
      "removed-text": "#ad6e2540",
      "added-gutter": "#5c78e2",
      "removed-gutter": "#ad6e25"
    }
  },
  "syntax": {
    "keyword": "#a626a4",
    "name": "#e45649",
    "function": "#4078f2",
    "constant": "#986801",
    "type": "#c18401",
    "operator": "#0184bc",
    "comment": { "color": "#a0a1a7", "italic": true },
    "string": "#50a14f",
    "plain": "#383a42"
  },
  "terminal": {
    "cursor": "#5c78e2",
    "selection": "#5c78e23d",
    "scrollbar": "#383a414c",
    "scrollbar-hover": "#383a4180",
    "black": "#383a42",
    "red": "#de3e35",
    "green": "#3f953a",
    "yellow": "#d2b67c",
    "blue": "#2f5af3",
    "magenta": "#950095",
    "cyan": "#0997b3",
    "white": "#bbbbbb",
    "bright-black": "#696c77",
    "bright-red": "#de3e35",
    "bright-green": "#3f953a",
    "bright-yellow": "#d2b67c",
    "bright-blue": "#2f5af3",
    "bright-magenta": "#a00095",
    "bright-cyan": "#0bbcd6",
    "bright-white": "#f7f7f5",
    "search-match": "#e4d9c2",
    "search-match-active": "#d6c091",
    "search-match-border": "#b08d45",
    "search-ruler": "#d2b67c",
    "search-ruler-active": "#e08a5a",
    "line-height": 1.2
  },
  "fonts": {
    "ui": { "family": "\"IBM Plex Sans\", system-ui, sans-serif" },
    "ui-mono": { "family": "\"IBM Plex Mono\", monospace" },
    "code": { "family": "\"Hive Mono\", \"Symbols Nerd Font\", monospace", "size": 12, "line-height": 18 },
    "terminal": { "family": "\"Hive Mono\", \"Symbols Nerd Font\", monospace", "size": 13 }
  },
  "text": {
    "title": 15,
    "body": 13,
    "small": 12,
    "mono": 12.5,
    "meta": 11,
    "kbd": 10.5,
    "button-small": 11.5,
    "body-line-height": 18
  },
  "space": { "1": 2, "2": 4, "3": 6, "4": 8, "5": 10, "6": 12, "7": 16, "8": 24 },
  "sizes": {
    "bar": 34,
    "status": 24,
    "row-project": 26,
    "row-worktree": 24,
    "row-file": 22,
    "row-picker": 30,
    "row-branch": 24,
    "row-subagent": 18,
    "button": 28,
    "button-ghost": 24,
    "field": 30
  },
  "radius": { "sm": 3, "base": 4, "md": 6, "lg": 8 },
  "borders": { "width": 1, "focus-width": 1 }
}
```

`syntax.plain` `#383a42` is Atom One Light's plain text (One Light's editor has no value of its own today, 2.3). The `colorblind` values are placeholders until 28.6 merges.
