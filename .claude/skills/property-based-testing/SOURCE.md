# Source of `property-based-testing`

- **Upstream:** https://github.com/trailofbits/skills, path `plugins/property-based-testing/skills/property-based-testing/`
- **Commit:** `149d7b21f4067765b82e67029730b42d228ef4b4`
- **License:** CC-BY-SA-4.0
- **Review:** `docs/review/skills.md` (task 28.3); recommendation **Take**, approved by the human on 2026-11-27.
- **Copied:** Hive task 28.4. Fetched on 2026-11-27 as the pinned commit's tarball (`codeload.github.com/<repo>/tar.gz/<commit>`) into a new empty folder outside the repository (the agent's worktree isolation refused a `git clone` there); read as text, no script from it was run.

## What changed

- Copied `SKILL.md` and `references/` unchanged; not copied: `README.md`, `agents/openai.yaml`, `assets/`, and the plugin's `evals/` / `evals-extra/` (its `run.sh` and `effectiveness.sh` run the real `claude`).
- Appended an "In Hive" section: `proptest` approved by the human (2026-11-27) but added only by the first task that writes a property test (`cargo add -p <crate> --dev proptest`); deterministic runs, no `proptest-regressions/` in the repository.

## License notice

Copyright Trail of Bits (https://www.trailofbits.com/). The source repository is licensed under
the Creative Commons Attribution-ShareAlike 4.0 International License
(https://creativecommons.org/licenses/by-sa/4.0/; full legal code:
https://creativecommons.org/licenses/by-sa/4.0/legalcode). The files in this folder are an
adaptation of that work and are shared under the same license (CC-BY-SA-4.0). Changes were
made; they are listed above. No warranties are given (section 5 of the license).
