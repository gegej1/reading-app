import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { loadKb, type KbModel } from "../domain";
import { canWrite, createStore, storeLabel } from "../store/selectStore";
import type { KbStore } from "../store/types";
import { KbContext, type KbContextValue } from "./KbContext";
import { desktop, kbLoadSummary, type Desktop } from "./desktop";
import { createKbEditor } from "./editFile";
import { makeLinkResolver } from "./links";
import { createPhoneExporter } from "./phoneExport";

type LoadState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; store: KbStore; model: KbModel };

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** Label of the window this provider runs in on the desktop. */
const MAIN_WINDOW = "main";

/** Payload of the backend's `kb-changed` event (src-tauri/src/events.rs). */
export interface KbChanged {
  reason: "write" | "create" | "capture" | "sync" | "run" | "settings" | "bridge";
  path: string | null;
  /** The window that made the change, if a window did. */
  window: string | null;
}

/** The iPhone export of the desktop app: one per window, kept up to date by every load. */
const phoneExporter = createPhoneExporter(desktop);

/** Hands a fresh model to the iPhone export (desktop app only). */
function exportForPhone(store: KbStore, model: KbModel) {
  if (store.kind === "tauri") phoneExporter.loaded(model);
}

/** The tray menu shows how many captures and pending rulings there are (desktop only). */
function reportCounts(bridge: Desktop, store: KbStore, model: KbModel) {
  if (!bridge.active || store.kind !== "tauri") return;
  bridge
    .call("tray_set_counts", {
      inbox: model.dashboard.inboxCaptures,
      rulings: model.dashboard.pendingRulings.total,
    })
    .catch((error: unknown) => bridge.log("warn", `tray_set_counts failed: ${errorText(error)}`));
}

/** KB folder of stores that know it (the desktop app); null otherwise. */
async function rootPath(store: KbStore): Promise<string | null> {
  const candidate = store as Partial<{ root(): Promise<{ path: string }> }>;
  if (typeof candidate.root !== "function") return null;
  try {
    return (await candidate.root()).path;
  } catch {
    return null;
  }
}

export interface KbProviderProps {
  children: ReactNode;
  /** Use this store instead of picking one for the runtime (tests, demos). */
  store?: KbStore;
}

/** Loads the knowledge base once and provides it to the app. */
export function KbProvider({ children, store: givenStore }: KbProviderProps) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [reloading, setReloading] = useState(false);
  const [pendingEdits, setPendingEdits] = useState(0);
  const [notice, setNotice] = useState<string | null>(null);
  const [sourceDetail, setSourceDetail] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const noticeTimer = useRef<number | undefined>(undefined);
  // Every load gets a number; only the latest one may update the screen, so a
  // slow reload can never overwrite the result of a newer one.
  const loadSeq = useRef(0);

  useEffect(() => {
    let cancelled = false;
    const seq = ++loadSeq.current;
    setState({ status: "loading" });
    (async () => {
      try {
        const store = givenStore ?? (await createStore());
        const root = await rootPath(store);
        if (!cancelled) setSourceDetail(root);
        const model = await loadKb(store);
        if (!cancelled && seq === loadSeq.current) {
          setState({ status: "ready", store, model });
          desktop.log("info", kbLoadSummary("kb loaded", model, store));
          reportCounts(desktop, store, model);
          exportForPhone(store, model);
        }
      } catch (error) {
        if (!cancelled && seq === loadSeq.current) {
          setState({ status: "error", message: errorText(error) });
          desktop.log("error", `kb load failed: ${errorText(error)}`);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [givenStore, attempt]);

  useEffect(() => () => window.clearTimeout(noticeTimer.current), []);

  const notify = useCallback((text: string) => {
    setNotice(text);
    window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNotice(null), 4200);
  }, []);

  const ready = state.status === "ready" ? state : null;
  const store = ready?.store;

  const reload = useCallback(async () => {
    if (!store) return;
    const seq = ++loadSeq.current;
    setReloading(true);
    try {
      const model = await loadKb(store);
      if (seq === loadSeq.current) {
        setState({ status: "ready", store, model });
        desktop.log("info", kbLoadSummary("kb reloaded", model, store));
        reportCounts(desktop, store, model);
        exportForPhone(store, model);
      }
    } catch (error) {
      if (seq === loadSeq.current) {
        notify(`重新载入失败：${errorText(error)}`);
        desktop.log("warn", `kb reload failed: ${errorText(error)}`);
      }
    } finally {
      if (seq === loadSeq.current) setReloading(false);
    }
  }, [store, notify]);

  const reloadRef = useRef(reload);
  useEffect(() => {
    reloadRef.current = reload;
  }, [reload]);

  // Changes made elsewhere (the capture window, the command line, a sync
  // that pulled) reload the model. This window's own edits reload already.
  const watching = store?.kind === "tauri";
  useEffect(() => {
    if (!watching) return;
    return desktop.listen<KbChanged>("kb-changed", (change) => {
      if (change.window !== MAIN_WINDOW) void reloadRef.current();
    });
  }, [watching]);

  // One editor (and so one queue) per store: edits never interleave, and the
  // model is reloaded after each attempt so the screen shows what is on disk.
  const editor = useMemo(
    () =>
      store
        ? createKbEditor(store, {
            afterEdit: () => reloadRef.current(),
            onPendingChange: setPendingEdits,
          })
        : null,
    [store],
  );

  const editFile = useCallback<KbContextValue["editFile"]>(
    async (path, edit, message) => {
      if (!editor) throw new Error("知识库还没有载入");
      return editor(path, edit, message);
    },
    [editor],
  );

  const value = useMemo<KbContextValue | null>(() => {
    if (!ready) return null;
    return {
      store: ready.store,
      model: ready.model,
      writable: canWrite(ready.store),
      reloading,
      editing: pendingEdits > 0,
      reload,
      editFile,
      resolveLink: makeLinkResolver(ready.model),
      notice,
      notify,
      sourceLabel: storeLabel(ready.store),
      sourceDetail,
    };
  }, [ready, reloading, pendingEdits, reload, editFile, notice, notify, sourceDetail]);

  if (state.status === "loading") {
    return (
      <div className="boot" role="status" aria-live="polite">
        <div className="boot__mark" aria-hidden="true">
          底色
        </div>
        <p className="boot__text">正在翻开知识库…</p>
      </div>
    );
  }

  if (state.status === "error" || !value) {
    return (
      <div className="boot boot--error" role="alert">
        <div className="boot__mark" aria-hidden="true">
          底色
        </div>
        <h1 className="boot__title">打不开知识库</h1>
        <p className="boot__text">{state.status === "error" ? state.message : "未知错误"}</p>
        <button type="button" className="btn" onClick={() => setAttempt((n) => n + 1)}>
          再试一次
        </button>
      </div>
    );
  }

  return <KbContext.Provider value={value}>{children}</KbContext.Provider>;
}
