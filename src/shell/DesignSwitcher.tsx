// THROWAWAY (25.3 design): the candidate layouts of the panel's view switcher, picked by
// localStorage "hive.design". Never merged.
import { CaretDownIcon, CheckIcon, DotsThreeIcon, NotePencilIcon } from "@phosphor-icons/react";
import { type KeyboardEvent, type ReactNode, useLayoutEffect, useRef, useState } from "react";
import type { PanelView } from "../store";
import { setPanelView, useHive } from "../store";
import { ContextMenu } from "./WorktreeMenu";
import "./design.css";

export type DView = { view: PanelView; label: string; icon: ReactNode };
export type Layout = "strip" | "more" | "rail" | "menu";

export function designOf(): {
  layout?: Layout;
  extra?: boolean;
  badges?: boolean;
  iconsOnly?: boolean;
} {
  try {
    return JSON.parse(localStorage.getItem("hive.design") ?? "{}");
  } catch {
    return {};
  }
}

const GROUPS: { name: string; views: string[] }[] = [
  { name: "This worktree", views: ["files", "changes", "scripts", "extra"] },
  { name: "Repository", views: ["pulls", "actions"] },
  { name: "History", views: ["sessions"] },
];

export function withDesignViews(views: DView[]): DView[] {
  if (!designOf().extra || views.length < 3) return views;
  return [
    ...views,
    {
      view: "extra" as PanelView,
      label: "Notes",
      icon: <NotePencilIcon size={14} aria-hidden="true" />,
    },
  ];
}

const grouped = (views: DView[]) =>
  GROUPS.map((g) => ({
    name: g.name,
    views: views.filter((v) => g.views.includes(v.view)),
  })).filter((g) => g.views.length > 0);

const BADGES: Record<string, { text: string; name: string; tone?: string }> = {
  pulls: { text: "3", name: "3 open" },
  actions: { text: "!", name: "a run failed", tone: "bad" },
  changes: { text: "12", name: "12 changed" },
};

function Badge({ view }: { view: PanelView }) {
  const b = designOf().badges ? BADGES[view] : undefined;
  return b ? (
    <span className="d-badge" data-tone={b.tone}>
      {b.text}
    </span>
  ) : null;
}

const nameOf = (v: DView) => {
  const b = designOf().badges ? BADGES[v.view] : undefined;
  return b ? `${v.label}, ${b.name}` : v.label;
};

/** Arrows, Home and End move along a tablist (WAI-ARIA tabs), selecting as they go. */
function moveInTabs(
  e: KeyboardEvent<HTMLElement>,
  views: DView[],
  view: PanelView,
  vertical: boolean,
) {
  const i = views.findIndex((v) => v.view === view);
  const next = {
    [vertical ? "ArrowDown" : "ArrowRight"]: (i + 1) % views.length,
    [vertical ? "ArrowUp" : "ArrowLeft"]: (i - 1 + views.length) % views.length,
    Home: 0,
    End: views.length - 1,
  }[e.key];
  if (next === undefined) return;
  e.preventDefault();
  const to = views[next] as DView;
  setPanelView(to.view);
  const list = e.currentTarget;
  requestAnimationFrame(() => list.querySelector<HTMLElement>(`[data-view="${to.view}"]`)?.focus());
}

function Tab({ v, view }: { v: DView; view: PanelView }) {
  return (
    <button
      type="button"
      role="tab"
      data-view={v.view}
      aria-selected={view === v.view}
      aria-label={nameOf(v)}
      title={v.label}
      tabIndex={view === v.view ? 0 : -1}
      onClick={() => setPanelView(v.view)}
    >
      <span className="d-icon">{v.icon}</span>
      <span className="tab-label">{v.label}</span>
      <Badge view={v.view} />
    </button>
  );
}

/** A: icons in groups; the selected view always has its label; all labels once they fit. */
export function Strip({ views, view }: { views: DView[]; view: PanelView }) {
  const order = grouped(views);
  const flat = order.flatMap((g) => g.views);
  return (
    <div
      className="panel-views d-strip"
      role="tablist"
      aria-label="Panel"
      onKeyDown={(e) => moveInTabs(e, flat, view, false)}
    >
      {order.map((g, i) => (
        <span key={g.name} className="d-group" role="presentation">
          {i > 0 && <span className="d-sep" aria-hidden="true" />}
          {g.views.map((v) => (
            <Tab key={v.view} v={v} view={view} />
          ))}
        </span>
      ))}
    </div>
  );
}

const MORE_W = 30;

