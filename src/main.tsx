import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { followPanel } from "./files";
import { notify } from "./notify";
import { apply } from "./store";
import { transport } from "./transport";

void transport.connect((message) => {
  notify(message);
  apply(message);
});
followPanel(transport);

export const root = createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
);
