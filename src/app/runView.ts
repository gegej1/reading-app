// Plain words and numbers for AI runs: what a phase or an outcome is called,
// how long a run took, what it cost, and how a step reads. Pure functions,
// shared by the run panel, the history and the sidebar indicator.

import type { RunTask } from "../domain";
import type { EngineKind, RunPhase, RunRecord, RunStatus, RunStep } from "./runs";

/** A kept run with its steps (what `run_record` returns). */
export interface RunDetail {
  record: RunRecord;
  steps: RunStep[];
}

export const TASK_LABEL: Record<RunTask, string> = { ingest: "入库", landRulings: "落裁决" };
export const ENGINE_LABEL: Record<EngineKind, string> = { claude: "Claude", codex: "Codex" };

/** What the queue of a task is called: `3 条随手记`, `2 份回答`. */
export function queueText(task: RunTask, count: number): string {
  return task === "ingest" ? `${count} 条随手记` : `${count} 份回答`;
}

export function otherEngine(engine: EngineKind): EngineKind {
  return engine === "claude" ? "codex" : "claude";
}

/** A run holds the knowledge base from its start until a person decided about its result. */
export function holdsKb(phase: RunPhase): boolean {
  return phase !== "idle" && phase !== "finished";
}

/** The run that holds the knowledge base right now, if any. */
export function activeRun(status: RunStatus | null): RunRecord | null {
  if (!status || !holdsKb(status.phase) || !status.run) return null;
  return status.run;
}

/** Why tracked files (底色 ✅, audit checks) cannot be edited now, or null. */
export function editBlockedHint(status: RunStatus | null): string | null {
  const run = activeRun(status);
  if (!run) return null;
  const label = TASK_LABEL[run.task];
  if (status?.phase !== "needsDecision") return `正在${label}，结束后再改`;
  return run.reason === "needsApproval" ? `${label}的结果还在等你确认，确认后再改` : `${label}的结果还在等你处理，处理完再改`;
}

/**
 * Why a new answer cannot go to inbox/裁决/ now, or null. Only a 落裁决 is
 * in the way: it works on that folder. During an ingest answers still work.
 */
export function answerBlockedHint(status: RunStatus | null): string | null {
  const run = activeRun(status);
  if (!run || run.task !== "landRulings") return null;
  return status?.phase === "needsDecision" ? "落裁决的结果还在等你处理，处理完再回答" : "正在落裁决，结束后再回答";
}

/** What a waiting run waits for, in a few words. */
export function waitingLabel(record: RunRecord): string {
  switch (record.reason) {
    case "needsApproval":
      return "有改动需要你确认";
    case "commitFailed":
      return "检查通过了，但没能提交";
    default:
      return "没通过检查，等你处理";
  }
}

/** Where a run stands, in a few words. */
export function phaseLabel(phase: RunPhase, task: RunTask): string {
  const label = TASK_LABEL[task];
  switch (phase) {
    case "idle":
      return "没有在运行";
    case "starting":
      return "准备中：先同步一次";
    case "running":
      return `AI 正在${label}`;
    case "checking":
      return "正在检查改动";
    case "committing":
      return "正在提交";
    case "needsDecision":
      return "没通过检查，等你处理";
    case "finished":
      return "已经结束";
  }
}

/** Which kind of result screen a run gets. */
export type ResultKind =
  | "running"
  | "decision"
  | "approval"
  | "commitFailed"
  | "success"
  | "discarded"
  | "nothing"
  | "refused"
  | "failed"
  | "cancelled";

export function resultKind(record: RunRecord): ResultKind {
  if (record.phase === "needsDecision") {
    if (record.reason === "needsApproval") return "approval";
    if (record.reason === "commitFailed") return "commitFailed";
    return "decision";
  }
  if (record.phase !== "finished") return "running";
  switch (record.outcome) {
    case "committed":
    case "kept":
      return "success";
    case "discarded":
      return "discarded";
    case "nothingToDo":
      return "nothing";
    case "refused":
      return "refused";
    case "cancelled":
      return "cancelled";
    case "failed":
    case null:
      return "failed";
  }
}

export type OutcomeTone = "ok" | "attention" | "quiet" | "failed" | "running";

/** The outcome as the history shows it. */
export function outcomeLabel(record: RunRecord): { text: string; tone: OutcomeTone } {
  switch (resultKind(record)) {
    case "running":
      return { text: "进行中", tone: "running" };
    case "decision":
      return { text: "等你处理", tone: "attention" };
    case "approval":
      return { text: "等你确认", tone: "attention" };
    case "commitFailed":
      return { text: "没能提交", tone: "attention" };
    case "success":
      if (record.outcome === "kept") return { text: "保留提交", tone: "attention" };
      return record.decision === "approved" ? { text: "已确认提交", tone: "ok" } : { text: "已提交", tone: "ok" };
    case "discarded":
      return { text: "已丢弃", tone: "quiet" };
    case "nothing":
      return { text: "队列是空的", tone: "quiet" };
    case "refused":
      return { text: "没有开始", tone: "quiet" };
    case "cancelled":
      return { text: "已停止", tone: "quiet" };
    case "failed":
      return { text: "出错了", tone: "failed" };
  }
}

