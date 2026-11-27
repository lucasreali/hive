# Spike 28.12: a Linux desktop app

Task 28.12 (`TODO.md`). No product code, no experiment on a Linux desktop: this is desk research against the official Tauri v2 docs, the `tauri-plugin-updater` source and the Hive code at `main` `89ec3828` (v0.32.0, Tauri 2.12.5, updater plugin 2.14.1). Written 2026-11-27. It also decides whether 28.13 (a Nix flake) goes ahead.

## 1. Recommendation in one paragraph

**Do not build a Linux desktop app now.** D17.0 put macOS first and froze new Windows-only work; a Linux app would be a third desktop platform with the same problem Windows has today (nobody on the team uses it daily), plus a hardware problem Windows does not have: the human's only machine is an Apple Silicon Mac, and a Linux VM on it has a virtual GPU, so the part most likely to break (WebKitGTK's GPU path under xterm.js's WebGL renderer, §3) cannot be checked by the people who would ship it. The good news is that most of the cost is already paid: the app **compiles and is tested on Linux in every `ci.yml` run** (the Linux runner links WebKitGTK to build `src-tauri`), the service runs on Linux (WSL, remote hosts), and Tauri's Linux tray, single-instance, window-state and updater all work there. What is missing is about a dozen macOS-only pieces of `src-tauri/src/macos.rs` and `crates/hive/src/macos.rs` that need a Linux version, **four of them security guards** (§6), plus packaging and a release job. If the human wants Linux later, §9 lists the work: about 9 tasks, roughly 45 points, and one new direct dependency (`webkit2gtk`, already in the lockfile through wry). **28.13 (the Nix flake) should not go ahead either** (§10).

## 2. What exists today

- **The app already builds on Linux.** `.github/actions/setup-rust` installs `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev` on `ubuntu-latest`, and `ci.yml`'s `rust` (`cargo clippy --workspace --all-targets`) and `coverage` (`cargo llvm-cov --workspace`, 100% lines) jobs build and test `src-tauri` there. `ci.yml`'s `remote` job runs `hive-app`'s remote links on Linux too (headless, no webview). So Linux `cfg` code in the app is already covered by mutants and coverage the way every other Linux code path is, with no new job.
- **Linux branches already in the app:** `fds.rs` (`RLIMIT_NOFILE = 7`, the descriptor raise at start, 17.6), the notification plugin's notifier ("Linux (development only)", no click, `main.rs`), the opener plugin for "Open in default app" (`open_file`, not macOS's `open -t` fallback), single instance (a D-Bus name, `lib.rs`).
- **The service on Linux** is complete: `/proc` for processes (`procs.rs`), inotify strategies (`files`, `registry`, `session_watch`), `renameat2` for the no-replace rename (`file.rs`), the login shell (`terminal::login`), the static musl builds for remote hosts (26.13). A Linux desktop app would run the same `hive` locally, like macOS does.
- **The frontend's keys** already have the non-Mac path (Ctrl, `e2e/host.ts`), used by Windows; and Playwright's WebKit runs the e2e specs on Linux in `ci.yml`'s `frontend` job. That is WebKit on Linux, but Playwright's own build, not the distribution's WebKitGTK the app would link.

## 3. Tauri on Linux: WebKitGTK and WebGL

Tauri v2 on Linux renders with **WebKitGTK 4.1** (GTK 3, libsoup 3). The build needs the packages above (official prerequisites: `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`); the user needs the runtime libraries, which a `.deb` declares as dependencies and an AppImage carries (§4). Distributions with WebKitGTK 4.1: Ubuntu 22.04+, Debian 12+, Fedora 37+, current Arch.

**Known issues, from Tauri's "Linux Graphics Issues" page** (the first is the one that matters to Hive):

