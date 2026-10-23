#!/usr/bin/env bash
# Tests the portable scripts on this OS, Linux or macOS (TODO 17.2):
#   - release.sh in a scratch repository holding a copy of the manifests (a fake `cargo`)
#   - gates-lib.sh: a failing gate or mutants run is reported and counted (a fake `cargo`)
#   - ci.sh with a fake `gh`: watch waits for ci and macos (--all adds windows), green asks for
#     both (D18.1)
#   - memwatch.sh reads the available memory, stops a process below its threshold, and leaves it
#     alone above
#   - macos-bundle.sh (macOS only): its bundle check passes a correctly signed bundle and fails
#     each broken one (TODO 17.11)
# Never touches the real environment: a temporary HOME, a scratch repository outside this one.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
repo=$PWD
tmp=$(mktemp -d)
probe=hive-memprobe
trap 'pkill -x $probe 2>/dev/null; rm -rf "$tmp"' EXIT
bad=0
check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; bad=$((bad + 1)); fi; }

# --- release.sh ---------------------------------------------------------------------------------
export HOME=$tmp/home GIT_CONFIG_NOSYSTEM=1
unset XDG_CONFIG_HOME
mkdir -p "$HOME/.cargo/bin" "$tmp/rel/scripts" "$tmp/rel/src-tauri" "$tmp/rel/crates/hive"
printf '#!/bin/sh\nexit 0\n' >"$HOME/.cargo/bin/cargo"
chmod +x "$HOME/.cargo/bin/cargo"
manifests="src-tauri/tauri.conf.json package.json src-tauri/Cargo.toml crates/hive/Cargo.toml"
for f in $manifests scripts/release.sh; do cp "$repo/$f" "$tmp/rel/$f"; done
git -C "$tmp/rel" init -q
git -C "$tmp/rel" config user.name test
git -C "$tmp/rel" config user.email test@example.invalid
git -C "$tmp/rel" add -A
git -C "$tmp/rel" commit -qm base
check "release: refuses a bad version" '! "$tmp/rel/scripts/release.sh" 1.2 2>/dev/null'
"$tmp/rel/scripts/release.sh" 9.8.7 >/dev/null
check "release: tagged v9.8.7" '[ "$(git -C "$tmp/rel" tag)" = v9.8.7 ]'
check "release: commit message" '[ "$(git -C "$tmp/rel" log -1 --format=%s)" = "chore(release): v9.8.7" ]'
# One line per manifest, its own version: never a dependency's.
check "release: one line per manifest" \
  '[ "$(git -C "$tmp/rel" diff --numstat HEAD~1 | awk "\$1 == 1 && \$2 == 1" | wc -l | tr -d " ")" = 4 ]'
check "release: json versions" \
  '[ "$(grep -h "^  \"version\": \"9.8.7\",$" "$tmp/rel/package.json" "$tmp/rel/src-tauri/tauri.conf.json" | wc -l | tr -d " ")" = 2 ]'
check "release: toml versions" \
  '[ "$(grep -h "^version = \"9.8.7\"$" "$tmp/rel/src-tauri/Cargo.toml" "$tmp/rel/crates/hive/Cargo.toml" | wc -l | tr -d " ")" = 2 ]'

