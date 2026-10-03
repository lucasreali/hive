import { goToClicked, notify } from "./notify";
import type { ServiceMessage } from "./protocol";
import { apply } from "./reduce";
import { openLocated, restore } from "./sessions";
import { transport } from "./transport";
import { openTarget } from "./viewer/external";

/** Every service message: `notify` first (it compares with the state still stored), then the store. */
function onMessage(message: ServiceMessage): void {
  notify(message);
  apply(message);
  if (message.type === "session_located") void openLocated(message);
  if (message.type === "restore_sessions") void restore(message.sessions);
  if (message.type === "editor_target") void openTarget(message);
  if (message.type === "notification_clicked") goToClicked(message.agent);
}

/** Connects to the service, at startup and on "Reconnect", always with the same handler. */
export const connect = () => transport.connect(onMessage);
