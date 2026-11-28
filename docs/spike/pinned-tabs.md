# Spike 29.1: pin a tab

Design task 29.1 (`TODO.md`, ⏸ checkpoint: no production code before the human approves). Written 2026-11-28 against `main` at `d97b89c3`. The human's request (2026-11-26): "quero conseguir fixar um chat para evitar que feche acidentalmente um chat". Read: #18, #35 (as changed by 28.7's keymap), #37, #40 and #44 (the in-app chat, removed by 9.2: a "chat" today is a terminal running `claude`, opened by the sidebar's "New chat"), #57, 6.3 (the palette), 4.12 (sessions back after a restart), 8.21 (tab order), 11.1 (the preview tab), 28.19 / 28.27 (back to where you were), Stage 26 (remote services, kept terminals) and 28.14's approved multiplayer design (`docs/spike/multiplayer.md`, D29.1–D29.10). Code: `src/shell/TerminalArea.tsx` (`TabItem`, `TerminalTab`, `FileTab`, the tab menus), `src/tabs.ts` (`barKey`, `inBarOrder`), `src/store.ts` (`removeTab`, `tabOrder`), `src/persist.ts` (`hive.tabOrder`, `hive.places`, `hive.hostPlaces`), `src/shortcuts.ts` (`closeActiveTab`), `src/keymap.ts` (`close-tab`), `src/shell/WorktreeMenu.tsx` (`RemoveWorktreeDialog`).

## 1. Recommendation in one paragraph

