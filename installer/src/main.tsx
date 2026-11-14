import "@fontsource-variable/inter";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { applyScheme } from "./lib/theme";
import "@app/styles/globals.scss";
import "./styles/setup.scss";

applyScheme();
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
