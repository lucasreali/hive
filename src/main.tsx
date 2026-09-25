import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { connect } from "./connect";
import { followOpenFile, followPanel, followView } from "./follow";
import { setFocused } from "./store";
import { transport } from "./transport";
import { watchFocus } from "./window";

// The update check answers on the channel `connect` gives, so it goes second.
void connect().then(() => transport.checkUpdate());
followOpenFile(transport);
followPanel(transport);
watchFocus(setFocused);
followView(transport);

export const root = createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
);
