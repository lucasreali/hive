#!/usr/bin/env bash
# Tests the portable scripts on this OS, Linux or macOS (TODO 17.2):
#   - release.sh in a scratch repository holding a copy of the manifests (a fake `cargo`)
#   - gates-lib.sh: a failing gate or mutants run is reported and counted (a fake `cargo`)
#   - ci.sh with a fake `gh`: watch waits for ci and macos (--all adds windows), green asks for
#     both (D18.1)
#   - memwatch.sh reads the available memory, stops a process below its threshold, and leaves it
#     alone above
#   - energy.sh: its argument checks, and finding Hive's processes in and parsing canned `ps` and
#     `top` output
#   - release-notes.sh: the notes of a scratch repository's commits since its previous tag
#   - macos-bundle.sh check-updater: every macOS key of latest.json has the one tarball (TODO 26.5)
#   - macos-bundle.sh (macOS only): its bundle check passes a correctly signed universal bundle and
#     fails each broken one, in either architecture (TODO 17.11, 26.5); join-dsyms; smoke, with a
#     fake `arch` (the real one under Rosetta runs in macos.yml's bundle job)
# Never touches the real environment: a temporary HOME, a scratch repository outside this one.
# The one exception is for a CI runner only, whose keychain search list is disposable: with both
# GITHUB_ACTIONS and REQUIRE_REAL_CERTIFICATE set (macos.yml), a real certificate signs a bundle in
# a temporary keychain put on that list. Never set both on a machine of yours.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
repo=$PWD
tmp=$(mktemp -d)
probe=hive-memprobe
trap 'pkill -x $probe 2>/dev/null; rm -rf "$tmp"' EXIT
bad=0
check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; bad=$((bad + 1)); fi; }

# --- release.sh ---------------------------------------------------------------------------------
real_home=$HOME
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
# holds <text> <part>...: every part is somewhere in the text, as it is written.
holds() { local text=$1; shift; for part in "$@"; do [[ $text == *"$part"* ]] || return 1; done; }
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
check "ci.sh dispatch: only task branches" 'has "$o" "ci.sh: only task/\* branches, or push on main (on other)" "exit=2"'
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

# --- energy.sh ----------------------------------------------------------------------------------
for bad_args in "" "0" "abc" "5 6" "-1" "100000"; do
  o=$(scripts/energy.sh $bad_args 2>&1; echo "exit=$?")
  check "energy: refuses '$bad_args'" 'has "$o" "usage: scripts/energy.sh <seconds>" "exit=2"'
done
o=$( source scripts/energy.sh; find_procs <<'EOF'
100 1 01:00:00 /Applications/Hive.app/Contents/MacOS/hive-app
101 1 01:00:01 /System/Library/Frameworks/WebKit.framework/Versions/A/XPCServices/com.apple.WebKit.GPU.xpc/Contents/MacOS/com.apple.WebKit.GPU
102 1 59:58 /System/Library/Frameworks/WebKit.framework/Versions/A/XPCServices/com.apple.WebKit.WebContent.xpc/Contents/MacOS/com.apple.WebKit.WebContent
103 1 01-02:00:00 /System/Library/Frameworks/WebKit.framework/Versions/A/XPCServices/com.apple.WebKit.GPU.xpc/Contents/MacOS/com.apple.WebKit.GPU
104 100 01:00:00 /Users/x/.local/share/hive/bin/hive bridge
105 104 01:00:00 /Users/x/.local/share/hive/bin/hive daemon
106 1 05:00 /tmp/target/debug/hive daemon
107 1 01:00:00 /tmp/target/debug/hive bridge
108 105 03:00 /opt/homebrew/bin/fish -l
EOF
)
check "energy: finds the app, its helpers by start time, its bridge and daemon" \
  '[ "$(sort <<<"$o" | tr "\n" ";")" = "100 hive-app;101 WebKit.GPU;102 WebKit.WebContent;104 bridge;105 daemon;" ]'
