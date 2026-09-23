#!/bin/sh
# Notes a moment to correlate with the hook records, e.g. `mark.sh pressed Esc during a tool`.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$root/target/spike"
printf '%s\t%s\n' "$(date +%s%3N)" "$*" >> "$root/target/spike/marks.tsv"
