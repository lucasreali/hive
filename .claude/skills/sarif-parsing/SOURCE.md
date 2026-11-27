# Source of `sarif-parsing`

- **Upstream:** https://github.com/trailofbits/skills, path `plugins/static-analysis/skills/sarif-parsing/`
- **Commit:** `149d7b21f4067765b82e67029730b42d228ef4b4`
- **License:** CC-BY-SA-4.0
- **Review:** `docs/review/skills.md` (task 28.3); recommendation **Take**, approved by the human on 2026-11-27.
- **Copied:** Hive task 28.4. Fetched on 2026-11-27 as the pinned commit's tarball (`codeload.github.com/<repo>/tar.gz/<commit>`) into a new empty folder outside the repository (the agent's worktree isolation refused a `git clone` there); read as text, no script from it was run.

## What changed

- Copied `SKILL.md`, `resources/jq-queries.md`, `resources/sarif_helpers.py` (standard library only; needs Python 3.10+) and `resources/fixtures/`. Not copied: `resources/test_sarif_helpers.py`.
- Removed the global `ajv-cli` install hint and the npm-package row; `brew install jq` became macOS's built-in `jq`; the `uv run --with <package>` examples now say to ask the human before fetching a third-party package; the helpers' line and usage string note Python 3.10+ (the host's stock `python3` is 3.9) and `-I`.

## License notice

Copyright Trail of Bits (https://www.trailofbits.com/). The source repository is licensed under
the Creative Commons Attribution-ShareAlike 4.0 International License
(https://creativecommons.org/licenses/by-sa/4.0/; full legal code:
https://creativecommons.org/licenses/by-sa/4.0/legalcode). The files in this folder are an
adaptation of that work and are shared under the same license (CC-BY-SA-4.0). Changes were
made; they are listed above. No warranties are given (section 5 of the license).
