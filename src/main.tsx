import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { desktop, forwardErrorsToLog } from "./app/desktop";
import { startGateHost } from "./app/gateHost";
import "./styles.css";

// Desktop app only: uncaught errors, unhandled rejections and CSP violations
// go to the app log.
forwardErrorsToLog();

// Desktop app only: this window checks the result of every AI run before the
// backend commits it. Started before React, so it also works while the window
// is hidden or the knowledge base could not be loaded.
startGateHost(desktop);

const container = document.getElementById("root");
if (!container) {
  throw new Error("Root element #root is missing from index.html");
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
