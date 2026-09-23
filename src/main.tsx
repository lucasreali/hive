import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { apply } from "./store";
import { transport } from "./transport";

void transport.connect(apply);

export const root = createRoot(document.getElementById("root") as HTMLElement);
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
);
