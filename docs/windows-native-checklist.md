# Native Windows: manual checklist (12.5.7)

Run on a real Windows 10/11 machine with the installer from ci.yml's `hive-windows` artifact (or a release). Tick each line; note anything off, with the Diagnostics text (Settings → About) and `%LOCALAPPDATA%\hive\run\daemon.log`.

Before: Git for Windows (Git Bash) and Claude Code for Windows installed and logged in; `gh` installed and logged in for the PR/Actions checks. Ideally one machine (or user) without WSL and one with WSL and a distribution.

## Install and first run

- [ ] The installer runs (SmartScreen: More info → Run anyway) and Hive starts.
- [ ] **Without WSL**: no question; the status bar shows "Windows connected". Settings → Terminal has a Shell select and no Service select.
- [ ] **With WSL**: the first run asks "Where should Hive run?" (WSL focused). Choosing Windows connects ("Windows connected"); closing and reopening Hive does not ask again.
- [ ] `%LOCALAPPDATA%\hive\bin` holds `hive.exe` and `claude.exe`; Task Manager shows one `hive.exe` service while Hive runs, none a few seconds after it closes.

## Projects

- [ ] Add project (Ctrl+Shift+O): the field starts at `C:\Users\<you>\`, no WSL/Windows select. Typing `C:\` lists `C:\`'s folders; typing `C` lists the drives; a clicked folder is added with `\`; ↑ goes up; `C:\` has no ↑ row. Both `\` and `/` work while typing.
- [ ] Add a git repository on `C:\` (and a subfolder of one: its repository is added). Its worktrees show, with health (ahead/behind, changes).

## Terminals

- [ ] A terminal in the default shell opens in the worktree: `pwsh` when installed, else Windows PowerShell. Typing, colours, resize (drag the window), copy/paste (Ctrl+Shift+C/V) and scrollback work.
- [ ] Settings → Terminal → Shell "Command Prompt", then a new terminal: `cmd.exe` in the worktree.
- [ ] Shell "Git Bash", then a new terminal: bash in the worktree; `which claude` is Hive's `bin` folder first; your `~/.bashrc` / `~/.bash_profile` ran.
- [ ] Closing a tab ends its shell and whatever it started (Task Manager).

## Claude

- [ ] In a terminal, run the real `claude`: the agent appears under its worktree, idle, then working on a prompt, waiting for you at the end of the turn; a permission prompt turns it yellow and rings; the bell counts it, F8 jumps to it.
- [ ] A subagent (Task tool) shows as a line under its agent.
- [ ] The status bar shows the session usage ("Session N% · resets …") once Claude's statusline has run; your own statusline, if any, still prints in Claude.
- [ ] Settings → Accounts: add a second Claude config folder ("Log in…" logs it in), pick it in the status bar's account select: `claude` in a new terminal runs as that account (its sessions in that folder), and the session usage follows it.
- [ ] An npm-installed `claude` (if you have one) is detected too.
- [ ] Sessions tab: the sessions of the worktree list; Resume, Fork, Open log and Reveal folder work (Explorer opens).
- [ ] "Copy Resume Command", pasted into a new terminal of each shell (PowerShell, Command Prompt, Git Bash, switched in Settings → Terminal), resumes the session there.
- [ ] Add project with `\\localhost\c$\` (or any `\\host\share`) typed: refused ("network and device paths are not supported"), nothing listed.
- [ ] Closing Hive with sessions open, then reopening: they come back with `claude --resume`.

## Worktrees

- [ ] New worktree (Ctrl+Shift+N) from a branch: created under `.claude\worktrees\<name>`, `.worktreeinclude` files copied, a terminal with `claude` started.
- [ ] `claude -w <name>` in a Hive terminal: the worktree shows in the sidebar, and goes when Claude removes it.
- [ ] Remove a worktree with a terminal sitting in it (PowerShell `cd` into it, and Command Prompt): refused, naming the process. Close that terminal: removal works, the archive script (if set) runs in the chosen shell.
- [ ] Rename a worktree; open its folder (Explorer); copy its path (a `C:\` path).

## Files, diff, chats, GitHub

- [ ] Files panel lists the worktree and updates live while `claude` edits; ignored folders (`node_modules`, `target`) are not listed.
- [ ] Diff tab: changes against HEAD and against the branch base; the viewer shows a file's diff.
- [ ] Edit and save a file (Ctrl+S). A file open in another program without sharing (e.g. an open Excel file) refuses the save with "is open in another program". A change on disk refuses a stale save.
- [ ] New file, rename, move (drag) and delete in the Files tree.
- [ ] Open in editor opens the Windows editor on the file.
- [ ] PRs and Actions views list through `gh` with the space's account; a run's failed job log shows.

## Modes and updates

- [ ] **With WSL**: Settings → Terminal → Service → WSL asks first, ends every terminal, reconnects to the WSL service with its own projects; switching back to Windows shows the Windows projects again.
- [ ] Update the app while its service runs (install a newer build over it, or Update to vX): the install succeeds, Hive restarts and connects to the new service (no version mismatch); old `*.old` copies in `%LOCALAPPDATA%\hive\bin` are gone after the next start.
- [ ] If a version mismatch ever shows, the connection dialog's `taskkill /F /IM hive.exe` then Reconnect fixes it.
