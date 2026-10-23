import { bounceFor } from "./dock";
import { runMenuItem } from "./menu";
import { goToClicked, notify } from "./notify";
import type { ServiceMessage } from "./protocol";
import { apply } from "./reduce";
import { openLocated, restore } from "./sessions";
import { requestQuit } from "./shell/CloseAppDialog";
import { dropFiles, reattachTerminal, showOpenFailure } from "./terminals";
import { transport } from "./transport";
import { openTarget } from "./viewer/external";

/** Every service message: `notify` first (it compares with the state still stored), then the store. */
function onMessage(message: ServiceMessage): void {
  notify(message);
  bounceFor(message);
  apply(message);
  if (message.type === "session_located") void openLocated(message);
  if (message.type === "restore_sessions") void restore(message.sessions);
  if (message.type === "editor_target") void openTarget(message);
  // An OS notification's click (13.5) or a pending agent chosen in the menu bar icon (17.22).
  if (message.type === "notification_clicked" || message.type === "tray_clicked") {
    goToClicked(message.agent);
  }
  if (message.type === "quit_requested") requestQuit();
  if (message.type === "menu") runMenuItem(message.item);
  if (message.type === "files_dropped") dropFiles(message.text, message.x, message.y);
  if (message.type === "terminal_kept") {
    showOpenFailure(reattachTerminal(message.channel, message.cwd));
  }
}

/** Connects to the service, at startup and on "Reconnect", always with the same handler. */
export const connect = () => transport.connect(onMessage);
