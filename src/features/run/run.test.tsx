// @vitest-environment jsdom
// The run panel in a DOM, against a fake backend: the start sheet (nothing
// starts without confirming), the live steps, every result state, the
// actions on a run that failed the check, the history and the notification.

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { AppRoutes } from "../../App";
import { createDesktop } from "../../app/desktop";
import { startGateHost, type GateRequest } from "../../app/gateHost";
import { KbProvider } from "../../app/KbProvider";
import { RunProvider } from "../../app/RunProvider";
import type { RunRecord, RunSettingsView, RunStatus, RunStep, StreamSummary } from "../../app/runs";
import { runGate, type GateReport } from "../../domain";
import { fixtureKb } from "../../domain/__fixtures__";
import { createMemoryStore } from "../../store/memoryStore";
import type { KbStore } from "../../store/types";

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  Element.prototype.scrollTo ??= () => undefined;
  Element.prototype.scrollIntoView ??= () => undefined;
});

let root: Root | null = null;
let container: HTMLElement | null = null;
let stopGate: (() => void) | null = null;

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  stopGate?.();
  root = null;
  container = null;
  stopGate = null;
});

async function settle(ms = 0) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

async function waitFor(check: () => boolean, what: string) {
  for (let i = 0; i < 250; i += 1) {
    if (check()) return;
    await settle(10);
  }
  throw new Error(`timed out waiting for ${what}: ${container?.textContent?.slice(0, 400)}`);
}

const SETTINGS: RunSettingsView = {
  engine: "claude",
  model: null,
  budgetUsd: 10,
  runTimeoutMinutes: 30,
  claudePath: null,
  codexPath: null,
  claudeProgram: "/opt/homebrew/bin/claude",
  codexProgram: null,
  problems: [],
};

const STREAM: StreamSummary = {
  model: "claude-sonnet-x",
  keySource: "none",
  finished: true,
  isError: false,
  subtype: "success",
  finalMessage: null,
  error: null,
  costUsd: 0.42,
  turns: 12,
  durationMs: 80_000,
  toolCalls: 20,
  denials: 0,
  inputTokens: null,
  outputTokens: null,
};

const RUN_ID = "2026-10-12T14-30-05-ingest";
const STARTED = new Date(2026, 9, 12, 14, 30, 5).getTime();

function record(patch: Partial<RunRecord> = {}): RunRecord {
  return {
    id: RUN_ID,
    task: "ingest",
    engine: "claude",
    model: null,
    trigger: "ui",
    date: "2026-10-12",
    startedAt: STARTED,
    endedAt: null,
    phase: "running",
    outcome: null,
    message: null,
    queue: ["inbox/2026-09-10-想法.md", "inbox/2026-09-11-摘抄.md"],
    base: "0".repeat(40),
    engineReport: null,
    gate: null,
    keptDespite: [],
    commit: null,
    discardedTo: null,
    ...patch,
  };
}

const engineReport = (patch: Partial<StreamSummary> = {}, failure: string | null = null, timedOut = false) => ({
  ok: failure === null,
  failure,
  exit: { code: failure ? null : 0, cancelled: false, timedOut, spawnError: null, stderrTail: "" },
  stream: { ...STREAM, ...patch },
  durationMs: 80_000,
});

const status = (phase: RunStatus["phase"], run: RunRecord | null, gateAttempt = 0): RunStatus => ({ phase, run, gateAttempt });

/** A fake backend: answers commands from `handlers`, records every call, and sends events. */
function fakeBackend(handlers: Record<string, (args: Record<string, unknown> | undefined) => unknown>) {
  const listeners = new Map<string, Set<(payload: unknown) => void>>();
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const bridge = createDesktop({
    active: true,
    invoke: async (command, args) => {
      if (command === "plugin:log|log") return undefined;
      calls.push({ command, args });
      const handler = handlers[command];
      if (!handler) throw new Error(`unexpected command ${command}`);
      return handler(args);
    },
    listen: async (event, handler) => {
      const set = listeners.get(event) ?? new Set();
      set.add(handler);
      listeners.set(event, set);
      return () => set.delete(handler);
    },
  });
  const emit = (event: string, payload: unknown) =>
    act(() => {
      for (const handler of listeners.get(event) ?? []) handler(payload);
    });
  const called = (command: string) => calls.filter((call) => call.command === command);
  return { bridge, calls, emit, called };
}

