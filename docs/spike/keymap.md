# Spike 28.7: Custom keyboard shortcuts (the keymap)

Design task 28.7 (`TODO.md`, ⏸ checkpoint: no production code before the human approves). It **changes #35** (fixed shortcuts) and adds to #47 (the settings file); the defaults stay #35's, as the app has them today. Written 2026-11-27 against `main` at `89ec3828`. Read: #35, #37, #47, #53, #57, #58; 6.3 (the palette), 8.20 (no browser dialogs), 17.13 (the app menu), D17.2 / 17.14 / 18.34 / D18.10 (macOS keys); code: `src/shortcuts.ts`, `src/menu.ts`, `src/terminals.ts` (`keys`, `CLIPBOARD_KEYS`, `MAC_TERMINAL_KEYS`), `src/window.ts` (`commandKey`, `keyText`), `src/shell/Palette.tsx`, `src/shell/SettingsDialog.tsx` (`Shortcuts`), `src/viewer/editor.ts`, `src-tauri/src/menu.rs`, `crates/hive/src/settings.rs`, `crates/hive-protocol/src/lib.rs` (`Settings`).

## 1. Recommendation in one paragraph

The settings file gets a `keymap` object holding **only the user's changes**: a command id of `COMMANDS` (the palette's commands, 6.3) mapped to a chord, a list of chords, or `[]` for no shortcut. A command not in it keeps #35's default, so a later change of a default still reaches the user, "reset" is "remove the entry" and "reset all" is "empty the object". Chords are written as the platform names its keys (`Cmd+Shift+P` on macOS, `Ctrl+Shift+P` on Windows; `Ctrl` is always the Control key). The service owns the file and the **grammar**: it parses every chord, normalises the valid ones, and lists every unknown command, bad chord or fixed key as a **problem line** sent with `settings`; a bad entry is ignored and kept in the file as written, **never** a refused file and never a panic. The chord grammar lives in `hive_protocol::keymap`, so the app side (`src-tauri`) uses the same parser to keep the macOS menu's accelerators in step. The UI keeps what is presentation (#37): the command table (labels, actions, contexts, #35's defaults), the resolver that matches a key event to a command, the conflict view and the recorder. Settings → **Keyboard** (today's read-only Shortcuts section) lists every command with its chords, filters them, records a new chord, shows conflicts, and resets one or all. What stays fixed: the terminal's own keys (clipboard, the macOS line keys, everything not bound), the editor's and the dialogs' local keys, the macOS menu's native items, WebView2's browser accelerators off (#53) and 8.20's lint.

## 2. Today

- `COMMANDS` (`src/shortcuts.ts`): 28 commands, each `{id, label, keys, macKeys?, mac?, when?, run}`. `keys` is written for Windows, `Ctrl` meaning Cmd on macOS and `Control` the Control key there; `macKeys` is a second chord on macOS; `mac` commands exist only on macOS; `when` limits a command to a state (a terminal shown, a Markdown file shown), its keys going to the terminal otherwise.
- `shortcut(event)` finds the first platform command whose keys the event presses (`presses`: modifiers exact, the key by its character or, for `[ ] = -` and digits, by `event.code`) and whose `when` holds; nothing runs under the connection block or with a dialog open (`commandsBlocked`). A focused terminal asks `interceptKeys` first; every other key is the terminal's, including its clipboard chords (`CLIPBOARD_KEYS`: Ctrl+Shift+C/V, ⌘C/⌘V).
- The palette lists every `COMMANDS` entry with `commandKeys(c)`, plus `paletteCommands` extras with no id and no keys ("Remove merged worktrees…", "Run: <name>", "Send open comments", "Who wrote this line?"…).
- Settings → Shortcuts lists `platformCommands()` and, on macOS, `MAC_TERMINAL_KEYS`, read-only.
- The macOS app menu (`src-tauri/src/menu.rs`) has three accelerators of Hive's own items that are also commands: Settings… `CmdOrCtrl+,` (`settings`), the side panel `CmdOrCtrl+Shift+B` (`toggle-panel`), Close Tab `CmdOrCtrl+W` (`close-tab`); plus Quit ⌘Q and AppKit's native items (⌘H, ⌥⌘H, ⌘Z, ⇧⌘Z, ⌘X, ⌘C, ⌘V, ⌘A, ⌃⌘F, ⌘M).
- `hive::settings::read` refuses the **whole file** for any invalid value (the defaults plus `settings_failed`), and `set_settings` refuses the whole save. The file is read at start only; a hand edit applies on the next service start or is overwritten by the next save from the dialog.