1. **WebGL can be silently slow.** *"WebGL2 context creation succeeds even when the result is backed by a software rasterizer or a slow presentation path."* WebKitGTK masks the renderer string (`WEBGL_debug_renderer_info` says `Apple GPU` on every Linux machine), so the app cannot detect it. Symptoms named by the page: *"high input latency and low frame rates in WebGL-heavy views such as terminals"*. Tauri's advice: a non-WebGL fallback on Linux and a user setting to switch renderers. Hive uses `@xterm/addon-webgl` 0.19 with a DOM fallback **only on context loss** (`src/terminals.ts`), which never fires in this case. Ligatures (`docs/spike/terminal-font.md`) work only under the WebGL renderer, so a DOM fallback also loses them.
2. **NVIDIA and the DMABUF renderer:** blank or white window, flicker or crash on resize, `AcceleratedSurfaceDMABuf was unable to construct a complete framebuffer`, and on Wayland `Error 71 (Protocol error)`. Workarounds in order: `nvidia_drm.modeset=1`; `__NV_DISABLE_EXPLICIT_SYNC=1` (keeps acceleration); `WEBKIT_DISABLE_DMABUF_RENDERER=1` (loses the fast path); `WEBKIT_DISABLE_COMPOSITING_MODE=1` (last resort). The page warns to set an override in `main()` only once the app is shown to be affected.
3. **WebGL context loss** after the DMABUF refactor is an open WebKit regression (bugs.webkit.org 311335, "[GTK][WPE] WebGL context lost after dma-buf/EGLImage refactorization"); Hive's fallback handles it, minus ligatures.
4. **No GUI `PATH`.** A GUI app on Linux does not inherit the shell's `PATH` (Tauri's AppImage page). Hive already handles this for macOS: the service's terminals run a login shell (`terminal::login`), and the app finds `hive` by its bundled path, so this needs no new work, only the Linux branch of `bridge_command` (§6).
5. **Wayland:** a client cannot place its own window, so `tauri-plugin-window-state` restores size but not position there. Hive's window is undecorated (`"decorations": false`) and draws its own title bar; on Linux it would need Hive's own window buttons (as on Windows, `title_bar_action`) and a check that the undecorated window still resizes from its edges under GNOME/Wayland and KDE. Not verified here: there is no Linux desktop to try it on.

## 4. Packaging

| Format | What Tauri does | Fit for Hive |
|---|---|---|
| **AppImage** | One file carrying its libraries (70+ MB instead of 2–6 MB). Build on the oldest base you support that has WebKitGTK 4.1 (Tauri: Ubuntu 22.04 or Debian 12), or glibc errors follow. `linuxdeploy` cannot cross-build, so aarch64 needs an ARM runner (`ubuntu-22.04-arm` exists). Extra files only under `/usr/` (`bundle > linux > appimage > files`). | **Yes, the main one.** The updater replaces it in place without root (§5). The bundled `hive` goes in as a Tauri resource, as on macOS. |
| **.deb** | `tauri build -b deb`; declares WebKitGTK as a dependency, so it is small and uses the system's (newer, distribution-patched) WebKitGTK. | **Yes, second.** Ubuntu/Debian users expect it. Updating it needs root (§5). |
| **.rpm** | `-b rpm`, same as `.deb` for Fedora/openSUSE. | Optional; costs only the bundle step. |
| **Flatpak** | A manifest on the GNOME 51 runtime, built from the `.deb` or from source; published by a pull request to Flathub. Sandbox: `--socket=wayland --socket=fallback-x11 --device=dri --share=ipc`, plus `org.kde.StatusNotifierWatcher` for the tray. Updates through `flatpak update`, not the Tauri updater. | **No.** Hive's service spawns the user's login shell, `git`, `gh` and `claude` on PTYs, writes Claude Code's hooks into each account's settings, and watches arbitrary folders. Inside a sandbox every one of these would go through `flatpak-spawn --host` (a sandbox escape in all but name) and Flathub review would reject or require broad holes (`--filesystem=host`, `--talk-name=org.freedesktop.Flatpak`). Snap's classic confinement has the same problem and needs a manual approval. |

Hive already ships a glibc `hive` built on `ubuntu-22.04` for the Windows installer (`release.yml` `linux`), which is the right base for the AppImage too.

## 5. The updater

`tauri-plugin-updater` 2.x supports every Linux bundle: AppImage, `.deb` and `.rpm` (since 2.1 for `.deb`). From the plugin source (`src/updater.rs`):

