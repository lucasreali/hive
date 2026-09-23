import { AddProjectDialog } from "./shell/AddProjectDialog";
import { ConnectionBlock } from "./shell/ConnectionBlock";
import { RightPanel } from "./shell/RightPanel";
import { Sidebar } from "./shell/Sidebar";
import { StatusBar } from "./shell/StatusBar";
import { TerminalArea } from "./shell/TerminalArea";
import { TitleBar } from "./shell/TitleBar";
import { useHive } from "./store";

export function App() {
  const rightPanel = useHive((s) => s.rightPanel);
  const status = useHive((s) => s.connection.status);
  const modal = useHive((s) => s.modal);
  // Nothing works without the service: the workspace is inert under the block (#29).
  // The title bar stays usable, so the window can still be closed.
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
        <ConnectionBlock />
      </div>
      <StatusBar />
    </div>
  );
}
