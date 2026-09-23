#!/usr/bin/env bash
# Builds the Windows app inside WSL and runs it on Windows (TODO 1.0).
#   - cross-compiles src-tauri for x86_64-pc-windows-msvc with cargo-xwin
#   - copies the .exe to %LOCALAPPDATA%\hive-dev (Windows cannot run it from \\wsl$ reliably)
#   - builds target/debug/hive and points the app's bridge at it (HIVE_BRIDGE, HIVE_WSL_DISTRO)
#   - starts Vite in WSL on port 1420; Windows reaches it through WSL localhost forwarding,
#     so the debug build loads devUrl and hot reload works
# Needs: rustup target x86_64-pc-windows-msvc, cargo-xwin, and clang/lld/llvm from apt.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH=$HOME/.cargo/bin:$PATH

# Static CRT: stock Windows has no VC++ redistributable (VCRUNTIME140_1.dll).
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static"
cargo xwin build -p hive-app --target x86_64-pc-windows-msvc
cargo build -p hive

appdata=$(wslpath "$(cmd.exe /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')")
dest="$appdata/hive-dev"
mkdir -p "$dest"
cp target/x86_64-pc-windows-msvc/debug/hive-app.exe "$dest/"

bun run dev &
vite=$!
trap 'kill $vite 2>/dev/null' EXIT
until curl -fs http://localhost:1420 >/dev/null; do
  kill -0 $vite 2>/dev/null || { echo "win-dev: vite exited" >&2; exit 1; }
  sleep 0.2
done

# The app runs `wsl.exe -d $HIVE_WSL_DISTRO --exec ... $HIVE_BRIDGE bridge` (see docs/architecture.md).
# WSLENV hands both variables, unchanged, to powershell.exe and so to the app.
export HIVE_BRIDGE="$PWD/target/debug/hive" HIVE_WSL_DISTRO="$WSL_DISTRO_NAME"
export WSLENV="${WSLENV:+$WSLENV:}HIVE_BRIDGE:HIVE_WSL_DISTRO"

# Launching the .exe straight through WSL interop exits with code 53; PowerShell works.
cd /mnt/c
powershell.exe -NoProfile -Command 'Start-Process -Wait "$env:LOCALAPPDATA\hive-dev\hive-app.exe"'