# top -l 3 -s 1: the first sample has no CPU or POWER yet; IDLEW is a running count.
o=$( source scripts/energy.sh; parse_top 1 <<'EOF'
Processes: 703 total, 3 running, 700 sleeping, 3643 threads
2026/10/27 08:57:24

PID    %CPU POWER IDLEW
34362  0.0  0.0   29005
34355  0.0  0.0   284811
9      0.0  0.0   1K
Processes: 703 total, 2 running, 701 sleeping, 3666 threads

PID    %CPU POWER IDLEW
34362  1.0  1.2   29007+
34355  1.4  2.9   284849+
9      0.0  0.0   1.5K+

PID    %CPU POWER IDLEW
34362  0.6  0.6   29009+
34355  3.2  4.3   284875+
9      0.0  0.0   2K+
EOF
)
check "energy: parses a top sample" \
  'has "$o" "34362 0.8 1.0 0.9 2.0" "34355 2.3 3.2 3.6 32.0" "9 0.0 0.0 0.0 500.0" "total 3.1 3.8 4.5 534.0"'

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
own="HIVE_CERTIFICATE HIVE_CERTIFICATE_PASSWORD HIVE_CERTIFICATE_SHA1"
# own_mode <how many of Hive's certificate secrets> <how many of the Apple ones> [<value of the others>].
own_mode() {
  ( n=0
    for v in $own; do
      if [ $n -lt "$1" ]; then export "$v=x"; elif [ $# = 3 ]; then export "$v=$3"; else unset "$v"; fi
      n=$((n + 1))
    done
    if [ $# = 3 ]; then mode "$2" "$3"; else mode "$2"; fi )
}
check "signing-mode: the certificate's three secrets are self-signed" '[ "$(own_mode 3 0)" = self-signed ]'
check "signing-mode: its three with the others empty are self-signed" '[ "$(own_mode 3 0 "")" = self-signed ]'
check "signing-mode: none of them, empty, is ad-hoc" '[ "$(own_mode 0 0 "")" = adhoc ]'
check "signing-mode: one of the certificate's fails" '[ "$(own_mode 1 0)" = failed ]'
check "signing-mode: two of the certificate's fail" '[ "$(own_mode 2 0 "")" = failed ]'
check "signing-mode: all of both is Developer ID" '[ "$(own_mode 3 6)" = developer-id ]'
check "signing-mode: the certificate and a partial Apple set fail" '[ "$(own_mode 3 2)" = failed ]'
check "signing-mode: Developer ID and a partial certificate fail" '[ "$(own_mode 1 6)" = failed ]'

# --- macos-bundle.sh release-notes --------------------------------------------------------------
notes() { printf '%s' "$2" | scripts/macos-bundle.sh release-notes "$1"; }
first=$(notes self-signed "")
check "release-notes: the first self-signed release says to allow Hive once more" \
  'holds "$first" "signed with its own certificate" "allow it one last time" "Screen & System Audio Recording" "Accessibility"'
check "release-notes: it stays silent once an earlier release said it" \
  '[ -z "$(notes self-signed "$(printf "older\n\n%s\n" "$first")")" ]'
check "release-notes: other earlier notes do not silence it" \
  '[ "$(notes self-signed "something else")" = "$first" ]'
check "release-notes: an ad-hoc release has none" '[ -z "$(notes adhoc "")" ]'
check "release-notes: a Developer ID release has none" '[ -z "$(notes developer-id "")" ]'

# --- macos-bundle.sh check-updater --------------------------------------------------------------
# What an installed Hive reads: each macOS key, with the one universal tarball.
updater_json() { # <jq filter on the good file>
  jq -n '{x: {url: "https://example.com/Hive_universal.app.tar.gz", signature: "sig"}} |
    {platforms: {"windows-x86_64": {url: "w", signature: "w"}, "darwin-aarch64": .x, "darwin-aarch64-app": .x,
      "darwin-x86_64": .x, "darwin-x86_64-app": .x}}' | jq "$1" >"$tmp/latest.json"
}
updater_check() { scripts/macos-bundle.sh check-updater "$tmp/latest.json" >/dev/null 2>&1; }
updater_json .
check "check-updater: all four macOS keys pass" 'updater_check'
for key in darwin-aarch64 darwin-aarch64-app darwin-x86_64 darwin-x86_64-app; do
  updater_json "del(.platforms[\"$key\"])"
  check "check-updater: no $key fails" '! updater_check'
done
updater_json '.platforms["darwin-x86_64"].url = "https://example.com/other.app.tar.gz"'
check "check-updater: another tarball for Intel fails" '! updater_check'
updater_json '.platforms["darwin-x86_64-app"].signature = "other"'
check "check-updater: another signature for Intel fails" '! updater_check'
updater_json '.platforms |= map_values(.url = "https://example.com/Hive.dmg")'
check "check-updater: a .dmg, which the updater cannot unpack, fails" '! updater_check'
updater_json '.platforms |= map_values(.signature = "")'
check "check-updater: no signature fails" '! updater_check'
check "check-updater: no file fails" '! scripts/macos-bundle.sh check-updater "$tmp/no-such.json" 2>/dev/null'

# --- release-notes.sh ---------------------------------------------------------------------------
# Each case builds a scratch repository: `rn_new`, `c <subject> [body]`, `tagged <tag>`, then
# `rn <tag> [mode] [earlier notes]`; `section` is one heading's lines.
rn_dir=$tmp/release-notes
rn_new() {
  rm -rf "$rn_dir"
  git init -q "$rn_dir"
  git -C "$rn_dir" config user.name test
  git -C "$rn_dir" config user.email test@example.invalid
}
c() { git -C "$rn_dir" commit -q --allow-empty -m "$1" ${2:+-m "$2"}; }
tagged() { git -C "$rn_dir" tag "$1"; }
rn() { (cd "$rn_dir" && printf '%s' "${3:-}" | "$repo/scripts/release-notes.sh" "$1" "${2:-adhoc}"); }
section() { awk -v head="## $2" '/^## /{on = ($0 == head); next} on && /^- /' <<<"$1"; }
link='[the first-launch steps](https://github.com/lucasreali/hive#macos)'

# Commits since the previous tag, in their sections.
rn_new; c "feat: before"; c "fix(old): before too"; tagged v1.0.0
c "feat: a feature"; c "fix: a fix"; c "perf: faster"; c "feat: a newer feature"; tagged v1.1.0
o=$(rn v1.1.0)
check "notes: only the commits since the previous tag" '! holds "$o" "before"'
check "notes: feat, newest first" '[ "$(section "$o" Features)" = "$(printf -- "- a newer feature\n- a feature")" ]'
check "notes: fix" '[ "$(section "$o" Fixes)" = "- a fix" ]'
check "notes: perf" '[ "$(section "$o" Performance)" = "- faster" ]'
check "notes: the first-launch link comes last" '[ "$(tail -n 1 <<<"$o")" = "First install on a Mac: follow $link." ]'
check "notes: no section stands empty" '! holds "$o" "## Breaking"'
check "notes: the link's heading is in the README" 'grep -qx "### macOS" "$repo/README.md"'

# Which tag is the previous one.
rn_new; c "feat: first"; tagged v1.0.0; c "feat: second"; tagged v2.0.0; c "feat: third"
o=$(rn v3.0.0)
check "notes: a dry run, its tag not there yet, counts since the last tag" \
  '[ "$(section "$o" Features)" = "- third" ]'
git -C "$rn_dir" tag -a v3.0.0 -m "an annotated tag"; c "feat: fourth"; tagged v4.0.0
o=$(rn v4.0.0)
check "notes: an annotated previous tag" '[ "$(section "$o" Features)" = "- fourth" ]'
tagged other-tag; c "feat: fifth"; tagged v5.0.0
o=$(rn v5.0.0)
check "notes: the tag at HEAD is not the previous one" '[ "$(section "$o" Features)" = "- fifth" ]'
c "feat: sixth"; tagged not-a-release; tagged v6.0.0
o=$(rn v6.0.0)
check "notes: a tag not named v<digit> is no release" '[ "$(section "$o" Features)" = "- sixth" ]'
rn_new; c "feat: base"; tagged v1.0.0
git -C "$rn_dir" switch -q -c elsewhere; c "feat: on a branch"; tagged v9.0.0
git -C "$rn_dir" switch -q -; c "feat: on main"; tagged v2.0.0
o=$(rn v2.0.0)
check "notes: a tag not in this history is not the previous one" \
  '[ "$(section "$o" Features)" = "- on main" ]'

# The first tag.
rn_new; c "docs: the start"; c "feat: all there is"; tagged v0.1.0
o=$(rn v0.1.0)
check "notes: a first tag lists every commit" '[ "$(section "$o" Features)" = "- all there is" ]'
rn_new; c "feat: untagged at all"
check "notes: no tag at all, and none for it yet" '[ "$(section "$(rn v0.1.0)" Features)" = "- untagged at all" ]'

# Scopes.
rn_new; c "feat(viewer): edit a table"; c "fix(a b): two words"; c "feat(): an empty scope"; tagged v0.1.0
o=$(rn v0.1.0)
check "notes: a scope leads the line" '[ "$(section "$o" Features)" = "- **viewer:** edit a table" ]'
check "notes: a scope of two words" '[ "$(section "$o" Fixes)" = "- **a b:** two words" ]'
check "notes: an empty scope is not a Conventional Commit" '! holds "$o" "an empty scope"'

# Breaking changes.
rn_new; c "feat(api)!: a breaking feature"; c "fix!: a breaking fix"; c "refactor(core)!: a breaking refactor"
c "feat: with a footer" "BREAKING CHANGE: it breaks"; c "docs: a dashed footer" "BREAKING-CHANGE: it breaks"
c "fix: only says so in passing" "Not a BREAKING CHANGE: mid-line"; c "chore: !: no marker"; tagged v0.1.0
o=$(rn v0.1.0)
check "notes: every breaking change under its heading, whatever its type" \
  '[ "$(section "$o" "Breaking changes")" = "$(printf -- "- a dashed footer\n- with a footer\n- **core:** a breaking refactor\n- a breaking fix\n- **api:** a breaking feature")" ]'
check "notes: breaking changes come first" '[ "$(grep -m1 "^## " <<<"$o")" = "## Breaking changes" ]'
check "notes: a breaking feature is not a feature too" '[ -z "$(section "$o" Features)" ]'
check "notes: BREAKING CHANGE counts only at the start of a line" \
  '[ "$(section "$o" Fixes)" = "- only says so in passing" ]'
check "notes: a chore that only holds \"!:\" stays out" '! holds "$o" "no marker"'

# Merges.
rn_new; c "docs: base"; tagged v1.0.0; git -C "$rn_dir" switch -q -c topic; c "feat: from the branch"
git -C "$rn_dir" switch -q -; c "fix: on main"
git -C "$rn_dir" merge -q --no-ff -m "chore(merge): task/1-topic" topic
git -C "$rn_dir" switch -q -c other; c "fix: from another"; git -C "$rn_dir" switch -q -
git -C "$rn_dir" merge -q --no-ff -m "feat: a merge commit called a feature" other
c "chore(merge): not even a merge"; tagged v1.1.0
o=$(rn v1.1.0)
check "notes: a merged branch's commits are listed" 'holds "$o" "- from the branch" "- from another" "- on main"'
check "notes: chore(merge) and merge commits are left out" '! holds "$o" "merge"'

# Everything else.
rn_new
for subject in "docs: d" "chore: e" "test: f" "refactor: g" "build: h" "ci: i" "style: j" "chore(release): v1.0.0" \
  "feature: k" "fixes: l" "perfect: m" "Fix: n" "FEAT: o" "feat:p" "feat : q" "feat(r: s" "feat(t)(u): v" "fix the thing" \
  "Merge branch 'main' into task/1"; do
  c "$subject"
done
tagged v0.1.0
o=$(rn v0.1.0)
check "notes: other types and lookalikes leave only the link" '[ "$o" = "First install on a Mac: follow $link." ]'

# Reverts.
rn_new; c "docs: base"; tagged v1.0.0
c "feat(a): kept"; c "feat(b): gone"; c 'Revert "feat(b): gone"'
c "fix: back again"; c 'Revert "fix: back again"'; c 'Revert "Revert "fix: back again""'
c "perf: also back"; c 'Revert "perf: also back"'; c 'Reapply "perf: also back"'
c 'Revert "feat: from an earlier release"'; tagged v1.1.0
o=$(rn v1.1.0)
check "notes: a reverted commit and its revert are left out" \
  '[ "$(section "$o" Features)" = "- **a:** kept" ] && ! holds "$o" "Revert"'
check "notes: a reverted revert is listed once" '[ "$(section "$o" Fixes)" = "- back again" ]'
check "notes: a reapplied commit is listed once" \
  '[ "$(section "$o" Performance)" = "- also back" ] && ! holds "$o" "Reapply"'

# Untrusted subjects: data all the way, into the JSON too.
rn_new; evaluated=$tmp/evaluated
nasty='a $(touch '$evaluated') `touch '$evaluated'` "quoted" \back ${HOME} ; touch '$evaluated' | <b>'
c "fix: $nasty"; c "feat($nasty): in a scope"; c "$(printf 'perf: a \033[31mred\033[0m\ttabbed\177 one')"; tagged v0.1.0
o=$(rn v0.1.0)
check "notes: a subject comes through unchanged" '[ "$(section "$o" Fixes)" = "- $nasty" ]'
check "notes: control characters are removed" '[ "$(section "$o" Performance)" = "- a [31mred[0mtabbed one" ]'
check "notes: jq --arg carries them unchanged" \
  '[ "$(jq -n --arg notes "$o" "{notes: \$notes}" | jq -r .notes)" = "$o" ]'
check "notes: nothing in a subject is evaluated" '[ ! -e "$evaluated" ]'

# 25.5's one-time note joins them.
rn_new; c "feat: a feature"; tagged v0.1.0
o=$(rn v0.1.0 self-signed)
check "notes: the first self-signed release says to allow Hive once more, first" \
  '[ "${o:0:${#first}}" = "$first" ] && holds "$o" "- a feature" "$link"'
check "notes: the note and the changes keep a blank line" 'holds "$o" "$first

## Features"'
check "notes: a later self-signed release, given those notes, stays silent" \
  '! holds "$(rn v0.2.0 self-signed "$o")" "signed with its own certificate"'
check "notes: other signing modes have only the changes" \
  '! holds "$(rn v0.1.0 adhoc)" "signed with its own certificate" &&
   ! holds "$(rn v0.1.0 developer-id)" "signed with its own certificate"'

# What stops it.
check "notes: a wrong call fails" '! (cd "$rn_dir" && "$repo/scripts/release-notes.sh" v0.1.0 </dev/null 2>/dev/null)'
c "feat: more"; git clone -q --depth 1 "file://$rn_dir" "$tmp/shallow" 2>/dev/null
check "notes: a shallow clone fails, so no release says the wrong thing" \
  '! (cd "$tmp/shallow" && "$repo/scripts/release-notes.sh" v0.2.0 adhoc </dev/null >/dev/null 2>&1)'
mkdir "$tmp/no-repository"
check "notes: outside a repository it fails" \
  '! (cd "$tmp/no-repository" && GIT_CEILING_DIRECTORIES=$tmp "$repo/scripts/release-notes.sh" v0.2.0 adhoc </dev/null >/dev/null 2>&1)'

# The size GitHub and an argument take: whole lines out, and counted.
rn_new; c "fix: a short one"; c "fix: $(head -c 100000 /dev/zero | tr '\0' x)"; c "perf: another short one"; tagged v0.1.0
o=$(rn v0.1.0)
check "notes: past the limit, lines are left out and counted" \
  'holds "$o" "- a short one" "- another short one" "And 1 more, left out for length." && [ "${#o}" -lt 1000 ]'

# The workflow: the history for it, and the same text in both places.
publish=$(awk '/^  publish:/,0' "$repo/.github/workflows/release.yml")
check "notes: the publish job checks out the whole history" \
  'grep -A2 "actions/checkout" <<<"$publish" | grep -q "fetch-depth: 0"'
check "notes: latest.json takes them with --arg, and after they are made" \
  'holds "$publish" "--arg notes \"\$notes\"" "notes: \$notes" &&
   [ "$(grep -n "name: notes" <<<"$publish" | cut -d: -f1)" -lt "$(grep -n "name: latest.json" <<<"$publish" | cut -d: -f1)" ]'
check "notes: the release takes the same file" 'holds "$publish" "> notes.md" "--notes-file notes.md"'

# --- macos-bundle.sh check (macOS only: codesign) -----------------------------------------------
# A small bundle shaped like Hive.app, with the real Info.plist keys and entitlements, signed
# ad-hoc as the macos.yml bundle is; each case breaks one thing and re-signs the app.
if [ "$(uname)" = Darwin ]; then
  unset APPLE_SIGNING_IDENTITY
  app=$tmp/Fake.app
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  # Stdin would reach only the first architecture's compiler, so the source is a file.
  printf 'int main(void) { return 0; }\n' >"$tmp/main.c"
  universal() { cc -arch arm64 -arch x86_64 -o "$1" "$tmp/main.c"; }
  universal "$app/Contents/MacOS/fake"
  plutil -convert xml1 -o "$app/Contents/Info.plist" src-tauri/Info.plist
  plutil -insert CFBundleExecutable -string fake "$app/Contents/Info.plist"
  plutil -insert CFBundleIdentifier -string com.example.fake "$app/Contents/Info.plist"
  plutil -insert CFBundleIconName -string AppIcon "$app/Contents/Info.plist"
  cp src-tauri/icons/Assets.car "$app/Contents/Resources/Assets.car"
  new_hive() { universal "$app/Contents/Resources/hive"; }
  seal() { codesign --force --sign - --options runtime "$@" "$app" 2>/dev/null; }
  bundle_check() { scripts/macos-bundle.sh check "$app" "$@" >/dev/null 2>&1; }
  new_hive
  scripts/macos-bundle.sh sign-hive "$app/Contents/Resources/hive" 2>/dev/null
  seal --entitlements src-tauri/Entitlements.plist
  check "bundle: a signed bundle passes" 'bundle_check'
  for slice in x86_64 arm64; do
    check "bundle: the nested hive is signed in $slice" \
      'has "$(codesign --arch "$slice" -dv "$app/Contents/Resources/hive" 2>&1)" "Signature=adhoc" "Identifier=com.lucas.hive.service"'
  done
  # A thin hive or app: what a build that skipped `lipo` or the universal target would ship.
  hive_bin=$app/Contents/Resources/hive app_bin=$app/Contents/MacOS/fake
  check_says() { [[ $(scripts/macos-bundle.sh check "$app" 2>&1) == *"$1"* ]]; }
  for slice in x86_64 arm64; do
    missing=arm64
    [ "$slice" = arm64 ] && missing=x86_64
    cp "$hive_bin" "$tmp/hive.saved"
    lipo "$tmp/hive.saved" -thin "$slice" -output "$hive_bin"
    scripts/macos-bundle.sh sign-hive "$hive_bin" 2>/dev/null
    seal --entitlements src-tauri/Entitlements.plist
    check "bundle: a hive of $slice alone fails" 'check_says "hive has no $missing slice"'
    cp "$tmp/hive.saved" "$hive_bin"
    cp "$app_bin" "$tmp/app.saved"
    lipo "$tmp/app.saved" -thin "$slice" -output "$app_bin"
    seal --entitlements src-tauri/Entitlements.plist
    check "bundle: an app of $slice alone fails" 'check_says "fake has no $missing slice"'
    cp "$tmp/app.saved" "$app_bin"
    seal --entitlements src-tauri/Entitlements.plist
  done
  check "bundle: both again, it passes" 'bundle_check'
  # `lipo` after signing, or a signature of one slice only: the other slice is not properly signed.
  cp "$hive_bin" "$tmp/hive.saved"
  cc -arch arm64 -o "$tmp/hive-arm" "$tmp/main.c"
  scripts/macos-bundle.sh sign-hive "$tmp/hive-arm" 2>/dev/null
  cc -arch x86_64 -o "$tmp/hive-intel" "$tmp/main.c"
  lipo -create -output "$hive_bin" "$tmp/hive-arm" "$tmp/hive-intel"
  seal --entitlements src-tauri/Entitlements.plist
  # The linker signs only arm64 by default: an x86_64 slice straight from it has no signature.
  check "bundle: a hive with its Intel slice unsigned fails" 'check_says "nested hive does not validate"'
  cc -arch x86_64 -Wl,-adhoc_codesign -o "$tmp/hive-intel" "$tmp/main.c"
  lipo -create -output "$hive_bin" "$tmp/hive-arm" "$tmp/hive-intel"
  seal --entitlements src-tauri/Entitlements.plist
  check "bundle: a hive with its Intel slice only linker-signed fails" 'check_says "x86_64 slice is only linker-signed"'
  cc -arch x86_64 -o "$tmp/hive-intel" "$tmp/main.c"
  cc -arch arm64 -o "$tmp/hive-arm" "$tmp/main.c"
  scripts/macos-bundle.sh sign-hive "$tmp/hive-intel" 2>/dev/null
  lipo -create -output "$hive_bin" "$tmp/hive-arm" "$tmp/hive-intel"
  seal --entitlements src-tauri/Entitlements.plist
  check "bundle: a hive with its arm64 slice only linker-signed fails" 'check_says "arm64 slice is only linker-signed"'
  cp "$tmp/hive.saved" "$hive_bin"
  seal --entitlements src-tauri/Entitlements.plist
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
  plutil -replace CFBundleIconName -string Other "$app/Contents/Info.plist"
  seal --entitlements src-tauri/Entitlements.plist
  check "bundle: an icon name not in Assets.car fails" '! bundle_check'
  plutil -remove CFBundleIconName "$app/Contents/Info.plist"
  seal --entitlements src-tauri/Entitlements.plist
  check "bundle: no CFBundleIconName fails" '! bundle_check'
  plutil -insert CFBundleIconName -string AppIcon "$app/Contents/Info.plist"
  mv "$app/Contents/Resources/Assets.car" "$tmp/Assets.car"
  seal --entitlements src-tauri/Entitlements.plist
  check "bundle: no Assets.car fails" '! bundle_check'
  mv "$tmp/Assets.car" "$app/Contents/Resources/Assets.car"
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
  # The same in the slice this machine does not run: only asking for it by name would see it.
  sed 's|case "\$\*" in \*-dvv\*/Resources/hive)|case "$*" in *x86_64*-dvv*/Resources/hive)|' \
    "$tmp/signer/codesign" >"$tmp/signer/codesign.intel"
  mkdir -p "$tmp/signer-intel"
  mv "$tmp/signer/codesign.intel" "$tmp/signer-intel/codesign"
  chmod +x "$tmp/signer-intel/codesign"
  check "bundle: another signer only in x86_64 fails, naming it" \
    '[[ $(PATH=$tmp/signer-intel:$PATH scripts/macos-bundle.sh check "$app" 2>&1) == *"different signers in x86_64"* ]]'

  # --- macos-bundle.sh join-dsyms ------------------------------------------------------------------
  # Two dSYMs per architecture whose DWARF files differ in name, as cargo's hashes make them.
  dsym_of() { # <folder> <name> <arch>
    local object=$tmp/$3-$2.o name=$2-$3-hash
    mkdir -p "$1"
    cc -g -arch "$3" -c -o "$object" -DNAME="\"$2-$3\"" -x c - <<<'const char *n = NAME; int main(void) { return n[0]; }'
    cc -arch "$3" -o "$1/$name" "$object"
    dsymutil -o "$1/$name.dSYM" "$1/$name" 2>/dev/null
    ln -s "$name.dSYM" "$1/$2.dSYM"
    mv "$1/$name" "$1/$2"
  }
  for name in hive hive-app; do
    dsym_of "$tmp/dsym-arm" "$name" arm64
    dsym_of "$tmp/dsym-intel" "$name" x86_64
  done
  dsym_app=$tmp/Dsym.app
  mkdir -p "$dsym_app/Contents/MacOS" "$dsym_app/Contents/Resources"
  plutil -create xml1 "$dsym_app/Contents/Info.plist"
  plutil -insert CFBundleExecutable -string hive-app "$dsym_app/Contents/Info.plist"
  lipo -create -output "$dsym_app/Contents/MacOS/hive-app" "$tmp/dsym-arm/hive-app" "$tmp/dsym-intel/hive-app"
  lipo -create -output "$dsym_app/Contents/Resources/hive" "$tmp/dsym-arm/hive" "$tmp/dsym-intel/hive"
  check "join-dsyms: it makes the two" \
    'scripts/macos-bundle.sh join-dsyms "$tmp/dsym-arm" "$tmp/dsym-intel" "$tmp/dsym-joined" 2>/dev/null'
  check "join-dsyms: the joined dSYMs match the universal binaries" \
    'scripts/macos-bundle.sh dsyms "$dsym_app" "$tmp/dsym-joined" >/dev/null 2>&1'
  for name in hive hive-app; do
    check "join-dsyms: $name holds both architectures" \
      '[ "$(dwarfdump --uuid "$tmp/dsym-joined/$name.dSYM" | wc -l)" -eq 2 ]'
  done
  check "join-dsyms: one architecture's dSYM does not match the universal binaries" \
    '! scripts/macos-bundle.sh dsyms "$dsym_app" "$tmp/dsym-arm" >/dev/null 2>&1'
  check "join-dsyms: an existing folder fails" \
    '! scripts/macos-bundle.sh join-dsyms "$tmp/dsym-arm" "$tmp/dsym-intel" "$tmp/dsym-joined" 2>/dev/null'
  check "join-dsyms: a missing dSYM fails" \
    '! scripts/macos-bundle.sh join-dsyms "$tmp/dsym-arm" "$tmp/no-such" "$tmp/dsym-none" 2>/dev/null'
  check "join-dsyms: the same architecture twice fails" \
    '! scripts/macos-bundle.sh join-dsyms "$tmp/dsym-arm" "$tmp/dsym-arm" "$tmp/dsym-same" 2>/dev/null'

  # --- macos-bundle.sh smoke -----------------------------------------------------------------------
  # A fake `arch`: this Mac may have no Rosetta, and a fake hive and app show what each slice does.
  smoke_app=$tmp/Smoke.app
  mkdir -p "$smoke_app/Contents/MacOS" "$smoke_app/Contents/Resources" "$tmp/fake-arch"
  plutil -create xml1 "$smoke_app/Contents/Info.plist"
  plutil -insert CFBundleExecutable -string smoke-app "$smoke_app/Contents/Info.plist"
  cat >"$tmp/fake-arch/arch" <<'EOF'
#!/bin/sh
echo "$1 $2" >>"$FAKE_ARCH_LOG"
FAKE_ARCH=$1
export FAKE_ARCH
shift
exec "$@"
EOF
  cat >"$smoke_app/Contents/Resources/hive" <<'EOF'
#!/bin/sh
[ "$FAKE_ARCH" != "$FAKE_BROKEN" ] || exit 1
[ "$FAKE_ARCH" != "$FAKE_OLDER" ] || { echo "hive 0.0.1"; exit 0; }
[ "$FAKE_ARCH" != "$FAKE_OTHER" ] || { echo "not it"; exit 0; }
echo "hive 1.2.3"
EOF
  cat >"$smoke_app/Contents/MacOS/smoke-app" <<'EOF'
#!/bin/sh
echo "$HOME" >"$FAKE_APP_HOME"
[ -z "$FAKE_APP_ENDS" ] || exit 1
exec sleep 30
EOF
  chmod +x "$tmp/fake-arch/arch" "$smoke_app/Contents/Resources/hive" "$smoke_app/Contents/MacOS/smoke-app"
  export FAKE_ARCH_LOG=$tmp/arch.log FAKE_APP_HOME=$tmp/app.home SMOKE_APP_TICKS=2
  smoke_run() { env PATH="$tmp/fake-arch:$PATH" GITHUB_ACTIONS=1 "$@" scripts/macos-bundle.sh smoke "$smoke_app" 2>&1; }
  check "smoke: both slices and the Intel app pass" 'smoke_run >/dev/null'
  check "smoke: it runs hive in both architectures, then the app in x86_64" \
    '[ "$(tr "\n" " " <"$FAKE_ARCH_LOG")" = "-arm64 $smoke_app/Contents/Resources/hive -x86_64 $smoke_app/Contents/Resources/hive -x86_64 $smoke_app/Contents/MacOS/smoke-app " ]'
  check "smoke: the app has a temporary HOME, gone after" \
    '[ "$(cat "$FAKE_APP_HOME")" != "$HOME" ] && [ ! -e "$(cat "$FAKE_APP_HOME")" ]'
  check "smoke: outside CI it refuses" \
    '[[ $(env -u GITHUB_ACTIONS PATH="$tmp/fake-arch:$PATH" scripts/macos-bundle.sh smoke "$smoke_app" 2>&1) == *"CI only"* ]]'
  check "smoke: a hive that does not run in x86_64 fails, naming Rosetta" \
    '[[ $(smoke_run FAKE_BROKEN=-x86_64) == *"x86_64 hive does not run"*Rosetta* ]]'
  check "smoke: a hive that does not run in arm64 fails" '! smoke_run FAKE_BROKEN=-arm64 >/dev/null'
  check "smoke: slices of different versions fail" '[[ $(smoke_run FAKE_OLDER=-x86_64) == *"slices disagree"* ]]'
  check "smoke: another answer fails" '[[ $(smoke_run FAKE_OTHER=-x86_64) == *"answers: not it"* ]]'
  check "smoke: an app that ends at once fails" '[[ $(smoke_run FAKE_APP_ENDS=1) == *"x86_64 app ended"* ]]'
  check "smoke: the ended app's HOME is gone too" '[ ! -e "$(cat "$FAKE_APP_HOME")" ]'
  unset FAKE_ARCH_LOG FAKE_APP_HOME SMOKE_APP_TICKS

  # --- macos-bundle.sh new-certificate ------------------------------------------------------------
  sha1=$(umask 022; scripts/macos-bundle.sh new-certificate "$tmp/certificate" 2>/dev/null)
  check "certificate: it prints a SHA-1" '[[ $sha1 =~ ^[0-9A-F]{40}$ ]]'
  check "certificate: only the .p12 and its password stay, in a private folder" \
    '[ "$(ls "$tmp/certificate" | xargs)" = "certificate.p12 password" ] && [ "$(stat -f %Lp "$tmp/certificate")" = 700 ]'
  check "certificate: the .p12 and its password are private, whatever the umask" \
    '[ "$(stat -f %Lp "$tmp/certificate/certificate.p12")" = 600 ] && [ "$(stat -f %Lp "$tmp/certificate/password")" = 600 ]'
  shown=$(/usr/bin/openssl pkcs12 -in "$tmp/certificate/certificate.p12" -nokeys \
    -passin "file:$tmp/certificate/password" 2>/dev/null |
    /usr/bin/openssl x509 -noout -fingerprint -sha1 -subject -text 2>/dev/null)
  check "certificate: the .p12 holds that certificate, for code signing" \
    'holds "${shown//:/}" "$sha1" "Hive Self-Signed" "Code Signing"'
  check "certificate: an existing folder fails" \
    '! scripts/macos-bundle.sh new-certificate "$tmp/certificate" 2>/dev/null'
  check "certificate: an existing folder keeps its files" \
    '[ "$(ls "$tmp/certificate" | xargs)" = "certificate.p12 password" ]'
  # A failure after openssl wrote the unencrypted key: uuidgen is the next step, and can be replaced.
  mkdir -p "$tmp/no-uuidgen"
  printf '#!/bin/sh\nexit 1\n' >"$tmp/no-uuidgen/uuidgen"
  chmod +x "$tmp/no-uuidgen/uuidgen"
  check "certificate: a failure fails" \
    '! PATH=$tmp/no-uuidgen:$PATH scripts/macos-bundle.sh new-certificate "$tmp/failed" >/dev/null 2>&1'
  check "certificate: a failure leaves no key or certificate" \
    '[ -d "$tmp/failed" ] && [ ! -e "$tmp/failed/key.pem" ] && [ ! -e "$tmp/failed/certificate.pem" ]'
  check "certificate: importing it outside CI fails, before any keychain" \
    '[[ $(env -u GITHUB_ACTIONS scripts/macos-bundle.sh import-certificate "$tmp/never" 2>&1) == *"CI only"* ]] && [ ! -e "$tmp/never" ]'

  # --- macos-bundle.sh sign-hive and check --self-signed -------------------------------------------
  # A real certificate signature needs a keychain on the search list (the real environment; CI's
  # is below): a wrapper signs ad-hoc whatever the identity, keeping the other options and logging
  # them, and reports each requirement as that of the certificate in $FAKE_SHA1 (for the nested
  # hive, $FAKE_HIVE_SHA1 if set).
  mkdir -p "$tmp/selfsign"
  cat >"$tmp/selfsign/codesign" <<'EOF'
#!/bin/bash
args=("$@")
case " $* " in
  *" -d -r- "*)
    path=${args[${#args[@]} - 1]}
    case $path in
      */Resources/hive) id=com.lucas.hive.service sha1=${FAKE_HIVE_SHA1:-$FAKE_SHA1} ;;
      *) id=${FAKE_ID:-com.example.fake} sha1=$FAKE_SHA1 ;;
    esac
    echo "Executable=$path" >&2
    echo "designated => identifier \"$id\" and certificate root = H\"$sha1\""
    exit 0
    ;;
  *" --sign "*)
    echo "$*" >>"$FAKE_LOG"
    for i in "${!args[@]}"; do [ "${args[$i]}" != --sign ] || args[i + 1]=-; done
    ;;
esac
exec /usr/bin/codesign "${args[@]}"
EOF
  chmod +x "$tmp/selfsign/codesign"
  lower=$(tr 'A-F' 'a-f' <<<"$sha1")
  other=0123456789012345678901234567890123456789
  export FAKE_LOG=$tmp/selfsign/log FAKE_SHA1=$lower
  saved_path=$PATH
  export PATH=$tmp/selfsign:$PATH
  HIVE_CERTIFICATE_SHA1=$sha1 scripts/macos-bundle.sh sign-hive "$app/Contents/Resources/hive" 2>/dev/null
  check "sign-hive: it signs with the certificate, the runtime and no timestamp" \
    'holds "$(cat "$FAKE_LOG")" "--sign $sha1 --identifier com.lucas.hive.service --options runtime --timestamp=none"'
  : >"$FAKE_LOG"
  APPLE_SIGNING_IDENTITY=dev HIVE_CERTIFICATE_SHA1=$sha1 \
    scripts/macos-bundle.sh sign-hive "$app/Contents/Resources/hive" 2>/dev/null
  check "sign-hive: a Developer ID comes first, with a secure timestamp" \
    '[[ $(cat "$FAKE_LOG") == *"--sign dev --identifier com.lucas.hive.service --options runtime --timestamp "* ]]'
  HIVE_CERTIFICATE_SHA1=$sha1 scripts/macos-bundle.sh sign-hive "$app/Contents/Resources/hive" 2>/dev/null
  seal --entitlements src-tauri/Entitlements.plist
  check "self-signed: this certificate's bundle passes" 'bundle_check --self-signed "$sha1"'
  check "self-signed: its SHA-1 may be lowercase" 'bundle_check --self-signed "$lower"'
  check "self-signed: another certificate fails" '! bundle_check --self-signed "$other"'
  check "self-signed: no SHA-1 fails" '! bundle_check --self-signed'
  check "self-signed: a SHA-1 of another shape fails" '! bundle_check --self-signed "abc.*"'
  check "self-signed: an extra word after a mode fails" '! bundle_check --developer-id "$sha1"'
  check "self-signed: a nested hive under another certificate fails" \
    '! FAKE_HIVE_SHA1=$other bundle_check --self-signed "$sha1"'
  check "self-signed: an app of another identifier fails" \
    '! FAKE_ID=com.example.other bundle_check --self-signed "$sha1"'
  codesign --force --sign - --entitlements src-tauri/Entitlements.plist "$app" 2>/dev/null
  check "self-signed: an app without the hardened runtime fails" '! bundle_check --self-signed "$sha1"'
  seal --entitlements src-tauri/Entitlements.plist
  export PATH=$saved_path
  check "self-signed: an ad-hoc signature (a cdhash) fails" '! bundle_check --self-signed "$sha1"'
  new_hive
  scripts/macos-bundle.sh sign-hive "$app/Contents/Resources/hive" 2>/dev/null
  seal --entitlements src-tauri/Entitlements.plist
  export PATH=$tmp/selfsign:$PATH
  check "self-signed: a nested hive without the hardened runtime fails" \
    '[[ $(scripts/macos-bundle.sh check "$app" --self-signed "$sha1" 2>&1) == *"hardened runtime"* ]]'
  export PATH=$saved_path

  # The real thing, on a CI runner only, whose keychain search list is disposable: the certificate
  # new-certificate made, imported as the release does, signs the bundle; unlike the wrapper's, this
  # requirement is what macOS will compare its grants with. The app is signed under the runtime as
  # Tauri does (`codesign --force -s <identity> --options runtime --entitlements`), and it still runs.
  if [ -n "${GITHUB_ACTIONS:-}" ] && [ -n "${REQUIRE_REAL_CERTIFICATE:-}" ]; then
    saved_home=$HOME
    export HOME=$real_home
    keychain=$tmp/self-signed.keychain-db
    SIGNING_CERTIFICATE=$(base64 -i "$tmp/certificate/certificate.p12") \
      SIGNING_CERTIFICATE_PASSWORD=$(cat "$tmp/certificate/password") \
      scripts/macos-bundle.sh import-certificate "$keychain" >/dev/null 2>&1
    check "real certificate: it is imported" 'security find-identity -p codesigning "$keychain" | grep -q "$sha1"'
    HIVE_CERTIFICATE_SHA1=$sha1 scripts/macos-bundle.sh sign-hive "$app/Contents/Resources/hive"
    codesign --force -s "$sha1" --options runtime --entitlements src-tauri/Entitlements.plist "$app"
    check "real certificate: the bundle passes" \
      'scripts/macos-bundle.sh check "$app" --self-signed "$sha1"'
    check "real certificate: another one fails" '! bundle_check --self-signed "$other"'
    check "real certificate: the requirement names it, not a cdhash" \
      'holds "$(codesign -d -r- "$app" 2>&1)" "identifier \"com.example.fake\"" "H\"$lower\""'
    check "real certificate: the signed app and hive run" \
      '"$app/Contents/MacOS/fake" && "$app/Contents/Resources/hive"'
    security delete-keychain "$keychain"
    export HOME=$saved_home
  elif [ -n "${REQUIRE_REAL_CERTIFICATE:-}" ]; then
    check "real certificate: required, but this is no CI runner" 'false'
  else
    echo "skip  real certificate: a CI runner only (it needs the keychain search list)"
  fi
fi

[ $bad = 0 ]
