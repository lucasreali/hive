# Source of `sharp-edges`

- **Upstream:** https://github.com/trailofbits/skills, path `plugins/sharp-edges/skills/sharp-edges/`
- **Commit:** `149d7b21f4067765b82e67029730b42d228ef4b4`
- **License:** CC-BY-SA-4.0
- **Review:** `docs/review/skills.md` (task 28.3); recommendation **Adapt**, approved by the human on 2026-11-27.
- **Copied:** Hive task 28.4. Fetched on 2026-11-27 as the pinned commit's tarball (`codeload.github.com/<repo>/tar.gz/<commit>`) into a new empty folder outside the repository (the agent's worktree isolation refused a `git clone` there); read as text, no script from it was run.

## What changed

- `SKILL.md`: description narrowed to Hive's settings, protocol, flags, limits and spike designs; the `sharp-edges-analyzer` agent section replaced by a "Hive design checklist" drawn from `config-patterns.md` (0/-1/empty meanings, bounds, precedence, paths escaping their folder, string booleans, dangerous switches, silent failure, environment).
- Kept `references/config-patterns.md`, `references/lang-rust.md` (its "use `expect()` in binaries" example replaced by Hive's no-`expect` rule) and `references/lang-javascript.md`; dropped the crypto, auth, case-study and other-language references and the agent.

## License notice

Copyright Trail of Bits (https://www.trailofbits.com/). The source repository is licensed under
the Creative Commons Attribution-ShareAlike 4.0 International License
(https://creativecommons.org/licenses/by-sa/4.0/; full legal code:
https://creativecommons.org/licenses/by-sa/4.0/legalcode). The files in this folder are an
adaptation of that work and are shared under the same license (CC-BY-SA-4.0). Changes were
made; they are listed above. No warranties are given (section 5 of the license).
