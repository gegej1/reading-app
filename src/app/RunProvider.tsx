import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { RunTask } from "../domain";
import { useKb } from "./KbContext";
import { desktop, type Desktop } from "./desktop";
import { RunContext, type RunContextValue, type RunPanelView, type StartRequest } from "./RunContext";
import type { EngineKind, RunRecord, RunSettingsView, RunStatus, RunStep } from "./runs";
import { activeRun, answerBlockedHint, editBlockedHint, holdsKb, type RunDetail } from "./runView";

// The panel, the start sheet, the collation view and the history only load
// when they are first opened.
const RunOverlay = lazy(() => import("../features/run/RunOverlay"));

/** Steps kept for the live view (the backend keeps the same number). */
const KEEP_STEPS = 400;

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** True when `status` is the end of a run (a result, or a decision to make). */
function isEnding(status: RunStatus): boolean {
  return status.phase === "needsDecision" || (status.phase === "idle" && status.run?.phase === "finished");
}

/**
 * AI runs in the desktop app: follows `run-status` and `run-step`, offers the
 * start sheet and the run panel, and tells the screens what a run blocks.
 * Elsewhere (web, demo, read-only preview) it provides "no runs".
 */
export function RunProvider({ children, bridge = desktop }: { children: ReactNode; bridge?: Desktop }) {
  const { store } = useKb();
  const available = bridge.active && store.kind === "tauri";
  const [status, setStatus] = useState<RunStatus | null>(null);
  const [steps, setSteps] = useState<RunStep[]>([]);
  const [panelOpen, setPanelOpen] = useState(false);
  const [panelView, setPanelView] = useState<RunPanelView>("current");
  const [pendingStart, setPendingStart] = useState<StartRequest | null>(null);
  const [unseen, setUnseen] = useState(false);
  const panelOpenRef = useRef(false);
  const lastEnding = useRef<string | null>(null);
  const stepsRun = useRef<string | null>(null);

  useEffect(() => {
    panelOpenRef.current = panelOpen;
    if (panelOpen) setUnseen(false);
  }, [panelOpen]);

  const loadSteps = useCallback(() => {
    bridge
      .call<RunStep[]>("run_steps")
      .then((list) => {
        stepsRun.current = list[0]?.runId ?? null;
        setSteps(list.slice(-KEEP_STEPS));
      })
      .catch((error: unknown) => bridge.log("warn", `run_steps failed: ${errorText(error)}`));
  }, [bridge]);

  const onStatus = useCallback(
    (next: RunStatus) => {
      setStatus(next);
      const id = next.run?.id ?? null;
      if (id !== null && stepsRun.current !== id && holdsKb(next.phase)) {
        // A new run: its steps start from scratch.
        stepsRun.current = id;
        setSteps([]);
      }
      if (isEnding(next)) {
        const key = `${id}#${next.phase}`;
        if (lastEnding.current !== key) {
          lastEnding.current = key;
          if (!panelOpenRef.current) setUnseen(true);
        }
      }
    },
    [],
  );

  useEffect(() => {
    if (!available) return;
    let alive = true;
    const stopStatus = bridge.listen<RunStatus>("run-status", onStatus);
    const stopSteps = bridge.listen<RunStep>("run-step", (step) => {
      if (stepsRun.current !== step.runId) {
        stepsRun.current = step.runId;
        setSteps([step]);
        return;
      }
      setSteps((list) => [...list.slice(-(KEEP_STEPS - 1)), step]);
    });
    const stopShow = bridge.listen("run-show", () => {
      setPanelView("current");
      setPanelOpen(true);
    });
    bridge
      .call<RunStatus>("run_status")
      .then((first) => {
        if (!alive) return;
        setStatus((current) => current ?? first);
        // A result that was waiting before this window opened is not news.
        if (isEnding(first)) lastEnding.current = `${first.run?.id ?? null}#${first.phase}`;
      })
      .catch((error: unknown) => bridge.log("warn", `run_status failed: ${errorText(error)}`));
    loadSteps();
    return () => {
      alive = false;
      stopStatus();
      stopSteps();
      stopShow();
    };
  }, [available, bridge, onStatus, loadSteps]);

  const openPanel = useCallback((view: RunPanelView = "current") => {
    setPanelView(view);
    setPanelOpen(true);
  }, []);
  const closePanel = useCallback(() => setPanelOpen(false), []);

  const requestStart = useCallback(
    (task: RunTask, engine: EngineKind | null = null) => {
      if (!available) return;
      setPendingStart({ task, engine });
    },
    [available],
  );
  const cancelStart = useCallback(() => setPendingStart(null), []);

  const confirmStart = useCallback(async () => {
    if (!pendingStart) return;
    const { task, engine } = pendingStart;
    const started = await bridge.call<RunStatus>("run_start", engine ? { task, engine } : { task });
    stepsRun.current = started.run?.id ?? null;
    setSteps([]);
    onStatus(started);
    setPendingStart(null);
    setPanelView("current");
    setPanelOpen(true);
  }, [bridge, pendingStart, onStatus]);

  const command = useCallback(
    async (name: string, args?: Record<string, unknown>) => {
      const next = await bridge.call<RunStatus>(name, args);
      onStatus(next);
    },
    [bridge, onStatus],
  );

  const value = useMemo<RunContextValue>(() => {
    const run = activeRun(status);
    return {
      available,
      status,
      steps,
      activeTask: run?.task ?? null,
      editBlocked: available ? editBlockedHint(status) : null,
      answerBlocked: available ? answerBlockedHint(status) : null,
      unseen,
      panelOpen,
      panelView,
      openPanel,
      closePanel,
      pendingStart,
      requestStart,
      cancelStart,
      confirmStart,
      loadSettings: () => bridge.call<RunSettingsView>("run_settings"),
      cancel: () => command("run_cancel"),
      discard: (runId) => command("run_discard", { runId }),
      keep: (runId) => command("run_keep", { runId }),
      recheck: (runId) => command("run_recheck", { runId }),
      loadRun: (runId) => bridge.call<RunDetail>("run_record", { runId }),
      loadHistory: () => bridge.call<RunRecord[]>("run_history"),
      readSides: async (path) => {
        const [before, after] = await Promise.all([
          bridge.call<string | null>("kb_read_head", { path }).catch(() => null),
          bridge.call<string>("kb_read", { path }).catch(() => null),
        ]);
        return { before, after };
      },
    };
  }, [
    available,
    status,
    steps,
    unseen,
    panelOpen,
    panelView,
    openPanel,
    closePanel,
    pendingStart,
    requestStart,
    cancelStart,
    confirmStart,
    command,
    bridge,
  ]);

  return (
    <RunContext.Provider value={value}>
      {children}
      {available && (panelOpen || pendingStart) ? (
        <Suspense fallback={null}>
          <RunOverlay />
        </Suspense>
      ) : null}
    </RunContext.Provider>
  );
}
