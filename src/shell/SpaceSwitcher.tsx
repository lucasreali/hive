import { CaretDownIcon, CaretRightIcon } from "@phosphor-icons/react";
import { type KeyboardEvent, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { spaceLabel } from "../hosts";
import {
  type HiveState,
  hostKey,
  hostView,
  knownHosts,
  linkDown,
  openModal,
  showHost,
  useHive,
} from "../store";
import { transport } from "../transport";
import { HostDot, hostName, hostStatus, linkText, MANY_HOSTS, matches } from "./Hosts";
import { CheckIcon } from "./icons";
import { useKept } from "./panelPlace";
import { SearchField } from "./SearchField";

// The space switcher (6.14; hosts 26.18; many hosts 27.8), at the top of the sidebar: its
// trigger names the space and, for a remote's, its host ("Default · gpu-box"); its list has a
// section per host, this Mac first, each with its link's dot. With many hosts a filter comes on
// top and the shown host's section first, the others collapsed to their header until opened or
// matched (the hosts opened stay open for this run). One "New space…" (its dialog asks where) and
// "Edit space…" end it.

/** No host opened yet. */
const NONE: string[] = [];
/** The tallest the list gets before it scrolls. */
const LIST_MAX = 420;

/** A host's spaces in the switcher. */
export type HostSpaces = { host: string | null; spaces: { id: string; name: string }[] };

/** A section as the switcher lists it: open shows its spaces; collapsible has a header to click. */
export type SwitcherSection = HostSpaces & { open: boolean; collapsible: boolean };

/**
 * The spaces of every host the store knows (26.18, 27.8), each as "<host>\n<id>\n<name>" (host
 * "" for this Mac): this Mac's, then each remote's, a down one's too (dimmed).
 */
export function spaceChoices(s: HiveState): string[] {
  return knownHosts(s).flatMap((host) =>
    (hostView(s, host).spaces ?? []).map((x) => `${hostKey(host)}\n${x.id}\n${x.name}`),
  );
}

/** Every known host (`hostKey`) with its spaces, from `spaceChoices`. */
export function hostSpaces(hosts: string[], choices: string[]): HostSpaces[] {
  return hosts.map((key) => ({
    host: key || null,
    spaces: choices.flatMap((choice) => {
      const [at, id, name] = choice.split("\n") as [string, string, string];
      return at === key ? [{ id, name }] : [];
    }),
  }));
}

/**
 * The sections the switcher shows (27.8). With few hosts, every one open in the store's order.
 * With more than `MANY_HOSTS` remotes, the shown host's first; with no `query`, it and the ones
 * the user `opened` are open, the others collapsed; a query keeps the hosts whose name matches
 * (with all their spaces) and the spaces whose name does, all open.
 */
export function switcherSections(
  all: HostSpaces[],
  shown: string | null,
  query: string,
  opened: string[],
): SwitcherSection[] {
  if (all.length - 1 <= MANY_HOSTS) {
    return all.map((x) => ({ ...x, open: true, collapsible: false }));
  }
  const ordered = [...all.filter((x) => x.host === shown), ...all.filter((x) => x.host !== shown)];
  if (!query.trim()) {
    return ordered.map((x) => ({
      ...x,
      open: x.host === shown || opened.includes(hostKey(x.host)),
      collapsible: x.host !== shown,
    }));
  }
  return ordered.flatMap((x) => {
    const named = matches(hostName(x.host), query);
    const spaces = named ? x.spaces : x.spaces.filter((space) => matches(space.name, query));
    return named || spaces.length > 0 ? [{ ...x, spaces, open: true, collapsible: false }] : [];
  });
}

/** What the keyboard moves through: collapsible headers, pickable spaces and the two actions. */
type Item =
  | { kind: "host"; host: string | null }
  | { kind: "space"; host: string | null; id: string }
  | { kind: "new" }
  | { kind: "edit" };

const itemKey = (item: Item) =>
  item.kind === "host"
    ? `h\n${hostKey(item.host)}`
    : item.kind === "space"
      ? `s\n${hostKey(item.host)}\n${item.id}`
      : item.kind;

/** Shows space `id` of `host` (26.18): its host's spaces, then that space there. */
function pickSpace(host: string | null, id: string): void {
  showHost(host);
  if (useHive.getState().currentSpace !== id) void transport.selectSpace(id);
}

/** The space switcher: a combobox named "Space" whose listbox is grouped by host. */
export function SpaceSwitcher() {
  const choices = useHive(useShallow(spaceChoices));
  // The hosts with a link (or shown): one whose link is not known yet is left out.
  const hosts = useHive(
    useShallow((s) =>
      knownHosts(s)
        .filter((h) => h === null || h === s.host || h in s.links)
        .map(hostKey),
    ),
  );
  const links = useHive((s) => s.links);
  const connection = useHive((s) => s.connection);
  const shown = useHive((s) => s.host);
  const current = useHive((s) => s.currentSpace);
  const label = useHive((s) =>
    spaceLabel(
      (s.spaces ?? []).find((x) => x.id === s.currentSpace),
      s.host,
    ),
  );
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  // The hosts opened stay open the next time it opens, for this run (28.27).
  const [opened, setOpened] = useKept<string[]>("space-switcher-hosts", NONE);
  const [active, setActive] = useState(0);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const id = useId();

  const all = hostSpaces(hosts, choices);
  const many = all.length - 1 > MANY_HOSTS;
  const sections = switcherSections(all, shown, query, opened);
  const items: Item[] = [
    ...sections.flatMap((x): Item[] => [
      ...(x.collapsible ? [{ kind: "host" as const, host: x.host }] : []),
      ...(x.open && !linkDown({ links }, x.host)
        ? x.spaces.map((space) => ({ kind: "space" as const, host: x.host, id: space.id }))
        : []),
    ]),
    { kind: "new" },
    { kind: "edit" },
  ];
  const at = (item: Item) => items.findIndex((i) => itemKey(i) === itemKey(item));
  const currentAt = Math.max(0, at({ kind: "space", host: shown, id: current ?? "" }));
  const activeAt = Math.min(active, items.length - 1);

  const show = () => {
    setQuery("");
    setActive(currentAt);
    setOpen(true);
  };
  const hide = (refocus: boolean) => {
    setOpen(false);
    if (refocus) trigger.current?.focus();
  };
  const toggle = (host: string | null) => {
    const key = hostKey(host);
    setOpened((was) => (was.includes(key) ? was.filter((k) => k !== key) : [...was, key]));
  };
  const choose = (item: Item) => {
    if (item.kind === "host") return toggle(item.host);
    hide(true);
    if (item.kind === "space") pickSpace(item.host, item.id);
    else openModal(item.kind === "new" ? "new-space" : "edit-space");
  };

  useLayoutEffect(() => {
    if (!open) return;
    const box = (trigger.current as HTMLButtonElement).getBoundingClientRect();
    const el = list.current as HTMLDivElement;
    el.style.left = `${box.left}px`;
    el.style.top = `${box.bottom + 2}px`;
    el.style.minWidth = `${Math.max(box.width, 240)}px`;
    el.style.maxHeight = `${Math.max(120, Math.min(LIST_MAX, window.innerHeight - box.bottom - 10))}px`;
    if (many) el.querySelector("input")?.focus();
  }, [open, many]);

  useEffect(() => {
    if (open) document.getElementById(`${id}-${activeAt}`)?.scrollIntoView({ block: "nearest" });
  }, [open, activeAt, id]);

  // A click outside closes it; so does resizing, which would leave it away from its trigger.
  useEffect(() => {
    if (!open) return;
    const outside = (e: MouseEvent) => {
      const target = e.target as Node;
      // Inside, the focus stays on the trigger or in the filter.
      if (list.current?.contains(target)) {
        if (!(target instanceof HTMLInputElement)) e.preventDefault();
      } else if (!trigger.current?.contains(target)) setOpen(false);
    };
    const resized = () => setOpen(false);
    document.addEventListener("mousedown", outside);
    window.addEventListener("resize", resized);
    return () => {
      document.removeEventListener("mousedown", outside);
      window.removeEventListener("resize", resized);
    };
  }, [open]);

  const onKeyDown = (e: KeyboardEvent<HTMLElement>, inField: boolean) => {
    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key];
    const item = items[activeAt];
    if (!open) {
      if (step || e.key === "Enter" || e.key === " ") show();
      else return;
    } else if (step) setActive(Math.max(0, Math.min(items.length - 1, activeAt + step)));
    else if (e.key === "Home" && !inField) setActive(0);
    else if (e.key === "End" && !inField) setActive(items.length - 1);
    else if (e.key === "Enter" || (e.key === " " && !inField)) choose(item);
    else if ((e.key === "ArrowRight" || e.key === "ArrowLeft") && item.kind === "host") {
      const isOpen = opened.includes(hostKey(item.host));
      if (isOpen === (e.key === "ArrowLeft")) toggle(item.host);
    } else if (e.key === "Escape") {
      // Closing the list must not also close a dialog around it, nor clear the filter twice.
      e.stopPropagation();
      hide(true);
    } else if (e.key === "Tab") {
      hide(false);
      return;
    } else return;
    e.preventDefault();
  };

  if (current === null) return null;
  const listId = `${id}-list`;
  const rows = (x: SwitcherSection) => {
    const down = linkDown({ links }, x.host);
    if (x.spaces.length === 0) {
      const why = x.host !== null && links[x.host] ? linkText(links[x.host]) : "no spaces";
      return <div className="space-hint">{down ? why : "No spaces"}</div>;
    }
    return x.spaces.map((space) => {
      const item: Item = { kind: "space", host: x.host, id: space.id };
      const n = down ? -1 : at(item);
      const selected = x.host === shown && space.id === current;
      return (
        // biome-ignore lint/a11y/useKeyWithClickEvents: the trigger or the filter handles the keys.
        <div
          key={space.id}
          id={n < 0 ? undefined : `${id}-${n}`}
          role="option"
          tabIndex={-1}
          aria-label={spaceLabel(space, x.host)}
          aria-selected={selected}
          aria-disabled={down || undefined}
          data-active={n === activeAt}
          data-dimmed={down || undefined}
          title={down && x.host !== null ? `${x.host} is not connected` : undefined}
          onMouseMove={() => n >= 0 && setActive(n)}
          onClick={() => n >= 0 && choose(item)}
        >
          <span className="select-value">{space.name}</span>
          {selected && (
            <span className="select-check">
              <CheckIcon />
            </span>
          )}
        </div>
      );
    });
  };
  const header = (x: SwitcherSection) => {
    const status = hostStatus({ connection, links }, x.host);
    const state = x.host !== null && links[x.host]?.state !== "connected" && links[x.host];
    const body = (
      <>
        {x.collapsible &&
          (x.open ? (
            <CaretDownIcon className="space-host-caret" size={10} aria-hidden="true" />
          ) : (
            <CaretRightIcon className="space-host-caret" size={10} aria-hidden="true" />
          ))}
        <HostDot status={status} />
        <span className="select-value">{hostName(x.host)}</span>
        {state && <span className="space-host-state">{linkText(state)}</span>}
      </>
    );
    const down = linkDown({ links }, x.host) || undefined;
    if (!x.collapsible) {
      return (
        <div className="space-host" id={`${id}-h-${hostKey(x.host)}`} data-dimmed={down}>
          {body}
        </div>
      );
    }
    const item: Item = { kind: "host", host: x.host };
    const n = at(item);
    return (
      // biome-ignore lint/a11y/useKeyWithClickEvents: the trigger or the filter handles the keys.
      <div
        className="space-host"
        id={`${id}-${n}`}
        role="option"
        tabIndex={-1}
        aria-selected={false}
        aria-label={`${hostName(x.host)}, ${x.spaces.length === 1 ? "1 space" : `${x.spaces.length} spaces`}, ${x.open ? "open" : "collapsed"}`}
        data-active={n === activeAt}
        data-dimmed={down}
        onMouseMove={() => setActive(n)}
        onClick={() => choose(item)}
      >
        {body}
      </div>
    );
  };
  const action = (kind: "new" | "edit") => {
    const n = at({ kind });
    return (
      // biome-ignore lint/a11y/useKeyWithClickEvents: the trigger or the filter handles the keys.
      <div
        className="space-action"
        id={`${id}-${n}`}
        role="option"
        tabIndex={-1}
        aria-selected={false}
        data-active={n === activeAt}
        onMouseMove={() => setActive(n)}
        onClick={() => choose({ kind })}
      >
        <span className="select-value">{kind === "new" ? "New space…" : "Edit space…"}</span>
      </div>
    );
  };
  return (
    <div className="select space-select">
      <button
        ref={trigger}
        type="button"
        role="combobox"
        className="select-trigger"
        aria-label="Space"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listId}
        aria-activedescendant={open && !many ? `${id}-${activeAt}` : undefined}
        onMouseDown={(e) => {
          if (e.button !== 0) return;
          e.preventDefault();
          trigger.current?.focus();
          if (open) hide(false);
          else show();
        }}
        onKeyDown={(e) => onKeyDown(e, false)}
      >
        <span className="select-value">{label}</span>
        <CaretDownIcon className="select-caret" size={12} aria-hidden="true" />
      </button>
      {open && (
        <div ref={list} className="select-list space-list">
          {many && (
            <SearchField
              aria-label="Filter spaces"
              placeholder="Filter spaces or hosts"
              role="combobox"
              aria-expanded
              aria-controls={listId}
              aria-activedescendant={`${id}-${activeAt}`}
              value={query}
              onChange={(value) => {
                setQuery(value);
                setActive(0);
              }}
              onKeyDown={(e) => onKeyDown(e, true)}
            />
          )}
          <div id={listId} role="listbox" aria-label="Space">
            {sections.map((x) => (
              <fieldset
                key={hostKey(x.host)}
                aria-labelledby={x.collapsible ? undefined : `${id}-h-${hostKey(x.host)}`}
                aria-label={x.collapsible ? hostName(x.host) : undefined}
                className="space-section"
              >
                {header(x)}
                {x.open && rows(x)}
              </fieldset>
            ))}
            {sections.length === 0 && <div className="space-hint">No space or host matches</div>}
            <hr />
            {action("new")}
            {action("edit")}
          </div>
        </div>
      )}
    </div>
  );
}
