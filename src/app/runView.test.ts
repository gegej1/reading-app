import { describe, expect, it } from "vitest";
import type { RunRecord, RunStatus, RunStep, StreamSummary } from "./runs";
import {
  activeRun,
  answerBlockedHint,
  editBlockedHint,
  finalFigures,
  finalNote,
  finalReportPath,
  formatBudget,
  formatClock,
  formatCost,
  formatDuration,
  formatRunTime,
  formatTokenCount,
  outcomeLabel,
  phaseLabel,
  queueText,
  resultKind,
  runDurationMs,
  stepTone,
  usageText,
} from "./runView";

function record(overrides: Partial<RunRecord> = {}): RunRecord {
  return {
    id: "2026-10-12T14-30-05-ingest",
    task: "ingest",
    engine: "claude",
    model: null,
    trigger: "ui",
    date: "2026-10-12",
    startedAt: new Date(2026, 9, 12, 14, 30, 5).getTime(),
    endedAt: null,
    phase: "running",
    outcome: null,
    message: null,
    queue: ["inbox/2026-10-12T09-00-00-mac.md"],
    base: "0".repeat(40),
    engineReport: null,
    gate: null,
    reason: null,
    decision: null,
    keptDespite: [],
    approved: [],
    commitMessage: null,
    commit: null,
    discardedTo: null,
    ...overrides,
  };
}

const status = (phase: RunStatus["phase"], run: RunRecord | null): RunStatus => ({ phase, run, gateAttempt: 0 });

describe("which run holds the knowledge base", () => {
  it("is the run from its start until a decision; a finished run holds nothing", () => {
    for (const phase of ["starting", "running", "checking", "committing", "needsDecision"] as const) {
      expect(activeRun(status(phase, record({ phase })))?.id, phase).toBe("2026-10-12T14-30-05-ingest");
    }
    expect(activeRun(status("idle", record({ phase: "finished", outcome: "committed" })))).toBeNull();
    expect(activeRun(null)).toBeNull();
  });

  it("blocks edits of tracked files with a hint that names the task", () => {
    expect(editBlockedHint(status("running", record()))).toBe("正在入库，结束后再改");
    expect(editBlockedHint(status("checking", record({ task: "landRulings", phase: "checking" })))).toBe(
      "正在落裁决，结束后再改",
    );
    expect(editBlockedHint(status("needsDecision", record({ phase: "needsDecision" })))).toBe(
      "入库的结果还在等你处理，处理完再改",
    );
    expect(editBlockedHint(status("needsDecision", record({ phase: "needsDecision", reason: "needsApproval" })))).toBe(
      "入库的结果还在等你确认，确认后再改",
    );
    expect(editBlockedHint(status("idle", null))).toBeNull();
  });

  it("blocks new answers only while a 落裁决 holds the knowledge base", () => {
    expect(answerBlockedHint(status("running", record()))).toBeNull();
    expect(answerBlockedHint(status("running", record({ task: "landRulings" })))).toBe("正在落裁决，结束后再回答");
    expect(answerBlockedHint(status("needsDecision", record({ task: "landRulings", phase: "needsDecision" })))).toBe(
      "落裁决的结果还在等你处理，处理完再回答",
    );
    expect(answerBlockedHint(status("idle", record({ task: "landRulings", phase: "finished" })))).toBeNull();
  });
});

describe("results", () => {
  it("sorts every phase and outcome into one kind of result screen", () => {
    expect(resultKind(record())).toBe("running");
    expect(resultKind(record({ phase: "needsDecision" }))).toBe("decision");
    expect(resultKind(record({ phase: "needsDecision", reason: "violations" }))).toBe("decision");
    expect(resultKind(record({ phase: "needsDecision", reason: "incomplete" }))).toBe("decision");
    expect(resultKind(record({ phase: "needsDecision", reason: "needsApproval" }))).toBe("approval");
    expect(resultKind(record({ phase: "needsDecision", reason: "commitFailed" }))).toBe("commitFailed");
    const done = (outcome: RunRecord["outcome"]) => resultKind(record({ phase: "finished", outcome }));
    expect(done("committed")).toBe("success");
    expect(done("kept")).toBe("success");
    expect(done("discarded")).toBe("discarded");
    expect(done("nothingToDo")).toBe("nothing");
    expect(done("refused")).toBe("refused");
    expect(done("failed")).toBe("failed");
    expect(done("cancelled")).toBe("cancelled");
    expect(done(null)).toBe("failed");
  });

  it("names outcomes for the history", () => {
    expect(outcomeLabel(record({ phase: "finished", outcome: "committed" }))).toEqual({ text: "已提交", tone: "ok" });
    expect(outcomeLabel(record({ phase: "finished", outcome: "kept" })).text).toBe("保留提交");
    expect(outcomeLabel(record({ phase: "needsDecision" }))).toEqual({ text: "等你处理", tone: "attention" });
    expect(outcomeLabel(record({ phase: "needsDecision", reason: "needsApproval" })).text).toBe("等你确认");
    expect(outcomeLabel(record({ phase: "needsDecision", reason: "commitFailed" })).text).toBe("没能提交");
    expect(outcomeLabel(record({ phase: "finished", outcome: "committed", decision: "approved" }))).toEqual({
      text: "已确认提交",
      tone: "ok",
    });
    expect(outcomeLabel(record({ phase: "finished", outcome: "failed" })).tone).toBe("failed");
  });

  it("reads the figures, the report and the note of the final JSON line", () => {
    const final = { 任务: "入库", 条数: 3, 自动: 2, 判例裁决: 1, 待裁决: 0, 报告: "2026-10-12-入库报告.md", 备注: " 本次没有拉取 Notion " };
    expect(finalFigures(final, "ingest")).toEqual([
      { label: "条随手记", value: 3 },
      { label: "自动整理", value: 2 },
      { label: "按判例裁决", value: 1 },
      { label: "新的待裁决", value: 0 },
    ]);
    expect(finalFigures({ 任务: "落裁决", 条数: 2 }, "landRulings")).toEqual([{ label: "份回答", value: 2 }]);
    expect(finalFigures(null, "ingest")).toEqual([]);
    expect(finalFigures({ 条数: "很多" }, "ingest")).toEqual([]);
    expect(finalReportPath(final)).toBe("reports/2026-10-12-入库报告.md");
    expect(finalReportPath({ 报告: "reports/a.md" })).toBe("reports/a.md");
    expect(finalReportPath({ 报告: "" })).toBeNull();
    expect(finalNote(final)).toBe("本次没有拉取 Notion");
    expect(finalNote({ 备注: "" })).toBeNull();
  });
});

