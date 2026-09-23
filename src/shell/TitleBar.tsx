import { HiveIcon, MaximizeIcon, MinimizeIcon, WindowCloseIcon } from "./icons";
import { windowAction } from "./window";

// The window has no native decorations; this bar drags it and holds the window buttons.
// ponytail: no "project / worktree" breadcrumb yet; it shows the active tab once tabs exist (1.7).
export function TitleBar() {
  return (
    <header className="titlebar">
      <div className="titlebar-brand" data-tauri-drag-region>
        <HiveIcon />
        <span>Hive</span>
      </div>
      <div className="window-controls">
        <button type="button" title="Minimize" onClick={() => windowAction("minimize")}>
          <MinimizeIcon />
        </button>
        <button type="button" title="Maximize" onClick={() => windowAction("toggleMaximize")}>
          <MaximizeIcon />
        </button>
        <button type="button" title="Close" className="close" onClick={() => windowAction("close")}>
          <WindowCloseIcon />
        </button>
      </div>
    </header>
  );
}
