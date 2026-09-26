#!/bin/sh
# Push the current task branch and follow its CI, with the personal account's token (CLAUDE.md
# rule 4): the active gh account is the work one and is never switched. Only task/* branches.
#   scripts/ci.sh push          push the branch (sets upstream)
#   scripts/ci.sh watch         wait for every run of HEAD (ci, macos); exit 1 if one failed
#   scripts/ci.sh logs <run-id> the failed steps' logs
#   scripts/ci.sh log <job-id>  one job's full log (e.g. the mutants gather counts)
set -eu
branch=$(git rev-parse --abbrev-ref HEAD)
case "$branch" in
  task/*) ;;
  *) echo "ci.sh: only task/* branches (on $branch)" >&2; exit 2 ;;
esac
GH_TOKEN=$(gh auth token -u lucasreali)
export GH_TOKEN

runs() {
  gh run list --branch "$branch" --commit "$(git rev-parse HEAD)" --json databaseId,url \
    -q '.[] | "\(.databaseId) \(.url)"'
}

case "${1:-}" in
  push) git push -u origin "$branch" ;;
  watch)
    # Runs show up a few seconds after the push; ci and macos both run on task/*.
    tries=0
    while [ "$(runs | wc -l)" -lt 2 ] && [ "$tries" -lt 30 ]; do
      tries=$((tries + 1))
      sleep 20
    done
    failed=0
    runs | while read -r id url; do
      # Poll every 2 min: the gh API quota is shared with the human's other work.
      if gh run watch "$id" --interval 120 --exit-status >/dev/null; then
        echo "green $url"
      else
        echo "FAILED $url (scripts/ci.sh logs $id)"
        exit 1
      fi
    done || failed=1
    exit "$failed"
    ;;
  logs) gh run view "${2:?run id}" --log-failed ;;
  log) gh api "repos/{owner}/{repo}/actions/jobs/${2:?job id}/logs" ;;
  *) echo "usage: scripts/ci.sh push | watch | logs <run-id> | log <job-id>" >&2; exit 2 ;;
esac
