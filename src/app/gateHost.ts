// The gate host: the place where the gate (src/domain/gate.ts) actually runs
// in the desktop app.
//
// The backend owns a run from start to commit, but the gate needs the
// Markdown, 底色 and wikilink parsers, which exist only in TypeScript. So
// when an engine is done, the backend lists the changes, announces a check
// (`run-status` with phase `checking`), and waits for this window to fetch
// the input (`run_gate_input`), run the gate and hand in the report
// (`run_gate_report`). Nothing is committed without that report, and the
// backend checks the report against the changes before it uses it.
//
// This is plain module code, not a React component: it also works while the
// window is hidden (a run started from the command line) and when the
// knowledge base failed to load.

import { runGate, type GateInput, type GateIssue, type GateReport } from "../domain";
import type { Desktop } from "./desktop";
import type { RunStatus } from "./runs";

/** What `run_gate_input` returns: the gate's input, and which check it belongs to. */
export type GateRequest = GateInput & { runId: string; attempt: number };

// The newest input checked for each run, so the run panel can show exactly
// what the gate saw (the before and after of every changed file). Kept in
// memory only: it holds knowledge-base content.
const checked = new Map<string, GateRequest>();
const watchers = new Set<() => void>();
/** How many runs' inputs are kept. */
const KEEP_INPUTS = 5;

function remember(input: GateRequest) {
  const previous = checked.get(input.runId);
  if (previous && previous.attempt > input.attempt) return;
  checked.delete(input.runId);
  checked.set(input.runId, input);
  while (checked.size > KEEP_INPUTS) {
    const oldest = checked.keys().next().value;
    if (oldest === undefined) break;
    checked.delete(oldest);
  }
  for (const watcher of watchers) watcher();
}

/** The newest gate input of a run this window checked, or null. */
export function checkedInput(runId: string): GateRequest | null {
  return checked.get(runId) ?? null;
}

/** Calls `watcher` whenever a new input was checked; returns the unsubscribe function. */
export function watchCheckedInputs(watcher: () => void): () => void {
  watchers.add(watcher);
  return () => watchers.delete(watcher);
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * The report for a gate that threw: nothing is approved and no queue file
 * counts as copied, so the run waits for a person. Queue files the run
 * touched are put back.
 */
export function crashedReport(input: GateInput, error: unknown): GateReport {
  const queued = new Set(input.queue.map((file) => file.path));
  const issue: GateIssue = {
    rule: "gate",
    path: null,
    message: `检查程序自己出错了，没有完成检查：${errorText(error)}`,
  };
  return {
    ok: false,
    violations: [issue],
    warnings: [],
    approvals: [],
    commitPaths: input.changes.map((change) => change.path).filter((path) => !queued.has(path)),
    deleteQueuePaths: [],
    restoreQueuePaths: input.changes.map((change) => change.path).filter((path) => queued.has(path)),
    summary: "没通过检查：检查程序出错了",
    final: null,
  };
}

/**
 * Answers every check the backend announces, until the returned function is
 * called. Does nothing outside the desktop app.
 */
export function startGateHost(
  bridge: Desktop,
  gate: (input: GateInput) => GateReport = runGate,
): () => void {
  if (!bridge.active) return () => undefined;
  let stopped = false;
  // Checks that were answered or are being answered: `<run>#<attempt>`.
  const taken = new Set<string>();

  const answer = async (status: RunStatus) => {
    if (stopped || status.phase !== "checking" || !status.run) return;
    const runId = status.run.id;
    const attempt = status.gateAttempt;
    const key = `${runId}#${attempt}`;
    if (taken.has(key)) return;
    taken.add(key);
    try {
      const input = await bridge.call<GateRequest>("run_gate_input", { runId, attempt });
      remember(input);
      let report: GateReport;
      try {
        report = gate(input);
      } catch (error) {
        report = crashedReport(input, error);
        bridge.log("error", `gate crashed on run ${runId}: ${errorText(error)}`);
      }
      await bridge.call("run_gate_report", { runId, attempt, report });
      bridge.log(
        "info",
        `gate: run ${runId} check ${attempt}: ok=${report.ok} violations=${report.violations.length} approvals=${report.approvals.length} warnings=${report.warnings.length} changes=${input.changes.length}`,
      );
    } catch (error) {
      // The backend moved on to another check, or was not ready yet: the
      // next status it sends is answered again.
      taken.delete(key);
      bridge.log("warn", `gate: run ${runId} check ${attempt} not answered: ${errorText(error)}`);
    }
  };

  const stopListening = bridge.listen<RunStatus>("run-status", (status) => void answer(status));
  // A check may already be waiting (the window opened after the engine was done).
  bridge.call<RunStatus>("run_status").then(
    (status) => void answer(status),
    (error: unknown) => bridge.log("warn", `run_status failed: ${errorText(error)}`),
  );

  return () => {
    stopped = true;
    stopListening();
  };
}
