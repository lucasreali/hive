import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { followPanel } from "./files";
import { notify } from "./notify";
import { apply } from "./store";
import { transport } from "./transport";
import { openExternal } from "./viewer/external";
import { followOpenFile } from "./viewer/follow";

void transport.connect((message) => {
  notify(message);
  apply(message);
  if (message.type === "editor_target") void openExternal(message);
});
followOpenFile(transport);
followPanel(transport);

export const root = createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
);
