---
description: Review UI code for Web Interface Guidelines compliance (Hive's subset)
argument-hint: <file-or-pattern>
---

# Web Interface Guidelines

Review these files for compliance: $ARGUMENTS

Read files, check against rules below. Output concise but comprehensive—sacrifice grammar for brevity. High signal-to-noise.

Hive is a desktop app in a Tauri WebView (macOS first): no URLs, no server rendering, no touch.
The prototype (`docs/prototype/`), `docs/ui-reference.md` and `docs/hive.md` win over any rule below.

## Rules

### Accessibility

- Icon-only buttons need `aria-label`
- Form controls need `<label>` or `aria-label`
- Interactive elements need keyboard handlers (`onKeyDown`/`onKeyUp`)
- `<button>` for actions (not `<div onClick>`)
- Images need `alt` (or `alt=""` if decorative)
- Decorative icons need `aria-hidden="true"`
- Async updates (toasts, validation) need `aria-live="polite"`
- Use semantic HTML (`<button>`, `<label>`, `<table>`) before ARIA

### Focus States

- Interactive elements need visible focus (`:focus-visible` ring or equivalent)
- Never `outline: none` without focus replacement
- Use `:focus-visible` over `:focus` (avoid focus ring on click)
- Group focus with `:focus-within` for compound controls
- Sticky headers/footers/overlays must not cover the focused element

### Forms

- Never block paste (`onPaste` + `preventDefault`)
- Labels clickable (`htmlFor` or wrapping control)
- Disable spellcheck on codes, branch names, paths (`spellCheck={false}`)
- Checkboxes/radios: label + control share single hit target (no dead zones)
- Errors inline next to fields; focus first error on submit

### Animation

- Honor `prefers-reduced-motion` (provide reduced variant or disable)
- Animate `transform`/`opacity` only (compositor-friendly)
- Never `transition: all`—list properties explicitly
- Set correct `transform-origin`
- SVG: transforms on `<g>` wrapper with `transform-box: fill-box; transform-origin: center`
- Animations interruptible—respond to user input mid-animation

### Typography

- `font-variant-numeric: tabular-nums` for number columns/comparisons
- Loading states follow the prototype's wording

### Content Handling

- Text containers handle long content: truncation, line clamp, or `overflow-wrap`
- Flex children need `min-width: 0` to allow text truncation
- Handle empty states—don't render broken UI for empty strings/arrays
- User-generated content (branch names, paths, agent output): anticipate short, average, and very long inputs

### Performance

- Large lists (>50 items): virtualize or `content-visibility: auto`
- No layout reads in render (`getBoundingClientRect`, `offsetHeight`, `offsetWidth`, `scrollTop`)
- Batch DOM reads/writes; avoid interleaving
- Prefer uncontrolled inputs; controlled inputs must be cheap per keystroke

### State

- Destructive actions need a confirmation (a Hive dialog, never the WebView's `confirm`) or an undo window—never immediate
- Drag gestures need click and keyboard alternatives (in Hive, e.g. Alt+↑/↓ to reorder)

### Dark Mode & Theming

- `color-scheme` set for the active theme (fixes scrollbars, inputs)
- Native `<select>`: explicit `background-color` and `color`
- Colours come from the theme tokens (`docs/ui-reference.md`), not literals

### Hover & Interactive States

- Buttons need a hover state (visual feedback)
- Interactive states increase contrast: hover/active/focus more prominent than rest

### Anti-patterns (flag these)

- `onPaste` with `preventDefault`
- `transition: all`
- `outline: none` without focus-visible replacement
- `<div>` or `<span>` with click handlers (should be `<button>`)
- Large arrays `.map()` without virtualization
- Form inputs without labels
- Icon buttons without `aria-label`
- `autoFocus` without clear justification
- Drag-only action without click and keyboard alternative

## Output Format

Group by file. Use `file:line` format. Terse findings.

```text
## src/ui/Button.tsx

src/ui/Button.tsx:42 - icon button missing aria-label
src/ui/Button.tsx:55 - animation missing prefers-reduced-motion
src/ui/Button.tsx:67 - transition: all → list properties

## src/ui/Card.tsx

✓ pass
```

State issue + location. Skip explanation unless fix non-obvious. No preamble.
