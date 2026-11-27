# Inspecting Element Attributes

When the snapshot doesn't show an element's `id`, `class`, `data-*` attributes, or other DOM properties, use `eval` to inspect them. (`pw` is `cd "$OUT" && "$PW" cli`, see SKILL.md.)

## Examples

```bash
pw snapshot
# snapshot shows a button as e7 but doesn't reveal its id or data attributes

# get the element's id
pw eval "el => el.id" e7

# get all CSS classes
pw eval "el => el.className" e7

# get a specific attribute
pw eval "el => el.getAttribute('data-testid')" e7
pw eval "el => el.getAttribute('aria-label')" e7

# get a computed style property
pw eval "el => getComputedStyle(el).display" e7
```
