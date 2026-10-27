import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { connect } from "./connect";
import { followDock } from "./dock";
import { followOpenFile, followPanel, followView } from "./follow";
import { appliedTheme, setFocused, setSystemTheme, setVisible, useHive } from "./store";
import { transport } from "./transport";
import { followTray } from "./tray";
import { isMac, watchColorScheme, watchFocus, watchVisible } from "./window";

// The update check answers on the channel `connect` gives, so it goes second.
void connect().then(() => void transport.checkUpdate());
followOpenFile(transport);
followPanel(transport);
watchFocus(setFocused);
watchVisible(setVisible);
followView(transport);
followDock();
followTray();
watchColorScheme(setSystemTheme);
// Before the first render: the last run's theme (18.33), and macOS' own scrollbars and cursor.
document.documentElement.dataset.theme = appliedTheme(useHive.getState());
if (isMac()) document.documentElement.dataset.mac = "";

export const root = createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
);