/** `42 秒`, `3 分 12 秒`, `1 小时 5 分`. */
export function formatDuration(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    const rest = seconds % 60;
    return rest === 0 ? `${minutes} 分` : `${minutes} 分 ${rest} 秒`;
  }
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} 小时` : `${hours} 小时 ${rest} 分`;
}

/** A stopwatch: `03:12`, `1:02:03`. */
export function formatClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return hours > 0 ? `${hours}:${two(minutes)}:${two(seconds)}` : `${two(minutes)}:${two(seconds)}`;
}

/** `$0.42`; tiny amounts read `不到 $0.01`. */
export function formatCost(usd: number): string {
  if (usd > 0 && usd < 0.005) return "不到 $0.01";
  return `$${usd.toFixed(2)}`;
}

/** `$10` for a budget (no cents when there are none). */
export function formatBudget(usd: number): string {
  return Number.isInteger(usd) ? `$${usd}` : `$${usd.toFixed(2)}`;
}

/** `12.3k`, `980`, `1.2M` tokens. */
export function formatTokenCount(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 10_000) return `${Math.round(count / 1000)}k`;
  if (count >= 1000) return `${(count / 1000).toFixed(1)}k`;
  return String(count);
}

/** What a run cost, or how many tokens it used; null when the engine did not say. */
export function usageText(record: RunRecord): string | null {
  const stream = record.engineReport?.stream;
  if (!stream) return null;
  if (stream.costUsd !== null) return formatCost(stream.costUsd);
  if (stream.inputTokens !== null || stream.outputTokens !== null) {
    const parts: string[] = [];
    if (stream.inputTokens !== null) parts.push(`输入 ${formatTokenCount(stream.inputTokens)}`);
    if (stream.outputTokens !== null) parts.push(`输出 ${formatTokenCount(stream.outputTokens)}`);
    return `${parts.join(" · ")} tokens`;
  }
  return null;
}

/** The same in a few characters, for the history table: `$0.42` or `194k tokens`. */
export function usageShort(record: RunRecord): string | null {
  const stream = record.engineReport?.stream;
  if (!stream) return null;
  if (stream.costUsd !== null) return formatCost(stream.costUsd);
  if (stream.inputTokens === null && stream.outputTokens === null) return null;
  return `${formatTokenCount((stream.inputTokens ?? 0) + (stream.outputTokens ?? 0))} tokens`;
}

/** How long a run took: from its start to its end (or to now while it goes on). */
export function runDurationMs(record: RunRecord, now: number = Date.now()): number {
  return Math.max(0, (record.endedAt ?? now) - record.startedAt);
}

/** `10 月 12 日 14:30`; today only `14:30`. */
export function formatRunTime(at: number, now: number = Date.now()): string {
  const then = new Date(at);
  const today = new Date(now);
  const time = `${String(then.getHours()).padStart(2, "0")}:${String(then.getMinutes()).padStart(2, "0")}`;
  const sameDay =
    then.getFullYear() === today.getFullYear() &&
    then.getMonth() === today.getMonth() &&
    then.getDate() === today.getDate();
  return sameDay ? `今天 ${time}` : `${then.getMonth() + 1} 月 ${then.getDate()} 日 ${time}`;
}

/** How a step looks in the live view. */
export type StepTone =
  | "read"
  | "write"
  | "edit"
  | "find"
  | "delete"
  | "run"
  | "tool"
  | "denied"
  | "text"
  | "note"
  | "init"
  | "result"
  | "error";

const VERB_TONES: Record<string, StepTone> = {
  读: "read",
  写: "write",
  改: "edit",
  查找: "find",
  删: "delete",
  运行: "run",
};

export function stepTone(step: RunStep): StepTone {
  switch (step.kind) {
    case "tool":
      return VERB_TONES[step.verb ?? ""] ?? "tool";
    case "denied":
      return "denied";
    case "text":
      return "text";
    case "note":
      return "note";
    case "init":
      return "init";
    case "result":
      return "result";
    case "error":
      return "error";
  }
}

/** The numbers of the engine's final JSON line, in the order AGENTS.md lists them. */
export function finalFigures(final: Record<string, unknown> | null, task: RunTask): { label: string; value: number }[] {
  if (!final) return [];
  const keys =
    task === "ingest"
      ? [
          ["条数", "条随手记"],
          ["自动", "自动整理"],
          ["判例裁决", "按判例裁决"],
          ["待裁决", "新的待裁决"],
        ]
      : [["条数", "份回答"]];
  return keys.flatMap(([key, label]) => {
    const value = Number(final[key ?? ""]);
    return Number.isFinite(value) && final[key ?? ""] !== undefined ? [{ label: label ?? "", value }] : [];
  });
}

/** The report the final JSON line names (`reports/…`), or null. */
export function finalReportPath(final: Record<string, unknown> | null): string | null {
  const named = final?.["报告"];
  if (typeof named !== "string" || named.trim() === "") return null;
  const path = named.trim();
  return path.startsWith("reports/") ? path : `reports/${path}`;
}

/** The note of the final JSON line, or null. */
export function finalNote(final: Record<string, unknown> | null): string | null {
  const note = final?.["备注"];
  return typeof note === "string" && note.trim() !== "" ? note.trim() : null;
}
