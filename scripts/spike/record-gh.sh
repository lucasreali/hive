#!/bin/sh
# Records the shapes of the gh output that Hive parses (TODO 9.30-9.32) into /var/tmp/hive-gh-rec/.
# Read-only commands only, on lucasreali/hive, with the personal account's token as GH_TOKEN
# (never `gh auth switch`, never the work account). gh runs with an empty temporary GH_CONFIG_DIR,
# so the human's gh config is neither read nor written. About 11 API calls in all; a second run
# only records what is missing. Needs python3 (to pick ids out of the JSON).
# Run from a checkout of this repository: scripts/spike/record-gh.sh
set -eu
# The recordings may hold a private repository's data: readable by this user only.
umask 077
out=/var/tmp/hive-gh-rec
repo=lucasreali/hive
mkdir -p "$out"
GH_TOKEN=$(gh auth token -u lucasreali)
export GH_TOKEN
GH_CONFIG_DIR=$(mktemp -d)
export GH_CONFIG_DIR GH_PROMPT_DISABLED=1 NO_COLOR=1 GH_PAGER=cat
trap 'rm -rf "$GH_CONFIG_DIR"' EXIT

# Each command's stdout, stderr and exit code, side by side; a step already recorded is kept.
rec() {
  name=$1
  shift
  if [ -f "$out/$name.code" ]; then echo "$name: kept"; return; fi
  code=0
  "$@" >"$out/$name.out" 2>"$out/$name.err" || code=$?
  echo "$code" >"$out/$name.code"
  echo "$name: exit $code"
}

# A field of the first item of a recorded JSON list (empty when the list is).
first() {
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d[0][sys.argv[2]] if d else "")' "$@"
}

rec version gh --version

# No account at all (no network): what "not logged in" looks like.
rec auth-status-none env -u GH_TOKEN gh auth status

# The GH_TOKEN account, plus a second login whose token is invalid (a made-up token, so the
# failure shape is recorded without using another account's quota).
cat >"$GH_CONFIG_DIR/hosts.yml" <<'EOF'
github.com:
    users:
        hive-fake-login:
            oauth_token: gho_hiveRecordingNotARealToken000000000
    git_protocol: https
    user: hive-fake-login
    oauth_token: gho_hiveRecordingNotARealToken000000000
EOF
rec auth-status gh auth status
rec auth-status-hostname gh auth status --hostname github.com
rm "$GH_CONFIG_DIR/hosts.yml"

rec repo-view gh repo view "$repo" --json nameWithOwner,url,defaultBranchRef,mergeCommitAllowed,squashMergeAllowed,rebaseMergeAllowed,viewerDefaultMergeMethod

pr_fields=number,title,url,state,isDraft,headRefName,baseRefName,author,reviewDecision,statusCheckRollup,updatedAt,mergedAt,closedAt,isCrossRepository,reviewRequests
rec pr-list gh pr list --repo "$repo" --state all --limit 5 --json "$pr_fields"
pr=$(first "$out/pr-list.out" number)
if [ -n "$pr" ]; then
  rec pr-view gh pr view "$pr" --repo "$repo" --json "$pr_fields,body,reviews,comments,files,additions,deletions,changedFiles,mergeable,mergeStateStatus"
fi

run_fields=databaseId,number,name,workflowName,displayTitle,headBranch,headSha,event,status,conclusion,createdAt,startedAt,updatedAt,url
rec run-list gh run list --repo "$repo" --limit 5 --json "$run_fields"
if [ ! -f "$out/run-view.code" ]; then
  rec run-failed gh run list --repo "$repo" --status failure --limit 1 --json databaseId
  run=$(first "$out/run-failed.out" databaseId)
  [ -z "$run" ] || rec run-view gh run view "$run" --repo "$repo" --json "$run_fields,jobs"
fi
if [ -f "$out/run-view.out" ] && [ ! -f "$out/job-log.out" ]; then
  job=$(python3 -c 'import json,sys; j=[j for j in json.load(open(sys.argv[1]))["jobs"] if j["conclusion"]=="failure"]; print(j[0]["databaseId"] if j else "")' "$out/run-view.out")
  if [ -n "$job" ]; then
    # The tail only: logs can be megabytes.
    gh api "repos/$repo/actions/jobs/$job/logs" 2>"$out/job-log.err" | tail -c 65536 >"$out/job-log.out" || true
    echo "job-log: $(wc -c <"$out/job-log.out") bytes"
  fi
fi

# gh prints a token only with --show-token, never used here; check anyway before sharing.
grep -rlE 'gh[opsu]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{20,}' "$out" && echo "WARNING: a token-like string is in the files above" || true
echo "recorded into $out"
