#!/bin/sh
# Longest pauses in a recorded terminal session (question 2: does output keep flowing
# during a long tool call?). Usage: gaps.sh /tmp/hive-spike/logs/<run>.timing
set -eu
awk '{ t += $1; if ($1 > 0.5) printf "%8.2fs gap ending at %8.2fs\n", $1, t }' "${1:?timing file}" | sort -rn | head -15
