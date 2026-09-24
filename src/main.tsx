import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { connect } from "./connect";
import { followOpenFile, followPanel } from "./follow";
import { transport } from "./transport";

void connect();
followOpenFile(transport);
followPanel(transport);

export const root = createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
);
