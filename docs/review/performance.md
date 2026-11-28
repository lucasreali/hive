# Review 29.8: performance baselines

Task 29.8 (`TODO.md`). Measured only: nothing was optimised. Later tasks compare their numbers with these, using the same commands.

- **Commit measured:** `ea619251` (`main`, Hive 0.33.0), plus the two measuring tools this task adds (`crates/hive/benches/service_perf.rs`, `e2e/baseline.perf.ts`). The service numbers come from the release profile (`cargo bench`: `lto`, one codegen unit).
- **Machine:** Apple M5 (10 cores), 24 GB, macOS 27.2.1 (26B131). fish 4.9.3, git 2.54.0 (Apple Git-157), Playwright 1.65.0 (its own Chromium and WebKit builds, headless).
- **Date:** 2026-11-28. **The machine was not quiet:** other agents worked in parallel (load average 1.7–7 during the runs, the higher figures during the service runs). Timings below a millisecond and the worst cases are the most affected. Re-run on a quiet machine before comparing small differences.
- **Environment:** every measurement runs in a throwaway `HOME`, `XDG_*` and `HIVE_HOME` (a `tempfile` folder) or against the browser's mock transport. No process named `claude` is started (the hook bench sends hook events directly; the fake `claude` of `crates/hive/tests/fixtures/fake_claude.c` was not needed), and `~/.claude` is never read. The one reading of the human's own session (section 7) is `scripts/energy.sh`, read only.

## 1. Commands

| What | Command |
|---|---|
| Service: start, idle, terminals, streaming, flood, echo, refresh | `cargo bench -p hive --bench service_perf` (all, about 5 min) or `-- <scenario> [args]`, scenarios in the file's header |
| Hook → service handled | `cargo bench -p hive --bench hook_latency` (1.x, unchanged) |
| Frontend: output → painted, agent state → shown, cold load, memory, CPU profile | `E2E_PORT=<port> bunx playwright test -c e2e/perf.config.ts` from the repository's root (about 3 min; it always makes a fresh production build); JSON in `target/perf/`, the CPU profile in `target/perf/replay-20.cpuprofile` |
| Load test (1.11) | `E2E_PORT=<port> bunx playwright test -c e2e --project load --no-deps` |
| The installed app | `scripts/energy.sh <seconds>` (29.10) |
| Service profile | build with symbols: `CARGO_PROFILE_BENCH_STRIP=false CARGO_PROFILE_BENCH_DEBUG=line-tables-only cargo build -p hive --profile bench --benches --target-dir target/perf-symbols`, start a scenario from `target/perf-symbols/release/deps/service_perf-*` (it prints `service pid <n>` on stderr once the service is up), then `sample <n> 8 -file <out>` (macOS' `sample`, a host tool) |

`perf.config.ts` matches `*.perf.ts` only, so `bun run e2e` and CI never run these: they measure, they do not check. Neither `samply` nor `cargo flamegraph` is installed on the host (nothing was installed); `sample` and Chromium's profiler (the Performance panel's, through the DevTools protocol) were used.

## 2. The service (`hive daemon`)

CPU is the service process's own user + system time over the wall time (100% = one core); the terminals' shells and programs are not counted. RSS from `ps`; footprint from `footprint` (what Activity Monitor shows). One run of 30 s per row (10 s for flood) after 3 s of warm-up; every byte of output acknowledged at once, as an app that keeps up does.