- The platform key is tried as `{os}-{arch}-{installer}` first (e.g. `linux-x86_64-appimage`, `linux-x86_64-deb`), then `{os}-{arch}`; the installer is the bundle type Tauri stamps in the binary at bundle time. `latest.json` therefore needs one entry per Linux bundle and CPU, signed like the others; `release.yml`'s `publish` job builds it with `jq` and would gain `linux-x86_64-appimage`, `linux-x86_64-deb` (and the aarch64 ones), and `linux-x86_64` pointing at the AppImage.
- **AppImage:** the running file (`$APPIMAGE`) is moved aside to a temporary folder on the same device and the new bytes are written in its place, the old one restored on failure. No root. It fails when the AppImage sits in a folder the user cannot write.
- **.deb / .rpm:** the package is written to a temporary folder and installed with `dpkg -i` / `rpm -U` through `pkexec`, else a `zenity`/`kdialog` password prompt piped to `sudo -S`, else `sudo`. So every update of a `.deb` asks for an administrator password. Many apps leave `.deb` updates to the package manager instead; Hive's "Install update" (`with_install`) would have to say which applies.
- Hive's own checks around the updater (`translocated`, the app bundle for uninstall 23.5) are macOS-only and have no Linux meaning; uninstalling (23.5) would remove the AppImage or tell a `.deb` user to run `apt remove`.

## 6. What of the macOS code has a Linux twin

### The app (`src-tauri/src/macos.rs`, 2 702 lines, and the `#[cfg(target_os = "macos")]` parts of `main.rs` / `lib.rs`)

