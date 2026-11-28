# Source of `semgrep`

- **Upstream:** https://github.com/trailofbits/skills, path `plugins/static-analysis/skills/semgrep/`
- **Commit:** `149d7b21f4067765b82e67029730b42d228ef4b4`
- **License:** CC-BY-SA-4.0
- **Review:** `docs/review/skills.md` (task 28.3); recommendation **Adapt**, approved by the human on 2026-11-27.
- **Copied:** Hive task 28.4. Fetched on 2026-11-27 as the pinned commit's tarball (`codeload.github.com/<repo>/tar.gz/<commit>`) into a new empty folder outside the repository (the agent's worktree isolation refused a `git clone` there); read as text, no script from it was run.

## What changed

- `SKILL.md` rewritten around a fixed ruleset (`p/rust`, `p/typescript`, `p/react`, `p/github-actions`, `p/secrets`) and one `semgrep scan` command with `--metrics=off`, output in a temp folder and a temporary `HOME`.
- Semgrep is a host tool: never installed by an agent; if missing, stop. How it could run in CI instead is written down for the human to decide.
- Dropped: third-party rule repositories (cloning), Decurity, Semgrep Pro, the approval-free `/static-analysis:semgrep-scan` workflow, `scripts/run-scans.sh`, `scripts/merge_sarif.py` (needs `uv`), `workflows/`, `references/rulesets.md`.
- `references/scan-modes.md`: kept the important-only pre-filter, metadata criteria and the single-file jq filter; dropped the sections about the scripts and third-party repos.

- `allowed-tools` removed (nothing pre-approves Bash).

## License notice

Copyright Trail of Bits (https://www.trailofbits.com/). The source repository is licensed under
the Creative Commons Attribution-ShareAlike 4.0 International License
(https://creativecommons.org/licenses/by-sa/4.0/; full legal code:
https://creativecommons.org/licenses/by-sa/4.0/legalcode). The files in this folder are an
adaptation of that work and are shared under the same license (CC-BY-SA-4.0). Changes were
made; they are listed above. No warranties are given (section 5 of the license).
