import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { apply } from "./store";
import { transport } from "./transport";

void transport.connect(apply);

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