| macOS piece | Linux twin | Notes |
|---|---|---|
| `guard_ipc` (24.14): only the main frame at the app's origin reaches Tauri's IPC | **Needed, security.** wry's WebKitGTK webview takes IPC through a `UserContentManager` script message handler and the `ipc://` custom scheme; the twin checks the frame and `Origin` there (`webkit2gtk` crate, through `with_webview`'s `inner()`). | Without it a framed `hive-file:` page or a worktree's dev server can call every app command. Windows has no gate today (frozen, D17.0); Linux must not ship without one. |
| `guard_frames` (24.4): frames stay in their scheme | **Needed, security.** WebKitGTK's `decide-policy` signal sees subframe navigations; `frame_navigation` (portable, tested) decides. | |
| `disable_peer_connections` (24.4): no WebRTC | WebKitGTK's `enable-webrtc` setting (off by default; set it explicitly). | Small. |
| `app_url` / `scheme::file_url` / `ipc_from_app` with `macos: bool` | WebKitGTK serves `tauri://localhost` and custom schemes as macOS does (WebView2 is the odd one with `http://<scheme>.localhost`). | Every `cfg!(target_os = "macos")` passed to these must become "not Windows". Easy to miss: with `false` the app's own origin would be refused (no app) or a Windows origin admitted. |
| `bridge_command(macos, …)` | `/bin/sh -c BRIDGE_SCRIPT` as on macOS (it now runs `wsl.exe` whenever `macos` is false). | Same flag split. |
| `on_web_content_process_terminate` (18.21) | WebKitGTK's `web-process-terminated` signal; Tauri exposes the hook only on macOS. | Through `webkit2gtk`. |
| Notifications: `UNUserNotificationCenter`, permission, clicks (17.19, 19.10) | `org.freedesktop.Notifications` over D-Bus with a `default` action, whose `ActionInvoked` signal is the click. The notification plugin shows notifications on Linux but its actions are mobile-only, so no click. | Needs a D-Bus client (`zbus`, or `notify-rust`'s `wait_for_action`): a new dependency. Linux has no permission prompt, so `check_notifications` would answer granted. |
| The tone (`afplay`, 17.19) | `pw-play` / `paplay` / `canberra-gtk-play`, whichever exists, run like `afplay` with separate arguments. | The tested `afplay` shape fits a second program. |
| `ask_before_terminating` (⌘Q, Dock "Quit", log out, 17.7) | No Dock or app menu: the window's close goes through Tauri's `CloseRequested`, whose guard already exists (#18), as on Windows. Logging out: GTK has no simple hook (a session manager inhibitor over D-Bus). | Leave logging out unguarded. |
| `take_file_drops` (18.1): files dropped from the file manager | Tauri's own drag-drop event (`dragDropEnabled`) gives paths on Linux, but turning it on disables HTML5 drag inside the page; else WebKitGTK's drop of `text/uri-list`. | Needs a check in a real session. |
| `Pasteboard` (18.3): ⌘V pastes copied files' paths | GTK clipboard targets `text/uri-list` / `x-special/gnome-copied-files`. | Through `gtk` (already in the tree via tao). |
| `traffic_lights` (18.4), `app_menu` (17.13) | None: Hive draws its own window buttons (as on Windows) and has no menu bar; the keys are the frontend's. | |
| `terminal_permissions` (25.5, Screen Recording / Accessibility) | None (no TCC on Linux). Settings hides the section, as on Windows. | |
| `observe_wake` (26.17, Q7) | logind's `PrepareForSleep(false)` signal on the system bus. | D-Bus again. |
| Tray (17.22) | Tauri's tray works through libayatana-appindicator. *"Linux: Unsupported"* for tray mouse events: only the menu shows, which is already Hive's design (a click opens the menu). GNOME shows it only with the AppIndicator extension (on by default on Ubuntu, not on Fedora). No template image: the calm icon needs a fixed colour per theme. | Small. |
| Dock badge and bounce (17.16) | `request_user_attention` sets the urgency hint (X11; ignored by most Wayland compositors). A badge needs the Unity launcher API (Ubuntu dock, KDE). | Leave out, as on Windows. |
| App log and panic hook (D18.6) | `log::start` is `cfg(unix)` code, only called on macOS: turn it on under the XDG state folder. | Small. |
| Remote hosts over ssh (26.17) | `with_remote` is macOS-only only because of D17.0; the code is portable Unix (it is tested on Linux). | Free. |

### The service (`crates/hive/src/macos.rs`, 1 126 lines)

| macOS piece | Linux | Notes |
|---|---|---|
| Process table, arguments, open files, cwd (`libproc`, `KERN_PROCARGS2`) | Done: `/proc` (`procs.rs`). | |
| `footprint` (Activity Monitor's memory) | `workbook.rs` has a Linux branch. | |
| `user_temp_dir` (`confstr`) | Done: `XDG_RUNTIME_DIR` (`paths.rs`). | |
| `rename_new` | Done: `renameat2` (`file.rs`). | |
| `system_locale` (`defaults read -g AppleLocale`) | `C.UTF-8` today (18.18); a desktop session has `LANG`. | Small. |
| `apple_git` / `xcrun` | None needed. | |
| `trash` (D18.10e) | The freedesktop Trash (`~/.local/share/Trash`, a `.trashinfo` file); today Linux deletes for good. | Small, portable logic; needs a check on another volume (`$topdir/.Trash-$uid`). |
| `power` (`agents.keep_awake`, 17.17) | A logind inhibitor (`Inhibit("idle:sleep", …)` over D-Bus, or `systemd-inhibit` as a child process held while agents work). | D-Bus or a process. |
| `folder_warning` (TCC-protected folders) | None. | |
| `volume` / `remote` (network volumes) | `statfs` `f_type` (NFS, SMB, FUSE). | Small. |
| `native_path` | None (no WSL path conversion). | |

## 7. The CI cost

Measured on the v0.32.0 tag (2026-11-27), wall-clock minutes per job:

| Run | Jobs |
|---|---|
| `ci` (11 min) | rust 1, coverage 3, frontend 10, linux-hive 2, linux-remote ×2 2, remote 6, windows 9 |
| `macos` (25 min) | test 7, linux-remote ×2 2, bundle 12 |
| `release` (31 min) | linux 2, linux-remote ×2 2, windows 14, macos 23 (the critical path), publish 0 |

A Linux app would add:

- **Gates: nothing new.** Its `cfg(target_os = "linux")` code is compiled, covered and mutated by `ci.yml`'s existing `rust`, `coverage` and `mutants` jobs. The GTK/WebKitGTK twins need a display in tests (`xvfb-run`, which the `frontend` job already installs), or the same stand-in approach `src-tauri/src/macos.rs` uses for its Objective-C classes.
- **A `linux-bundle` job on branches that touch the app** (`ci.yml`, `ubuntu-22.04`): release build of `hive` and the app, AppImage and `.deb`, a check of the bundle's contents. Estimated 10–14 min (the Windows bundle takes 9–14), in parallel with the rest, so the `ci` run's wall clock barely moves.
- **Release:** one `linux-app` job for x86_64, and one on `ubuntu-22.04-arm` if aarch64 ships (linuxdeploy cannot cross-build). About 12–15 min each, in parallel with `macos` (23 min), so the release does not get longer. `publish` grows the `latest.json` entries and the uploads.
- **Minutes:** roughly +12 runner-minutes per app-touching branch and +25 per release on Linux runners (the cheapest; macOS runners count ten times on a private repository's quota).

The real cost is not CI: it is that no automated job can tell whether terminals are fast on a real GPU under Wayland, and nobody on the team would notice.

## 8. Decisions for the human

1. **Build a Linux desktop app?** Recommendation: **no, not now** (D17.0; no daily Linux user; the GPU path cannot be checked on the human's hardware). Keep the app compiling on Linux as today, which costs nothing.
2. If yes: **AppImage and `.deb`** (recommended), `.rpm` optional, **no Flatpak or Snap** (§4).
3. If yes: **`.deb` updates through the updater** (an administrator password each time) or left to the package manager.
4. If yes: **the terminal renderer on Linux**: WebGL by default with a Settings switch to the DOM renderer (Tauri's advice; ligatures only with WebGL), or DOM by default.
5. If yes: **aarch64 Linux** too, or x86_64 only.
6. If yes: **`webkit2gtk`** (and a D-Bus client, `zbus`) as direct dependencies of `hive-app`, Linux only (rule 3).
7. **28.13 (Nix flake):** skip it (§10).

## 9. If the human says yes: the tasks

About 45 points, in this order (each with its own tests; the security pair first):

1. **Platform flags** (3): every `cfg!(target_os = "macos")` passed as an origin or bridge flag becomes the right one for Linux (`app_url`, `scheme`, `ipc_from_app`, `bridge_command`), the app log and panic hook, remote hosts on Linux.
2. **IPC and frame guards on WebKitGTK** (8): `guard_ipc`, `guard_frames`, WebRTC off, web-process crash reload. Security review required.
3. **Notifications with clicks, the tone** (5): `org.freedesktop.Notifications`, `pw-play`/`paplay`.
4. **Window and tray** (5): Hive's own window buttons, edge resizing, the tray's colour icons, Wayland position.
5. **Files in and out** (5): drops from the file manager, copied files pasted, the freedesktop Trash in the service.
6. **Wake and keep awake** (3): logind's `PrepareForSleep` and an idle inhibitor.
7. **Terminal renderer setting** (3): WebGL / DOM switch on Linux.
8. **Packaging and CI** (8): AppImage and `.deb` (resources, the bundled `hive`, desktop file, icons), the `linux-bundle` job, `release.yml`'s Linux jobs and `latest.json` entries, README install section.
9. **A real-hardware check** (5): the app run on a physical Linux machine (Intel/AMD GPU, and NVIDIA if available) under GNOME Wayland and KDE: terminal latency, resizing, tray, notifications, updater. Without a Linux machine this task cannot be done, which is the main reason for the recommendation.

## 10. 28.13, the Nix flake

28.13 is "optional, after 28.12 if the human wants Linux", and would build the service "and the app, if 28.12 goes ahead". With the recommendation above it builds only the service, which helps only a Nix user running Hive's service without the app: on a remote Linux host the macOS app already uploads its own static `hive` (26.13) and a flake-built one of another version would be refused by the handshake (#29); on macOS the app bundles `hive`. A flake would also add a CI job (`nix flake check`) and a second build of the service to keep green. **Recommendation: do not do 28.13 now; revisit it together with a Linux app.**

## Sources

- Tauri v2: [Prerequisites](https://v2.tauri.app/start/prerequisites/), [Linux Graphics Issues](https://v2.tauri.app/develop/debug/linux-graphics/), [AppImage](https://v2.tauri.app/distribute/appimage/), [Flatpak](https://v2.tauri.app/distribute/flatpak/), [Updater plugin](https://v2.tauri.app/plugin/updater/), [Updater releases](https://v2.tauri.app/release/updater/all-versions/), [Notification plugin](https://v2.tauri.app/plugin/notification/), [System tray](https://v2.tauri.app/learn/system-tray/) (read 2026-11-27).
- `tauri-plugin-updater` source, [`src/updater.rs`](https://docs.rs/crate/tauri-plugin-updater/latest/source/src/updater.rs): target keys, AppImage replacement, `dpkg`/`rpm` through `pkexec`.
- WebKit [bug 311335](https://bugs.webkit.org/show_bug.cgi?id=311335) (WebGL context lost after the dma-buf refactor).
- CI timings: `scripts/ci.sh jobs` on runs 46038914842 (`release`), 46038914927 (`ci`), 46038914846 (`macos`) of v0.32.0.
