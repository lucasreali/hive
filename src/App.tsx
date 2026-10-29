import { useEffect } from "react";
import { AddProjectDialog } from "./shell/AddProjectDialog";
import { CloseAppDialog, confirmClose } from "./shell/CloseAppDialog";
import { ConfirmDialog } from "./shell/ConfirmDialog";
import { ConnectionBlock } from "./shell/ConnectionBlock";
import { FileMenu, FileNameDialog } from "./shell/FileMenu";
import { NewPullDialog } from "./shell/NewPullDialog";
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
import { Toasts } from "./shell/Toasts";
import { TranslocatedDialog } from "./shell/TranslocatedDialog";
import {
  ProjectMenu,
  RemoveMergedDialog,
  RemoveWorktreeDialog,
  RenameWorktreeDialog,
  WorktreeMenu,
} from "./shell/WorktreeMenu";
import { WorktreePicker } from "./shell/WorktreePicker";
import { installShortcuts } from "./shortcuts";
import { appliedTheme, useHive } from "./store";
import { guardClose, nativeTheme } from "./window";

export function App() {
  const rightPanel = useHive((s) => s.rightPanel);
  const sidebarShown = useHive((s) => s.sidebarShown);
  const status = useHive((s) => s.connection.status);
  const modal = useHive((s) => s.modal);
  // A question asked from a dialog keeps that dialog open underneath, edits and all.
  const under = useHive((s) => (s.modal === "confirm" ? s.question?.back : s.modal));
  const hasProjects = useHive((s) => Object.keys(s.projects ?? {}).length > 0);
  const setting = useHive((s) => s.settings.appearance.theme);
  const theme = useHive(appliedTheme);
  const focused = useHive((s) => s.focused);
  // Nothing works without the service: the workspace is inert under the block (#29).
  // The title bar stays usable, so the window can still be closed.
  useEffect(installShortcuts, []);
  useEffect(() => guardClose(confirmClose), []);
  // The theme's CSS variables hang off this attribute (src/styles.css); the window follows.
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    nativeTheme(setting, theme);
  }, [setting, theme]);
  // The endless animations rest while Hive is not focused (D19.8, src/styles.css).
  useEffect(() => {
    document.documentElement.toggleAttribute("data-focused", focused);
  }, [focused]);
  // The first run on Windows with WSL asks where the service runs before any starts (12.5.4).
  const asking = useHive((s) => s.appMode?.mode === null);
  const blocked = status === "version_mismatch" || status === "disconnected" || asking;
  return (
    <div className="app">
      <TitleBar />
      <div className="workspace">
        <main className="main" inert={blocked}>
          {sidebarShown && <Sidebar />}
          <TerminalArea />
          {rightPanel === "files" && <RightPanel />}
        </main>
        {modal === "add-project" && !blocked && <AddProjectDialog />}
        {modal === "new-worktree" && !blocked && hasProjects && <NewWorktreeDialog />}
        {modal === "worktree-picker" && !blocked && <WorktreePicker />}
        {modal === "remove-worktree" && !blocked && <RemoveWorktreeDialog />}
        {modal === "rename-worktree" && !blocked && <RenameWorktreeDialog />}
        {under === "new-space" && !blocked && <SpaceDialog editing={false} />}
        {under === "edit-space" && !blocked && <SpaceDialog editing />}
        {modal === "settings" && !blocked && <SettingsDialog />}
        {modal === "remove-merged" && !blocked && <RemoveMergedDialog />}
        {modal === "palette" && !blocked && <Palette />}
        {modal === "file-name" && !blocked && <FileNameDialog />}
        {modal === "confirm" && !blocked && <ConfirmDialog />}
        {modal === "new-pull" && !blocked && <NewPullDialog />}
        {modal === "close-app" && <CloseAppDialog />}
        {modal === "update-app" && <CloseAppDialog updating />}
        {modal === "translocated" && <TranslocatedDialog />}
        {!blocked && <WorktreeMenu />}
        {!blocked && <ProjectMenu />}
        {!blocked && <SessionMenu />}
        {!blocked && <FileMenu />}
        <ConnectionBlock />
      </div>
      <Toasts />
      <StatusBar />
    </div>
  );
}
