# Source of the survivor triage in `stage/SKILL.md`

Step 1.2's equivalence procedure and rationalizations to reject (Hive task 28.4) are adapted from
Trail of Bits' `mutation-testing` skill: https://github.com/trailofbits/skills, commit `149d7b21f4067765b82e67029730b42d228ef4b4`,
`plugins/mutation-testing/skills/mutation-testing/references/equivalent-mutants.md` and
`workflows/analyzing-results.md`. Changed: condensed to five checks; "equivalent" never means skipped
(refactored away or sent to the human, `CLAUDE.md` rule 6); mewt/muton, blockchain, severity tiers and
the report file dropped. The rest of `stage/` is Hive's own.

## License notice

Copyright Trail of Bits (https://www.trailofbits.com/). The source repository is licensed under
the Creative Commons Attribution-ShareAlike 4.0 International License
(https://creativecommons.org/licenses/by-sa/4.0/; full legal code:
https://creativecommons.org/licenses/by-sa/4.0/legalcode). The adapted passage in `SKILL.md` is an
adaptation of that work and are shared under the same license (CC-BY-SA-4.0). Changes were
made; they are listed above. No warranties are given (section 5 of the license).
