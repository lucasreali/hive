import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { connect } from "./connect";
import { followPanel } from "./files";
import { transport } from "./transport";
import { followOpenFile } from "./viewer/follow";

void connect();
followOpenFile(transport);
followPanel(transport);

export const root = createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
);