type Backend = ReturnType<typeof fakeBackend>;

function mount(backend: Backend, route = "/") {
  // The desktop app's store, as far as the screens can tell.
  const store: KbStore = { ...createMemoryStore(fixtureKb), kind: "tauri" };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  const tree: ReactNode = (
    <KbProvider store={store}>
      <MemoryRouter initialEntries={[route]}>
        <RunProvider bridge={backend.bridge}>
          <AppRoutes />
        </RunProvider>
      </MemoryRouter>
    </KbProvider>
  );
  act(() => root?.render(tree));
  return container;
}

const buttonNamed = (label: string | RegExp) =>
  [...(container?.querySelectorAll("button, a") ?? [])].find((element) =>
    typeof label === "string" ? element.textContent?.trim() === label : label.test(element.textContent ?? ""),
  ) as HTMLButtonElement | HTMLAnchorElement | undefined;

const click = (element: Element | undefined | null) => act(() => (element as HTMLElement | null)?.click());

function standardBackend(extra: Record<string, (args: Record<string, unknown> | undefined) => unknown> = {}) {
  return fakeBackend({
    run_status: () => status("idle", null),
    run_steps: () => [],
    run_settings: () => SETTINGS,
    run_start: () => status("starting", record({ phase: "starting" })),
    run_cancel: () => status("running", record()),
    run_history: () => [],
    ...extra,
  });
}

async function openTodayAndStart(backend: Backend) {
  mount(backend);
  await waitFor(() => buttonNamed(/^入库 ▶/) !== undefined, "the 入库 button");
  click(buttonNamed(/^入库 ▶/));
  await waitFor(() => container?.querySelector('[role="dialog"][aria-label="开始入库"]') != null, "the start sheet");
  await waitFor(() => buttonNamed("开始入库")?.hasAttribute("disabled") === false, "the settings");
}

describe("the start sheet", () => {
  it("shows what will run and what it may cost, and nothing starts without confirming", async () => {
    const backend = standardBackend();
    await openTodayAndStart(backend);
    const sheet = container?.querySelector('[aria-label="开始入库"]');
    const text = sheet?.textContent ?? "";
    expect(text).toContain("inbox/ 里有 2 条随手记");
    expect(text).toContain("Claude Code");
    expect(text).toContain("默认模型 · 跟随你自己的 Claude 设置");
    expect(text).toContain("$10");
    expect(text).toContain("按用量计费，这次最多花 $10。");
    expect(text).toContain("超过 30 分钟就停下");
    expect(backend.called("run_start")).toEqual([]);

    // 先不了: the sheet goes, nothing ran.
    click(buttonNamed("先不了"));
    expect(container?.querySelector('[aria-label="开始入库"]')).toBeNull();
    expect(backend.called("run_start")).toEqual([]);

    // Esc closes it too.
    click(buttonNamed(/^入库 ▶/));
    await waitFor(() => container?.querySelector('[aria-label="开始入库"]') != null, "the sheet again");
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(container?.querySelector('[aria-label="开始入库"]')).toBeNull();
    expect(backend.called("run_start")).toEqual([]);
  });

  it("starts only on 开始入库, then shows the run", async () => {
    const backend = standardBackend();
    await openTodayAndStart(backend);
    click(buttonNamed("开始入库"));
    await waitFor(() => backend.called("run_start").length === 1, "run_start");
    expect(backend.called("run_start")[0]?.args).toEqual({ task: "ingest" });
    await waitFor(() => container?.querySelector(".run-live") != null, "the live view");
    expect(container?.querySelector('[aria-label="开始入库"]')).toBeNull();
  });

  it("says when the engine cannot be found and names a chosen model", async () => {
    const backend = standardBackend({ run_settings: () => ({ ...SETTINGS, model: "sonnet", claudeProgram: null }) });
    await openTodayAndStart(backend);
    const text = container?.querySelector('[aria-label="开始入库"]')?.textContent ?? "";
    expect(text).toContain("sonnet");
    expect(text).toContain("没在这台 Mac 上找到 claude 程序");
  });
});

