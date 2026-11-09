import { useEffect } from "react";
import { showBrowserTab, useHive } from "../store";
import { transport } from "../transport";
import { openLink } from "./Markdown";

/**
 * The tab of the worktree's agent's browser (24.6, 25.2), view only: its page's URL, a button to
 * open it in the system's browser, the agent's last action, and the page, scaled to fit the tab's
 * width. Its screencast runs only while the tab is shown and the window visible; each frame is
 * acknowledged once drawn, so the next comes. Stopped, it stays with what it last showed.
 */
export function BrowserView(props: { worktree: string; side?: "left" | "right" }) {
  const { worktree, side } = props;
  const visible = useHive((s) => s.visible);
  const status = useHive((s) => s.browsers[worktree]);
  const action = useHive((s) => s.browserActions[worktree]);
  const frame = useHive((s) => (s.browserFrame?.worktree === worktree ? s.browserFrame : null));
  useEffect(() => {
    if (!visible) return;
    void transport.watchBrowser(worktree);
    return () => void transport.watchBrowser(null);
  }, [worktree, visible]);
  useEffect(() => {
    if (!frame) return;
    const drawn = requestAnimationFrame(() => void transport.browserDrawn());
    return () => cancelAnimationFrame(drawn);
  }, [frame]);
  const url = status?.url ?? null;
  // Only a web page: the page chose where it went, and never a local file or an app's scheme.
  const link = url && /^https?:/i.test(url) ? url : "";
  return (
    <section
      className="browser"
      aria-label="Agent's browser"
      data-pane={side}
      onPointerDown={() => showBrowserTab(worktree)}
    >
      <div className="browser-bar">
        <span className="browser-url" title={url ?? undefined}>
          {url ?? "No page"}
        </span>
        {!status?.running && <span className="browser-state">Browser closed</span>}
        <button
          type="button"
          className="secondary"
          disabled={!link}
          onClick={() => void openLink(link)}
        >
          Open in my browser
        </button>
      </div>
      {action && (
        <div className="browser-caption" title={action.action}>
          {action.agentTitle ?? "Claude"} · {action.action}
        </div>
      )}
      {status?.error && <div className="browser-empty">{status.error}</div>}
      <div className="browser-page">
        {frame && (
          <img
            className="browser-frame"
            src={`data:image/jpeg;base64,${frame.data}`}
            alt="The agent's browser"
            draggable={false}
          />
        )}
      </div>
    </section>
  );
}