| Scenario | CPU | RSS | Footprint | Output |
|---|---|---|---|---|
| Idle, app connected, no terminal | 0.0% | 8.5 MB | 2.6 MB | — |
| 10 shells at their prompt | 0.0% | 10.3 MB | 4.3 MB | — |
| 10 terminals streaming (~26 KB/s each, an agent's rate) | 1.0% | 10.4 MB | 4.4 MB | 263 KiB/s |
| 30 terminals streaming | 2.5% | 10.9 MB | 4.8 MB | 815 KiB/s |
| 1 terminal flooding (flow control is the only limit) | 43% | 10.1 MB | 4.4 MB | 35 MiB/s |
| 10 terminals flooding | 186% | 16.1 MB | 10.2 MB | 137 MiB/s |

The streaming program is `while :; do cat chunk.txt; sleep 0.04; done` (12 truecolor lines, 1 320 bytes); the flood one `cat`s 64 of them (84 480 bytes) without pause. Memory stays flat: each terminal adds about 0.15 MB, and the flow control (`HIGH_WATER` 512 KiB) bounds the flood.

**Start.** `hive bridge` started in a fresh folder until `welcome` (the bridge, the service it starts, its first-run setup, the handshake): **p50 26.5 ms, max 26.9 ms** (10 runs).

**Key → echo through the service** (a key sent to a terminal running `cat`, until its echo is back on the socket: socket → service → PTY → the kernel's echo → service → socket), 300 keys:

| Other terminals streaming | p50 | p95 | p99 | max |
|---|---|---|---|---|
| 0 | 0.09 ms | 0.15 ms | 0.21 ms | 1.8 ms |
| 10 | 0.06 ms | 0.12 ms | 0.17 ms | 0.3 ms |
| 30 | 0.10 ms | 0.37 ms | 0.98 ms | 1.9 ms |

This is an upper bound of the service's share of "bytes written in the PTY → painted": a program's output takes only the second half of the trip. Every 200 keys the line is ended and the bench waits for both its echo and `cat`'s copy before the next key, so no late echo is counted for a later key.

**Hook → service handled** (`hive hook SessionStart` started until the app connection has its `agent_detected`), 300 runs: **p50 2.8 ms, p99 4.5 ms, max 5.2 ms** (target p99 < 20 ms). Most of it is starting the `hive hook` process.

**Refresh of a repository with 20 worktrees** (`list_projects` until `projects`, which carries every worktree's fresh health; 500 files, every linked worktree one commit ahead, every third one with a modified and an untracked file):

| Worktrees (with the main one) | p50 | p95 | max |
|---|---|---|---|
| 1 | 14 ms | 21 ms | 21 ms |
| 6 | 108 ms | 133 ms | 133 ms |
| 21 | 405–449 ms | 524–547 ms | 551–560 ms (two runs) |

Linear: about 19 ms per worktree (hot spot 1).

## 3. The frontend against the mock transport

Headless Playwright builds; WebKit is the engine of WKWebView (close to the app, not identical), Chromium runs WebGL on Metal as the load test does. WebKit's clock is rounded to 1 ms. Headless WebKit does not hold `requestAnimationFrame` to the display's 60 Hz as Chromium does, so its "painted" figures are lower than a real window's. "Painted" is the next `requestAnimationFrame` callback, the frame the change is drawn in: a lower bound of the moment it reaches the screen (compositing and the display's refresh come after).

**Output → painted** (bytes reach the terminal manager, the mock's echo of `transport.writeTerminal`, delivered as the Tauri transport delivers a PTY frame, until the frame after xterm.js parsed them; 100 keys, 10 a second):

| Browser | Other terminals replaying an agent (~23 KB/s each; with 10, the measured terminal replays too: 11 stream) | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| Chromium | 0 | 1.2 ms | 9.7 ms | 17.7 ms | 17.7 ms |
| Chromium | 10 | 11.3 ms | 16.6 ms | 17.4 ms | 17.4 ms |
| WebKit | 0 | 2 ms | 2 ms | 3 ms | 3 ms |
| WebKit | 10 | 6 ms | 17 ms | 29 ms | 29 ms |

The load test (1.11, 20 terminals, key press → painted) is the end-to-end view of the same path; its last recorded result is in `docs/architecture.md` (Load test).

**Agent state → shown** (an `agent_state` applied as the transport applies a service message, until the agent's row shows the new state, then the next frame), 200 changes:

| Browser | DOM committed p50 / p95 / max | Painted p50 / p95 / max |
|---|---|---|
| Chromium | 1.8 / 3.3 / 4.0 ms | 16.5 / 16.9 / 190 ms |
| WebKit | 2 / 3 / 4 ms | 2 / 5 / 6 ms |

So hook → state on screen is about 2.8 ms (service, p50) + the Tauri event (not measured, section 6) + 2 ms (React) + up to one frame.

**Cold load of the production bundle** (`vite build` served by `vite preview`, `/?mock`, a fresh browser context each time, 5 measured runs after one warm-up):

| Browser | First paint p50 | DOMContentLoaded p50 | Project rows shown p50 / max |
|---|---|---|---|
| Chromium | 12 ms | 74 ms | 95 / 97 ms |
| WebKit | 108 ms | 77 ms | 108 / 130 ms |

The bundle: 133 JS files, 3.9 MB in all; the startup chunk `index-*.js` is 1.7 MB and `utils-*.js` 539 KB (Vite warns about chunks over 500 kB). In the app the files come from the app itself, not over HTTP, so these figures are the webview's share of a cold start only.

**Memory with 30 terminals** (Chromium, after a forced GC; each terminal received a 449 KB agent replay): JS heap **20.1 → 30.6 MiB**, DOM nodes **260 → 36 173**, event listeners 216 → 1 427. xterm.js's buffers are typed arrays whose memory sits outside the JS heap figure, and WebGL's textures outside the page's; the real figure is WebContent's footprint (section 7).

**Main thread with 20 terminals replaying** (Chromium's CPU profiler, 10 s, sampling every 0.2 ms): the main thread was busy **5.3%** of the time. Of that busy time:

| Share | Function |
|---|---|
| 29.9% | (program): the browser's own work outside JavaScript (style, layout, paint) |
| 17.2% | xterm.js `WidthCache._measure` |
| 8.9% | (garbage collector) |
| 7.4% | xterm.js DOM renderer `createRow` |
| 4.5% | `replaceChildren` (DOM) |
| 3.1% | WebGL addon `_updateModel` |
| 2.9% | xterm.js `getTrimmedLength` |
| 2.5% | Hive's `ligatureRanges` (`src/terminals.ts`) |
| 1.7% each | xterm.js `_processSGR0`, `createElement` (DOM) |

## 4. Profiles: what the service spends its time on

`sample` of the service while 10 terminals flood (8 s, symbols build). Most samples are idle worker threads (`__psynch_cvwait`). The busy ones are system calls: the PTY reads (`read`, 1 563 samples) and the socket writes (`writev`, 714), tokio's I/O polling (`kevent`, 1 282), and waits on tokio's internal mutexes (`__psynch_mutexwait`, 984, with `pthread_mutex_lock`/`unlock` and `Notify::notify_waiters`, 129). Hive's own code comes next: the PTY copy loop with its OSC scanner (`daemon::terminals::copy`, 229), the framed writer's flush (44), `write_prioritized` (21), and decoding every `ack` as JSON (`Frame::to_control` with serde's buffered `Content` deserializer for the tagged `Control`, about 20). Nothing in Hive's code dominates: at an agent's rate 30 terminals cost 2.5% of a core.

## 5. Hot spots worth a task

1. **A refresh lists each worktree's health one after another** (`health::fill`: about four `git` runs per worktree, `log`, `status`, `diff --numstat`, `rev-list`): ~19 ms per worktree, 0.4–0.45 s for 20, every "Refresh worktrees", every `projects` after a worktree hook and every message carrying projects. Running the worktrees' statuses in parallel (bounded, e.g. by the cores) would bring 20 worktrees near the time of one or two.
2. **Hidden terminals still render to the DOM** (found in 1.11, not fixed): a hidden terminal loses its WebGL addon and xterm.js's DOM renderer rebuilds rows and measures glyph widths that are never cached in a `display: none` pane: `_measure` + `createRow` + `replaceChildren` + `createElement` + `setAttribute` are about a third of the main thread's busy time with 20 terminals, and 30 terminals leave 36 000 DOM nodes. Skipping that work for hidden terminals (1.11's experiment removed it entirely) frees the main thread and memory.
3. **WebContent's memory** in the human's real session: 473 MB, peak 1.09 GB, after 51 minutes (section 7); 29.10 saw 546 MB / 905 MB. Measure what holds it (29.11's scrollback measurement, and the one-hour run below) before choosing a fix.
4. **The startup chunk is 1.7 MB** (plus 539 KB `utils`): a cold load is still about 0.1 s in a browser, so this is cheap to leave; worth a look only if the app's cold start (section 6) proves slow.
5. Minor: `ligatureRanges` is 2.5% of the busy main thread; every `ack` is a JSON control frame decoded through serde's buffered tagged-enum path. Neither is worth a task alone.

## 6. Pending: measurements that need the human's app

The built app's webview has no Web Inspector in a release build, and these need a real window and session, so they are the human's to run. Results go into this file.

**a. App cold start to first frame** (Hive 0.33.0 installed):
1. Quit Hive (⌘Q) and wait 10 s, so its service ends too.
2. Start a QuickTime Player screen recording (File → New Screen Recording, the whole screen; it records at 60 fps).
3. Click Hive in the Dock. Wait until the sidebar shows its projects. Stop the recording.
4. In QuickTime, step frame by frame (← →): the frame where the Dock icon first changes (the click) and the first frame where the window shows the sidebar's project rows. The difference ÷ 60 is the cold start. Do it three times; note the median.
5. The parts measured above: the service starts in ~27 ms, and the bundle loads to its project rows in ~0.1 s in a browser.

**b. The app's memory after an hour with many tabs:**
1. Quit and start Hive again (`energy.sh` misses a WebContent relaunched after a crash).
2. `scripts/energy.sh 30`: note the footprint and peak of WebContent, hive-app, WebKit.GPU and the daemon.
3. Open 10 terminals, run agents in 3 of them, open 10 file tabs (a diff and a Markdown preview among them); use Hive as usual for an hour, the window in front most of the time.
4. `scripts/energy.sh 30` at 15, 30 and 60 minutes, the same numbers each time, and the number of open tabs.
5. Leave it 10 minutes untouched and minimised, then `scripts/energy.sh 30` once more (does memory come back down?).

**c. The Tauri leg of output → painted** (the service's ~0.1 ms and the webview's 1–11 ms p50 are measured; the event between them is not): in a dev build (`scripts/mac-dev.sh`), open Web Inspector (right click → Inspect Element), Timelines tab, record 10 s while typing in a terminal running `cat`, and note, for a few keys, the time from the `keydown` event to the next "Composite" or "Paint" record.

## 7. The human's running session (read only)

`scripts/energy.sh 30` on the installed Hive 0.33.0 (up 51 minutes, 9 processes under the daemon, window state unknown, while agents worked), 2026-11-28 11:49:

| Process | CPU avg / max | Energy | Wakeups/s | Footprint | Peak | fds |
|---|---|---|---|---|---|---|
| hive-app | 0.1% / 0.4% | 0.1 | 2.1 | 74 MB | 83 MB | 25 |
| WebKit.GPU | 0.2% / 1.1% | 0.5 | 6.9 | 107 MB | 214 MB | 8 |
| WebKit.WebContent | 0.2% / 1.9% | 0.4 | 3.5 | 473 MB | 1 092 MB | 3 |
| WebKit.Networking | 0.0% | 0.0 | 0.0 | 6 MB | 6 MB | 12 |
| daemon | 0.1% / 0.8% | 0.3 | 5.5 | 15 MB | 19 MB | 157 |
| bridge | 0.0% | 0.0 | 0.0 | 3 MB | 3 MB | 12 |
| **total** | 0.5% / 3.5% | 1.3 | 18.0 | 677 MB | — | 217 |
