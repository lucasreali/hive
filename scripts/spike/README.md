# Hooks spike (TODO 0.12)

These scripts run real Claude Code sessions in a scratch repository under `/tmp/hive-spike`.
The model is `haiku` and `--strict-mcp-config` is on. Hooks come from `--settings`, so `~/.claude/settings.json` is never touched.
Every hook call is recorded with `hive hook --record` in `/tmp/hive-spike/records/*.jsonl`.
Terminal output is recorded with timing in `/tmp/hive-spike/logs/`.

```sh
scripts/spike/setup.sh            # build hive, create /tmp/hive-spike (wipes a previous run)
```

## Session A: `scripts/spike/run.sh observe`

Type each prompt and wait for it to finish unless a step says otherwise.

1. **Q2, spinner during a long tool.** Type `Run the shell command "sleep 45" and then say done.` and approve the command if asked.
2. **Q5, Esc on text.** Type `Write the numbers from 1 to 300, one per line.` Press **Esc** while it is writing, then wait 5 seconds.
3. **Q5, Esc on a tool.** Type `Run the shell command "sleep 60".` When the command is running, press **Esc** and wait 5 seconds.
4. **Q4, external edit.** First type `Read notes.txt and show it.`
   - Then, in another terminal: `echo "changed outside" >> /tmp/hive-spike/repo/notes.txt`
   - Back in the session, type `Using the Edit tool, replace "first line" with "edited by claude" in notes.txt. Do not read the file again first.`
5. Type `/exit`.

## Session B: `scripts/spike/run.sh worktree-hooks`

In this session Hive's worktree hooks are registered: WorktreeCreate and WorktreeRemove run `hive worktree hook-create` and `hook-remove`, and are recorded.

6. **Q1, Q3 and Q6.** Type `Use the isolated agent to create a file hello.txt containing "hi". Do not commit.`
7. Type `/exit`. If Claude asks about keeping or removing a worktree, choose **remove** and note what it said.
8. In a normal terminal, run:
   ```sh
   git -C /tmp/hive-spike/repo worktree list
   git -C /tmp/hive-spike/repo branch -a
   ```

## Session C: `scripts/spike/run.sh observe` (Claude's default worktrees, no Hive hook)

9. **Q6 without the hook.** Type the same prompt as step 6, then `/exit` (choose **remove** if asked).
10. Run the two `git` commands from step 8 again.

## Hand-off

Tell the agent the runs are done and paste anything Claude printed that looked like an error. The agent reads `/tmp/hive-spike/records`, `/tmp/hive-spike/logs` and the repository state, then writes `docs/spike/stage-0.md`.

To look at the pauses in terminal output (Q2): `scripts/spike/gaps.sh /tmp/hive-spike/logs/observe-*.timing`