describe("the live view", () => {
  it("streams the steps as readable lines, refusals highlighted, and can be cancelled", async () => {
    const backend = standardBackend();
    await openTodayAndStart(backend);
    click(buttonNamed("开始入库"));
    await waitFor(() => container?.querySelector(".run-live") != null, "the live view");
    backend.emit("run-status", status("running", record()));
    const step = (seq: number, patch: Partial<RunStep>): RunStep => ({
      runId: RUN_ID,
      seq,
      elapsedMs: seq * 1000,
      kind: "tool",
      verb: null,
      target: null,
      text: null,
      ...patch,
    });
    for (const one of [
      step(1, { kind: "note", text: "先同步一次，把别处记下的内容拿到本机" }),
      step(2, { kind: "init", text: "Claude Code 2.1 · 模型 claude-sonnet-x" }),
      step(3, { verb: "读", target: "AGENTS.md" }),
      step(4, { kind: "text", text: "先读一下规则。" }),
      step(5, { verb: "查找", target: "self/判例/*.md" }),
      step(6, { verb: "写", target: "raw/inbox/2026-10-12-收件箱.md" }),
      step(7, { verb: "改", target: "index.md" }),
      step(8, { kind: "denied", verb: "写", target: "/etc/hosts", text: "不在知识库里" }),
    ]) {
      backend.emit("run-step", one);
    }
    const lines = [...(container?.querySelectorAll(".step") ?? [])];
    expect(lines).toHaveLength(8);
    const read = container?.querySelector(".step--read");
    expect(read?.querySelector(".step__chop")?.textContent).toBe("读");
    expect(read?.querySelector(".step__target")?.textContent).toBe("AGENTS.md");
    expect(read?.querySelector(".step__time")?.textContent).toBe("00:03");
    expect(container?.querySelector(".step--write .step__target")?.textContent).toBe("raw/inbox/2026-10-12-收件箱.md");
    expect(container?.querySelector(".step--edit .step__chop")?.textContent).toBe("改");
    expect(container?.querySelector(".step--find .step__chop")?.textContent).toBe("查找");
    expect(container?.querySelector(".step--text")?.textContent).toContain("先读一下规则。");
    const denied = container?.querySelector(".step--denied");
    expect(denied?.textContent).toContain("被拒绝");
    expect(denied?.textContent).toContain("/etc/hosts");
    expect(container?.querySelector(".run-live__phase")?.textContent).toBe("AI 正在入库");
    expect(container?.querySelector(".run-live__clock")?.textContent).toMatch(/^\d\d:\d\d$/);

    click(buttonNamed("取消"));
    await waitFor(() => backend.called("run_cancel").length === 1, "run_cancel");
  });
});

function reportFixture(patch: Partial<GateReport> = {}): GateReport {
  return {
    ok: true,
    violations: [],
    warnings: [],
    commitPaths: ["raw/inbox/2026-10-12-收件箱.md", "reports/2026-09-08-入库报告-2.md"],
    deleteQueuePaths: ["inbox/2026-09-10-想法.md"],
    restoreQueuePaths: [],
    summary: "通过检查：2 条随手记都逐字进了原料层，提交 2 个文件",
    final: { 任务: "入库", 条数: 2, 自动: 1, 判例裁决: 1, 待裁决: 0, 报告: "reports/2026-09-08-入库报告-2.md", 备注: "本次没有拉取 Notion" },
    ...patch,
  };
}

async function showResult(backend: Backend, result: RunStatus) {
  mount(backend);
  await waitFor(() => container?.querySelector(".today") != null, "今日");
  backend.emit("run-show", null);
  backend.emit("run-status", result);
  await waitFor(() => container?.querySelector(".run-result") != null, "the result");
}

