# Source of `react-best-practices`

- **Upstream:** https://github.com/vercel-labs/agent-skills, path `skills/react-best-practices/`
- **Commit:** `5a37a6329fbad3666c3207392a6a7c0110e92efb`
- **License:** MIT
- **Review:** `docs/review/skills.md` (task 28.3); recommendation **Adapt**, approved by the human on 2026-11-27.
- **Copied:** Hive task 28.4. Fetched on 2026-11-27 as the pinned commit's tarball (`codeload.github.com/<repo>/tar.gz/<commit>`) into a new empty folder outside the repository (the agent's worktree isolation refused a `git clone` there); read as text, no script from it was run.

## What changed

- `SKILL.md` rewritten for a Tauri SPA with the React Compiler: Hive's rules first (#37, no manual memoization, no new dependencies, measure first), trigger narrowed.
- Kept 37 rule files: the `rerender-*` rules except the memo ones, `rendering-activity`, `rendering-content-visibility`, `rendering-conditional-render`, all `js-*` and `advanced-*`, `client-passive-event-listeners`, `client-localstorage-schema`, `bundle-barrel-imports` (its Next.js-only fix removed, a Hive note on `@phosphor-icons/react` added), `async-parallel`.
- Dropped: every `server-*`, hydration, script/resource-hint, `async-api-routes`, `async-suspense-boundaries`, `client-swr-dedup`, `rerender-memo*`, `rendering-hoist-jsx`, `rendering-svg-precision` (`npx svgo`), the other bundle/async rules, the compiled `AGENTS.md`, `README.md`, `metadata.json`.

## License notice

```
MIT License

Copyright (c) Vercel (vercel-labs/agent-skills states "MIT" in its README and in the skill's
frontmatter and ships no LICENSE file; no year is given)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
