import { notify } from "./notify";
import { apply, type ServiceMessage } from "./store";
import { transport } from "./transport";
import { openExternal, openFolder } from "./viewer/external";

/** Every service message: `notify` first (it compares with the state still stored), then the store. */
function onMessage(message: ServiceMessage): void {
  notify(message);
  apply(message);
  if (message.type === "editor_target") {
    void (message.path === "" ? openFolder(message) : openExternal(message));
  }
}

/** Connects to the service, at startup and on "Reconnect", always with the same handler. */
export const connect = () => transport.connect(onMessage);
