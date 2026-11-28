---
name: react-best-practices
description: React performance rules for Hive's frontend (a Tauri single-page app with React 19 and the React Compiler, no Next.js, no server rendering). Use when writing or reviewing a component that re-renders often, an effect, a list, an event listener, a hot JavaScript path, or when auditing the frontend's performance (e.g. 32.4). Not for every React edit.
license: MIT
metadata:
  author: vercel
  version: "1.0.0"
---

# React best practices (adapted for Hive)

Performance rules from Vercel Engineering, narrowed to what applies to Hive: a single-page
app in a Tauri WebView, React 19, the React Compiler on (`vite.config.ts`), no Next.js, no
server components, no hydration, no data-fetching library (the service pushes state over the
transport; `src/store.ts` holds it).

## Hive's rules come first

- `CLAUDE.md` #37: the frontend only presents. A "performance fix" that moves domain logic
  (paths, git, worktrees, agent states) into TypeScript is wrong; ask the service for the
  derived data instead.
- **The React Compiler memoizes for you.** Do not add `memo`, `useMemo` or `useCallback` by hand
  to fix a re-render unless a profile shows the compiler could not; the rules about manual
  memoization and hoisting static JSX were not copied for that reason.
- **No new dependencies** for any rule here (`swr`, `better-all`, `lru-cache`, a bundler plugin):
  a dependency is the human's decision (`CLAUDE.md` rule 3), added with `bun add` only after a yes.
- **Measure first.** A change made for performance shows its gain (React profiler on the mock
  transport, the browser's performance panel, `bun run build`'s chunk sizes). 100% line coverage
  still applies to whatever you change.

## Rule categories

| Priority | Category | Prefix |
|---|---|---|
| 1 | Re-render optimization | `rerender-` |
| 2 | Rendering performance | `rendering-` |
| 3 | Client patterns (listeners, storage) | `client-` |
| 4 | Bundle size | `bundle-` |
| 5 | JavaScript performance | `js-` |
| 6 | Advanced patterns | `advanced-` |
| 7 | Async | `async-` |

## Quick reference

### 1. Re-render optimization

- `rerender-defer-reads` - Don't subscribe to state only used in callbacks
- `rerender-dependencies` - Use primitive dependencies in effects
- `rerender-derived-state` - Subscribe to derived booleans, not raw values
- `rerender-derived-state-no-effect` - Derive state during render, not effects
- `rerender-functional-setstate` - Use functional setState for stable callbacks
- `rerender-lazy-state-init` - Pass function to useState for expensive values
- `rerender-split-combined-hooks` - Split hooks with independent dependencies
- `rerender-move-effect-to-event` - Put interaction logic in event handlers
- `rerender-transitions` - Use startTransition for non-urgent updates
- `rerender-use-deferred-value` - Defer expensive renders to keep input responsive
- `rerender-use-ref-transient-values` - Use refs for transient frequent values
- `rerender-no-inline-components` - Don't define components inside components

### 2. Rendering performance

- `rendering-activity` - Use Activity component for show/hide
- `rendering-content-visibility` - Use content-visibility for long lists
- `rendering-conditional-render` - Use ternary, not && for conditionals

### 3. Client patterns

- `client-passive-event-listeners` - Use passive listeners for scroll and wheel
- `client-localstorage-schema` - Version and minimize localStorage data (Hive keeps
  persistent state in the service; only UI state may live in the WebView)

### 4. Bundle size

- `bundle-barrel-imports` - Import directly, avoid barrel files (see its Hive note on
  `@phosphor-icons/react`)

### 5. JavaScript performance

- `js-batch-dom-css` - Group CSS changes via classes or cssText
- `js-index-maps` - Build Map for repeated lookups
- `js-cache-property-access` - Cache object properties in loops
- `js-cache-function-results` - Cache function results in module-level Map
- `js-cache-storage` - Cache localStorage/sessionStorage reads
- `js-combine-iterations` - Combine multiple filter/map into one loop
- `js-length-check-first` - Check array length before expensive comparison
- `js-early-exit` - Return early from functions
- `js-hoist-regexp` - Hoist RegExp creation outside loops
- `js-min-max-loop` - Use loop for min/max instead of sort
- `js-set-map-lookups` - Use Set/Map for O(1) lookups
- `js-tosorted-immutable` - Use toSorted() for immutability
- `js-flatmap-filter` - Use flatMap to map and filter in one pass
- `js-request-idle-callback` - Defer non-critical work to browser idle time

### 6. Advanced patterns

- `advanced-effect-event-deps` - Don't put `useEffectEvent` results in effect deps
- `advanced-event-handler-refs` - Store event handlers in refs
- `advanced-init-once` - Initialize app once per app load
- `advanced-use-latest` - useLatest for stable callback refs

### 7. Async

- `async-parallel` - Use Promise.all() for independent operations

## How to use

Read the rule file for the case in front of you (`rules/<name>.md`): why it matters, an
incorrect and a correct example. A rule that conflicts with `CLAUDE.md`, the prototype
(`docs/prototype/`) or `docs/hive.md` loses.
