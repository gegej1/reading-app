import { useCallback, useEffect, useRef, useState } from "react";
import { buildCapture, isBlankCapture, saveCapture, type CaptureSource } from "../domain/capture";
import type { Desktop } from "../app/desktop";
import { CapturePanel } from "../features/capture/CapturePanel";
import type { DraftStore } from "../features/capture/drafts";
import type { KbStore } from "../store/types";

/** Sent by the backend every time the capture window is shown. */
export const SHOWN_EVENT = "capture-shown";

export interface CaptureWindowProps {
  store: Pick<KbStore, "create">;
  drafts: DraftStore;
  bridge: Desktop;
}

/**
 * The page of the floating capture window (capture.html). On the desktop it
 * hides itself through the backend, refocuses whenever it is shown, and
 * hides when another app is clicked, unless something is written in it.
 */
export function CaptureWindow({ store, drafts, bridge }: CaptureWindowProps) {
  const [shownAt, setShownAt] = useState(() => Date.now());
  const text = useRef(drafts.load());
  const source: CaptureSource = bridge.active ? "mac" : "web";

  const hide = useCallback(() => {
    if (!bridge.active) return;
    bridge.call("capture_hide").catch((error: unknown) => {
      bridge.log("warn", `capture_hide failed: ${error instanceof Error ? error.message : String(error)}`);
    });
  }, [bridge]);

  useEffect(() => bridge.listen(SHOWN_EVENT, () => setShownAt(Date.now())), [bridge]);

  useEffect(() => {
    const onBlur = () => {
      if (isBlankCapture(text.current)) hide();
    };
    window.addEventListener("blur", onBlur);
    return () => window.removeEventListener("blur", onBlur);
  }, [hide]);

  const save = useCallback(
    async (value: string) => {
      const saved = await saveCapture(store, buildCapture({ text: value, source, now: new Date() }));
      bridge.log("info", `capture saved: ${saved.path}`);
    },
    [store, source, bridge],
  );

  return (
    <main className="capture-window">
      <CapturePanel
        variant="window"
        save={save}
        hide={hide}
        drafts={drafts}
        shownAt={shownAt}
        onTextChange={(value) => (text.current = value)}
      />
    </main>
  );
}
