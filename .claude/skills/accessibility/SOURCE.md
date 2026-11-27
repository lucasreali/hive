# Source of `accessibility`

- **Upstream:** https://github.com/addyosmani/web-quality-skills, path `skills/accessibility/`
- **Commit:** `afa8da942115f2961fdbfa80807ea0b232ff6c00`
- **License:** MIT
- **Review:** `docs/review/skills.md` (task 28.3); recommendation **Adapt**, approved by the human on 2026-11-27.
- **Copied:** Hive task 28.4. Fetched on 2026-11-27 as the pinned commit's tarball (`codeload.github.com/<repo>/tar.gz/<commit>`) into a new empty folder outside the repository (the agent's worktree isolation refused a `git clone` there); read as text, no script from it was run.

## What changed

- The evidence workflow and the automated testing section use Playwright on the mock transport (accessibility snapshots, keyboard paths, `toMatchAriaSnapshot`) instead of Lighthouse and a global axe CLI; `@axe-core/playwright` approved (2026-11-27) but added only by the first task that uses it.
- Dropped website-only sections: media alternatives, skip links, page language, accessible authentication; the drag alternative names Hive's Alt+↑/↓ (#57).
- `references/A11Y-PATTERNS.md` without the skip-link and screen-reader-commands sections; `references/WCAG.md` with the testing-tools table updated.

## License notice

```
MIT License

Copyright (c) 2026 Addy Osmani

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
