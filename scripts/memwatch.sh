#!/usr/bin/env bash
# Memory watchdog for scripts/gates.sh: stops every process named <name> (exact name) when the
# available memory falls below <min-kib>, and appends why to <log>. Exits once no such process is
# left. Checks every MEMWATCH_INTERVAL seconds (default 2).
#   memwatch.sh <name> <min-kib> <log>
#   memwatch.sh --available        prints the available memory in KiB
# Linux: MemAvailable from /proc/meminfo. macOS: free, inactive and speculative pages from vm_stat
# (what can be reused without swapping; purgeable pages are left out, so it errs low).
set -u

available_kib() {
  case $(uname -s) in
    Darwin)
      vm_stat | awk '
        /page size of/ { size = $8 }
        /^Pages (free|inactive|speculative):/ { sub(/\.$/, "", $NF); pages += $NF }
        END { if (size > 0 && pages > 0) printf "%d\n", pages * size / 1024 }' ;;
    *) awk '/^MemAvailable:/ { print $2 }' /proc/meminfo ;;
  esac
}

if [ "${1:-}" = --available ]; then
  kib=$(available_kib)
  [[ $kib =~ ^[0-9]+$ ]] || { echo "memwatch: cannot read the available memory" >&2; exit 1; }
  echo "$kib"
  exit 0
fi

[ $# = 3 ] || { echo "usage: $0 <name> <min-kib> <log> | --available" >&2; exit 2; }
name=$1 min=$2 log=$3
while sleep "${MEMWATCH_INTERVAL:-2}"; do
  pgrep -x "$name" >/dev/null || exit 0  # -x: by name, never another shell's command line
  kib=$(available_kib)
  if ! [[ $kib =~ ^[0-9]+$ ]]; then
    # Never fail silently (it used to error every 2 s on macOS, where /proc does not exist).
    echo "gates: memory watchdog cannot read the available memory, $name stopped" >>"$log"
    pkill -x "$name"
    exit 1
  fi
  if [ "$kib" -lt "$min" ]; then
    echo "gates: memory low (${kib} KiB available), $name stopped" >>"$log"
    pkill -x "$name"
    exit 0
  fi
done
