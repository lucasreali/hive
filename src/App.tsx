import { useEffect } from "react";
import { AddProjectDialog } from "./shell/AddProjectDialog";
import { CloseAppDialog, confirmClose } from "./shell/CloseAppDialog";
import { ConnectionBlock } from "./shell/ConnectionBlock";
import { NewWorktreeDialog } from "./shell/NewWorktreeDialog";
import { RightPanel } from "./shell/RightPanel";
import { Sidebar } from "./shell/Sidebar";
import { StatusBar } from "./shell/StatusBar";
import { TerminalArea } from "./shell/TerminalArea";
import { TitleBar } from "./shell/TitleBar";
import { WorktreePicker } from "./shell/WorktreePicker";
import { installShortcuts } from "./shortcuts";
import { useHive } from "./store";
import { guardClose } from "./window";

export function App() {
  const rightPanel = useHive((s) => s.rightPanel);
  const status = useHive((s) => s.connection.status);
  const modal = useHive((s) => s.modal);
  const hasProjects = useHive((s) => Object.keys(s.projects ?? {}).length > 0);
  // Nothing works without the service: the workspace is inert under the block (#29).
  // The title bar stays usable, so the window can still be closed.
  useEffect(installShortcuts, []);
  useEffect(() => guardClose(confirmClose), []);
  const blocked = status === "version_mismatch" || status === "disconnected";
  return (
    <div className="app">
      <TitleBar />
      <div className="workspace">
        <main className="main" inert={blocked}>
          <Sidebar />
          <TerminalArea />
          {rightPanel === "files" && <RightPanel />}
        </main>
        {modal === "add-project" && !blocked && <AddProjectDialog />}
        {modal === "new-worktree" && !blocked && hasProjects && <NewWorktreeDialog />}
        {modal === "worktree-picker" && !blocked && <WorktreePicker />}
        {modal === "close-app" && <CloseAppDialog />}
        <ConnectionBlock />
      </div>
      <StatusBar />
    </div>
  );
}