describe("results", () => {
  it("success: the final line's figures, cost and duration, the report and what to audit", async () => {
    const backend = standardBackend();
    const done = record({
      phase: "finished",
      outcome: "committed",
      endedAt: STARTED + 192_000,
      gate: reportFixture(),
      engineReport: engineReport(),
      commit: "abcdef1234567890",
      message: "通过检查：2 条随手记都逐字进了原料层，提交 2 个文件，已提交 abcdef1234",
    });
    await showResult(backend, status("idle", done));
    const text = container?.querySelector(".run-result")?.textContent ?? "";
    expect(text).toContain("入库完成了");
    expect(text).toContain("条随手记");
    expect(text).toContain("按判例裁决");
    expect(text).toContain("花费 $0.42");
    expect(text).toContain("用时 3 分 12 秒");
    expect(text).toContain("abcdef1234");
    expect(text).toContain("AI 的备注：本次没有拉取 Notion");
    const report = buttonNamed("看入库报告") as HTMLAnchorElement | undefined;
    expect(report?.getAttribute("href")).toBe("/page/reports/2026-09-08-%E5%85%A5%E5%BA%93%E6%8A%A5%E5%91%8A-2.md");
    // The report on disk has one unchecked numbered item… and one without a number.
    const audit = [...(container?.querySelectorAll(".run-result a") ?? [])].find((link) => link.textContent?.startsWith("去抽查"));
    expect(audit?.textContent).toMatch(/去抽查 2 条/);
    expect(audit?.getAttribute("href")).toBe("/rulings?tab=audit");
  });

  it("an engine error: plain words and another engine to try", async () => {
    const backend = standardBackend();
    const failed = record({
      phase: "finished",
      outcome: "failed",
      endedAt: STARTED + 1_800_000,
      engineReport: engineReport({ finished: false }, "超过 30 分钟没有结束，已停止", true),
      message: "超过 30 分钟没有结束，已停止。AI 没有留下任何改动，队列原样留着",
    });
    await showResult(backend, status("idle", failed));
    const text = container?.querySelector(".run-result")?.textContent ?? "";
    expect(text).toContain("入库超时了，已停下");
    expect(text).toContain("队列原样留着");
    expect(text).toContain("什么都没有提交");
    click(buttonNamed("换用 Codex 再试"));
    await waitFor(() => container?.querySelector('[aria-label="开始入库"]') != null, "the start sheet");
    await waitFor(() => (container?.querySelector('[aria-label="开始入库"]')?.textContent ?? "").includes("Codex"), "Codex");
    const sheet = container?.querySelector('[aria-label="开始入库"]')?.textContent ?? "";
    expect(sheet).toContain("Codex 不支持花费上限");
    expect(sheet).toContain("没在这台 Mac 上找到 codex 程序");
    click(buttonNamed("开始入库"));
    await waitFor(() => backend.called("run_start").length === 1, "run_start");
    expect(backend.called("run_start")[0]?.args).toEqual({ task: "ingest", engine: "codex" });
  });

  it("cancelled, empty queue, refused and discarded runs each say what happened", async () => {
    const cases: [Partial<RunRecord>, string][] = [
      [{ outcome: "cancelled", message: "已停止。AI 没有留下任何改动，队列原样留着" }, "入库已停止"],
      [{ outcome: "nothingToDo", message: "收件箱是空的，没有要入库的随手记" }, "队列是空的"],
      [{ outcome: "refused", message: "知识库里有还没提交的改动（a.md），先提交或撤销它们" }, "入库没有开始"],
      [{ outcome: "discarded", message: "已丢弃这次入库的改动：还原 1 个文件，删除 2 个新文件" }, "这次的改动已经丢弃"],
    ];
    for (const [patch, title] of cases) {
      const backend = standardBackend();
      await showResult(backend, status("idle", record({ phase: "finished", endedAt: STARTED + 5000, ...patch })));
      const text = container?.querySelector(".run-result")?.textContent ?? "";
      expect(text, title).toContain(title);
      expect(text, title).toContain(patch.message ?? "");
      act(() => root?.unmount());
      container?.remove();
    }
  });
});

