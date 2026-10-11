import { describe, expect, it } from "vitest";
import casesJson from "./__fixtures__/capture-cases.json?raw";
import {
  buildCapture,
  buildCaptureAt,
  captureAttemptPath,
  CAPTURE_SOURCES,
  isBlankCapture,
  localTime,
  MAX_CAPTURE_ATTEMPTS,
  saveCapture,
  type CaptureSource,
  type LocalTime,
} from "./capture";

interface Cases {
  accept: {
    name: string;
    text: string;
    source: CaptureSource;
    at: LocalTime;
    path: string;
    content: string;
    message: string;
  }[];
  reject: string[];
  suffixes: { path: string; attempt: number; expected: string }[];
}

// The same cases test the Rust side (src-tauri/src/kb/capture.rs), so a
// capture looks the same whether it came from a window or the command line.
const cases = JSON.parse(casesJson) as Cases;

/** A store that only knows `create`, with optional files that already exist. */
function target(existing: string[] = [], failWith?: Error) {
  const files = new Map<string, { content: string; message: string }>(
    existing.map((path) => [path, { content: "已有\n", message: "旧" }]),
  );
  const calls: string[] = [];
  return {
    files,
    calls,
    async create(path: string, content: string, message: string) {
      calls.push(path);
      if (failWith) throw failWith;
      if (files.has(path)) throw Object.assign(new Error(`文件已存在：${path}`), { code: "exists" });
      files.set(path, { content, message });
    },
  };
}

describe("buildCaptureAt", () => {
  it.each(cases.accept.map((c) => [c.name, c] as const))("%s", (_name, c) => {
    expect(buildCaptureAt({ text: c.text, source: c.source, at: c.at })).toEqual({
      path: c.path,
      content: c.content,
      message: c.message,
    });
  });

  it("refuses empty and whitespace-only text", () => {
    expect(cases.reject.length).toBeGreaterThanOrEqual(8);
    const at = cases.accept[0]!.at;
    for (const text of cases.reject) {
      expect(() => buildCaptureAt({ text, source: "mac", at }), JSON.stringify(text)).toThrow(/空/);
      expect(isBlankCapture(text)).toBe(true);
    }
    expect(isBlankCapture(" 有字 ")).toBe(false);
  });

  it("only knows the three sources", () => {
    expect(CAPTURE_SOURCES).toEqual(["mac", "iphone", "web"]);
    const at = cases.accept[0]!.at;
    expect(() => buildCaptureAt({ text: "x", source: "fax" as CaptureSource, at })).toThrow(/来源/);
  });
});

describe("buildCapture", () => {
  it("uses the local time of `now`, with its offset", () => {
    const now = new Date(2026, 9, 12, 14, 3, 22, 987);
    const at = localTime(now);
    expect(at).toEqual({
      year: 2026,
      month: 10,
      day: 12,
      hour: 14,
      minute: 3,
      second: 22,
      offsetMinutes: -now.getTimezoneOffset(),
    });
    const capture = buildCapture({ text: "清单越短，越容易做完。", source: "web", now });
    expect(capture.path).toBe("inbox/2026-10-12T14-03-22-web.md");
    expect(capture).toEqual(buildCaptureAt({ text: "清单越短，越容易做完。", source: "web", at }));
    expect(capture.content).toMatch(/^---\n来源: web\n时间: 2026-10-12T14:03:22[+-]\d\d:\d\d\n---\n\n/);
  });
});

describe("captureAttemptPath", () => {
  it.each(cases.suffixes.map((c) => [c.path, c.attempt, c.expected] as const))("%s, attempt %i", (path, attempt, expected) => {
    expect(captureAttemptPath(path, attempt)).toBe(expected);
  });
});

describe("saveCapture", () => {
  const capture = buildCaptureAt({ text: "一条", source: "mac", at: cases.accept[0]!.at });

  it("creates the file under its own name", async () => {
    const store = target();
    expect(await saveCapture(store, capture)).toEqual({ path: capture.path, attempts: 1 });
    expect(store.files.get(capture.path)).toEqual({ content: capture.content, message: capture.message });
  });

  it("tries -2, -3, … when the name is taken, and never overwrites", async () => {
    const taken = [capture.path, captureAttemptPath(capture.path, 2)];
    const store = target(taken);
    const saved = await saveCapture(store, capture);
    expect(saved).toEqual({ path: "inbox/2026-10-12T14-03-22-mac-3.md", attempts: 3 });
    expect(store.calls).toEqual([...taken, saved.path]);
    for (const path of taken) expect(store.files.get(path)?.content).toBe("已有\n");
  });

  it("gives up after the last name", async () => {
    const all = Array.from({ length: MAX_CAPTURE_ATTEMPTS }, (_, i) => captureAttemptPath(capture.path, i + 1));
    const store = target(all);
    await expect(saveCapture(store, capture)).rejects.toThrow(/没有记下/);
    expect(store.calls).toHaveLength(MAX_CAPTURE_ATTEMPTS);
  });

  it("does not retry other failures", async () => {
    const store = target([], new Error("知识库正忙：正在同步（已 3 秒），请稍后再试"));
    await expect(saveCapture(store, capture)).rejects.toThrow(/知识库正忙/);
    expect(store.calls).toEqual([capture.path]);
  });
});