# --- memwatch.sh --------------------------------------------------------------------------------
check "memwatch: available memory" '[[ $(scripts/memwatch.sh --available) =~ ^[1-9][0-9]*$ ]]'
# A process with a name of its own, so pkill hits only it. Built with cc: macOS kills a copy of a
# system binary (/bin/dash, /bin/sleep) run from another folder.
printf '#include <unistd.h>\nint main(void) { sleep(30); return 0; }\n' | cc -x c -o "$tmp/$probe" -
export MEMWATCH_INTERVAL=0.2
# Below the threshold (more than any machine has): stopped, and the log says why.
"$tmp/$probe" &
low=$!
scripts/memwatch.sh $probe 999999999999 "$tmp/low.log"
fired=$?
wait $low 2>/dev/null
low_status=$?
check "memwatch: stops below the threshold" '[ $low_status = 143 ] && [ $fired = 3 ]'  # 128 + SIGTERM
check "memwatch: logs the stop" 'grep -q "memory low" "$tmp/low.log"'
# Above it: left running; the watchdog exits once the process is gone.
"$tmp/$probe" &
high=$!
scripts/memwatch.sh $probe 0 "$tmp/high.log" &
watch=$!
sleep 1
check "memwatch: leaves it above the threshold" 'kill -0 $high 2>/dev/null && kill -0 $watch 2>/dev/null'
kill $high
wait $high 2>/dev/null
wait $watch
watch_status=$?
check "memwatch: exits with the process" '[ $watch_status = 0 ] && [ ! -s "$tmp/high.log" ]'

# --- gates-lib.sh -------------------------------------------------------------------------------
# gates.sh's accounting with a fake `cargo` that exits with $FAKE_STATUS: a failed gate or mutants
# run prints FAIL with its reason and is counted, so gates.sh exits non-zero.
mkdir -p "$tmp/bin" "$tmp/out"
printf '#!/bin/sh\nexit "${FAKE_STATUS:-0}"\n' >"$tmp/bin/cargo"
chmod +x "$tmp/bin/cargo"
gates() {  # gates <cargo status> <function and args…>: its output, then "failures=<n>"
  ( FAKE_STATUS=$1; export FAKE_STATUS; shift
    PATH=$tmp/bin:$PATH out=$tmp/out
    source scripts/gates-lib.sh
    "$@"
    echo "failures=$failures" )
}
# has <output> <line regex>…: every regex matches a whole line (no pipe into `grep -q`, whose early
# exit would fail the pipeline under pipefail).
has() { local text=$1; shift; for re in "$@"; do grep -qx -- "$re" <<<"$text" || return 1; done; }
o=$(gates 0 run fmt cargo fmt)
check "gates: a passing gate" 'has "$o" "ok    fmt" "failures=0"'
o=$(gates 1 run fmt cargo fmt)
check "gates: a failing gate counts" 'has "$o" "FAIL  fmt .*" "failures=1"'
o=$(gates 0 mutants m)
check "gates: mutants pass" 'has "$o" "ok    m" "failures=0"'
for case in "2:missed mutants" "3:timeouts" "4:baseline failed" "1:cargo-mutants exited 1"; do
  o=$(gates "${case%%:*}" mutants m)
  check "gates: mutants exit ${case%%:*} fails" 'has "$o" "FAIL  m (${case#*:}, .*" "failures=1"'
done
o=$(gates 0 mutants_verdict m 0 3)
check "gates: the watchdog's stop fails" 'has "$o" "FAIL  m (stopped by the memory watchdog, .*" "failures=1"'
o=$(gates 0 mutants_verdict m 0 1)
check "gates: an unreadable memory fails" 'has "$o" "FAIL  m (stopped by the memory watchdog, .*" "failures=1"'

# --- ci.sh --------------------------------------------------------------------------------------
# In a scratch repository on a task branch, with a fake `gh`: it logs each call, lists the runs
# in $tmp/ci-runs ("workflow id url" per line); `run view` reports the ids in $FAKE_FAILED failed and
# the others succeeded, after one look per line of $tmp/ci-later-<id> ("in_progress", or "error":
# gh fails).
mkdir -p "$tmp/gh" "$tmp/ci"
cat >"$tmp/gh/gh" <<'EOF'
#!/bin/sh
echo "$*" >>"$FAKE_GH_LOG"
case "$1 $2" in
  "auth token") echo token ;;
  "run list")
    case "$3" in
      # `--workflow <name>.yml … --status success`: the first green run's url, as `-q` prints it.
      --workflow) while read -r name id url; do
          case " $FAKE_FAILED " in *" $id "*) continue ;; esac
          if [ "$name.yml" = "$4" ]; then echo "$url"; break; fi
        done <"$FAKE_RUNS" ;;
      *) cat "$FAKE_RUNS" ;;
    esac ;;
  "run view")
    later=$(dirname "$FAKE_RUNS")/ci-later-$3
    if [ -s "$later" ]; then
      look=$(head -n 1 "$later"); sed -i.bak 1d "$later"
      if [ "$look" = error ]; then exit 1; fi
      echo "in_progress "; exit 0
    fi
    case " $FAKE_FAILED " in *" $3 "*) echo "completed failure" ;; *) echo "completed success" ;; esac ;;
