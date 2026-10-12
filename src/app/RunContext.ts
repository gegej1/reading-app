// What the screens know about AI runs (入库, 落裁决): whether runs are
// possible here, whether one holds the knowledge base right now, and how to
// open the run panel or the start sheet. The provider (RunProvider.tsx) only
// exists in the desktop app; everywhere else the default value applies: no
// runs, nothing blocked.

import { createContext, useContext } from "react";
import type { RunTask } from "../domain";
import type { EngineKind, RunSettingsView, RunStatus, RunStep } from "./runs";
import type { RunDetail } from "./runView";

export type RunPanelView = "current" | "history";

/** What the start sheet asks for: a task, and maybe another engine than the settings say. */
export interface StartRequest {
  task: RunTask;
  engine: EngineKind | null;
}

export interface RunContextValue {
  /** Runs are possible here: the desktop app on the knowledge base of this Mac. */
  available: boolean;
  /** The latest status from the backend; null until it is known. */
  status: RunStatus | null;
  /** The steps of the run going on, else of the last one. */
  steps: RunStep[];
  /** The task of the run that holds the knowledge base (from its start until a decision), else null. */
  activeTask: RunTask | null;
  /** Why tracked files cannot be edited right now (`正在入库，结束后再改`), or null. */
  editBlocked: string | null;
  /** Why answers cannot be added to inbox/裁决/ right now (a 落裁决 is going on), or null. */
  answerBlocked: string | null;
  /** A result came in that has not been looked at yet. */
  unseen: boolean;
  panelOpen: boolean;
  panelView: RunPanelView;
  openPanel(view?: RunPanelView): void;
  closePanel(): void;
  /** The start sheet waits for a confirmation (null: closed). */
  pendingStart: StartRequest | null;
  /** Opens the start sheet. Nothing runs until {@link confirmStart}. */
  requestStart(task: RunTask, engine?: EngineKind | null): void;
  cancelStart(): void;
  /** Starts the run the start sheet shows. */
  confirmStart(): Promise<void>;
  /** The engine settings in effect, for the start sheet (loaded on demand). */
  loadSettings(): Promise<RunSettingsView>;
  cancel(): Promise<void>;
  discard(runId: string): Promise<void>;
  keep(runId: string): Promise<void>;
  /** 确认并提交: commits changes to the user's own layer they agreed to. */
  approve(runId: string): Promise<void>;
  /** 重试提交: git refused the commit of a run that passed. */
  retryCommit(runId: string): Promise<void>;
  recheck(runId: string): Promise<void>;
  /** A kept run with its steps (for the history). */
  loadRun(runId: string): Promise<RunDetail>;
  loadHistory(): Promise<RunDetail["record"][]>;
  /** A file as of the last commit (null when it is not in it), and as it is now (null when gone). */
  readSides(path: string): Promise<{ before: string | null; after: string | null }>;
}

const unavailable = () => Promise.reject(new Error("只能在 Mac 上运行"));

/** No runs: the web, the demo, the read-only preview, and tests without a provider. */
export const NO_RUNS: RunContextValue = {
  available: false,
  status: null,
  steps: [],
  activeTask: null,
  editBlocked: null,
  answerBlocked: null,
  unseen: false,
  panelOpen: false,
  panelView: "current",
  openPanel: () => undefined,
  closePanel: () => undefined,
  pendingStart: null,
  requestStart: () => undefined,
  cancelStart: () => undefined,
  confirmStart: unavailable,
  loadSettings: unavailable,
  cancel: unavailable,
  discard: unavailable,
  keep: unavailable,
  approve: unavailable,
  retryCommit: unavailable,
  recheck: unavailable,
  loadRun: unavailable,
  loadHistory: unavailable,
  readSides: unavailable,
};

export const RunContext = createContext<RunContextValue>(NO_RUNS);

export function useRun(): RunContextValue {
  return useContext(RunContext);
}
