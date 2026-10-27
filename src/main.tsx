import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { connect } from "./connect";
import { followDock } from "./dock";
import { followOpenFile, followPanel, followView } from "./follow";
import { setFocused, setVisible } from "./store";
import { transport } from "./transport";
import { followTray } from "./tray";
import { watchFocus, watchVisible } from "./window";

// The update check answers on the channel `connect` gives, so it goes second.
void connect().then(() => void transport.checkUpdate());
followOpenFile(transport);
followPanel(transport);
watchFocus(setFocused);
watchVisible(setVisible);
followView(transport);
followDock();
followTray();

export const root = createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
);