esac
EOF
chmod +x "$tmp/gh/gh"
git -C "$tmp/ci" init -q
git -C "$tmp/ci" config user.name test
git -C "$tmp/ci" config user.email test@example.invalid
git -C "$tmp/ci" commit -q --allow-empty -m base
git -C "$tmp/ci" switch -q -c task/1.0-test
# ci <runs> <failed ids> <args…>: ci.sh's output and exit code, "exit=<n>"; the gh calls in
# $tmp/gh.log.
ci() {
  printf '%s\n' "$1" | sed '/^$/d' >"$tmp/ci-runs"
  : >"$tmp/gh.log"
  ( cd "$tmp/ci" && FAKE_RUNS=$tmp/ci-runs FAKE_FAILED=$2 FAKE_GH_LOG=$tmp/gh.log CI_SH_WAIT=0 CI_SH_POLL=0 \
      PATH=$tmp/gh:$PATH "$repo/scripts/ci.sh" "${@:3}" 2>&1; echo "exit=$?" )
}
three="ci 1 u/1
macos 2 u/2
windows 3 u/3"
o=$(ci "$three" "" watch)
check "ci.sh watch: ci and macos green" 'has "$o" "green u/1" "green u/2" "exit=0"'
check "ci.sh watch: windows not waited for" '! grep -q "^run view 3 " "$tmp/gh.log"'
# A run still going, then a failed gh call: polled until it ends.
printf 'in_progress\nerror\nin_progress\n' >"$tmp/ci-later-2"
o=$(ci "$three" "" watch)
check "ci.sh watch: polls a running run until it ends" \
  'has "$o" "green u/2" "exit=0" && [ "$(grep -c "^run view 2 " "$tmp/gh.log")" = 4 ]'
