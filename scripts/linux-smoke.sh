#!/usr/bin/env bash
# The remote Linux `hive` (TODO 26.13) runs where it will run: linux-remote.yml runs this in an
# old-glibc container (debian:10), on the build's own architecture.
#
#   linux-smoke.sh <hive> <version>
#       `hive --version` prints "hive <version>"; then `hive daemon`, with a temporary HOME and
#       runtime folder, starts, accepts a connection on its socket, and ends with status 0 on
#       SIGTERM, its socket gone.
#
# Needs bash and perl (Debian's perl-base has IO::Socket::UNIX; nothing else is in the image).
# Every wait is bounded: SMOKE_TICKS tenths of a second (default 300).
set -euo pipefail

die() { echo "linux-smoke: $*" >&2; exit 1; }

SMOKE_TICKS=${SMOKE_TICKS:-300}

[ $# = 2 ] || die "usage: $0 <hive> <version>"
hive=$1 version=$2
[ -f "$hive" ] && [ -x "$hive" ] || die "not an executable: $hive"

out=$("$hive" --version) || die "$hive --version failed"
[ "$out" = "hive $version" ] || die "$hive --version prints '$out', not 'hive $version'"
echo "ok    $out"

tmp=$(mktemp -d)
pid=
# A daemon this script started never outlives it.
trap '[ -z "$pid" ] || kill -KILL "$pid" 2>/dev/null; rm -rf "$tmp"' EXIT
mkdir -m 700 "$tmp/home" "$tmp/run"
socket=$tmp/run/hive/hive.sock
log() { cat "$tmp/daemon.log" >&2; }

HOME=$tmp/home XDG_RUNTIME_DIR=$tmp/run "$hive" daemon 2>"$tmp/daemon.log" &
pid=$!

# A connection the daemon accepts, then closed without a word: no app, so it keeps serving.
connects() { perl -MIO::Socket::UNIX -e 'IO::Socket::UNIX->new(Peer => $ARGV[0]) or exit 1' "$socket" 2>/dev/null; }
tick=0
until connects; do
  kill -0 "$pid" 2>/dev/null || { log; die "the daemon ended before it listened"; }
  tick=$((tick + 1))
  [ "$tick" -lt "$SMOKE_TICKS" ] || { log; die "the daemon did not listen on $socket"; }
  sleep 0.1
done
echo "ok    the daemon listens"

kill -TERM "$pid"
tick=0
while kill -0 "$pid" 2>/dev/null; do
  tick=$((tick + 1))
  [ "$tick" -lt "$SMOKE_TICKS" ] || { log; die "the daemon still runs after SIGTERM"; }
  sleep 0.1
done
status=0
wait "$pid" || status=$?
pid=
[ "$status" = 0 ] || { log; die "the daemon ended with status $status on SIGTERM"; }
[ ! -e "$socket" ] || die "the daemon left its socket"
echo "ok    the daemon ends on SIGTERM"
