import { XIcon } from "@phosphor-icons/react";
import { useEffect } from "react";
import { dismissNotice, type Notice, useHive } from "../store";
import { ICON } from "./icons";

/** How long a confirmation ("info") shows; its fade (styles.css) ends then. */
export const INFO_MS = 4000;

function Toast({ notice: { id, kind, text } }: { notice: Notice }) {
  useEffect(() => {
    if (kind !== "info") return;
    const timer = setTimeout(() => dismissNotice(id), INFO_MS);
    return () => clearTimeout(timer);
  }, [id, kind]);
  return (
    <div className="toast" data-kind={kind}>
      <span className="toast-text">{text}</span>
      <button
        type="button"
        className="toast-close"
        aria-label="Dismiss"
        onClick={() => dismissNotice(id)}
      >
        <XIcon {...ICON} />
      </button>
    </div>
  );
}

/**
 * The notices (10.3) in the window's bottom-right corner, above the status bar, the newest at
 * the bottom. The region is always there, so a screen reader announces what gets added to it.
 */
export function Toasts() {
  const notices = useHive((s) => s.notices);
  return (
    <div className="toasts" role="status" aria-live="polite" aria-label="Messages">
      {notices.map((n) => (
        <Toast key={n.id} notice={n} />
      ))}
    </div>
  );
}