describe("a run that failed the check", () => {
  const PRECEDENT = "self/判例/P-001-速度与退路.md";
  const before = fixtureKb[PRECEDENT] ?? "";
  const after = before.replace("先确认退路", "先确认后路");

  /** What the backend hands the gate: the engine changed a word in a precedent's 裁决. */
  const gateInput: GateRequest = {
    runId: RUN_ID,
    attempt: 1,
    task: "ingest",
    date: "2026-10-12",
    changes: [
      { path: PRECEDENT, status: "modified", before, after, opaque: null },
      {
        path: "raw/inbox/2026-10-12-收件箱.md",
        status: "added",
        before: null,
        after: "# 2026-10-12 收件箱\n\n## I-01\n周五下午做一次周回顾，只看三件事。\n",
        opaque: null,
      },
      { path: "reports/2026-10-12-入库报告.md", status: "added", before: null, after: "# 入库报告\n", opaque: null },
    ],
    unchanged: [],
    queue: [{ path: "inbox/a.md", content: "---\n来源: mac\n---\n\n周五下午做一次周回顾，只看三件事。\n" }],
    finalMessage: null,
    engineFailure: null,
    foreignCommits: [],
    untrackedChanged: [],
  };

  async function failedRun() {
    let reported: GateReport | null = null;
    const backend = standardBackend({
      run_gate_input: () => gateInput,
      run_gate_report: (args) => {
        reported = (args?.report as GateReport) ?? null;
        return null;
      },
      run_discard: () => status("idle", record({ phase: "finished", outcome: "discarded", message: "已丢弃" })),
      run_keep: () => status("idle", record({ phase: "finished", outcome: "kept", message: "保留了" })),
      run_recheck: () => status("checking", record({ phase: "checking" }), 2),
    });
    stopGate = startGateHost(backend.bridge, runGate);
    mount(backend);
    await waitFor(() => container?.querySelector(".today") != null, "今日");
    // The engine is done: the gate host checks it in this window.
    backend.emit("run-status", status("checking", record({ phase: "checking" }), 1));
    await waitFor(() => reported !== null, "the gate's report");
    const report = reported as unknown as GateReport;
    expect(report.ok).toBe(false);
    backend.emit("run-show", null);
    backend.emit(
      "run-status",
      status("needsDecision", record({ phase: "needsDecision", gate: report, message: report.summary, engineReport: engineReport() })),
    );
    await waitFor(() => container?.querySelector(".run-decision") != null, "the decision view");
    return { backend, report };
  }

  it("lists every violation in plain Chinese and shows the changed files with a before/after view", async () => {
    const { report } = await failedRun();
    expect(report.violations.map((issue) => issue.rule)).toContain("precedentRuling");
    const text = container?.querySelector(".run-decision")?.textContent ?? "";
    for (const issue of report.violations) expect(text).toContain(issue.message);
    expect(text).toContain(PRECEDENT);
    expect(text).toContain("改了的文件 3");
    // The file with the problem is open: one character changed, marked on both sides.
    const open = container?.querySelector(".changed.is-open");
    expect(open?.textContent).toContain(PRECEDENT);
    const marks = [...(open?.querySelectorAll(".diff__mark") ?? [])].map((mark) => mark.textContent);
    expect(marks).toEqual(["退", "后"]);
    expect(open?.querySelector(".diff__row.is-del")?.textContent).toContain("−");
    expect(open?.querySelector(".diff__row.is-add")?.textContent).toContain("+");
    // A new file opens on click: all added.
    click([...(container?.querySelectorAll(".changed__head") ?? [])].find((head) => head.textContent?.includes("收件箱.md")));
    expect(container?.querySelectorAll(".changed.is-open")).toHaveLength(2);
  });

  it("丢弃 discards, 重新检查 checks again, 仍然提交 asks first and then keeps", async () => {
    const { backend } = await failedRun();
    click(buttonNamed("重新检查"));
    await waitFor(() => backend.called("run_recheck").length === 1, "run_recheck");
    expect(backend.called("run_recheck")[0]?.args).toEqual({ runId: RUN_ID });
    // The backend checks again: the panel follows.
    await waitFor(() => container?.querySelector(".run-live__phase")?.textContent === "正在检查改动", "the check");

    backend.emit(
      "run-status",
      status("needsDecision", record({ phase: "needsDecision", gate: reportFixture({ ok: false, violations: [{ rule: "paths", path: "README.md", message: "改动了不允许改的文件：README.md" }] }), message: "没通过检查：1 个问题" })),
    );
    await waitFor(() => container?.querySelector(".run-decision") != null, "the decision view again");
    click(buttonNamed("仍然提交"));
    const confirm = container?.querySelector('[role="alertdialog"]');
    expect(confirm?.textContent).toContain("跳过检查");
    expect(backend.called("run_keep")).toEqual([]);
    click(buttonNamed("再想想"));
    expect(container?.querySelector('[role="alertdialog"]')).toBeNull();
    click(buttonNamed("仍然提交"));
    click(buttonNamed("确定提交"));
    await waitFor(() => backend.called("run_keep").length === 1, "run_keep");
    expect(backend.called("run_keep")[0]?.args).toEqual({ runId: RUN_ID });
    await waitFor(() => (container?.textContent ?? "").includes("按你的要求提交了"), "the kept result");

    backend.emit("run-status", status("needsDecision", record({ phase: "needsDecision", gate: reportFixture({ ok: false, violations: [{ rule: "paths", path: null, message: "x" }] }) })));
    click(buttonNamed("丢弃（可找回）"));
    await waitFor(() => backend.called("run_discard").length === 1, "run_discard");
    expect(backend.called("run_discard")[0]?.args).toEqual({ runId: RUN_ID });
  });

  it("without a report from the gate, keeping waits for a new check", async () => {
    const backend = standardBackend();
    await showResult(
      backend,
      status("needsDecision", record({ phase: "needsDecision", message: "没能完成检查：检查程序没有回应。可以重新检查，或者丢弃这次的改动" })),
    );
    expect(buttonNamed("仍然提交")?.hasAttribute("disabled")).toBe(true);
    expect(container?.textContent).toContain("检查程序没有回应");
  });
});

