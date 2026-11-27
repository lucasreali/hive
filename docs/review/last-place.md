# Where Hive loses the last choice (28.19 audit)

The human, 2026-11-27: "other places don't keep this state either". Every place below loses
the user's last choice on a **switch** (another worktree, space or host), a **close** (a tab,
a view, a popover) or a **restart** (quitting Hive, or a reload of the page). All of it is UI
state (#37, #57): it would be kept in `localStorage` like the tab order, or in memory for this
run only.

## Built in 28.19

| Place | Switch | Close | Restart |
|---|---|---|---|
| The tab shown, per worktree (a terminal, a chat or a file) | kept | closing the shown tab shows the tab of that worktree used before it (history of 20); the neighbour only without history | kept for files and chats (by session); a plain terminal's channel is one run's |
| The split layout's focused pane (6.11) | kept (the pair shows with the same focus) | closing a pane shows the other pane, as before | splits are not kept (see below) |
| The right panel's view, per worktree | kept | – | kept |
| Files and Diff trees: scroll and active row, per worktree | kept | kept when the view shows again | this run only |
| Comments and Sessions views: scroll, per worktree | kept | kept when the view shows again | this run only |
| The sidebar's selection, per space and host (11.5) | kept (already) | – | this run only (see below) |

## Not built: the human picks

| # | Place | What is lost today | When | Proposal |
|---|---|---|---|---|
| 1 | Sidebar selection at launch | nothing is selected after a restart (every tab shows) | restart | select the place the current space was left on (`spacePlaces` kept in `localStorage`) |
| 2 | Space places (11.5) | another space shows its first project after a restart | restart | keep `spacePlaces` between runs, with item 1 |
| 3 | The shown tab after a crash reload or a resume | the last reattached or resumed terminal shows, not the one shown before | restart | reattached tabs do not take the focus; the worktree's history picks |
| 4 | Split pairs (6.11, 20.5, 25.1) | every pair ends | restart | keep pairs of files and chats by their bar keys |
| 5 | Open file tabs | every file tab closes (only their order is kept) | restart | reopen them, without their unsaved edits (or ask) |
| 6 | Diff base, per worktree (`diffBases`) | back to the default base | restart | keep it per worktree; a turn or commit base no longer there falls back |
| 7 | Diff file shown | nothing records which file the Diff showed apart from the file tab | restart | covered by item 5 |
| 8 | "Unseen only" (27.1) | off again | restart | keep it (one switch, not per worktree) |
| 9 | Folders opened in the Files and Diff trees | all collapsed again | restart | keep the open folders per worktree, capped |
| 10 | Sidebar's collapsed projects and worktrees | expanded again | restart | keep them with item 9 |
| 11 | Files view search ("Find files", Names/Contents) | the query and mode go when the view closes; the query stays when the worktree changes (shown for the wrong worktree) | switch, close, restart | per worktree, this run only |
| 12 | Sessions view search | the query goes when the view closes | close, restart | per worktree, this run only |
| 13 | Actions view branch filter | back to the worktree's branch when the view shows again | switch, close | per worktree, this run only |
| 14 | Pull requests / Actions details (`openPull`, `openRun`) | the open pull request or run is per project, not per worktree; lost on restart | restart | leave as is |
| 15 | Settings section | opens on Terminal every time (or Projects/Hosts when asked) | close, restart | reopen the last section, this run only |
| 16 | Space switcher's expanded hosts (27.8) and its filter | collapsed and empty each time it opens | close | keep the expanded hosts, this run only |
| 17 | Right panel open or closed (Ctrl+Shift+B) and sidebar shown or hidden (17.13) | open and shown again | restart | keep both with the widths (`hive.widths`) |
| 18 | Palette query and selection (6.3) | empty each time | close | leave as is (VS Code and Zed start empty) |
| 19 | Terminal search bar (17.21) | closed when its terminal stops showing | switch | leave as is |
| 20 | Bell inbox (6.5) | empty | restart | leave as is (alerts are about this run) |
