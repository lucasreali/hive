# Source of `fp-check`

- **Upstream:** https://github.com/trailofbits/skills, path `plugins/fp-check/`
- **Commit:** `149d7b21f4067765b82e67029730b42d228ef4b4`
- **License:** CC-BY-SA-4.0
- **Review:** `docs/review/skills.md` (task 28.3); recommendation **Adapt**, approved by the human on 2026-11-27.
- **Copied:** Hive task 28.4. Fetched on 2026-11-27 as the pinned commit's tarball (`codeload.github.com/<repo>/tar.gz/<commit>`) into a new empty folder outside the repository (the agent's worktree isolation refused a `git clone` there); read as text, no script from it was run.

## What changed

- Copied `skills/fp-check/SKILL.md` and all of `references/`; `allowed-tools` removed (nothing pre-approves Bash); added an "In Hive" section (same-user threat model, denial of service of the service as real impact, `CLAUDE.md` baseline breaches always reported, read-only, PoCs only in a temp folder outside the repository).
- `references/false-positive-patterns.md`: items 8, 11 and 12 gain Hive notes (untrusted inputs, DoS is impact, the code baseline is never waived as defence in depth).
- `references/deep-verification.md`: Phase 4 (PoC) is done directly, with executable or unit-test PoCs only in a temp folder; agent names updated.
- The two read-only agents became `.claude/agents/fp-check-data-flow-analyzer.md` and `.claude/agents/fp-check-exploitability-verifier.md` (renamed; each gains the Hive threat-model paragraph).
- Not copied: `hooks/hooks.json` (prompt hooks on every Stop and SubagentStop), `agents/poc-builder.md` (writes and runs PoCs in the repository), `agents/openai.yaml`, `assets/`.

## License notice

Copyright Trail of Bits (https://www.trailofbits.com/). The source repository is licensed under
the Creative Commons Attribution-ShareAlike 4.0 International License
(https://creativecommons.org/licenses/by-sa/4.0/; full legal code:
https://creativecommons.org/licenses/by-sa/4.0/legalcode). The files in this folder are an
adaptation of that work and are shared under the same license (CC-BY-SA-4.0). Changes were
made; they are listed above. No warranties are given (section 5 of the license).