describe("the history", () => {
  it("lists the kept runs and opens one with its steps and check", async () => {
    const committed = record({
      phase: "finished",
      outcome: "committed",
      endedAt: STARTED + 192_000,
      engineReport: engineReport(),
      gate: reportFixture(),
      message: "通过检查",
    });
    const codex = record({
      id: "2026-10-11T09-00-00-land-rulings",
      task: "landRulings",
      engine: "codex",
      model: "gpt-5",
      startedAt: STARTED - 86_400_000,
      phase: "finished",
      outcome: "kept",
      endedAt: STARTED - 86_400_000 + 65_000,
      engineReport: engineReport({ costUsd: null, inputTokens: 12_345, outputTokens: 1234 }),
      gate: reportFixture({ ok: false, violations: [{ rule: "brokenLinks", path: "wiki/a.md", message: "wiki/a.md 里有新的断链 [[x]]" }] }),
      keptDespite: ["wiki/a.md 里有新的断链 [[x]]"],
    });
    const backend = standardBackend({
      run_history: () => [committed, codex],
      run_record: (args) =>
        args?.runId === codex.id
          ? {
              record: codex,
              steps: [{ runId: codex.id, seq: 1, elapsedMs: 0, kind: "tool", verb: "改", target: "self/待裁决/Q-004.md", text: null }],
            }
          : { record: committed, steps: [] },
    });
    mount(backend);
    await waitFor(() => container?.querySelector(".today") != null, "今日");
    backend.emit("run-show", null);
    await waitFor(() => buttonNamed("运行记录") !== undefined, "the panel");
    click(buttonNamed("运行记录"));
    await waitFor(() => container?.querySelectorAll(".history__row").length === 2, "the rows");
    const rows = [...(container?.querySelectorAll(".history__row") ?? [])].map((row) => row.textContent ?? "");
    expect(rows[0]).toContain("入库");
    expect(rows[0]).toContain("Claude");
    expect(rows[0]).toContain("默认");
    expect(rows[0]).toContain("已提交");
    expect(rows[0]).toContain("$0.42");
    expect(rows[0]).toContain("3 分 12 秒");
    expect(rows[1]).toContain("落裁决");
    expect(rows[1]).toContain("gpt-5");
    expect(rows[1]).toContain("保留提交");
    expect(rows[1]).toContain("输入 12k · 输出 1.2k tokens");

    click(container?.querySelectorAll(".history__row")[1]);
    await waitFor(() => container?.querySelector(".history-detail__head") != null, "the detail");
    const detail = container?.querySelector(".history-detail")?.textContent ?? "";
    expect(detail).toContain("检查结果");
    expect(detail).toContain("wiki/a.md 里有新的断链 [[x]]");
    expect(detail).toContain("保留时没通过的检查");
    expect(detail).toContain("self/待裁决/Q-004.md");
    expect(backend.called("run_record")[0]?.args).toEqual({ runId: codex.id });
    click(buttonNamed("← 运行记录"));
    expect(container?.querySelectorAll(".history__row")).toHaveLength(2);
  });
});

describe("while a run holds the knowledge base", () => {
  it("今日 offers the run instead of a new start, and the panel closes without stopping it", async () => {
    const backend = standardBackend({ run_status: () => status("running", record()) });
    mount(backend);
    await waitFor(() => buttonNamed("正在入库…") !== undefined, "the live button");
    click(buttonNamed("正在入库…"));
    await waitFor(() => container?.querySelector(".run-live") != null, "the live view");
    click(buttonNamed("收起"));
    expect(container?.querySelector(".run-panel")).toBeNull();
    expect(backend.called("run_cancel")).toEqual([]);
  });
});
