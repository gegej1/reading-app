import { describe, expect, it } from "vitest";
import type { GateInput, GateReport } from "../domain";
import { createDesktop } from "./desktop";
import { crashedReport, startGateHost, type GateRequest } from "./gateHost";
import type { RunRecord, RunStatus } from "./runs";

const QUEUED = "inbox/2026-10-12T09-00-00-mac.md";

function request(runId: string, attempt: number): GateRequest {
  return {
    runId,
    attempt,
    task: "ingest",
    date: "2026-10-12",
    changes: [
      {
        path: "raw/inbox/2026-10-12-收件箱.md",
        status: "added",
        before: null,
        after: "# 收件箱\n\n周五下午做一次周回顾。\n",
        opaque: null,
      },
      { path: "reports/2026-10-12-入库报告.md", status: "added", before: null, after: "# 入库报告\n", opaque: null },
    ],
    unchanged: [{ path: QUEUED, content: "---\n来源: mac\n---\n\n周五下午做一次周回顾。\n" }],
    queue: [{ path: QUEUED, content: "---\n来源: mac\n---\n\n周五下午做一次周回顾。\n" }],
    finalMessage: '{"任务": "入库", "条数": 1}',
    engineFailure: null,
    foreignCommits: [],
    untrackedChanged: [],
  };
}

function status(phase: RunStatus["phase"], runId = "run-1", gateAttempt = 1): RunStatus {
  return { phase, run: { id: runId } as RunRecord, gateAttempt };
}

interface Backend {
  /** The status `run_status` answers with. */
  current: RunStatus;
  /** Commands that fail, with their message. */
  failing: Record<string, string>;
}

/** A desktop bridge whose backend is this test. */
function fakeBridge(backend: Backend) {
  const calls: [string, Record<string, unknown> | undefined][] = [];
  const handlers: Record<string, ((payload: unknown) => void)[]> = {};
  const bridge = createDesktop({
    active: true,
    invoke: async (command, args) => {
      if (command === "plugin:log|log") return null;
      calls.push([command, args]);
      if (backend.failing[command]) throw new Error(backend.failing[command]);
      if (command === "run_status") return backend.current;
      if (command === "run_gate_input") {
        const { runId, attempt } = args as { runId: string; attempt: number };
        return request(runId, attempt);
      }
      return null;
    },
    listen: async (event, handler) => {
      (handlers[event] ??= []).push(handler);
      return () => {
        handlers[event] = (handlers[event] ?? []).filter((h) => h !== handler);
      };
    },
  });
  const emit = (event: string, payload: unknown) => {
    for (const handler of handlers[event] ?? []) handler(payload);
  };
  const reports = () =>
    calls
      .filter(([command]) => command === "run_gate_report")
      .map(([, args]) => args as { runId: string; attempt: number; report: GateReport });
  return { bridge, calls, emit, reports };
}

const settle = async () => {
  for (let i = 0; i < 10; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
};

describe("startGateHost", () => {
  it("answers a check the backend announces with the gate's report", async () => {
    const { bridge, calls, emit, reports } = fakeBridge({ current: status("idle"), failing: {} });
    const stop = startGateHost(bridge);
    await settle();
    expect(reports()).toEqual([]);

    emit("run-status", status("running"));
    emit("run-status", status("checking", "run-1", 1));
    await settle();

    expect(calls.map(([command]) => command)).toEqual(["run_status", "run_gate_input", "run_gate_report"]);
    expect(calls[1]?.[1]).toEqual({ runId: "run-1", attempt: 1 });
    const [sent] = reports();
    expect(sent?.runId).toBe("run-1");
    expect(sent?.attempt).toBe(1);
    // The real gate ran on what the backend sent.
    expect(sent?.report.ok).toBe(true);
    expect(sent?.report.deleteQueuePaths).toEqual([QUEUED]);
    expect(sent?.report.commitPaths).toEqual(["raw/inbox/2026-10-12-收件箱.md", "reports/2026-10-12-入库报告.md"]);
    stop();
  });

  it("answers a check that was already waiting when the window opened", async () => {
    const { bridge, reports } = fakeBridge({ current: status("checking", "run-7", 2), failing: {} });
    startGateHost(bridge);
    await settle();
    expect(reports().map((sent) => [sent.runId, sent.attempt])).toEqual([["run-7", 2]]);
  });

  it("answers each check once, and a later check of the same run again", async () => {
    const { bridge, emit, reports } = fakeBridge({ current: status("idle"), failing: {} });
    startGateHost(bridge);
    emit("run-status", status("checking", "run-1", 1));
    emit("run-status", status("checking", "run-1", 1));
    await settle();
    emit("run-status", status("checking", "run-1", 1));
    emit("run-status", status("needsDecision", "run-1", 1));
    emit("run-status", status("checking", "run-1", 2));
    await settle();
    expect(reports().map((sent) => sent.attempt)).toEqual([1, 2]);
  });

  it("reports a gate that crashes instead of leaving the backend waiting", async () => {
    const { bridge, emit, reports } = fakeBridge({ current: status("idle"), failing: {} });
    startGateHost(bridge, () => {
      throw new Error("意外的输入");
    });
    emit("run-status", status("checking"));
    await settle();
    const [sent] = reports();
    expect(sent?.report.ok).toBe(false);
    expect(sent?.report.violations.map((violation) => violation.rule)).toEqual(["gate"]);
    expect(sent?.report.violations[0]?.message).toContain("意外的输入");
    expect(sent?.report.deleteQueuePaths).toEqual([]);
  });

  it("tries again on the next status when the input could not be fetched", async () => {
    const backend: Backend = { current: status("idle"), failing: { run_gate_input: "这次检查已经结束，或者不是当前的运行" } };
    const { bridge, emit, reports } = fakeBridge(backend);
    startGateHost(bridge);
    emit("run-status", status("checking"));
    await settle();
    expect(reports()).toEqual([]);

    backend.failing = {};
    emit("run-status", status("checking"));
    await settle();
    expect(reports()).toHaveLength(1);
  });

  it("stops answering once it is stopped, and never starts on the web", async () => {
    const { bridge, emit, reports } = fakeBridge({ current: status("idle"), failing: {} });
    const stop = startGateHost(bridge);
    await settle();
    stop();
    emit("run-status", status("checking"));
    await settle();
    expect(reports()).toEqual([]);

    const web = createDesktop({
      active: false,
      invoke: async () => {
        throw new Error("the web build must not call the backend");
      },
    });
    expect(() => startGateHost(web)()).not.toThrow();
  });
});

describe("crashedReport", () => {
  it("approves nothing and puts touched queue files back", () => {
    const input: GateInput = {
      ...request("run-1", 1),
      changes: [
        { path: "wiki/概念/甲.md", status: "added", before: null, after: "# 甲\n", opaque: null },
        { path: QUEUED, status: "deleted", before: "x", after: null, opaque: null },
      ],
    };
    const report = crashedReport(input, new Error("坏了"));
    expect(report.ok).toBe(false);
    expect(report.commitPaths).toEqual(["wiki/概念/甲.md"]);
    expect(report.deleteQueuePaths).toEqual([]);
    expect(report.restoreQueuePaths).toEqual([QUEUED]);
    expect(report.violations[0]?.message).toContain("坏了");
  });
});