**Any tab can be pinned** (a chat, a plain terminal or a file), from its right-click menu or the palette's "Pin tab" / "Unpin tab" (a keymap command with no default shortcut). **A pinned tab cannot be closed by accident:** its × is replaced by a small pin icon that does nothing when clicked; ⌘W and the palette's "Close the tab" leave it open and show a toast ("<name> is pinned. Unpin it to close it."); its menu's "Close" is disabled with "Unpin to close" beside it. The tab **stays where it is** in the bar (#57 unchanged) and keeps its full label and state icon. A pin ends only when the user unpins, or when the tab's terminal cannot come back (its worktree deleted, or a plain terminal at the app's end): the Delete worktree dialog names the pinned tabs that close with it. The pin is **UI state in `localStorage`** (#37, #57), saved beside the tab order under the same bar keys (`session:<id>` for a chat, so a chat resumed after a restart (4.12) comes back pinned; the file key for a file), with the same fallback when storage is empty or throws. It stays out of the service: in 28.14's multiplayer design a pin is one person's protection against their own mistakes, so each person's app keeps their own.

## 2. Today

- **Ways a tab closes:** its × (`TabItem`'s `.tab-close`), its menu ("Close terminal", "Close file"), ⌘W (`close-tab`, macOS default; no Windows default since 28.7) and the palette's "Close the tab" (both `closeActiveTab`). A file with unsaved edits asks first (8.20). **A terminal closes at once, even with an agent working:** #18's confirmation is for quitting the app, not for one tab. The request's "accidentally" is exactly this.
- **No bulk close exists:** no "Close others", "Close all" or "Close to the right". Tabs of a worktree close in bulk only when the worktree is deleted (its terminals end in the service) or when the app quits (#18; a local service ends every terminal, a remote one keeps them, 26.15).
- **What comes back after a restart:** chats (a terminal whose `claude` session ran in Hive) are resumed with `claude --resume <id>` (4.12, `restore_sessions`); files reopen without their edits (28.27); plain local terminals do not come back. A remote's kept terminals are reattached (26.16).
- **The bar's keys** (`barKey`): `session:<id>` while a Claude session runs in the tab, else `tab:<channel>`; a file's is `file:<worktree>\n<path>`. `savedTabOrder` drops `tab:` keys between runs, since a channel names one run only.
- **"Pin" is already a word in the code:** 11.1's preview tab is "pinned on purpose" by a double click (`TabItem`'s `onPin`, `pinFile`), which only makes it a normal tab. The UI never says "pin" for it (the preview's tooltip says "Preview: opening another file replaces it").

## 3. Which tabs can be pinned

| Option | For | Against |
|---|---|---|
| **A. Every tab: chats, terminals, files (recommended)** | One rule, one menu item on every tab; a long-running dev server terminal or a reference file is worth keeping too. A chat whose `claude` exits is still the same tab, so a pin that followed "is a chat" would vanish under the user. | Pins of plain terminals do not survive a restart (§7), so they mean a little less. |
| B. Chats only (a terminal running `claude`) | Exactly the request. | The menu item comes and goes with the agent; an exited agent drops the pin, the moment it matters least to lose but most surprising. |
| C. Chats and terminals, not files | Files already ask before losing unsaved edits. | A saved file closes silently as well; the rule needs explaining. |

**Recommendation: A.** Pinning a preview file tab (11.1) also keeps it (no longer the preview), since a pinned tab cannot be replaced either. In code, 11.1's `onPin`/`pinFile` are renamed (`onKeep`/`keepFile`) so "pin" means one thing.

## 4. What a pin does

### Closing

| Path | Option 1: refused (recommended) | Option 2: asks | Option 3: only a mark |
|---|---|---|---|
| × | Not there: a pin icon in its place, not a button | × kept, asks "Close the pinned tab <name>?" | closes |
| Tab menu "Close …" | Disabled, with "Unpin to close" as its note; "Unpin tab" right above it | asks | closes |
| ⌘W, palette "Close the tab" | Nothing closes; a toast: "<name> is pinned. Unpin it to close it." | asks | closes |
| Delete worktree | Closes (the terminal ends anyway); the dialog lists "Pinned tabs that close with it: …" | same | same |
| App quit | Unchanged (#18's question for working agents); chats come back pinned (§7) | same | same |

**Recommendation: option 1.** A dialog is one Enter away from the same accident (⌘W then Enter, the habit of closing dialogs), while a refusal with a toast costs one deliberate step: unpin, then close. VS Code's `workbench.editor.preventPinnedEditorClose` makes the same choice for keyboard and middle-click closes. The pin icon is not a button, so a stray click on it does nothing (VS Code's pin button unpins on click, one click away from a ×).

- **A terminal that exits by itself** (`/exit`, the shell's `exit`): the tab stays, with 1.7's "exited" badge, pinned. Unpin, then close. Nothing else in Hive closes a tab for the user.
- **A future "Close others / Close all / Close to the right"** skips pinned tabs, as every editor does. None exists today, so 29.2 adds none (§9 Q7).
- **Split** (6.11, 20.5): a pinned tab can be paired; closing the pane of a pinned tab is refused the same way; ending the pair is not a close.
- **F8, ⌘1–9, ⇧⌘[ / ]** follow the bar as today.

### How it looks

| Option | For | Against |
|---|---|---|
| **In place, full width, a pin icon where the × was (recommended)** | No change to #57's order or to ⌘1–9's numbering; a chat keeps its name and state icon, which is why it is open. | Pinned tabs are not grouped. |
| Moved to the start of the bar, in pin order (browsers, VS Code) | A fixed spot for what matters. | **Changes #57** (order = opening order, by drag); ⌘1–9 shift when pinning; a drag across the group's edge needs rules. |
| Narrow, icon only (browsers' pinned tabs) | Takes little room. | A chat's icon is its state, so two pinned chats are indistinguishable; its name only in a tooltip. |

**Recommendation: in place.** The pin icon is Phosphor's `PushPinIcon` (already a dependency), 12 px, `--text-3`, in the 16 px slot of the ×, so the tab keeps its width; on a dirty file the dirty dot wins over the pin (unsaved edits are the louder fact) and the tab's tooltip gains "Pinned". The tab's accessible description gains "Pinned", read after the label like "Preview".

## 5. How to pin and unpin

- **The tab's right-click menu** (terminal, chat and file menus, and a file pane's header menu, 25.1): "Pin tab" / "Unpin tab" (`PushPinIcon` / `PushPinSlashIcon`), above "Close …".
- **The palette** (6.3): one keymap command `pin-tab`, labelled "Pin the tab" / "Unpin the tab" by the shown tab's state, context `any`, on the shown tab (the focused pane's in a split).
- **A shortcut:** since 28.7 (#35 as changed by the keymap), a command can exist **with no default** and the user binds one in Settings → Keyboard. **Recommendation: no default**, so #35's defaults stay as they are. (VS Code's ⌘K ⇧↩ is a chord sequence, which the keymap does not have; any free ⇧⌘<letter> would be a new default for the human to approve.)
- Double click is not used: on a file tab it already keeps the preview (11.1).

## 6. Where the pin is kept

| Option | Survives a restart | Another Mac / a reinstall | Multiplayer (28.14) | Cost |
|---|---|---|---|---|
| **A. `localStorage`, beside the tab order (recommended)** | Chats and files yes; plain terminals no (they do not come back either, locally) | No | Each person's own, by construction: a pin protects my tab from my mistakes | A list of bar keys; no protocol |
| B. The service (a `pinned` flag per terminal, in `open-sessions.json` and the kept-terminal list) | Yes, and for a remote's kept terminals | Remote: yes; local: yes, from the service's data folder | A per-terminal flag is **shared**: bob's pin would stop alice closing, which is a permission, not a preference; per person it needs a person key in the service (29.13's sessions) and a new message pair (`pin_tab`, `pins`) | New messages, files, migration; files are not terminals, so they would still need A |
| C. The settings file (#47) | Yes | Yes, if the user copies it | Owner only | Settings would hold transient session ids |

**Recommendation: A.** #37 and #57 already put tabs and their order in the UI's `localStorage`; a pin is the same kind of fact. 28.14's design settles the rest: terminal ownership between people is the **driver** (D29.3, enforced by the service), and a guest's tab of the owner's chat is a *watch* (29.14), so a pin needs no shared meaning. Losing pins on a reinstall or another Mac is the same loss as the tab order, which nobody has missed.

- **Storage:** `hive.pinnedTabs`, a JSON list of bar keys, capped like the tab order (500), read with the same fallback (`safeStorage`, try/catch: empty, unreadable or throwing storage means no pins, never an error). A remote host's pins live with that host's data where 28.27 keeps a host's places (`hive.hostPlaces`), so two hosts' same paths never mix.
- **Keys:** the bar's own (`barKey`). When a Claude session starts in a pinned `tab:<channel>`, the pin moves to `session:<id>` as the order's key does (one rename in the same place). `tab:` keys are kept **for this run only**, as `savedTabOrder` does: a channel number is reused by the next run, and a stale pin would pin a stranger.
- **Pruned** when the tab closes (after unpinning), when the worktree is deleted, and at load for keys whose session or file no longer exists once the snapshot has arrived (as the order is).

## 7. What survives what

| Event | Chat (`session:<id>`) | Plain terminal (`tab:<channel>`) | File |
|---|---|---|---|
| Switch worktree, space or host and back | Pinned | Pinned | Pinned |
| Reload of the window | Pinned (its session reattached) | Pinned only if the terminal is still there (remote kept, 26.16); else gone with it | Pinned |
| App quit and start, local service (#18) | Resumed (4.12) **and pinned** | Not restored, pin dropped | Reopened (28.27) and pinned |
| App quit and start, remote with "Keep running" (26.15) | Reattached and pinned | Reattached, **unpinned** (channel keys are per run) | Reopened and pinned |
| Worktree deleted | Closes, pin dropped, named in the dialog | same | same |
| `claude` exits in a pinned chat | Stays pinned (the key falls back to `tab:<channel>` for the rest of the run) | — | — |

The remote plain-terminal row is the one gap of option A; Q6 asks whether it matters.

## 8. Decisions that move

- **Changes #57** (tabs and order are UI state): add "an open tab can be pinned: a pinned tab has no ×, refuses ⌘W, the palette's close and its menu's Close (a toast says to unpin), keeps its place, and is remembered in `localStorage` with the order (a chat by its session, a file by its path)". The order rule itself does not move (recommended "in place"); with Q4's alternative, #57's order rule changes too.
- **#35:** no change with the recommended "no default shortcut" (the keymap of 28.7 lists `pin-tab` like any command). A default would add to #35's list.
- **#18:** no change. Its app-quit confirmation stays; pinning a tab adds no question at quit.
- **11.1** (the preview tab): its "pinned on purpose" becomes "kept" in the code and the ledger, so "pin" has one meaning; the behaviour is unchanged.

The human updates `docs/hive.md`; agents never edit it.

## 9. Questions for the human

Each is phrased for the questionnaire, with the recommended answer first.

| # | Question | Recommended | Alternatives |
|---|---|---|---|
| Q1 | Which tabs can be pinned? | Every tab: chats, plain terminals and files. | Chats only; chats and terminals. |
| Q2 | What happens when you try to close a pinned tab (⌘W, palette, menu)? | Nothing closes: a toast says "<name> is pinned. Unpin it to close it."; the menu's Close is disabled. | Ask "Close the pinned tab?"; close as today (the pin is only a mark). |
| Q3 | The pinned tab's × button? | Replaced by a pin icon that does nothing on click. | A pin icon that unpins on click (VS Code); the × kept, asking first. |
| Q4 | Where does a pinned tab sit in the bar? | Where it is, full width (#57 unchanged). | Moved to the start of the bar (changes #57's order); narrow, icon only. |
| Q5 | How to pin and unpin? | The tab's menu and a palette command, with **no default shortcut** (bindable in Settings → Keyboard). | Also a default shortcut (adds to #35). |
| Q6 | Where is the pin kept? | The app's `localStorage`, beside the tab order: chats and files keep their pin across restarts; plain terminals only for the run (also on a remote that kept them). | The service, per person, so a remote's kept plain terminals stay pinned and pins follow you to another Mac (new messages, 29.13's sessions first). |
| Q7 | Add "Close other tabs" / "Close all tabs" (skipping pinned ones) now? | Not now: none exists, and pinning does not need it. | Add them in 29.2 to the tab menu and the palette. |

**Defaults taken in this design (to the ledger as `pending` when 29.2 builds them):** the toast's wording; the pin icon's size and colour (12 px, `--text-3`); a dirty file shows its dot instead of the pin; the Delete worktree dialog lists pinned tabs; pinning a preview file keeps it; 11.1's `onPin`/`pinFile` renamed `onKeep`/`keepFile`.

## 10. The plan for 29.2 (after approval)

**29.2 Pinned tabs** (5 pts, `task/29.2-pinned-tabs`, frontend only: no service or protocol change, so no `security-reviewer`).

- `src/tabs.ts`: `pinnedTabs` helpers (`isPinned(s, item)`, `togglePin(s, item)`), the key rename when a session starts in a pinned terminal (with the order's), pruning with `removeTab` / file close.
- `src/store.ts`: `pinnedTabs: string[]` (and per host in the host data, as 28.27's places); `removeTab` and `closeFile` unchanged, the **guards are in the user-facing paths** (×, menu, `closeActiveTab`, palette) so the worktree-deleted and terminal-exited paths still clean up.
- `src/persist.ts`: `hive.pinnedTabs` (`savedPins` / `savePins`), 500 keys, `tab:` keys dropped at load, the `safeStorage` fallback; remote hosts in `hive.hostPlaces`.
- `src/keymap.ts`: `pin-tab` (no default on either platform); `src/shortcuts.ts`: `closeActiveTab` refuses a pinned tab with the toast (10.3).
- `src/shell/TerminalArea.tsx`: `TabItem` gets `pinned` (pin icon in the ×'s slot, not a button; "Pinned" in the tooltip and the accessible description); "Pin tab" / "Unpin tab" in the terminal, file and file-pane menus, "Close …" disabled with "Unpin to close" when pinned; 11.1's `onPin`/`pinFile` → `onKeep`/`keepFile`.
- `src/shell/WorktreeMenu.tsx`: the Delete worktree dialog lists the worktree's pinned tabs.
- `src/styles.css`: `.tab-pin` in the ×'s slot, both themes.
- **Tests (bun, 100 % lines):** pin and unpin from the menu and the palette; every close path refused for a pinned tab (×, menu, ⌘W, palette), and allowed after unpinning; the dirty file's dot over the pin; the order unchanged by pinning; the key moving from `tab:` to `session:`; a pinned preview file kept; persistence: saved and loaded, `tab:` keys dropped at load, empty, unreadable and throwing storage give no pins; worktree deleted prunes pins and the dialog names them; per host.
- **Playwright on the mock transport** (Chromium and WebKit, One Dark and One Light): pin a chat, try ×, menu, ⌘W and the palette (still open, toast shown), reload: the chat is still pinned and in its place; unpin and close; screenshots of a pinned tab in both themes; an axe check of the tab bar with a pinned tab.
- **Docs:** the README where it describes tabs; the Defaults ledger rows of §9.
- **29.3** stays the human's hand check in the real app.
