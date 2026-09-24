import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { notify } from "./notify";
import { apply } from "./store";
import { transport } from "./transport";
import { followOpenFile } from "./viewer/follow";

void transport.connect((message) => {
  notify(message);
  apply(message);
});
followOpenFile(transport);

export const root = createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
);
