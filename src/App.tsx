import { useEffect } from "react";
import { AddProjectDialog } from "./shell/AddProjectDialog";
import { CloseAppDialog, confirmClose } from "./shell/CloseAppDialog";
import { ConfirmDialog } from "./shell/ConfirmDialog";
import { ConnectionBlock } from "./shell/ConnectionBlock";
import { FileMenu, FileNameDialog } from "./shell/FileMenu";
import { NewWorktreeDialog } from "./shell/NewWorktreeDialog";
import { Palette } from "./shell/Palette";
import { RightPanel } from "./shell/RightPanel";
import { SessionMenu } from "./shell/SessionsView";
import { SettingsDialog } from "./shell/SettingsDialog";
import { Sidebar } from "./shell/Sidebar";
import { SpaceDialog } from "./shell/SpaceDialog";
import { StatusBar } from "./shell/StatusBar";
import { TerminalArea } from "./shell/TerminalArea";
import { TitleBar } from "./shell/TitleBar";
import {
  ProjectMenu,
  RemoveMergedDialog,
  RemoveWorktreeDialog,
  RenameWorktreeDialog,
  WorktreeMenu,
} from "./shell/WorktreeMenu";
import { WorktreePicker } from "./shell/WorktreePicker";
import { installShortcuts } from "./shortcuts";
import { useHive } from "./store";
import { guardClose } from "./window";

export function App() {
  const rightPanel = useHive((s) => s.rightPanel);
  const status = useHive((s) => s.connection.status);
  const modal = useHive((s) => s.modal);
  const hasProjects = useHive((s) => Object.keys(s.projects ?? {}).length > 0);
  const theme = useHive((s) => s.settings.appearance.theme);
  // Nothing works without the service: the workspace is inert under the block (#29).
  // The title bar stays usable, so the window can still be closed.
  useEffect(installShortcuts, []);
  useEffect(() => guardClose(confirmClose), []);
  // The theme's CSS variables hang off this attribute (src/styles.css).
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);
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
        {modal === "remove-worktree" && !blocked && <RemoveWorktreeDialog />}
        {modal === "rename-worktree" && !blocked && <RenameWorktreeDialog />}
        {modal === "new-space" && !blocked && <SpaceDialog editing={false} />}
        {modal === "edit-space" && !blocked && <SpaceDialog editing />}
        {modal === "settings" && !blocked && <SettingsDialog />}
        {modal === "remove-merged" && !blocked && <RemoveMergedDialog />}
        {modal === "palette" && !blocked && <Palette />}
        {modal === "file-name" && !blocked && <FileNameDialog />}
        {modal === "confirm" && !blocked && <ConfirmDialog />}
        {modal === "close-app" && <CloseAppDialog />}
        {modal === "update-app" && <CloseAppDialog updating />}
        {!blocked && <WorktreeMenu />}
        {!blocked && <ProjectMenu />}
        {!blocked && <SessionMenu />}
        {!blocked && <FileMenu />}
        <ConnectionBlock />
      </div>
      <StatusBar />
    </div>
  );
}
