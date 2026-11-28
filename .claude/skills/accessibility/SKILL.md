---
name: accessibility
description: Audit and improve the accessibility of Hive's UI following WCAG 2.2 (keyboard, focus, contrast in both themes, targets, ARIA, live regions, reduced motion), with evidence from Playwright on the mock transport. Use for UI tasks that add or change interactive components, when asked to "improve accessibility", "a11y audit", "keyboard navigation" or "screen reader support", and in the frontend audit.
license: MIT
metadata:
  author: web-quality-skills
  version: "2.0"
---

# Accessibility (a11y)

Comprehensive accessibility guidelines based on WCAG 2.2. Goal: make content usable by everyone, including people with disabilities.

## Evidence-led audit workflow

Hive is a desktop app (a Tauri WebView); its UI is checked in a browser against the Vite dev
server on the mock transport, never in the installed app. The look's authority stays
`docs/prototype/`, `docs/ui-reference.md` and `docs/hive.md` (shortcuts: #35): an accessibility
fix that changes the look or a shortcut is a question for the human first.

1. Start the dev server on your own port and open the flow with the `playwright-cli` skill
   (`pw snapshot`): the accessibility tree gives names, roles, states and landmarks.
2. Exercise the flow with the keyboard only (`pw press Tab`, `Enter`, `Escape`, the #35
   shortcuts), in the light and dark themes (`pw set-color-scheme dark`) and with
   `pw set-reduced-motion reduce`.
3. Localize each problem to its component under `src/` and fix the source.
4. Lock the fix in an e2e spec: a `toMatchAriaSnapshot` of the region (names and roles), keyboard
   steps with web-first assertions, focus checks (`toBeFocused()`).

Automated rule checks (axe): the human approved `@axe-core/playwright` as a dev-dependency on
2026-11-27, but it is not installed yet. The first task that writes an axe check adds it with
`bun add -d @axe-core/playwright` (never by editing `package.json`) and uses it inside an e2e
spec (`new AxeBuilder({ page }).analyze()`, asserting on the violations). No global axe CLI
and no page audit run against a URL. Automated tools detect only a subset of accessibility barriers: a clean run is
not WCAG conformance, and it does not replace the keyboard and tree checks above.

## WCAG Principles: POUR

| Principle | Description |
|-----------|-------------|
| **P**erceivable | Content can be perceived through different senses |
| **O**perable | Interface can be operated by all users |
| **U**nderstandable | Content and interface are understandable |
| **R**obust | Content works with assistive technologies |

## Conformance levels

| Level | Requirement | Target |
|-------|-------------|--------|
| **A** | Minimum accessibility | Must pass |
| **AA** | Standard compliance | Should pass (legal requirement in many jurisdictions) |
| **AAA** | Enhanced accessibility | Nice to have |

---

## Perceivable

### Text alternatives (1.1)

**Images require alt text:**
```html
<!-- ❌ Missing alt -->
<img src="chart.png">

<!-- ✅ Descriptive alt -->
<img src="chart.png" alt="Bar chart showing 40% increase in Q3 sales">

<!-- ✅ Decorative image (empty alt) -->
<img src="decorative-border.png" alt="" role="presentation">

<!-- ✅ Complex image with longer description -->
<figure>
  <img src="infographic.png" alt="2024 market trends infographic" 
       aria-describedby="infographic-desc">
  <figcaption id="infographic-desc">
    <!-- Detailed description -->
  </figcaption>
</figure>
```

**Icon buttons need accessible names:**
```html
<!-- ❌ No accessible name -->
<button><svg><!-- menu icon --></svg></button>

<!-- ✅ Using aria-label -->
<button aria-label="Open menu">
  <svg aria-hidden="true"><!-- menu icon --></svg>
</button>

<!-- ✅ Using visually hidden text -->
<button>
  <svg aria-hidden="true"><!-- menu icon --></svg>
  <span class="visually-hidden">Open menu</span>
</button>
```

**Visually hidden class:**
```css
.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
```

### Color contrast (1.4.3, 1.4.6)

| Text Size | AA minimum | AAA enhanced |
|-----------|------------|--------------|
| Normal text (< 18px / < 14px bold) | 4.5:1 | 7:1 |
| Large text (≥ 18px / ≥ 14px bold) | 3:1 | 4.5:1 |
| UI components & graphics | 3:1 | 3:1 |

```css
/* ❌ Low contrast (2.5:1) */
.low-contrast {
  color: #999;
  background: #fff;
}

/* ✅ Sufficient contrast (7:1) */
.high-contrast {
  color: #333;
  background: #fff;
}

/* ✅ Focus states need contrast too (3:1 against background, WCAG 1.4.11) */
:focus-visible {
  outline: 2px solid currentColor;
  outline-offset: 2px;
}
```

**Don't rely on color alone:**
```html
<!-- ❌ Only color indicates error -->
<input class="error-border">
<style>.error-border { border-color: red; }</style>

<!-- ✅ Color + icon + text -->
<div class="field-error">
  <input aria-invalid="true" aria-describedby="email-error">
  <span id="email-error" class="error-message">
    <svg aria-hidden="true"><!-- error icon --></svg>
    Please enter a valid email address
  </span>
</div>
```

## Operable

### Keyboard accessible (2.1)

**All functionality must be keyboard accessible.** Prefer native interactive elements — `<button>`, `<a href>`, and form controls handle Enter/Space activation, focus, and assistive-tech semantics for free. Only add manual keyboard handling when you cannot use a native element.

```html
<!-- ❌ Non-interactive element with click only: not focusable, no keyboard activation -->
<div class="card" onclick="handleAction()">Open</div>

<!-- ✅ Best: use a native button -->
<button type="button" onclick="handleAction()">Open</button>
```

```javascript
// ✅ When you MUST use a non-interactive element (e.g. div with role="button"),
// make it focusable AND handle keyboard activation. Do NOT add this to a native
// <button> — Enter/Space already fire click, so you'd double-trigger.
element.setAttribute('role', 'button');
element.setAttribute('tabindex', '0');
element.addEventListener('click', handleAction);
element.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' || e.key === ' ') {
    e.preventDefault();
    handleAction();
  }
});
```

**No keyboard traps.** Users must be able to Tab into and out of every component. Use the [modal focus trap pattern](references/A11Y-PATTERNS.md#modal-focus-trap) for dialogs—the native `<dialog>` element handles this automatically.

### Focus visible (2.4.7)

```css
/* ❌ Never remove focus outlines */
*:focus { outline: none; }

/* ✅ Use :focus-visible for keyboard-only focus */
:focus {
  outline: none;
}

:focus-visible {
  outline: 2px solid currentColor; /* inherits text color → already contrast-checked */
  outline-offset: 2px;
}

/* ✅ Or pick a brand color and verify ≥3:1 contrast against every background it lands on */
button:focus-visible {
  box-shadow: 0 0 0 3px rgba(0, 95, 204, 0.5);
}
```

### Focus not obscured (2.4.11) — new in 2.2

When an element receives keyboard focus, it must not be entirely hidden by other author-created content such as sticky headers, footers, or overlapping panels. At Level AAA (2.4.12), no part of the focused element may be hidden.

```css
/* ✅ Account for sticky headers when scrolling to focused elements */
:target {
  scroll-margin-top: 80px;
}

/* ✅ Ensure focused items clear fixed/sticky bars */
:focus {
  scroll-margin-top: 80px;
  scroll-margin-bottom: 60px;
}
```

### Target size (2.5.8) — new in 2.2

Interactive targets must be at least **24 × 24 CSS pixels** (AA). Exceptions: inline text links, elements where the browser controls the size, and targets where a 24px circle centered on the bounding box does not overlap another target.

```css
/* ✅ Minimum target size */
button,
[role="button"],
input[type="checkbox"] + label,
input[type="radio"] + label {
  min-width: 24px;
  min-height: 24px;
}

/* ✅ Comfortable target size (recommended 44×44) */
.touch-target {
  min-width: 44px;
  min-height: 44px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
```

### Dragging movements (2.5.7) — new in 2.2

Any action that requires dragging must have a single-pointer alternative (e.g., buttons, inputs) and a keyboard one. In Hive, reordering by drag also works with Alt+↑/↓ (#57).

### Timing (2.2)

```javascript
// Allow users to extend time limits
function showSessionWarning() {
  const modal = createModal({
    title: 'Session Expiring',
    content: 'Your session will expire in 2 minutes.',
    actions: [
      { label: 'Extend session', action: extendSession },
      { label: 'Log out', action: logout }
    ],
    timeout: 120000
  });
}
```

### Motion (2.3)

```css
/* Respect reduced motion preference */
@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
    scroll-behavior: auto !important;
  }
}
```

---

## Understandable

### Consistent navigation (3.2.3)

```html
<!-- Navigation should be consistent across pages -->
<nav aria-label="Main">
  <ul>
    <li><a href="/" aria-current="page">Home</a></li>
    <li><a href="/products">Products</a></li>
    <li><a href="/about">About</a></li>
  </ul>
</nav>
```

### Consistent help (3.2.6) — new in 2.2

If a help mechanism (contact info, chat widget, FAQ link, self-help option) is repeated across multiple pages, it must appear in the **same relative order** each time. Users who rely on consistent placement shouldn't have to hunt for help on every page.

### Form labels (3.3.2)

Every input needs a programmatically associated label. See the [form labels pattern](references/A11Y-PATTERNS.md#form-labels) for explicit, implicit, and instructional examples.

### Error handling (3.3.1, 3.3.3)

Announce errors to screen readers with `role="alert"` or `aria-live`, set `aria-invalid="true"` on invalid fields, and focus the first error on submit. See the [error handling pattern](references/A11Y-PATTERNS.md#error-handling) for full markup and JS.

### Redundant entry (3.3.7) — new in 2.2

Don't force users to re-enter information they already provided in the same session. Auto-populate from earlier steps, or let users select from previously entered values. Exceptions: security re-confirmation and content that has expired.

```html
<!-- ✅ Auto-fill shipping address from billing -->
<fieldset>
  <legend>Shipping address</legend>
  <label>
    <input type="checkbox" id="same-as-billing" checked>
    Same as billing address
  </label>
  <!-- Fields auto-populated when checked -->
</fieldset>
```

## Robust

### ARIA usage (4.1.2)

**Prefer native elements:**
```html
<!-- ❌ ARIA role on div -->
<div role="button" tabindex="0">Click me</div>

<!-- ✅ Native button -->
<button>Click me</button>

<!-- ❌ ARIA checkbox -->
<div role="checkbox" aria-checked="false">Option</div>

<!-- ✅ Native checkbox -->
<label><input type="checkbox"> Option</label>
```

**When ARIA is needed,** use the correct roles and states. See the [ARIA tabs pattern](references/A11Y-PATTERNS.md#aria-tabs) for a complete tablist example.

### Live regions (4.1.3)

Use `aria-live` regions to announce dynamic content changes without moving focus. See the [live regions pattern](references/A11Y-PATTERNS.md#live-regions-and-notifications) for markup and a `showNotification()` helper.

---

## Testing checklist

### Automated testing

In the e2e suite on the mock transport (`E2E_PORT=<port> bun run e2e`): aria snapshots of the
changed regions, keyboard paths, focus assertions, and, once `@axe-core/playwright` is added (see
the audit workflow above), an axe scan of the changed view with its violations asserted empty.

### Manual testing

- [ ] **Keyboard navigation:** Tab through the whole view, use Enter/Space to activate
- [ ] **Screen reader:** VoiceOver on macOS, in the dev server's page (a manual check for the human when it matters)
- [ ] **Zoom:** Content usable at 200% zoom
- [ ] **High contrast:** `pw set-forced-colors active` and `pw set-contrast more`
- [ ] **Reduced motion:** Test with `prefers-reduced-motion: reduce`
- [ ] **Focus order:** Logical and follows visual order
- [ ] **Target size:** Interactive elements meet 24×24px minimum


---

## Common issues by impact

### Critical (fix immediately)
1. Missing form labels
2. Missing image alt text
3. Insufficient color contrast
4. Keyboard traps
5. No focus indicators

### Serious (fix before launch)
1. Missing heading structure
3. Non-descriptive link text
4. Auto-playing motion without a reduced-motion variant

### Moderate (fix soon)
1. Missing ARIA labels on icons
2. Inconsistent navigation
3. Missing error identification
4. Timing without controls
5. Missing landmark regions

## References

- [WCAG 2.2 Quick Reference](https://www.w3.org/WAI/WCAG22/quickref/)
- [WAI-ARIA Authoring Practices](https://www.w3.org/WAI/ARIA/apg/)
- [Deque axe Rules](https://dequeuniversity.com/rules/axe/)
- [WCAG criteria reference](references/WCAG.md)
- [Accessibility code patterns](references/A11Y-PATTERNS.md)