describe("numbers", () => {
  it("formats durations, a stopwatch and times", () => {
    expect(formatDuration(42_400)).toBe("42 秒");
    expect(formatDuration(192_000)).toBe("3 分 12 秒");
    expect(formatDuration(180_000)).toBe("3 分");
    expect(formatDuration(3_900_000)).toBe("1 小时 5 分");
    expect(formatClock(192_999)).toBe("03:12");
    expect(formatClock(3_723_000)).toBe("1:02:03");
    const now = new Date(2026, 9, 12, 20, 0).getTime();
    expect(formatRunTime(new Date(2026, 9, 12, 14, 30).getTime(), now)).toBe("今天 14:30");
    expect(formatRunTime(new Date(2026, 9, 11, 9, 5).getTime(), now)).toBe("10 月 11 日 09:05");
  });

  it("formats cost, budgets and tokens", () => {
    expect(formatCost(0.4237)).toBe("$0.42");
    expect(formatCost(0.001)).toBe("不到 $0.01");
    expect(formatCost(0)).toBe("$0.00");
    expect(formatBudget(10)).toBe("$10");
    expect(formatBudget(1.5)).toBe("$1.50");
    expect(formatTokenCount(980)).toBe("980");
    expect(formatTokenCount(12_345)).toBe("12k");
    expect(formatTokenCount(1234)).toBe("1.2k");
    expect(formatTokenCount(2_500_000)).toBe("2.5M");
  });

  it("says what a run cost, or how many tokens it used", () => {
    const stream: StreamSummary = {
      model: null,
      keySource: null,
      finished: true,
      isError: false,
      subtype: null,
      finalMessage: null,
      error: null,
      costUsd: null,
      turns: null,
      durationMs: null,
      toolCalls: 0,
      denials: 0,
      inputTokens: null,
      outputTokens: null,
    };
    const report = (patch: Partial<StreamSummary>) => ({
      ok: true,
      failure: null,
      exit: { code: 0, cancelled: false, timedOut: false, spawnError: null, stderrTail: "" },
      stream: { ...stream, ...patch },
      durationMs: 1000,
    });
    expect(usageText(record())).toBeNull();
    expect(usageText(record({ engineReport: report({ costUsd: 0.42 }) }))).toBe("$0.42");
    expect(usageText(record({ engine: "codex", engineReport: report({ inputTokens: 12_345, outputTokens: 1234 }) }))).toBe(
      "输入 12k · 输出 1.2k tokens",
    );
    expect(usageText(record({ engineReport: report({}) }))).toBeNull();
    const started = record().startedAt;
    expect(runDurationMs(record({ endedAt: started + 65_000 }))).toBe(65_000);
    expect(runDurationMs(record(), started + 5000)).toBe(5000);
  });
});

describe("words", () => {
  it("names phases and queues", () => {
    expect(phaseLabel("running", "ingest")).toBe("AI 正在入库");
    expect(phaseLabel("running", "landRulings")).toBe("AI 正在落裁决");
    expect(phaseLabel("needsDecision", "ingest")).toBe("没通过检查，等你处理");
    expect(queueText("ingest", 3)).toBe("3 条随手记");
    expect(queueText("landRulings", 2)).toBe("2 份回答");
  });

  it("gives every step a tone", () => {
    const step = (patch: Partial<RunStep>): RunStep => ({
      runId: "r",
      seq: 1,
      elapsedMs: 0,
      kind: "tool",
      verb: null,
      target: null,
      text: null,
      ...patch,
    });
    expect(stepTone(step({ verb: "读" }))).toBe("read");
    expect(stepTone(step({ verb: "写" }))).toBe("write");
    expect(stepTone(step({ verb: "改" }))).toBe("edit");
    expect(stepTone(step({ verb: "查找" }))).toBe("find");
    expect(stepTone(step({ verb: "运行" }))).toBe("run");
    expect(stepTone(step({ verb: "Bash" }))).toBe("tool");
    expect(stepTone(step({ kind: "denied", verb: "写" }))).toBe("denied");
    expect(stepTone(step({ kind: "note" }))).toBe("note");
    expect(stepTone(step({ kind: "error" }))).toBe("error");
  });
});
