// AI runs (入库, 落裁决) as the desktop backend reports them: the shapes of
// the `run_*` commands and of the `run-status` and `run-step` events (see
// src-tauri/src/run_commands.rs and src-tauri/src/run/). Types only: the run
// panel and the settings screen build on these.
//
// | command           | arguments                      | returns        |
// |-------------------|--------------------------------|----------------|
// | `run_start`       | `{ task, engine?, model? }`    | `RunStatus`    |
// | `run_cancel`      | –                              | `RunStatus`    |
// | `run_status`      | –                              | `RunStatus`    |
// | `run_steps`       | –                              | `RunStep[]`    |
// | `run_discard`     | `{ runId }`                    | `RunStatus`    |
// | `run_keep`        | `{ runId }`                    | `RunStatus`    |
// | `run_recheck`     | `{ runId }`                    | `RunStatus`    |
// | `run_history`     | –                              | `RunRecord[]`  |
// | `run_settings`    | –                              | `RunSettingsView` |
// | `kb_read_head`    | `{ path }`                     | `string \| null` (a file as of the last commit) |
//
// `run_gate_input` and `run_gate_report` belong to the gate host (gateHost.ts).

import type { GateReport, RunTask } from "../domain";

export type EngineKind = "claude" | "codex";

/** Where a run stands. `idle`: no run is going on. */
export type RunPhase =
  | "idle"
  | "starting"
  | "running"
  | "checking"
  | "committing"
  | "needsDecision"
  | "finished";

/** How a run ended. */
export type RunOutcome =
  | "committed"
  | "kept"
  | "discarded"
  | "nothingToDo"
  | "refused"
  | "failed"
  | "cancelled";

export interface EngineExit {
  code: number | null;
  cancelled: boolean;
  timedOut: boolean;
  spawnError: string | null;
  stderrTail: string;
}

/** What the engine's own output said about the run. */
export interface StreamSummary {
  model: string | null;
  keySource: string | null;
  finished: boolean;
  isError: boolean;
  subtype: string | null;
  finalMessage: string | null;
  error: string | null;
  /** Claude Code only. */
  costUsd: number | null;
  turns: number | null;
  durationMs: number | null;
  toolCalls: number;
  denials: number;
  /** Codex only. */
  inputTokens: number | null;
  outputTokens: number | null;
}

export interface EngineReport {
  ok: boolean;
  /** Why the engine did not finish normally, in plain words. */
  failure: string | null;
  exit: EngineExit;
  stream: StreamSummary;
  durationMs: number;
}

/** One run (`run.json`). */
export interface RunRecord {
  id: string;
  task: RunTask;
  engine: EngineKind;
  model: string | null;
  /** `ui` or `command line`. */
  trigger: string;
  date: string;
  /** Milliseconds since the epoch. */
  startedAt: number;
  endedAt: number | null;
  phase: RunPhase;
  outcome: RunOutcome | null;
  /** One line for the user: what happened, or what is waited for. */
  message: string | null;
  queue: string[];
  base: string | null;
  engineReport: EngineReport | null;
  gate: GateReport | null;
  keptDespite: string[];
  commit: string | null;
  discardedTo: string | null;
}

/** Payload of `run-status`, and what the run commands return. */
export interface RunStatus {
  phase: RunPhase;
  /** The run going on, else the last one that ended. */
  run: RunRecord | null;
  /** While `phase` is `checking`: which check the gate is asked to answer. */
  gateAttempt: number;
}

export type RunStepKind = "init" | "tool" | "text" | "denied" | "result" | "error" | "note";

/** Payload of `run-step`: one step of the run going on. */
export interface RunStep {
  runId: string;
  /** Counts up from 1 within a run. */
  seq: number;
  /** Milliseconds since the run started. */
  elapsedMs: number;
  kind: RunStepKind;
  /** 读, 写, 改, 查找, 删, 运行 … */
  verb: string | null;
  /** A repo-relative path, a search pattern or a command. */
  target: string | null;
  text: string | null;
}

/** What `run_settings` returns: the engine settings of config.json in effect. */
export interface RunSettingsView {
  engine: EngineKind;
  model: string | null;
  budgetUsd: number;
  runTimeoutMinutes: number;
  claudePath: string | null;
  codexPath: string | null;
  /** Where the programs were found; null when they were not. */
  claudeProgram: string | null;
  codexProgram: string | null;
  /** Settings that were ignored, in plain words. */
  problems: string[];
}
