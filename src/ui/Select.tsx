import { CaretDownIcon } from "@phosphor-icons/react";
import { type KeyboardEvent, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { CheckIcon } from "../shell/icons";

export type SelectOption = { value: string; label: string };

/** How long a pause in typing starts a new type-ahead search. */
export const TYPE_AHEAD_MS = 500;
/** The tallest the list gets before it scrolls. */
const LIST_MAX = 240;

/**
 * A select in the app's look, in place of the native one (whose popup is the WebView's own).
 * The trigger is a `combobox` button that keeps the focus; the `listbox` under it opens on a
 * click, ↑/↓, Enter, Space or typing, and closes on a pick, Esc (which never reaches an
 * enclosing `<dialog>`), Tab, blur, scrolling or resizing. It is `position: fixed`, so the
 * dialog's own overflow does not clip it.
 */
export function Select(props: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  "aria-label"?: string;
  "aria-labelledby"?: string;
  disabled?: boolean;
  className?: string;
}) {
  const { value, options, onChange } = props;
  const current = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(current);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const typed = useRef({ text: "", at: 0 });
  const id = useId();

  const show = (at: number) => {
    setActive(Math.max(0, Math.min(options.length - 1, at)));
    setOpen(true);
  };
  const choose = (at: number) => {
    setOpen(false);
    trigger.current?.focus();
    if (options[at].value !== value) onChange(options[at].value);
  };

  useLayoutEffect(() => {
    if (!open) return;
    const box = (trigger.current as HTMLButtonElement).getBoundingClientRect();
    const el = list.current as HTMLDivElement;
    el.style.left = `${box.left}px`;
    el.style.minWidth = `${box.width}px`;
    // It opens upwards when there is more room above (e.g. the chat composer, at the bottom).
    const below = window.innerHeight - box.bottom - 8;
    const above = box.top - 8;
    if (below < LIST_MAX && above > below) {
      el.style.bottom = `${window.innerHeight - box.top + 2}px`;
      el.style.maxHeight = `${Math.min(LIST_MAX, above)}px`;
    } else {
      el.style.top = `${box.bottom + 2}px`;
      el.style.maxHeight = `${Math.max(80, Math.min(LIST_MAX, below))}px`;
    }
  }, [open]);

  useEffect(() => {
    if (open) document.getElementById(`${id}-${active}`)?.scrollIntoView({ block: "nearest" });
  }, [open, active, id]);

  // A fixed list would drift away from its trigger.
  useEffect(() => {
    if (!open) return;
    const hide = (e: Event) => {
      if (!list.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("scroll", hide, true);
    window.addEventListener("resize", hide);
    return () => {
      document.removeEventListener("scroll", hide, true);
      window.removeEventListener("resize", hide);
    };
  }, [open]);

  const typeAhead = (key: string) => {
    const now = Date.now();
    const t = typed.current;
    t.text = now - t.at < TYPE_AHEAD_MS ? t.text + key.toLowerCase() : key.toLowerCase();
    t.at = now;
    const from = open ? active : current;
    // A repeated first letter cycles through the options that start with it.
    const start = t.text.length === 1 ? from + 1 : from;
    for (let k = 0; k < options.length; k++) {
      const at = (start + k) % options.length;
      if (options[at].label.toLowerCase().startsWith(t.text)) return show(at);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key];
    if (step) show(open ? active + step : current);
    else if (e.key === "Home") show(0);
    else if (e.key === "End") show(options.length - 1);
    else if (e.key === "Enter" || e.key === " ") {
      if (open) choose(active);
      else show(current);
    } else if (e.key === "Escape" && open) {
      // Closing the list must not also close the dialog around it.
      e.stopPropagation();
      setOpen(false);
    } else if (e.key === "Tab") {
      setOpen(false);
      return;
    } else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) typeAhead(e.key);
    else return;
    e.preventDefault();
  };

  const listId = `${id}-list`;
  return (
    <div className={props.className ? `select ${props.className}` : "select"}>
      <button
        ref={trigger}
        type="button"
        role="combobox"
        className="select-trigger"
        aria-label={props["aria-label"]}
        aria-labelledby={props["aria-labelledby"]}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listId}
        aria-activedescendant={open ? `${id}-${active}` : undefined}
        disabled={props.disabled}
        // Mouse down, not click: keyboard clicks (Enter, Space) are handled in onKeyDown, and
        // WebKit does not focus a clicked button by itself.
        onMouseDown={(e) => {
          if (e.button !== 0) return;
          e.preventDefault();
          trigger.current?.focus();
          if (open) setOpen(false);
          else show(current);
        }}
        onKeyDown={onKeyDown}
        onBlur={() => setOpen(false)}
      >
        <span className="select-value">{options[current]?.label}</span>
        <CaretDownIcon className="select-caret" size={12} aria-hidden="true" />
      </button>
      {open && (
        <div
          ref={list}
          id={listId}
          role="listbox"
          className="select-list"
          aria-label={props["aria-label"]}
          aria-labelledby={props["aria-labelledby"]}
          // The focus stays on the trigger.
          onMouseDown={(e) => e.preventDefault()}
        >
          {options.map((o, at) => (
            // biome-ignore lint/a11y/useKeyWithClickEvents: the trigger handles the keys.
            <div
              key={o.value}
              id={`${id}-${at}`}
              role="option"
              tabIndex={-1}
              aria-selected={at === current}
              data-active={at === active}
              onMouseMove={() => setActive(at)}
              onClick={() => choose(at)}
            >
              <span className="select-value">{o.label}</span>
              {at === current && (
                <span className="select-check">
                  <CheckIcon />
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
