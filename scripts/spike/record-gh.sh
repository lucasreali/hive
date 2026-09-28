#!/bin/sh
# Records the shapes of the gh output that Hive parses (TODO 9.30-9.32) into /var/tmp/hive-gh-rec/.
# Read-only commands only, on lucasreali/hive (and cli/cli for pull requests), with the personal account's token as GH_TOKEN
# (never `gh auth switch`, never the work account). gh runs with an empty temporary GH_CONFIG_DIR,
# so the human's gh config is neither read nor written. About 14 API calls in all; a second run
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

# Pull request shapes (9.31) on a busy public repository, since lucasreali/hive has none: Hive's
# own list query (crates/hive/src/pulls.graphql) with searches that match without `@me`, one
# pull request's details, and the error for a repository that does not exist. Three calls.
public=cli/cli
query=$(cat crates/hive/src/pulls.graphql)
rec pr-search gh api graphql -f query="$query" -F owner=cli -F name=cli -F first=3 \
  -f mine="repo:$public is:pr is:merged sort:updated-desc" \
  -f review="repo:$public is:pr is:open sort:updated-desc"
rec pr-search-missing gh api graphql -f query="$query" -F owner=cli -F name=hive-no-such-repo \
  -F first=1 -f mine="repo:cli/hive-no-such-repo is:pr" -f review="repo:cli/hive-no-such-repo is:pr"
if [ ! -f "$out/pr-view-public.code" ]; then
  pr=$(python3 -c 'import json,sys; n=json.load(open(sys.argv[1]))["data"]["mine"]["nodes"]; print(n[0]["number"] if n else "")' "$out/pr-search.out")
  [ -z "$pr" ] || rec pr-view-public gh pr view "$pr" --repo "$public" --json number,title,body,url,state,isDraft,isCrossRepository,author,headRefName,headRefOid,baseRefName,reviewDecision,reviews,comments,statusCheckRollup,files,additions,deletions,mergeable,updatedAt
fi

# The pull request fixtures (crates/hive/tests/fixtures/gh/), scrubbed: people become octo-1…,
# ids and names go, texts are short stand-ins (one keeps an HTML comment, a link and an image,
# for the untrusted-input rules), at most 3 checks; the shapes stay as recorded.
python3 - "$out" <<'EOF'
import json, sys
out = sys.argv[1]
people = {}
def scrub(value):
    if isinstance(value, dict):
        value = {k: v for k, v in value.items() if k != "id"}
        if "login" in value:
            value.pop("name", None)
            value["login"] = people.setdefault(value["login"], f"octo-{len(people) + 1}")
        return {k: scrub(v) for k, v in value.items()}
    if isinstance(value, list):
        return [scrub(v) for v in value]
    return value
search = scrub(json.load(open(f"{out}/pr-search.out")))
view = scrub(json.load(open(f"{out}/pr-view-public.out")))
view["body"] = "<!-- a template comment -->\n### Description\n\nFixes [the bug](https://github.com/cli/cli/issues/1). ![shot](https://example.com/a.png)\n"
for i, item in enumerate(view["reviews"] + view["comments"]):
    item["body"] = f"Text {i + 1} with `code`." if item["body"] else ""
view["statusCheckRollup"] = view["statusCheckRollup"][:3]
for name, data in (("pr-search", search), ("pr-view", view)):
    with open(f"{out}/{name}.json", "w") as f:
        json.dump(data, f, indent=1)
        f.write("\n")
EOF

# gh prints a token only with --show-token, never used here; check anyway before sharing.
grep -rlE 'gh[opsu]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{20,}' "$out" && echo "WARNING: a token-like string is in the files above" || true
echo "recorded into $out"