## 3. The `keymap` section of the settings file

### 3.1 Shape

```json
{
  "keymap": {
    "palette": "Cmd+K",
    "next-pending": ["F8", "Cmd+J"],
    "comment": [],
    "close-tab": "Ctrl+Shift+W"
  }
}
```

- Key: a command id of `COMMANDS` (`palette`, `settings`, `worktree-picker`, `new-terminal`, `new-worktree`, `split-terminal`, `toggle-panel`, `add-project`, `send-reference`, `comment`, `next-pending`, `find-in-terminal`, `toggle-markdown`, `close-tab`, `next-tab`, `previous-tab`, `tab-1`…`tab-9`, `font-bigger`, `font-smaller`, `font-reset`; later tasks add theirs, e.g. 30.3's go to definition).
- Value: one chord (a string), several (a list, every one works; the first is the one shown in tooltips, the palette and the menu), or `[]` / `null` for no shortcut (the command stays in the palette).
- **Only changes are kept.** An absent command has its default (section 4). The dialog writes an entry only when the user's chords differ from the default, and removes it when they equal it again.
- Missing `keymap`: `{}`. It is optional like every other key (#47).

### 3.2 Chord grammar (`hive_protocol::keymap::Chord`)

- `modifier+…+key`, separated by `+`, case-insensitive, spaces around `+` allowed.
- Modifiers: `Ctrl` (alias `Control`): the Control key on both platforms; `Alt` (aliases `Option`, `Opt`); `Shift`; `Cmd` (alias `Command`): **macOS only**. Each at most once.
- Keys: `A`–`Z`, `0`–`9`, `F1`–`F24`, the unshifted punctuation of a US layout `` ` - = [ ] \ ; ' , . / ``, and `Tab`, `Enter`, `Esc`, `Space`, `Backspace`, `Delete`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`, `Up`, `Down`, `Left`, `Right`. A shifted character is written as Shift plus its key (`Shift+[`, never `{`), so the chord does not depend on the layout's Shift row; matching stays today's (the character, or the physical key for punctuation and digits).
- **Normalised** when valid and saved in that form: modifiers in macOS's order ⌃⌥⇧⌘ (`Ctrl+Alt+Shift+Cmd+P`), keys capitalised as above. A single chord is saved as a string, several as a list; duplicates inside one entry are dropped.
- A chord **needs a modifier other than Shift**, except `F1`–`F24` alone or with modifiers (F8 is a default). A letter, digit, punctuation, Enter, Tab, Space, Backspace, Esc or an arrow alone, or with only Shift, would take that key from every terminal and text field: a problem.
- One key per chord: no two-step sequences (⌘K ⌘S); see Q3.
- Size limits: an id at most 64 bytes, a chord at most 64 bytes, at most 8 chords per command, at most 128 entries. What is past a limit is ignored and listed as a problem; the file's 256 KiB limit still holds.

### 3.3 Service-side validation

`hive_protocol::keymap::check(&Keymap, Platform) -> Vec<String>` (pure, `Platform` a parameter so both platforms' rules are tested on every OS; the store passes `Platform::current()`: macOS on macOS, Windows elsewhere, since a Linux or WSL service serves a Windows app and a remote's settings never reach the UI, 26.18). It returns one line per problem and never fails:

| Case | Problem line (example) | What applies |
|---|---|---|
| Unknown command id | `keymap.palete: Hive has no command "palete"` | entry ignored |
| Not a string, a list of strings or null | `keymap.comment: a shortcut is a text like "Cmd+Shift+M" or a list of them (got 3)` | entry ignored |
| Bad chord | `keymap.palette: "Cmd+Shift+" is not a shortcut: a key is missing after +` / `"Hyper+P": Hyper is not a modifier` / `"Cmd+Cmd+P": Cmd twice` / `"Cmd+{": write it as Shift+[` | that chord ignored, the entry's other chords kept |
| `Cmd` on Windows | `keymap.palette: "Cmd+P": Cmd exists only on macOS` | that chord ignored |
| No modifier | `keymap.comment: "M" alone would take the key from the terminal: add Ctrl, Alt or Cmd` | that chord ignored |
| A fixed key (section 6) | `keymap.palette: "Cmd+C" is the terminal's Copy and stays fixed` / `"Cmd+Q" is the menu's Quit` | that chord ignored |
| Past a limit | `keymap: more than 128 entries, the rest are ignored` | rest ignored |

- Every quoted user text goes through Rust's `{:?}` (control characters escaped) and is cut at 64 bytes; at most 50 lines, then `… and N more problems`.
- **The file is never refused for its keymap**: `read` and `set_settings` keep every other setting. A bad entry stays in the file **as written** (the dialog's save does not erase a hand-typed mistake the user has not seen): `Keymap` deserialises leniently (any JSON value per entry, kept verbatim when it is not a valid binding; a `keymap` that is not an object is kept as `{}` with a problem), so a wrong type in `keymap` can no longer make the whole file "invalid".
- Conflicts are **not** a service problem (two commands on one chord can be intended, e.g. contexts that never meet); the Keyboard section shows them (section 5.3).

## 4. Defaults (#35, unchanged)

The defaults move from `keys`/`macKeys`/`mac` to one explicit list per platform, in the new notation; the effective chords are exactly today's.

| Command | Windows | macOS |
|---|---|---|
| `palette` | Ctrl+Shift+P | Shift+Cmd+P |
| `settings` | Ctrl+, | Cmd+, |
| `worktree-picker` | Ctrl+Shift+T | Shift+Cmd+T |
| `new-terminal` | — | Cmd+T |
| `new-worktree` | Ctrl+Shift+N | Shift+Cmd+N, Cmd+N |
| `split-terminal` | Ctrl+Shift+D | Shift+Cmd+D |
| `toggle-panel` | Ctrl+Shift+B | Shift+Cmd+B |
| `add-project` | Ctrl+Shift+O | Shift+Cmd+O |
| `send-reference` | Ctrl+Shift+L | Shift+Cmd+L |
| `comment` | Ctrl+Shift+M | Shift+Cmd+M |
| `next-pending` | F8 | F8, Shift+Cmd+J |
| `find-in-terminal` (terminal shown) | Ctrl+Shift+F | Shift+Cmd+F, Cmd+F |
| `toggle-markdown` (Markdown shown) | Ctrl+Shift+V | Shift+Cmd+V |
| `close-tab` | — | Cmd+W |
| `next-tab` / `previous-tab` | — | Shift+Cmd+] , Ctrl+Tab / Shift+Cmd+[ , Ctrl+Shift+Tab |
| `tab-1`…`tab-9` | — | Cmd+1…Cmd+9 |
| `font-bigger` / `font-smaller` / `font-reset` | — | Cmd+= / Cmd+- / Cmd+0 |

"—": no default there (today's `mac: true` commands, whose keys are the shell's on Windows). With a keymap there is no reason to hide them on Windows: they exist everywhere and can be bound (Q2).

## 5. The UI

### 5.1 Resolver (`src/keymap.ts`, new)

- `COMMANDS` loses `keys`, `macKeys` and `mac`; it gains `context: "any" | "terminal" | "markdown"` (today's `when`, named so conflicts can tell disjoint contexts) and `defaults: {windows: string[], mac: string[]}`.
- `bindings(settings.keymap, platform)`: each command's chords: the user's valid ones when it has an entry, else its defaults. The UI reads validity with the same rules as the service only to the extent of "does this parse" (`parseChord`, a small TS mirror of the grammar, checked against shared samples, section 8); the problem texts come from the service.
- `shortcut(event)`: the command whose chord the event presses, in its context, blocked as today (`commandsBlocked`). Precedence when two commands share a chord in one context: **a user's chord beats a default** (the user just chose it), then table order. The menu (`src/menu.ts`), the palette's key column, tooltips and `keyText` read the first effective chord.
- New guard: a key the focused editor (CodeMirror) or a text field already handled (`event.defaultPrevented`), or a chord with no Ctrl/Cmd typed into a text field or the editor (an Option character on macOS, AltGr on Windows), is the field's.

### 5.2 Settings → Keyboard

The Shortcuts section is renamed **Keyboard** (`SECTIONS`, its icon kept). From the top:

1. **Problems** from the service, if any: the lines, in the warning colour, with "Open settings file" (`open_settings_file`).
2. **Search** (`SearchField`): filters rows by label, id or chord text (`cmd p`, `⌘P`, `ctrl+shift+p` all match `Shift+Cmd+P`); the dialog's global search also finds the section by "keyboard"/"shortcut".
3. **Command rows**, in table order: label; each chord as a `kbd` chip with a remove ×; "+" to record another chord; a dot and **Reset** when the command has an entry (back to the default); "No shortcut" when it has none.
4. **Reset all** (only with entries): asks through Hive's `ask` dialog (8.20: never `confirm`), "Reset every shortcut to Hive's defaults?", then saves `keymap: {}`.
5. **Fixed keys**, read-only below: the terminal's clipboard keys, on macOS `MAC_TERMINAL_KEYS`, and the local keys of #35 (palette and tree arrows, Enter, Esc, the composer's #58, Alt+↑/↓ #57).

**Recording** ("+"): the chip becomes a focused capture field ("Press the keys…"). The next keydown that is not a lone modifier is the chord (`chordOf(event)`: modifiers from the event; the key from `event.code` for letters, digits and punctuation, so Option+P records `Alt+P`, not `π`; else from `event.key`). Esc cancels, a click outside cancels, nothing else the user presses runs (Settings is a dialog: `commandsBlocked` already holds). Then:
- **Refused** (shown under the field, nothing saved): no modifier, a fixed key ("⌘C is the terminal's Copy and stays fixed").
- **Conflict**: the chord is another command's in an overlapping context: "⇧⌘P is also Command palette's", with **Take it** (also removes it from the other command, writing its entry) and **Keep both** (saved; the new one wins, section 5.1). A chord the terminal would otherwise get (any Ctrl chord or, on macOS, Option chord: the shell's keys, #35's reason) is saved with a note: "Ctrl+R no longer reaches the terminal".
- Saving is `saveSettings` (whole settings, `set_settings`), as every other field; the service's answer is the truth (normalised chord, problems).

### 5.3 Conflicts

Computed in the UI from the effective bindings: two commands with an equal chord whose contexts overlap (`any` overlaps everything; `terminal` and `markdown` never meet, so `toggle-markdown` on Ctrl+Shift+V and the terminal's paste are no conflict, as today). Both rows show "Also <label>" in the warning colour, and the one that loses says so ("⇧⌘P runs Comment"). A chord equal to a fixed key cannot reach this point (refused).

## 6. What stays fixed

- **The terminal's keys.** Every key no command takes still goes to the terminal untouched (#35). Its clipboard chords (Ctrl+Shift+C/V on Windows, ⌘C/⌘V on macOS) are fixed and refused in the keymap. The macOS line keys (`MAC_TERMINAL_KEYS`: ⌘K, ⌘↑/↓, ⌘←/→, ⌘⌫, Fn+arrows) are fixed too: refused, so a terminal's habits never break by a stray binding (Q4).
- **Local keys** (#35: "teclas locais não são atalhos do app"): the tree's arrows and Enter, the palette's and pickers' arrows/Enter/Esc, the chat composer (#58), Alt+↑/↓ on a focused agent row (#57), the editor's own (CodeMirror: ⌘S/Ctrl+S save, Mod-F search, undo/redo…, which win while the editor has the focus, section 5.1). Not in the keymap; listed under Fixed keys.
- **The macOS menu's native items**: ⌘Q (Quit), ⌘H, ⌥⌘H, ⌘Z, ⇧⌘Z, ⌘X, ⌘C, ⌘V, ⌘A, ⌃⌘F, ⌘M: refused. Hive's own items that are commands (Settings…, the side panel, Close Tab) **follow the keymap** (section 7.2).
- **#53**: WebView2's browser accelerators stay off (`SetAreBrowserAcceleratorKeysEnabled(false)`, `main.rs`) whatever the keymap says: binding Ctrl+P, F5 or Ctrl+R to a Hive command works because the browser no longer acts on them, and the keymap can never turn a browser shortcut back on. The default context menu rule and 8.20's lint against `alert`/`confirm`/`prompt` are untouched (Reset all uses `ask`).
- The **reserved list** (clipboard, macOS terminal keys, native menu items) is `hive_protocol::keymap::FIXED`, per platform, with the text each problem names; the UI's recorder reads the same entries (checked equal by a bun test, section 8).

## 7. Protocol and app side

### 7.1 Messages

- `Settings` gains `keymap: Keymap` (`BTreeMap<String, Binding>`, `Binding` a lenient value: one chord, a list, null, or anything else kept verbatim). TS: `keymap: Record<string, string | string[] | null>` (an invalid value typed as `unknown` and passed back untouched). `DEFAULT_SETTINGS.keymap = {}`.
- `settings {settings, problems}`: new `problems: [string]`, the keymap's lines (section 3.3), empty when none; general so later sections can add theirs. Missing (an older service, not possible across `version_mismatch` but cheap): `[]`. The store keeps it as `settingsProblems`.
- No new request: record, remove, reset one and reset all are `set_settings`. `set_settings` is never refused for the keymap.
- `docs/architecture.md`: the Settings section (the key, the grammar, the problems), the Shortcuts section (renamed Keymap, the resolver, the precedence, the guard), the message catalog rows of `settings` and `set_settings`.

### 7.2 The macOS menu (`src-tauri`)

The app side already reads the local service's `settings` (`confirm_close`, `remote.hosts`). On each one it also sets the accelerators of the three items that are commands (`settings`, `toggle-panel`, `close-tab`): the command's first effective chord as `hive_protocol::keymap::Chord::accelerator()` (Tauri's `CmdOrCtrl`-free form: `Shift+Cmd+B` → `Shift+Super+B`), the menu's default when the keymap has no entry, none when the user removed it or another command's **user** chord took it (so the native accelerator never steals a key the UI gives to another command). `MenuItem::set_accelerator` at runtime; `menu.rs` keeps the decision pure and tested, `main.rs` (excluded wiring) applies it.

## 8. Build plan

Three tasks after approval, in order (each small enough to merge alone):

1. **28.7a Keymap in the settings (Rust).** `hive_protocol::keymap`: `Chord` (parse, normalise, `Display`, `accelerator()`), `Platform`, `COMMAND_IDS`, `FIXED`, `Keymap`/`Binding` lenient serde, `check`. `Settings.keymap`; `hive::settings` normalises valid chords on read and save, never refuses for the keymap, and sends `problems` with every `settings`. `docs/architecture.md` (Settings, catalog).
2. **28.7b The resolver and the menu.** `src/keymap.ts` (defaults table, `parseChord`, `bindings`, `chordOf`, `chordText` replacing `keyText` for chords, conflicts); `COMMANDS` moved to it (`keys`/`macKeys`/`mac` gone, `context` and `defaults` in); `shortcuts.ts`, `menu.ts`, the palette and tooltips read effective chords; the new field/editor guard; the mock transport stores `keymap` and answers `problems` (TS `check` mirror kept to what the mock needs: unknown ids and unparsable chords). `src-tauri/src/menu.rs`: accelerators from the keymap.
3. **28.7c Settings → Keyboard.** The section (problems, search, rows, record, conflicts, reset one/all, fixed keys), e2e.

`docs/hive.md` #35 is the human's to update (rule 1); a proposed wording is in Q7.

## 9. Test plan

- **Rust (`hive-protocol`, unit):** every grammar rule (each modifier and alias, order and case normalised, every named key, F1–F24 alone, punctuation, shifted characters refused with the hint, missing key, unknown modifier, a modifier twice, `Cmd` on Windows, no modifier, each fixed key per platform, 64-byte chord, 8 chords, 128 entries, 50 problem lines then "and N more", control characters escaped); `Binding` round trips (string, list, null, a number kept verbatim); `accelerator()` per key; `COMMAND_IDS` sorted and unique. Mutants on all of it (`cargo mutants --in-diff`).
- **Rust (`hive`, unit + integration):** a file with a bad keymap loads every other setting and sends `problems`; `set_settings` with a bad entry saves, keeps it verbatim and answers the problem; a valid hand-typed `cmd+shift+p` is saved as `Shift+Cmd+P`; an absent `keymap` is `{}`; the 256 KiB limit still applies. Integration over the socket (`common::Env`): `settings {problems}` after `welcome` for a seeded file.
- **Rust (`src-tauri`):** `menu.rs` accelerator decision: default, override, removed, taken by another command's user chord.
- **bun:** `COMMANDS` ids equal `COMMAND_IDS` and the recorder's fixed list equals `FIXED` (read from the Rust source, as `hosts.test.ts` does for `REMOTE_TO_UI`); `parseChord` against a shared sample table that the Rust tests also run (one fixture file read by both); `bindings` (default, override, `[]`, invalid ignored); `shortcut` (a rebound command runs on its new chord and not on its old one, user beats default, contexts, blocked under a dialog, the field/editor guard); conflicts (overlap, disjoint contexts); `chordOf` (Option+P is `Alt+P`, a lone modifier is nothing, `event.code` for punctuation); the Keyboard section (filter, record, refused, conflict Take it / Keep both, remove, reset one, Reset all asks with `ask` and saves `{}`, problems shown, fixed keys listed); the palette and the menu show the effective chord. 100% lines.
- **e2e (mock, Chromium and WebKit):** record ⇧⌘K (Ctrl+Shift+K on Linux) for the palette in Settings → Keyboard, close Settings, press it with the focus in a terminal: the palette opens, and the old chord now reaches the terminal; Reset brings the default back; a conflict shows on both rows.

## 10. Questions for the human

1. **Who owns the defaults and the conflicts?** Option A: the whole command catalogue (ids, defaults, contexts) in Rust, the service sending the effective keymap and its conflicts. Option B: the service owns the file, the grammar and the problems; the UI keeps the command table (it already holds labels and actions), the defaults, the resolver and the conflicts. **Recommendation: B.** Which key runs which UI action is presentation (#37: the UI keeps UI state and sends user actions; the service still validates what is saved), the TODO's test plan puts the resolver and conflicts in bun, and the one duplicated piece (the id list, the fixed list) is checked equal by a test, as `REMOTE_TO_UI` is.
2. **macOS-only commands on Windows** (close tab, tab 1–9, next/previous tab, font zoom, new terminal here): **Recommendation: they exist everywhere, with no default on Windows**, so a Windows user can bind e.g. Ctrl+Shift+W; today's hiding only protected the shell's keys, which an unbound command does not touch.
3. **Two-step sequences** (⌘K then ⌘S, VS Code's "chords"): **Recommendation: no**, one key per chord; the commands are few and a pending first key would swallow a terminal's next key.
4. **The macOS terminal keys** (⌘K, ⌘↑/↓, ⌘←/→, ⌘⌫): fixed (refused in the keymap) or bindable with a conflict shown? **Recommendation: fixed**, as the clipboard keys: they behave like a native terminal's and are listed under Fixed keys.
5. **Where problems show.** **Recommendation:** at the top of Settings → Keyboard, plus one warning toast ("The keymap has N problems: see Settings → Keyboard") when the list changes, since a hand edit is otherwise silent.
6. **Hand edits apply only at the next service start** (the file is read at start, for every setting). **Recommendation:** leave it to a separate task (watch `settings.json` for every setting, not only the keymap); 28.7 changes nothing there. Until then the Keyboard section is the way to change a shortcut live.
7. **#35's new text** (yours to write in `docs/hive.md`). Proposal: "Atalhos padrão do app com Ctrl+Shift+letra … (lista atual). **Personalizáveis** (28.7): a seção `keymap` do arquivo de configurações (#47) troca o atalho de qualquer comando da paleta; o serviço valida e lista os problemas; ficam fixos as teclas do terminal (copiar/colar, as de linha do macOS), as locais e os itens nativos do menu."
8. **A command that opens the keymap** (Delta's ⌘/). **Recommendation:** add `keyboard-settings`, "Keyboard shortcuts", opening Settings → Keyboard, default ⌘/ on macOS and Ctrl+Shift+/ on Windows (⇧⌘/ is macOS's Help search). It is a new default, so yours to approve.
9. **The palette's extras without an id** ("Remove merged worktrees…", "Send open comments", "Show <view>", "Who wrote this line?", "Run: <name>"). **Recommendation:** not in 28.7; the static ones can become commands with ids and no default later, and "Run: <name>" (per project, dynamic) stays unbindable.

Defaults taken in this design (each a "pending" row in the ledger when built): overrides only in the file; canonical modifier order ⌃⌥⇧⌘; the limits of section 3.2; user chords beating defaults; the section renamed Keyboard; Take it / Keep both on a conflict.
