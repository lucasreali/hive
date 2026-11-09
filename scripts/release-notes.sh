#!/usr/bin/env bash
# The notes of a release (TODO 26.6, D18.8), for the GitHub release and `latest.json`, printed from
# the checkout of the commit being released, with the earlier releases' notes on stdin:
#   release-notes.sh <tag> <signing mode>
# In order: the one note of the first self-signed release (macos-bundle.sh release-notes, 25.5), the
# Conventional Commits since the previous tag, the first-launch link.
#
# - The previous tag is the closest `v*` tag reachable from HEAD other than <tag>. With none, every
#   commit counts. A shallow clone stops here: its history would give wrong notes.
# - Listed: `feat`, `fix`, `perf`, newest first, as "**scope:** text" when there is a scope. A commit
#   of any type marked breaking (`!` or a `BREAKING CHANGE:` line) goes under "Breaking changes"
#   instead. Types are lowercase, as this repository writes them.
# - Left out: every other type, anything else that is not a Conventional Commit, merge commits (the
#   commits they bring are read on their own), `Revert "<subject>"` with the commit it reverts when
#   that is in the range, and `Reapply "<subject>"` with the revert it undoes.
# - Commit text is untrusted: it is only ever data here (never evaluated, control characters
#   removed), and the caller passes it on with `jq --arg`. Whole lines past MAX_BYTES are left out
#   and counted, since GitHub takes a release body of 125000 characters and Linux one argument of
#   128 KiB.
set -euo pipefail

die() { echo "release-notes: $*" >&2; exit 1; }
[ $# = 2 ] || die "usage: $0 <tag> <signing mode>"
tag=$1 mode=$2

MAX_BYTES=100000
FIRST_LAUNCH="First install on a Mac: follow [the first-launch steps](https://github.com/lucasreali/hive#macos)."

shallow=$(git rev-parse --is-shallow-repository)
[ "$shallow" = false ] || die "a shallow clone has no previous tag to compare with (actions/checkout: fetch-depth: 0)"

# describe fails when there is no such tag: the first release.
previous=$(git describe --tags --abbrev=0 --match 'v[0-9]*' --exclude "$tag" HEAD 2>/dev/null || true)
range=${previous:+$previous..}HEAD

breaking=$(git log --no-merges --format=%H -E --grep='^BREAKING[ -]CHANGE: ' "$range" | tr '\n' ' ')

# LC_ALL=C: lengths in bytes, and any bytes a subject holds are fine.
changes=$(git log --no-merges --format='%H %s' "$range" | LC_ALL=C tr -d '\001-\011\013-\037\177' |
  LC_ALL=C awk -v breaking=" $breaking " -v max="$MAX_BYTES" '
    {
      hash = $1
      subject = substr($0, length(hash) + 2)
      # Newest first, so a revert is read before what it reverts.
      if (undone[subject] > 0) { undone[subject]--; next }
      if (subject ~ /^Revert ".*"$/) { undone[substr(subject, 9, length(subject) - 9)]++; next }
      if (subject ~ /^Reapply ".*"$/) { undone["Revert \"" substr(subject, 10, length(subject) - 10) "\""]++; next }
      if (!match(subject, /^[a-z]+(\([^)]+\))?!?: /)) next
      head = substr(subject, 1, RLENGTH)
      text = substr(subject, RLENGTH + 1)
      type = head
      sub(/[(!:].*/, "", type)
      scope = ""
      if (head ~ /\(/) {
        scope = head
        sub(/^[^(]*\(/, "", scope)
        sub(/\)[^)]*$/, "", scope)
      }
      section = type
      if (head ~ /!: $/ || index(breaking, " " hash " ")) section = "breaking"
      count[section]++
      entries[section, count[section]] = "- " (scope == "" ? "" : "**" scope ":** ") text
    }
    END {
      names = split("breaking feat fix perf", name, " ")
      title["breaking"] = "Breaking changes"
      title["feat"] = "Features"
      title["fix"] = "Fixes"
      title["perf"] = "Performance"
      for (s = 1; s <= names; s++) {
        section = name[s]
        headed = 0
        for (i = 1; i <= count[section]; i++) {
          size = length(entries[section, i]) + 1
          if (used + size > max) { left_out++; continue }
          used += size
          if (!headed) {
            print gap "## " title[section] "\n"
            gap = "\n"
            headed = 1
          }
          print entries[section, i]
        }
      }
      if (left_out) print gap "And " left_out " more, left out for length."
    }')

once=$("$(dirname "$0")/macos-bundle.sh" release-notes "$mode")

gap=
for part in "$once" "$changes" "$FIRST_LAUNCH"; do
  [ -n "$part" ] || continue
  printf '%s%s\n' "$gap" "$part"
  gap=$'\n'
done
