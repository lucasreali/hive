#!/bin/sh
# Push the current task branch and follow its CI, with the personal account's token (CLAUDE.md
# rule 4): the active gh account is the work one and is never switched. Push, watch and dispatch
# only on task/* branches.
#   scripts/ci.sh push          push the branch (sets upstream)
#   scripts/ci.sh watch [--all] wait for HEAD's ci and macos runs, the two a merge needs (Windows
#                               is frozen, D17.0; --all adds windows); exit 1 if one failed
#   scripts/ci.sh green [<rev>] print the green `ci` and `macos` runs of a commit (default HEAD,
#                               any branch: /release checks main's HEAD); exit 1 if one is missing
#   scripts/ci.sh dispatch <workflow file>  a manual run of a workflow on this branch's pushed tip
#                               (e.g. release.yml's dry run, which stops at ci-green unless ci and
#                               macos are green there)
#   scripts/ci.sh runs [<rev>]  every run of a commit (default HEAD, any branch)
#   scripts/ci.sh logs <run-id> the failed steps' logs (empty for a reusable workflow's jobs,
#                               "mutants / …": gh 2.46; use `jobs` then `log`)
#   scripts/ci.sh cancel <run-id> cancel a run (e.g. one with a hung job, whose log GitHub
#                               gives only once it ends)
#   scripts/ci.sh jobs <run-id> the run's minutes, then each job's id, result, start, minutes, name
#   scripts/ci.sh log <job-id>  one job's full log (e.g. the mutants gather counts)
#   scripts/ci.sh survivors     the open nightly mutation testing issue, if any, with its comments
set -eu
branch=$(git rev-parse --abbrev-ref HEAD)
case "${1:-}:$branch" in
  push:task/* | watch:task/* | dispatch:task/*) ;;
  push:* | watch:* | dispatch:*) echo "ci.sh: only task/* branches (on $branch)" >&2; exit 2 ;;
esac
GH_TOKEN=$(gh auth token -u lucasreali)
export GH_TOKEN

# HEAD's runs on this branch, one per line: workflow name, id, url.
runs() {
  gh run list --branch "$branch" --commit "$(git rev-parse HEAD)" --json workflowName,databaseId,url \
    -q '.[] | "\(.workflowName) \(.databaseId) \(.url)"'
}

case "${1:-}" in
  push) git push -u origin "$branch" ;;
  watch)
    case "${2:-}" in
      "") want="ci macos" ;;
      --all) want="ci macos windows" ;;
      *) echo "usage: scripts/ci.sh watch [--all]" >&2; exit 2 ;;
    esac
    # The runs of the wanted workflows, as `runs` prints them.
    wanted() {
      runs | while read -r name id url; do
        case " $want " in *" $name "*) echo "$name $id $url" ;; esac
      done
    }
    # Runs show up a few seconds after the push: wait until each wanted workflow has one.
    tries=0
    while [ "$(wanted | cut -d' ' -f1 | sort -u | wc -l)" -lt "$(echo $want | wc -w)" ] &&
      [ "$tries" -lt 30 ]; do
      tries=$((tries + 1))
      sleep "${CI_SH_WAIT:-20}"
    done
    # A workflow still without a run fails (the push may not have reached it); the rest are watched.
    failed=0
    for name in $want; do
      wanted | grep -q "^$name " || { echo "MISSING $name: no run of HEAD"; failed=1; }
    done
    wanted | while read -r name id url; do
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
  green)
    # A release needs ci and macos green (D18.1; Windows is optional, D17.0), as release.yml's
    # ci-green job checks, which also accepts a version bump's parent.
    rev=$(git rev-parse "${2:-HEAD}")
    for workflow in ci macos; do
      url=$(gh run list --workflow "$workflow.yml" --commit "$rev" --status success \
        --json url -q '.[0].url // empty')
      [ -n "$url" ] || { echo "no green $workflow run for ${2:-HEAD}" >&2; exit 1; }
      echo "green $url"
    done
    ;;
  dispatch) gh workflow run "${2:?workflow file}" --ref "$branch" ;;
  runs)
    gh run list --commit "$(git rev-parse "${2:-HEAD}")" --json databaseId,workflowName,conclusion,url \
      -q '.[] | "\(.databaseId) \(.workflowName) \(.conclusion) \(.url)"'
    ;;
  logs) gh run view "${2:?run id}" --log-failed ;;
  cancel) gh run cancel "${2:?run id}" ;;
  jobs)
    # Minutes: the run's wall-clock, then each job's start (from the run's) and duration.
    gh run view "${2:?run id}" --json createdAt,updatedAt,jobs -q '(.createdAt | fromdate) as $t0
      | "run \(((.updatedAt | fromdate) - $t0) / 60 | floor)min",
        (.jobs[] | if .status != "completed" then "\(.databaseId) \(.status) \(.name)" else
          "\(.databaseId) \(.conclusion) +\(((.startedAt | fromdate) - $t0) / 60 | floor) \(((.completedAt | fromdate) - (.startedAt | fromdate)) / 60 | floor)min \(.name)" end)'
    ;;
  log) gh api --allow-escape-sequences "repos/{owner}/{repo}/actions/jobs/${2:?job id}/logs" ;;
  survivors)
    # The issue mutants-full.yml keeps open for surviving mutants: one comment per failed night.
    n=$(gh issue list --state open --search 'in:title "Nightly mutation testing: surviving mutants"' \
      --json number -q '.[0].number // empty')
    # --json: gh 2.46's plain view asks for the retired Projects (classic) and fails.
    [ -z "$n" ] || gh issue view "$n" --json url,body,comments -q '.url, .body, (.comments[] | .body)'
    ;;
  *) echo "usage: scripts/ci.sh push | watch [--all] | green [<rev>] | dispatch <workflow file> | runs [<rev>] | logs <run-id> | cancel <run-id> | jobs <run-id> | log <job-id> | survivors" >&2; exit 2 ;;
esac
