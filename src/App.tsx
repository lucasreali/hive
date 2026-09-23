import { RightPanel } from "./shell/RightPanel";
import { Sidebar } from "./shell/Sidebar";
import { StatusBar } from "./shell/StatusBar";
import { TerminalArea } from "./shell/TerminalArea";
import { TitleBar } from "./shell/TitleBar";
import { useHive } from "./store";

export function App() {
  const rightPanel = useHive((s) => s.rightPanel);
  return (
    <div className="app">
      <TitleBar />
      <main className="main">
        <Sidebar />
        <TerminalArea />
        {rightPanel === "files" && <RightPanel />}
      </main>
      <StatusBar />
    </div>
  );
}
