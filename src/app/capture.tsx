import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { buildCapture, saveCapture } from "../domain";
import { CaptureDialog } from "../features/capture/CaptureDialog";
import { createDraftStore } from "../features/capture/drafts";
import { useKb } from "./KbContext";
import { desktop, type Desktop } from "./desktop";

/** The global shortcut of the capture window (see src-tauri/src/shortcut.rs). */
export interface ShortcutStatus {
  setting: string;
  /** `⌃⌥N`; empty when the setting is not understood. */
  label: string;
  registered: boolean;
  /** Why it is not registered, in plain words. */
  error: string | null;
}

/** What `desktop_info` returns. */
export interface DesktopInfo {
  captureShortcut: ShortcutStatus | null;
}

export interface CaptureContextValue {
  /** Opens the capture slip: its own window on the desktop, a dialog in the page elsewhere. */
  open(): void;
  /** The desktop shortcut and whether it works; null on the web. */
  shortcut: ShortcutStatus | null;
}

/** Exported for tests; screens use {@link useCapture}. */
export const CaptureContext = createContext<CaptureContextValue>({ open: () => undefined, shortcut: null });

export function useCapture(): CaptureContextValue {
  return useContext(CaptureContext);
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** Provides 记一笔 to the screens. Lives below the KbProvider. */
export function CaptureProvider({ children, bridge = desktop }: { children: ReactNode; bridge?: Desktop }) {
  const { store, reload, notify } = useKb();
  const windowed = bridge.active && store.kind === "tauri";
  const [shownAt, setShownAt] = useState<number | null>(null);
  const [shortcut, setShortcut] = useState<ShortcutStatus | null>(null);
  const drafts = useMemo(() => createDraftStore(), []);

  useEffect(() => {
    if (!windowed) return;
    let alive = true;
    bridge
      .call<DesktopInfo>("desktop_info")
      .then((info) => {
        if (alive) setShortcut(info.captureShortcut);
      })
      .catch((error: unknown) => bridge.log("warn", `desktop_info failed: ${errorText(error)}`));
    return () => {
      alive = false;
    };
  }, [windowed, bridge]);

  const open = useCallback(() => {
    if (windowed) {
      bridge.call("capture_show").catch((error: unknown) => notify(`打不开记一笔：${errorText(error)}`));
    } else {
      setShownAt(Date.now());
    }
  }, [windowed, bridge, notify]);

  const save = useCallback(
    async (text: string) => {
      await saveCapture(store, buildCapture({ text, source: "web", now: new Date() }));
      void reload();
    },
    [store, reload],
  );

  const value = useMemo(() => ({ open, shortcut }), [open, shortcut]);

  return (
    <CaptureContext.Provider value={value}>
      {children}
      {shownAt !== null ? (
        <CaptureDialog save={save} hide={() => setShownAt(null)} drafts={drafts} shownAt={shownAt} />
      ) : null}
    </CaptureContext.Provider>
  );
}
