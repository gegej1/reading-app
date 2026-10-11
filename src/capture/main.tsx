// Entry of the capture window (capture.html): only the capture slip, none of
// the knowledge-base screens. Outside the desktop app (a browser preview of
// this page) it saves into the in-memory demo knowledge base.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { desktop, forwardErrorsToLog } from "../app/desktop";
import { createDraftStore } from "../features/capture/drafts";
import { createStore } from "../store/selectStore";
import { CaptureWindow } from "./CaptureWindow";
import "../capture.css";

forwardErrorsToLog();

const container = document.getElementById("root");
if (!container) {
  throw new Error("Root element #root is missing from capture.html");
}

void createStore().then((store) => {
  createRoot(container).render(
    <StrictMode>
      <CaptureWindow store={store} drafts={createDraftStore()} bridge={desktop} />
    </StrictMode>,
  );
});