/** B: labelled tabs while they fit, the rest in a "More" menu; the selected one always shows. */
export function More({ views, view }: { views: DView[]; view: PanelView }) {
  const strip = useRef<HTMLDivElement>(null);
  const measure = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const width = useHive((s) => s.panelWidth);
  const [count, setCount] = useState(views.length);
  const [open, setOpen] = useState<{ x: number; y: number } | null>(null);
  useLayoutEffect(() => {
    const room = strip.current?.clientWidth ?? 0;
    const sizes = [...(measure.current?.children ?? [])].map(
      (c) => c.getBoundingClientRect().width + 2,
    );
    const total = sizes.reduce((a, b) => a + b, 0);
    if (total <= room) return setCount(views.length);
    let used = MORE_W;
    let n = 0;
    for (const s of sizes) {
      if (used + s > room) break;
      used += s;
      n += 1;
    }
    setCount(n);
  }, [views, width]);
  // The selected view takes the last place when it would be in the menu.
  const at = views.findIndex((v) => v.view === view);
  const shown = views.slice(0, count);
  const rest = views.slice(count);
  if (at >= count && count > 0) {
    const out = shown[count - 1] as DView;
    shown[count - 1] = views[at] as DView;
    rest[at - count] = out;
  }
  return (
    <div className="d-more-row" ref={strip}>
      <div className="d-measure" ref={measure} aria-hidden="true">
        {views.map((v) => (
          <span key={v.view} className="d-measure-tab">
            <span className="d-icon">{v.icon}</span>
            <span>{v.label}</span>
            <Badge view={v.view} />
          </span>
        ))}
      </div>
      <div
        className="panel-views d-more"
        role="tablist"
        aria-label="Panel"
        onKeyDown={(e) => moveInTabs(e, shown, view, false)}
      >
        {shown.map((v) => (
          <Tab key={v.view} v={v} view={view} />
        ))}
      </div>
      {rest.length > 0 && (
        <button
          ref={button}
          type="button"
          className="d-more-button"
          aria-haspopup="menu"
          aria-expanded={open !== null}
          aria-label={`More views: ${rest.map((v) => v.label).join(", ")}`}
          title="More views"
          onClick={() => {
            const r = button.current?.getBoundingClientRect();
            setOpen(open || !r ? null : { x: r.left, y: r.bottom + 2 });
          }}
        >
          <DotsThreeIcon size={16} weight="bold" aria-hidden="true" />
          {designOf().badges && rest.some((v) => BADGES[v.view]) && <span className="d-dot" />}
        </button>
      )}
      {open && (
        <ContextMenu
          at={open}
          label="More views"
          anchor={button.current}
          onClose={() => setOpen(null)}
        >
          {rest.map((v) => (
            <button
              key={v.view}
              type="button"
              role="menuitem"
              aria-label={nameOf(v)}
              onClick={() => {
                setOpen(null);
                setPanelView(v.view);
              }}
            >
              {v.icon}
              {v.label}
              <Badge view={v.view} />
            </button>
          ))}
        </ContextMenu>
      )}
    </div>
  );
}

/** C: a vertical rail on the panel's outer edge; icons, with labels once the panel is wide. */
export function Rail({ views, view }: { views: DView[]; view: PanelView }) {
  const order = grouped(views);
  const flat = order.flatMap((g) => g.views);
  const wide = useHive((s) => s.panelWidth) >= 520 && !designOf().iconsOnly;
  return (
    <div
      className="d-rail"
      data-wide={wide}
      role="tablist"
      aria-label="Panel"
      aria-orientation="vertical"
      onKeyDown={(e) => moveInTabs(e, flat, view, true)}
    >
      {order.map((g, i) => (
        <span key={g.name} className="d-group" role="presentation">
          {i > 0 && <span className="d-sep" aria-hidden="true" />}
          {g.views.map((v) => (
            <Tab key={v.view} v={v} view={view} />
          ))}
        </span>
      ))}
    </div>
  );
}

/** D: the view's name is a menu button; the menu lists every view, in groups. */
export function Menu({ views, view }: { views: DView[]; view: PanelView }) {
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState<{ x: number; y: number } | null>(null);
  const shown = views.find((v) => v.view === view) as DView;
  const order = grouped(views);
  return (
    <div className="d-picker-row">
      <button
        ref={button}
        type="button"
        className="d-picker"
        aria-haspopup="menu"
        aria-expanded={open !== null}
        aria-label={`Panel view: ${nameOf(shown)}`}
        title="Switch view"
        onClick={() => {
          const r = button.current?.getBoundingClientRect();
          setOpen(open || !r ? null : { x: r.left, y: r.bottom + 2 });
        }}
      >
        <span className="d-icon">{shown.icon}</span>
        <span>{shown.label}</span>
        <Badge view={shown.view} />
        <CaretDownIcon size={12} aria-hidden="true" />
      </button>
      {open && (
        <ContextMenu
          at={open}
          label="Panel views"
          anchor={button.current}
          onClose={() => setOpen(null)}
        >
          {order.map((g) => (
            <div key={g.name} className="d-menu-group">
              {order.length > 1 && <div className="menu-note">{g.name}</div>}
              {g.views.map((v) => (
                <button
                  key={v.view}
                  type="button"
                  role="menuitem"
                  aria-label={nameOf(v)}
                  onClick={() => {
                    setOpen(null);
                    setPanelView(v.view);
                  }}
                >
                  {v.icon}
                  {v.label}
                  <Badge view={v.view} />
                  {v.view === view && (
                    <span className="d-check">
                      <CheckIcon size={12} aria-hidden="true" />
                    </span>
                  )}
                </button>
              ))}
            </div>
          ))}
        </ContextMenu>
      )}
    </div>
  );
}
