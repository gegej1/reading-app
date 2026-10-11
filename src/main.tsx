import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { forwardErrorsToLog } from "./app/desktop";
import "./styles.css";

// Desktop app only: uncaught errors, unhandled rejections and CSP violations
// go to the app log.
forwardErrorsToLog();

const container = document.getElementById("root");
if (!container) {
  throw new Error("Root element #root is missing from index.html");
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
