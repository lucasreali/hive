import { UserCircleIcon } from "@phosphor-icons/react";
import { useEffect, useLayoutEffect, useRef } from "react";
import type { LineCommit, LineOrigin } from "../protocol";
import {
  closeProvenance,
  commitLine,
  NO_SESSION,
  probably,
  showSession,
  whoWrote,
} from "../provenance";
import { ago } from "../sessions";
import { useHive } from "../store";
import { ICON } from "./icons";
import { ContextMenu } from "./WorktreeMenu";

// "Who wrote this line?" (27.5, spike §10): the line number menu, and the popover by the line
// with an answer that is not a session's row (probable, Hive's editor, unknown).

const closeMenu = () => useHive.setState({ lineMenu: null });

/** The editor's line number menu (a right click in its gutter). */
export function LineMenu() {
  const menu = useHive((s) => s.lineMenu);
  if (!menu) return null;
  return (
    <ContextMenu at={menu} label={`Line ${menu.line}`} onClose={closeMenu}>
      <button type="button" role="menuitem" onClick={() => whoWrote(menu)}>
        <UserCircleIcon {...ICON} />
        Who wrote this line?
      </button>
    </ContextMenu>
  );
}

/** What the popover says of `origin`, never a live session's (the Sessions view shows it). */
function Answer({ origin, path }: { origin: LineOrigin; path: string }) {
  if (origin.kind === "editor") return <p>You, in Hive's editor · {ago(origin.ms)}</p>;
  if (origin.kind === "unknown") {
    return (
      <>
        <p>{NO_SESSION}</p>
        {origin.commit && <p className="line-author-commit">{commitLine(origin.commit)}</p>}
      </>
    );
  }
  const { edit } = origin;
  const commit = origin.commit as LineCommit;
  return (
    <>
      <p>{probably(edit, commit)}</p>
      {edit.deleted && <p className="line-author-commit">Session deleted</p>}
      <div className="line-author-actions">
        <button type="button" className="secondary" onClick={() => showSession(edit, path)}>
          Show session
        </button>
      </div>
    </>
  );
}

/**
 * The answer by the line it is about (kept inside the window). It closes on Esc, a click
 * outside it, or another question.
 */
export function LinePopover() {
  const provenance = useHive((s) => s.provenance);
  const ref = useRef<HTMLDivElement>(null);
  const origin = provenance?.origin;
  const live = origin?.kind === "session" && !origin.commit;
  const shown = origin && !live ? provenance : null;
  useEffect(() => {
    if (!shown) return;
    const outside = (e: Event) => {
      if (!ref.current?.contains(e.target as Node)) closeProvenance();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && closeProvenance();
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", esc);
    };
  }, [shown]);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || !shown) return;
    el.style.left = `${Math.max(0, Math.min(shown.x, window.innerWidth - el.offsetWidth))}px`;
    el.style.top = `${Math.max(0, Math.min(shown.y, window.innerHeight - el.offsetHeight))}px`;
  }, [shown]);
  if (!shown?.origin) return null;
  return (
    <div ref={ref} className="line-author" role="dialog" aria-label="Who wrote this line?">
      <Answer origin={shown.origin} path={shown.path} />
    </div>
  );
}