printf 'in_progress\n' >"$tmp/ci-later-2"
o=$(ci "$three" "2" watch)
check "ci.sh watch: a run that ends red fails" 'has "$o" "FAILED u/2 (scripts/ci.sh logs 2)" "exit=1"'
# The poll is bounded: 240 looks (8 h at the real 2 min) and it gives up.
for _ in $(seq 241); do echo in_progress; done >"$tmp/ci-later-1"
o=$(ci "$three" "" watch)
check "ci.sh watch: gives up on a run that never ends" 'has "$o" "STILL RUNNING u/1 after 8 h" "exit=1"'
rm -f "$tmp/ci-later-1"
o=$(ci "ci 1 u/1
macos 2 u/2" "" watch)
# No wait for windows: a handful of listings, not the 30 tries of the wait.
check "ci.sh watch: no windows run, no wait" \
  'has "$o" "exit=0" && [ "$(grep -c "^run list" "$tmp/gh.log")" -lt 10 ]'
o=$(ci "$three" "" watch --all)
check "ci.sh watch --all: windows too" 'has "$o" "green u/1" "green u/2" "green u/3" "exit=0"'
o=$(ci "$three" "2" watch)
check "ci.sh watch: a red macos fails" 'has "$o" "FAILED u/2 (scripts/ci.sh logs 2)" "exit=1"'
o=$(ci "$three" "3" watch)
check "ci.sh watch: a red windows does not count" 'has "$o" "exit=0"'
o=$(ci "$three" "3" watch --all)
check "ci.sh watch --all: a red windows fails" 'has "$o" "FAILED u/3 .*" "exit=1"'
# A missing macos run: the wait is bounded (30 tries, CI_SH_WAIT=0 here), then it fails.
o=$(ci "ci 1 u/1" "" watch)
check "ci.sh watch: a missing macos run fails, after a bounded wait" \
  'has "$o" "MISSING macos: no run of HEAD" "green u/1" "exit=1" && [ "$(grep -c "^run list" "$tmp/gh.log")" -gt 30 ]'
o=$(ci "$three" "" watch --other)
check "ci.sh watch: an unknown option fails" 'has "$o" "usage: scripts/ci.sh watch \[--all\]" "exit=2"'
o=$(ci "" "" dispatch release.yml)
check "ci.sh dispatch: on this task branch" \
  'has "$o" "exit=0" && grep -qx "workflow run release.yml --ref task/1.0-test" "$tmp/gh.log"'
git -C "$tmp/ci" switch -q -c other
o=$(ci "" "" dispatch release.yml)
check "ci.sh dispatch: only task branches" 'has "$o" "ci.sh: only task/\* branches (on other)" "exit=2"'
git -C "$tmp/ci" switch -q task/1.0-test
# green: a release needs ci and macos green (D18.1); windows does not count.
o=$(ci "$three" "3" green)
check "ci.sh green: ci and macos green" 'has "$o" "green u/1" "green u/2" "exit=0"'
check "ci.sh green: asks for HEAD's runs" \
  'grep -q "^run list --workflow macos.yml --commit $(git -C "$tmp/ci" rev-parse HEAD) --status success" "$tmp/gh.log"'
o=$(ci "$three" "2" green)
check "ci.sh green: a red macos fails" 'has "$o" "green u/1" "no green macos run for HEAD" "exit=1"'
o=$(ci "macos 2 u/2" "" green task/1.0-test)
check "ci.sh green: no ci run fails" 'has "$o" "no green ci run for task/1.0-test" "exit=1"'

# --- macos-bundle.sh signing-mode ---------------------------------------------------------------
secrets="APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_P8"
# mode <how many of the secrets to set> [<value of the others>]: signing-mode's output, or "failed".
# The others are unset, or set to the given value (an absent GitHub secret is an empty string).
mode() {
  ( n=0
    for v in $secrets; do
      if [ $n -lt "$1" ]; then export "$v=x"; elif [ $# = 2 ]; then export "$v=$2"; else unset "$v"; fi
      n=$((n + 1))
    done
    scripts/macos-bundle.sh signing-mode 2>/dev/null || echo failed )
}
check "signing-mode: no secrets is ad-hoc" '[ "$(mode 0)" = adhoc ]'
check "signing-mode: six empty secrets are ad-hoc" '[ "$(mode 0 "")" = adhoc ]'
check "signing-mode: all six is Developer ID" '[ "$(mode 6)" = developer-id ]'
check "signing-mode: one secret fails" '[ "$(mode 1)" = failed ]'
check "signing-mode: five secrets fail" '[ "$(mode 5)" = failed ]'
check "signing-mode: five and an empty one fail" '[ "$(mode 5 "")" = failed ]'

# --- macos-bundle.sh check (macOS only: codesign) -----------------------------------------------
# A small bundle shaped like Hive.app, with the real Info.plist keys and entitlements, signed
# ad-hoc as the macos.yml bundle is; each case breaks one thing and re-signs the app.
if [ "$(uname)" = Darwin ]; then
  unset APPLE_SIGNING_IDENTITY
  app=$tmp/Fake.app
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  printf 'int main(void) { return 0; }\n' | cc -x c -o "$app/Contents/MacOS/fake" -
  plutil -convert xml1 -o "$app/Contents/Info.plist" src-tauri/Info.plist
  plutil -insert CFBundleExecutable -string fake "$app/Contents/Info.plist"
  plutil -insert CFBundleIdentifier -string com.example.fake "$app/Contents/Info.plist"
  new_hive() { printf 'int main(void) { return 0; }\n' | cc -x c -o "$app/Contents/Resources/hive" -; }
  seal() { codesign --force --sign - --options runtime "$@" "$app" 2>/dev/null; }
  bundle_check() { scripts/macos-bundle.sh check "$app" "$@" >/dev/null 2>&1; }
  new_hive
  scripts/macos-bundle.sh sign-hive "$app/Contents/Resources/hive" 2>/dev/null
  seal --entitlements src-tauri/Entitlements.plist
  check "bundle: a signed bundle passes" 'bundle_check'
  check "bundle: the nested hive is signed ad-hoc" \
    'has "$(codesign -dv "$app/Contents/Resources/hive" 2>&1)" "Signature=adhoc" "Identifier=com.lucas.hive.service"'
  check "bundle: an ad-hoc bundle fails the Developer ID check" '! bundle_check --developer-id'
  check "bundle: an unknown option fails" '! bundle_check --other'
  seal
  check "bundle: no JIT entitlement fails" '! bundle_check'
  # plutil key paths split on dots, so the JIT-only entitlements are written out.
  printf '%s\n' '<?xml version="1.0" encoding="UTF-8"?>' '<plist version="1.0"><dict>' \
    '<key>com.apple.security.cs.allow-jit</key><true/>' '</dict></plist>' >"$tmp/jit-only.plist"
  seal --entitlements "$tmp/jit-only.plist"
  check "bundle: no Apple Events entitlement fails" '! bundle_check'
  new_hive
  seal --entitlements src-tauri/Entitlements.plist
  check "bundle: a linker-signed hive fails" '! bundle_check'
  scripts/macos-bundle.sh sign-hive "$app/Contents/Resources/hive" 2>/dev/null
  echo tampered >>"$app/Contents/Resources/hive"
  check "bundle: a changed hive fails" '! bundle_check'
  new_hive
  scripts/macos-bundle.sh sign-hive "$app/Contents/Resources/hive" 2>/dev/null
  for key in NSDocumentsFolderUsageDescription NSDesktopFolderUsageDescription \
    NSDownloadsFolderUsageDescription NSRemovableVolumesUsageDescription \
    NSAppleEventsUsageDescription NSLocalNetworkUsageDescription; do
    cp "$app/Contents/Info.plist" "$tmp/Info.plist"
    plutil -remove "$key" "$app/Contents/Info.plist"
    seal --entitlements src-tauri/Entitlements.plist
    check "bundle: no $key fails" '! bundle_check'
    plutil -replace "$key" -string "This app reads the folder." "$app/Contents/Info.plist"
    seal --entitlements src-tauri/Entitlements.plist
    check "bundle: a $key not naming Hive fails" '! bundle_check'
    cp "$tmp/Info.plist" "$app/Contents/Info.plist"
  done
  seal --entitlements src-tauri/Entitlements.plist
  check "bundle: restored, it passes again" 'bundle_check'
  # Another signer for the nested hive: a second real identity would need a keychain on the
  # user's search list (the real environment), so a wrapper around /usr/bin/codesign reports the
  # hive's signature as a certificate's (an Authority chain and a team).
  mkdir -p "$tmp/signer"
  cat >"$tmp/signer/codesign" <<'EOF'
#!/bin/sh
/usr/bin/codesign "$@"; status=$?
case "$*" in *-dvv*/Resources/hive) echo "Authority=Developer ID Application: Other (OTHER1234)" >&2
  echo "TeamIdentifier=OTHER1234" >&2 ;; esac
exit $status
EOF
  chmod +x "$tmp/signer/codesign"
  check "bundle: a hive with another signer fails" '! PATH=$tmp/signer:$PATH bundle_check'
  check "bundle: it fails on the signers" \
    '[[ $(PATH=$tmp/signer:$PATH scripts/macos-bundle.sh check "$app" 2>&1) == *"different signers"* ]]'
fi

[ $bad = 0 ]
