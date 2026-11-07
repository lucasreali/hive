import { useEffect } from "react";
import { useHive } from "../store";
import { transport } from "../transport";

/**
 * The right panel's Browser tab (24.6): what the agent's browser of the worktree shows, view
 * only. Its screencast runs only while the tab is shown and the window visible; each frame is
 * acknowledged once drawn, so the next comes.
 */
export function BrowserView({ worktree }: { worktree: string }) {
  const visible = useHive((s) => s.visible);
  const status = useHive((s) => s.browsers[worktree]);
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
  if (!status?.running) {
    return (
      <div className="browser-empty">
        {status?.error ??
          "No browser yet: one starts when an agent of this worktree uses its browser tools."}
      </div>
    );
  }
  return (
    <div className="browser">
      <div className="browser-url" title={status.url ?? undefined}>
        {status.url ?? "No page"}
      </div>
      {frame && (
        <img
          className="browser-frame"
          src={`data:image/jpeg;base64,${frame.data}`}
          alt="The agent's browser"
          draggable={false}
        />
      )}
    </div>
  );
}
