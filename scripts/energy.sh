#!/usr/bin/env bash
# Energy baseline of a running Hive (TODO 19.2), macOS only:
#   scripts/energy.sh <seconds>
# Finds Hive's processes: the app (`hive-app`), the WebKit helpers it started (GPU, Networking,
# WebContent: launchd's children, matched by start time), the app's `hive bridge` and the
# `hive daemon` of the same binary. Samples them with `top` once a second for <seconds>, then prints
# per process and in total: average and max CPU, average energy impact (top's POWER), idle wakeups
# per second (the delta of top's IDLEW), `footprint` (now and peak) and open fds (`lsof`).
# No sudo, read-only: it never signals a process. Sourced, it only defines its functions
# (scripts/test-scripts.sh tests them).
set -uo pipefail

# Hive's processes from `ps -axo pid=,ppid=,etime=,args=` on stdin: "<pid> <role>" per line.
# shortcut: helpers started more than 2 s after the app (a WebContent relaunched after a crash) are
# missed; restart Hive before measuring if that matters.
find_procs() {
  awk '
    function secs(t,   d, n, p, i, s) {  # etime: [[dd-]hh:]mm:ss
      d = 0
      if (index(t, "-")) { d = substr(t, 1, index(t, "-") - 1); t = substr(t, index(t, "-") + 1) }
      n = split(t, p, ":"); s = 0
      for (i = 1; i <= n; i++) s = s * 60 + p[i]
      return d * 86400 + s
    }
    { n++; pid[n] = $1; ppid[n] = $2; age[n] = secs($3); exe[n] = $4; arg[n] = $5 }
    $4 ~ /(^|\/)hive-app$/ { app[$1] = age[n]; print $1, "hive-app" }
    END {
      for (i = 1; i <= n; i++)
        if (exe[i] ~ /(^|\/)hive$/ && arg[i] == "bridge" && ppid[i] in app) { bin[exe[i]] = 1; print pid[i], "bridge" }
      for (i = 1; i <= n; i++) {
        if (exe[i] in bin && arg[i] == "daemon") print pid[i], "daemon"
        if (exe[i] ~ /\/com\.apple\.WebKit\.[A-Za-z]+$/)
          for (a in app) {
            d = age[i] - app[a]
            if (d >= -2 && d <= 2) { sub(/.*\/com\.apple\./, "", exe[i]); print pid[i], exe[i]; break }
          }
      }
    }'
}

# `top -l <n> -s <interval> -stats pid,cpu,power,idlew` output on stdin: per pid
# "<pid> <cpu avg> <cpu max> <power avg> <idle wakeups/s>", then the same for "total" (its max is the
# highest per-sample sum). top's first sample has no CPU or POWER yet, so it only gives IDLEW's start.
parse_top() {
  awk -v interval="$1" '
    function count(v) {  # IDLEW: "12345+" (a count, maybe with K or M)
      m = 1
      if (v ~ /K/) m = 1000
      if (v ~ /M/) m = 1000000
      gsub(/[^0-9.]/, "", v)
      return v * m
    }
    /^PID[ \t]/ { s++; next }
    s && $1 ~ /^[0-9]+$/ && NF >= 4 {
      p = $1
      if (!(p in first)) { order[++np] = p; first[p] = count($4); at[p] = s }
      last[p] = count($4); upto[p] = s
      if (s > 1) {
        cpu[p] += $2; power[p] += $3; k[p]++
        if ($2 + 0 > max[p] + 0) max[p] = $2
        sum[s] += $2
      }
    }
    END {
      for (i = 1; i <= np; i++) {
        p = order[i]; c = k[p] ? cpu[p] / k[p] : 0; e = k[p] ? power[p] / k[p] : 0
        w = upto[p] > at[p] ? (last[p] - first[p]) / ((upto[p] - at[p]) * interval) : 0
        printf "%s %.1f %.1f %.1f %.1f\n", p, c, max[p] + 0, e, w
        tc += c; te += e; tw += w
      }
      for (j = 2; j <= s; j++) if (sum[j] > tm) tm = sum[j]
      printf "total %.1f %.1f %.1f %.1f\n", tc, tm + 0, te, tw
    }'
}

if [[ ${BASH_SOURCE[0]} != "$0" ]]; then return 0; fi

usage() { echo "usage: scripts/energy.sh <seconds>" >&2; exit 2; }
[ $# = 1 ] && [[ $1 =~ ^[1-9][0-9]{0,4}$ ]] || usage
[ "$(uname -s)" = Darwin ] || { echo "energy: macOS only" >&2; exit 1; }
seconds=$1

procs=$(ps -axo pid=,ppid=,etime=,args= | find_procs)
[ -n "$procs" ] || { echo "energy: Hive is not running" >&2; exit 1; }
pids=()
while read -r pid _; do pids+=(-pid "$pid"); done <<<"$procs"

echo "sampling $((${#pids[@]} / 2)) processes for $seconds s..." >&2
top_out=$(top -l $((seconds + 1)) -s 1 -stats pid,cpu,power,idlew "${pids[@]}")
stats=$(parse_top 1 <<<"$top_out")

mb() { awk -v b="$1" 'BEGIN { if (b == "") print "-"; else printf "%.0f MB\n", b / 1048576 }'; }
printf '%-20s %6s %8s %8s %7s %11s %10s %10s %5s\n' \
  process pid "cpu avg" "cpu max" energy "wakeups/s" footprint peak fds
total_fp=0 total_fds=0
while read -r pid role; do
  read -r _ cavg cmax eavg wps < <(grep "^$pid " <<<"$stats" || echo "$pid - - - -")
  fp=$(footprint -f bytes --noCategories -p "$pid" 2>/dev/null)
  now=$(awk '/phys_footprint:/ { print $2 }' <<<"$fp")
  peak=$(awk '/phys_footprint_peak:/ { print $2 }' <<<"$fp")
  # Numeric fds only (lsof also lists mapped files); a sandboxed WebKit helper shows few.
  fds=$(lsof -a -p "$pid" -d 0-999999 2>/dev/null | awk 'NR > 1' | wc -l | tr -d ' ')
  total_fp=$((total_fp + ${now:-0})) total_fds=$((total_fds + fds))
  printf '%-20s %6s %7s%% %7s%% %7s %11s %10s %10s %5s\n' \
    "$role" "$pid" "$cavg" "$cmax" "$eavg" "$wps" "$(mb "$now")" "$(mb "$peak")" "$fds"
done <<<"$procs"
read -r _ cavg cmax eavg wps < <(grep "^total " <<<"$stats")
printf '%-20s %6s %7s%% %7s%% %7s %11s %10s %10s %5s\n' \
  total - "$cavg" "$cmax" "$eavg" "$wps" "$(mb "$total_fp")" - "$total_fds"
