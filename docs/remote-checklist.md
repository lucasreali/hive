# Remote hosts: manual checklist (26.21)

Run on the Mac with an installed Hive release (from Applications) that carries Stage 26, against a real Linux host of your own (x86_64 or aarch64). Tick each line when the expected result happens; otherwise note what happened, the host chip's text, and the end of `~/Library/Logs/Hive/app.log` on the Mac and of the host's `daemon.log` (`$XDG_RUNTIME_DIR/hive/daemon.log`, else `/tmp/hive-<uid>/daemon.log`).

The CI job (26.20) already checks the install, echo latency, a dropped link, unknown and changed host keys and a faked `Darwin` host in a container; this list covers what only a real Mac and a real network show.

**Before:**

- A Linux host with `git`, `sha256sum` and Claude Code installed, and `claude` logged in there (run it once over ssh).
- An alias for it in `~/.ssh/config` (called `box` below), its key in your ssh agent (`ssh-add --apple-use-keychain ~/.ssh/<key>`), and no `ForwardAgent yes` for it.
- `ssh box` run once in Terminal, its host key checked and accepted.
- No `~/.hive` on the host yet, so the first install is seen.
- For the optional steps: a bastion host to jump through, a FIDO2 key (`ed25519-sk` or `ecdsa-sk`), a Mac with a macOS user you can ssh into.

## Add the host and install

- [ ] **Settings → Hosts** is there (macOS only). Type `box`, click **Add host**. *Expected:* the host's row and a status-bar chip go connecting → installing → connected within a few seconds (CI: 5.2 s from the first `ssh` to connected, on a 100 ms link).
- [ ] On the host: `ls -la ~/.hive/bin` shows `hive` (mode `-rwxr-xr-x`), no `hive.new`, the folder mode `drwx------`; `~/.hive/bin/hive --version` prints the app's version. *Expected:* exactly that.
- [ ] Type a bad alias (`-oProxyCommand=x` or `a b`). *Expected:* refused with a message saying what an alias may hold; nothing is linked.
- [ ] The space switcher lists the host's spaces as "<Space> · box"; **New space on box…** makes one there; add a project on the host (its folder browser lists the host's folders). *Expected:* the project's worktrees show under the remote space, with health.
- [ ] Open a terminal there. *Expected:* your `$SHELL` on the host as a login shell, in the worktree; `which claude` is `~/.hive/bin/claude`; typing feels like `ssh box` in Terminal.
- [ ] Run `claude` in it and give it a prompt that asks for a permission. *Expected:* the agent shows under its worktree, working then waiting for permission; the bell counts it and F8 goes to it (switching to the remote space); the notification names the space and the host ("<Space> · box").
- [ ] In the remote space, right-click a file and a worktree, and open the Sessions view's menu. *Expected:* no Open with Default App, Reveal in Finder, external editor, Open Log, Reveal Log or Open Working Directory; the shared folder's Open folder and the dev-server preview are hidden too.
- [ ] Drop a file from Finder on the remote terminal. *Expected:* nothing is typed; a toast says files from this Mac cannot be dropped into a terminal on box.
- [ ] **Test** in Settings → Hosts while connected. *Expected:* nothing changes (the link is up).

## Agents across a lid close and a Wi-Fi switch

- [ ] Start a long agent turn on the host (or `while true; do date; sleep 1; done` in a second terminal), then **close the lid** for at least 2 minutes and open it. *Expected:* right after wake the chip says connecting or "reconnecting since …", the host's terminals show "Reconnecting to box…" and take no keys, and within about 30 s the link is connected again with no reinstall; the `date` terminal shows every second written meanwhile (the missed output, up to its last 1 MiB); Claude Code redraws its screen; the agent's state is current.
- [ ] After that reattach, if agents started waiting while the lid was closed: *Expected:* one summary notification ("2 agents waiting on box"), not one per agent.
- [ ] **Switch Wi-Fi networks** (or turn Wi-Fi off for 20 s and on) during a turn. *Expected:* the chip shows "reconnecting since …" once ssh notices (about 30 s at most, from `ServerAliveInterval=10` × 3), then connects again after the next step of the 1, 2, 5, 10, 30 s ladder; the terminals reattach as above; nothing on the host was stopped (`ps -u $USER` there shows the same `claude`).
- [ ] With the link down, click **Reconnect** on the chip. *Expected:* it tries at once.
- [ ] Quit Hive with an agent working on the host. *Expected:* the dialog asks "Keep 1 agent running on box?", ticked. Keep it ticked and quit; on the host the agent's `claude` still runs. Open Hive again: *Expected:* the host connects and the kept terminal comes back as a tab under its worktree, with its last output.
- [ ] Quit again with the box unticked. *Expected:* on the host, the agent and its terminal end within a few seconds.

## `ProxyJump`

- [ ] Give the host a second alias (`box-jump`) with `ProxyJump <bastion>` in `~/.ssh/config`, keys for both in the agent, both host keys accepted (`ssh box-jump` once). Add `box-jump` in Settings → Hosts (remove `box` first: two aliases of one host reach the same service, and each link would take the other over). *Expected:* it connects and works as `box` did; a terminal there echoes normally.
- [ ] Close the lid for a minute with `box-jump` linked. *Expected:* it reconnects through the bastion after wake, as above.

## A FIDO2 key (if you have one)

Open question 1 of `docs/spike/ssh.md`: does the touch prompt work with `BatchMode=yes` and no tty?

- [ ] Use an alias whose only key is a FIDO2 one (`IdentityFile ~/.ssh/id_ed25519_sk`), the key plugged in. Add it. *Expected, either:* the key blinks, a touch connects it; *or* the chip says "disconnected" with the password-or-passphrase message (`auth`), not retried. Write down which one happened, and whether `ssh-add` of the `-sk` key changes it.
- [ ] If it connected: drop the link (Wi-Fi off and on). *Expected:* each reconnect asks for a touch again; note whether that is usable.

## An app update while terminals are kept (Q8)

Needs a newer Hive release than the installed one (or two local release builds of different versions).

- [ ] With an agent running on the host, click **Update to vX** and let Hive restart. The update's dialog has no per-host question: the restart sends no goodbye, so the host keeps its agents. *Expected:* after the restart the chip shows installing, then "disconnected: version <old>, this app <new>" with **Restart the remote service**, its tooltip naming the terminals it ends; the agent on the host still runs; `~/.hive/bin/hive --version` there is already the new version.
- [ ] Wait a minute without choosing. *Expected:* nothing retries; the host stays disconnected; the agent still runs there.
- [ ] Click **Restart the remote service**. *Expected:* the old service ends (its terminals with it), a new one of the app's version starts, the chip says connected, and the host's spaces and projects are back.

## A macOS host refused

- [ ] Enable Remote Login on another Mac (or this one: System Settings → General → Sharing → Remote Login), give it an alias (`mac`), accept its host key, put the key in the agent. Add `mac`. *Expected:* the chip says "disconnected" with "mac runs macOS; Hive connects only to Linux hosts.", it is not retried, and on that Mac `~/.hive/bin/hive` was not created or changed (`ls -la ~/.hive/bin`).
- [ ] Remove `mac` with its ×. *Expected:* its chip goes away.

## Afterwards

- [ ] Remove the test hosts from Settings → Hosts, and if wanted delete `~/.hive` on the host (after ending its service: `pkill -f 'hive daemon'` there).
- [ ] Note the results (and the FIDO2 answer) in `TODO.md`'s 26.21 entry.
